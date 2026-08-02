use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Component, Path};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fs4::{FileExt, TryLockError};
use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::{OFlag, openat};
use nix::sys::stat::{Mode, SFlag, fchmod, fstat, mkdirat};
use nix::unistd::geteuid;

use super::{AppDataResetCoordinatorError, AppDataResetCoordinatorErrorKind, Result, error};

pub(super) const COORDINATOR_DIRECTORY_NAME: &str = ".dux-app-data-reset-v1";
const MARKER_NAME: &str = ".dux-reset-store";
const LOCK_NAME: &str = ".dux-reset.writer.lock";
const JOURNAL_NAME: &str = "reset-journal-v1.json";
const JOURNAL_STAGE_NAME: &str = ".reset-journal-v1.stage";
const PROVISIONING_STAGE_PREFIX: &str = ".dux-app-data-reset-stage-";
const PROVISIONING_STAGE_HEX_LENGTH: usize = 32;
const RANDOM_ATTEMPTS: usize = 16;
const STORE_MARKER: &[u8; 16] = b"DUXRESETSTOREV1\0";
const LOCK_MARKER: &[u8; 16] = b"DUXRESETLOCKV1\0\0";
const DIRECTORY_MODE: Mode = Mode::S_IRWXU;
const FILE_MODE: Mode = Mode::S_IRUSR.union(Mode::S_IWUSR);
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_RETRY: Duration = Duration::from_millis(5);
const MAX_JOURNAL_BYTES: u64 = 4_096;
const MAX_INVENTORY_ENTRIES: usize = 4;
const MAX_INVENTORY_NAME_BYTES: usize = 256;
const MAX_PARENT_INVENTORY_ENTRIES: usize = 4_096;
const MAX_PARENT_INVENTORY_NAME_BYTES: usize = 1024 * 1024;
const PARENT_INVENTORY_TIMEOUT: Duration = Duration::from_millis(250);

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestJournalWriteFault {
    None,
    BeforeRename,
    AfterRename,
    AfterDirectorySync,
    DuringReadback,
}

#[cfg(test)]
std::thread_local! {
    static TEST_JOURNAL_WRITE_FAULT: std::cell::Cell<TestJournalWriteFault> =
        const { std::cell::Cell::new(TestJournalWriteFault::None) };
}

#[cfg(test)]
pub(crate) fn set_test_journal_write_fault(fault: TestJournalWriteFault) {
    TEST_JOURNAL_WRITE_FAULT.with(|current| current.set(fault));
}

#[cfg(test)]
fn take_test_journal_write_fault(expected: TestJournalWriteFault) -> bool {
    TEST_JOURNAL_WRITE_FAULT.with(|current| {
        if current.get() == expected {
            current.set(TestJournalWriteFault::None);
            true
        } else {
            false
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Publication {
    Published,
    Collision,
}

pub(super) struct ResetCoordinatorStorage {
    parent: File,
    parent_identity: Identity,
    data_root_name: OsString,
    directory: File,
    directory_identity: Identity,
    marker: File,
    marker_identity: Identity,
    lock: File,
    lock_identity: Identity,
    lock_in_use: Arc<AtomicBool>,
}

struct CoordinatorLock {
    file: File,
    in_use: Arc<AtomicBool>,
}

pub(super) struct ResetCoordinatorEngineLease {
    file: File,
}

impl Drop for CoordinatorLock {
    fn drop(&mut self) {
        // A failed unlock leaves this storage instance permanently busy.
        // Closing the descriptor remains the final release mechanism, which
        // is safer than admitting a nested same-process reset session after
        // an uncertain unlock.
        if FileExt::unlock(&self.file).is_ok() {
            self.in_use.store(false, Ordering::Release);
        }
    }
}

impl Drop for ResetCoordinatorEngineLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

impl ResetCoordinatorStorage {
    pub(super) fn open_existing_for_engine(data_root: &Path) -> Result<Option<Self>> {
        validate_data_root(data_root)?;
        let data_root_name = data_root
            .file_name()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidConfiguration))?
            .to_os_string();
        let parent_path = data_root
            .parent()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidConfiguration))?;
        let parent = open_absolute_directory(parent_path)?;
        let parent_identity = identity(&parent, ObjectKind::ParentDirectory)?;
        validate_retained(&parent, parent_identity, ObjectKind::ParentDirectory, false)?;
        let Some(directory) = open_existing_private_directory(&parent, COORDINATOR_DIRECTORY_NAME)?
        else {
            return Ok(None);
        };
        Self::from_directory_without_reconciliation(
            parent,
            parent_identity,
            data_root_name,
            directory,
        )
        .map(Some)
    }

    pub(super) fn open_or_create(data_root: &Path) -> Result<Self> {
        let deadline = Instant::now()
            .checked_add(LOCK_TIMEOUT)
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InternalState))?;
        Self::open_or_create_with_hook_until(data_root, deadline, || {})
    }

    pub(super) fn open_or_create_until(data_root: &Path, deadline: Instant) -> Result<Self> {
        Self::open_or_create_with_hook_until(data_root, deadline, || {})
    }

    fn open_or_create_with_hook(data_root: &Path, before_publish: impl FnOnce()) -> Result<Self> {
        let deadline = Instant::now()
            .checked_add(LOCK_TIMEOUT)
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InternalState))?;
        Self::open_or_create_with_hook_until(data_root, deadline, before_publish)
    }

    fn open_or_create_with_hook_until(
        data_root: &Path,
        deadline: Instant,
        before_publish: impl FnOnce(),
    ) -> Result<Self> {
        Self::open_or_create_with_hooks_until(data_root, deadline, || {}, before_publish)
    }

    #[cfg(test)]
    fn open_or_create_with_stage_lock_hook_until(
        data_root: &Path,
        deadline: Instant,
        before_stage_lock: impl FnOnce(),
    ) -> Result<Self> {
        Self::open_or_create_with_hooks_until(data_root, deadline, before_stage_lock, || {})
    }

    fn open_or_create_with_hooks_until(
        data_root: &Path,
        deadline: Instant,
        before_stage_lock: impl FnOnce(),
        before_publish: impl FnOnce(),
    ) -> Result<Self> {
        validate_data_root(data_root)?;
        let data_root_name = data_root
            .file_name()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidConfiguration))?
            .to_os_string();
        let parent_path = data_root
            .parent()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidConfiguration))?;
        let parent = open_absolute_directory(parent_path)?;
        let parent_identity = identity(&parent, ObjectKind::ParentDirectory)?;
        validate_retained(&parent, parent_identity, ObjectKind::ParentDirectory, false)?;

        if let Some(directory) =
            open_existing_private_directory(&parent, COORDINATOR_DIRECTORY_NAME)?
        {
            return Self::from_directory(
                parent,
                parent_identity,
                data_root_name,
                directory,
                deadline,
            );
        }
        Self::provision(
            parent,
            parent_identity,
            data_root_name,
            deadline,
            before_stage_lock,
            before_publish,
        )
    }

    fn provision(
        parent: File,
        parent_identity: Identity,
        data_root_name: OsString,
        deadline: Instant,
        before_stage_lock: impl FnOnce(),
        before_publish: impl FnOnce(),
    ) -> Result<Self> {
        let mut before_stage_lock = Some(before_stage_lock);
        let mut before_publish = Some(before_publish);
        for _ in 0..RANDOM_ATTEMPTS {
            let stage_name = random_stage_name()?;
            match mkdirat(&parent, stage_name.as_str(), DIRECTORY_MODE) {
                Ok(()) => {}
                Err(Errno::EEXIST) => continue,
                Err(error) => return Err(map_coordinator_open_error(error)),
            }
            let directory = open_private_directory(&parent, &stage_name)?;
            let directory_identity = identity(&directory, ObjectKind::PrivateDirectory)?;
            fchmod(&directory, DIRECTORY_MODE).map_err(|_| unavailable())?;
            validate_retained(
                &directory,
                directory_identity,
                ObjectKind::PrivateDirectory,
                false,
            )?;
            validate_named(
                &parent,
                &stage_name,
                &directory,
                directory_identity,
                ObjectKind::PrivateDirectory,
            )?;
            let lock = create_control(&directory, LOCK_NAME, LOCK_MARKER)?;
            let marker = create_control(&directory, MARKER_NAME, STORE_MARKER)?;
            directory.sync_all().map_err(|_| unavailable())?;
            if let Some(hook) = before_stage_lock.take() {
                hook();
            }
            lock_file_until(&lock.0, deadline)?;
            validate_retained(&parent, parent_identity, ObjectKind::ParentDirectory, false)?;
            validate_named(
                &parent,
                &stage_name,
                &directory,
                directory_identity,
                ObjectKind::PrivateDirectory,
            )?;
            if let Some(hook) = before_publish.take() {
                hook();
            }
            match publish_directory_no_replace(&parent, &stage_name, COORDINATOR_DIRECTORY_NAME)? {
                Publication::Published => {
                    parent.sync_all().map_err(|_| {
                        error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
                    })?;
                    validate_named(
                        &parent,
                        COORDINATOR_DIRECTORY_NAME,
                        &directory,
                        directory_identity,
                        ObjectKind::PrivateDirectory,
                    )
                    .map_err(|_| error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
                    let storage = Self {
                        parent,
                        parent_identity,
                        data_root_name,
                        directory,
                        directory_identity,
                        marker: marker.0,
                        marker_identity: marker.1,
                        lock: lock.0,
                        lock_identity: lock.1,
                        lock_in_use: Arc::new(AtomicBool::new(false)),
                    };
                    FileExt::unlock(&storage.lock).map_err(|_| {
                        error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
                    })?;
                    storage.validate()?;
                    let _ = storage.with_lock_until(deadline, |storage| {
                        storage.reconcile_provisioning_stages()
                    })?;
                    return Ok(storage);
                }
                Publication::Collision => {
                    let removal = remove_provisioning_stage(
                        &parent,
                        &stage_name,
                        &directory,
                        directory_identity,
                        &marker,
                        &lock,
                    );
                    FileExt::unlock(&lock.0).map_err(|_| {
                        error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
                    })?;
                    removal?;
                    let directory = open_private_directory(&parent, COORDINATOR_DIRECTORY_NAME)?;
                    return Self::from_directory(
                        parent,
                        parent_identity,
                        data_root_name,
                        directory,
                        deadline,
                    );
                }
            }
        }
        Err(unavailable())
    }

    fn from_directory(
        parent: File,
        parent_identity: Identity,
        data_root_name: OsString,
        directory: File,
        deadline: Instant,
    ) -> Result<Self> {
        let storage = Self::from_directory_without_reconciliation(
            parent,
            parent_identity,
            data_root_name,
            directory,
        )?;
        let _ =
            storage.with_lock_until(deadline, |storage| storage.reconcile_provisioning_stages())?;
        Ok(storage)
    }

    fn from_directory_without_reconciliation(
        parent: File,
        parent_identity: Identity,
        data_root_name: OsString,
        directory: File,
    ) -> Result<Self> {
        let directory_identity = identity(&directory, ObjectKind::PrivateDirectory)?;
        validate_retained(
            &directory,
            directory_identity,
            ObjectKind::PrivateDirectory,
            false,
        )?;
        validate_named(
            &parent,
            COORDINATOR_DIRECTORY_NAME,
            &directory,
            directory_identity,
            ObjectKind::PrivateDirectory,
        )?;

        let marker =
            open_control(&directory, MARKER_NAME, STORE_MARKER)?.ok_or_else(unsafe_coordinator)?;
        let lock =
            open_control(&directory, LOCK_NAME, LOCK_MARKER)?.ok_or_else(unsafe_coordinator)?;

        let storage = Self {
            parent,
            parent_identity,
            data_root_name,
            directory,
            directory_identity,
            marker: marker.0,
            marker_identity: marker.1,
            lock: lock.0,
            lock_identity: lock.1,
            lock_in_use: Arc::new(AtomicBool::new(false)),
        };
        storage.validate()?;
        Ok(storage)
    }

    pub(super) fn with_lock<T>(&self, operation: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.with_lock_timeout(LOCK_TIMEOUT, operation)
    }

    pub(super) fn data_root_binding(&self) -> ((u64, u64), &OsStr) {
        (
            (self.parent_identity.device, self.parent_identity.inode),
            self.data_root_name.as_os_str(),
        )
    }

    pub(super) fn with_lock_timeout<T>(
        &self,
        timeout: Duration,
        operation: impl FnOnce(&Self) -> Result<T>,
    ) -> Result<T> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InternalState))?;
        self.with_lock_until_mode(deadline, true, operation)
    }

    pub(super) fn with_lock_until<T>(
        &self,
        deadline: Instant,
        operation: impl FnOnce(&Self) -> Result<T>,
    ) -> Result<T> {
        self.with_lock_until_mode(deadline, false, operation)
    }

    fn with_lock_until_mode<T>(
        &self,
        deadline: Instant,
        allow_expired_initial_try: bool,
        operation: impl FnOnce(&Self) -> Result<T>,
    ) -> Result<T> {
        let _lock = self.acquire_lock_until(deadline, allow_expired_initial_try)?;
        self.validate()?;
        let result = operation(self);
        if result.is_ok() {
            self.validate()?;
        }
        result
    }

    pub(super) fn acquire_engine_lease(
        &self,
    ) -> Result<(ResetCoordinatorEngineLease, Option<Vec<u8>>)> {
        self.validate()?;
        let file = self.lock.try_clone().map_err(|_| unavailable())?;
        match FileExt::try_lock_shared(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
            }
            Err(TryLockError::Error(_)) => return Err(unavailable()),
        }
        let validation =
            validate_retained(&file, self.lock_identity, ObjectKind::PrivateFile, true)
                .and_then(|()| {
                    validate_named(
                        &self.directory,
                        LOCK_NAME,
                        &file,
                        self.lock_identity,
                        ObjectKind::PrivateFile,
                    )
                })
                .and_then(|()| self.read_journal_without_reconciliation())
                .and_then(|journal| {
                    self.validate()?;
                    Ok(journal)
                });
        match validation {
            Ok(journal) => Ok((ResetCoordinatorEngineLease { file }, journal)),
            Err(error) => {
                let _ = FileExt::unlock(&file);
                Err(error)
            }
        }
    }

    pub(super) fn read_journal(&self) -> Result<Option<Vec<u8>>> {
        self.reconcile_stage()?;
        self.read_journal_without_reconciliation()
    }

    /// Read the exact durable journal without reconciling a publication stage.
    /// Final reset-effect admission and post-effect certainty checks must not
    /// introduce another filesystem mutation or extend their absolute budget.
    pub(super) fn read_journal_exact_until(&self, deadline: Instant) -> Result<Option<Vec<u8>>> {
        if Instant::now() >= deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        if let Some((stage, stage_identity)) =
            open_private_file(&self.directory, JOURNAL_STAGE_NAME, false)?
        {
            validate_named(
                &self.directory,
                JOURNAL_STAGE_NAME,
                &stage,
                stage_identity,
                ObjectKind::PrivateFile,
            )?;
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let journal = self.read_journal_without_reconciliation()?;
        if Instant::now() >= deadline {
            Err(error(AppDataResetCoordinatorErrorKind::Busy))
        } else {
            Ok(journal)
        }
    }

    /// Bounded, read-only journal observation while an ordinary engine's
    /// shared lease is retained. A safe publication-stage remnant is not
    /// reconciled and does not invalidate an otherwise exact final journal.
    pub(super) fn read_journal_for_engine_validation_until(
        &self,
        deadline: Instant,
    ) -> Result<Option<Vec<u8>>> {
        if Instant::now() >= deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        self.validate()?;
        let journal = self.read_journal_without_reconciliation()?;
        self.validate()?;
        if Instant::now() < deadline {
            Ok(journal)
        } else {
            Err(error(AppDataResetCoordinatorErrorKind::Busy))
        }
    }

    fn read_journal_without_reconciliation(&self) -> Result<Option<Vec<u8>>> {
        let Some((mut journal, journal_identity)) =
            open_private_file(&self.directory, JOURNAL_NAME, false)?
        else {
            return Ok(None);
        };
        validate_named(
            &self.directory,
            JOURNAL_NAME,
            &journal,
            journal_identity,
            ObjectKind::PrivateFile,
        )?;
        let length = journal.metadata().map_err(|_| unavailable())?.len();
        if length == 0 || length > MAX_JOURNAL_BYTES {
            return Err(error(AppDataResetCoordinatorErrorKind::CorruptJournal));
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(length)
                .map_err(|_| error(AppDataResetCoordinatorErrorKind::CorruptJournal))?,
        );
        journal.read_to_end(&mut bytes).map_err(|_| unavailable())?;
        if u64::try_from(bytes.len()).ok() != Some(length) {
            return Err(error(AppDataResetCoordinatorErrorKind::CorruptJournal));
        }
        validate_named(
            &self.directory,
            JOURNAL_NAME,
            &journal,
            journal_identity,
            ObjectKind::PrivateFile,
        )?;
        Ok(Some(bytes))
    }

    pub(super) fn write_journal(&self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() || bytes.len() > usize::try_from(MAX_JOURNAL_BYTES).unwrap() {
            return Err(error(AppDataResetCoordinatorErrorKind::CorruptJournal));
        }
        self.reconcile_stage()?;
        if let Some((existing, existing_identity)) =
            open_private_file(&self.directory, JOURNAL_NAME, false)?
        {
            validate_named(
                &self.directory,
                JOURNAL_NAME,
                &existing,
                existing_identity,
                ObjectKind::PrivateFile,
            )?;
        }

        let (mut stage, stage_identity) = create_private_file(&self.directory, JOURNAL_STAGE_NAME)?;
        if stage
            .write_all(bytes)
            .and_then(|()| stage.sync_all())
            .is_err()
        {
            return Err(unavailable());
        }
        validate_named(
            &self.directory,
            JOURNAL_STAGE_NAME,
            &stage,
            stage_identity,
            ObjectKind::PrivateFile,
        )?;
        #[cfg(test)]
        if take_test_journal_write_fault(TestJournalWriteFault::BeforeRename) {
            return Err(unavailable());
        }
        // Once rename is attempted, the destination may have changed even if
        // the syscall reports failure. Every later error is therefore
        // outcome-unknown; only failures above this line are proven
        // pre-publication refusals.
        rename_stage_over_journal(&self.directory)?;
        #[cfg(test)]
        if take_test_journal_write_fault(TestJournalWriteFault::AfterRename) {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        if self.directory.sync_all().is_err() {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        #[cfg(test)]
        if take_test_journal_write_fault(TestJournalWriteFault::AfterDirectorySync) {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        let Some(current) = self
            .read_journal_after_publish()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
        else {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        };
        if current != bytes {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(())
    }

    fn read_journal_after_publish(&self) -> Result<Option<Vec<u8>>> {
        #[cfg(test)]
        if take_test_journal_write_fault(TestJournalWriteFault::DuringReadback) {
            return Err(unavailable());
        }
        self.read_journal()
    }

    fn acquire_lock_until(
        &self,
        deadline: Instant,
        allow_expired_initial_try: bool,
    ) -> Result<CoordinatorLock> {
        if !allow_expired_initial_try && Instant::now() >= deadline {
            return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
        }
        if self
            .lock_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
        }
        if let Err(error) = self.validate() {
            self.lock_in_use.store(false, Ordering::Release);
            return Err(error);
        }
        let file = match self.lock.try_clone() {
            Ok(file) => file,
            Err(_) => {
                self.lock_in_use.store(false, Ordering::Release);
                return Err(unavailable());
            }
        };
        let mut first_attempt = true;
        loop {
            if Instant::now() >= deadline && !(allow_expired_initial_try && first_attempt) {
                self.lock_in_use.store(false, Ordering::Release);
                return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
            }
            first_attempt = false;
            match FileExt::try_lock(&file) {
                Ok(()) => {
                    let validation =
                        validate_retained(&file, self.lock_identity, ObjectKind::PrivateFile, true)
                            .and_then(|()| {
                                validate_named(
                                    &self.directory,
                                    LOCK_NAME,
                                    &file,
                                    self.lock_identity,
                                    ObjectKind::PrivateFile,
                                )
                            });
                    if let Err(error) = validation {
                        if FileExt::unlock(&file).is_ok() {
                            self.lock_in_use.store(false, Ordering::Release);
                        }
                        return Err(error);
                    }
                    if !allow_expired_initial_try && Instant::now() >= deadline {
                        if FileExt::unlock(&file).is_ok() {
                            self.lock_in_use.store(false, Ordering::Release);
                        }
                        return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
                    }
                    return Ok(CoordinatorLock {
                        file,
                        in_use: Arc::clone(&self.lock_in_use),
                    });
                }
                Err(TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        self.lock_in_use.store(false, Ordering::Release);
                        return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
                    }
                    std::thread::sleep(
                        LOCK_RETRY.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(TryLockError::Error(_)) => {
                    self.lock_in_use.store(false, Ordering::Release);
                    return Err(unavailable());
                }
            }
        }
    }

    fn reconcile_stage(&self) -> Result<()> {
        let Some((stage, stage_identity)) =
            open_private_file(&self.directory, JOURNAL_STAGE_NAME, false)?
        else {
            return Ok(());
        };
        validate_named(
            &self.directory,
            JOURNAL_STAGE_NAME,
            &stage,
            stage_identity,
            ObjectKind::PrivateFile,
        )?;
        // A stage that was never atomically renamed did not commit. Discarding
        // this exact fixed private temporary preserves the prior singleton.
        remove_named_file(&self.directory, JOURNAL_STAGE_NAME, &stage, stage_identity)?;
        self.directory.sync_all().map_err(|_| unavailable())
    }

    fn validate(&self) -> Result<()> {
        if self.data_root_name.is_empty()
            || self.data_root_name == OsStr::new(".")
            || self.data_root_name == OsStr::new("..")
        {
            return Err(error(
                AppDataResetCoordinatorErrorKind::InvalidConfiguration,
            ));
        }
        validate_retained(
            &self.parent,
            self.parent_identity,
            ObjectKind::ParentDirectory,
            false,
        )?;
        validate_retained(
            &self.directory,
            self.directory_identity,
            ObjectKind::PrivateDirectory,
            false,
        )?;
        validate_named(
            &self.parent,
            COORDINATOR_DIRECTORY_NAME,
            &self.directory,
            self.directory_identity,
            ObjectKind::PrivateDirectory,
        )?;
        validate_control(
            &self.directory,
            MARKER_NAME,
            &self.marker,
            self.marker_identity,
            STORE_MARKER,
        )?;
        validate_control(
            &self.directory,
            LOCK_NAME,
            &self.lock,
            self.lock_identity,
            LOCK_MARKER,
        )?;
        let mut names = inventory(&self.directory)?;
        names.sort_unstable();
        for name in &names {
            if !matches!(
                name.as_str(),
                MARKER_NAME | LOCK_NAME | JOURNAL_NAME | JOURNAL_STAGE_NAME
            ) {
                return Err(unsafe_object());
            }
        }
        if names.iter().any(|name| name == JOURNAL_STAGE_NAME) {
            let (stage, stage_identity) =
                open_private_file(&self.directory, JOURNAL_STAGE_NAME, false)?
                    .ok_or_else(unsafe_object)?;
            validate_named(
                &self.directory,
                JOURNAL_STAGE_NAME,
                &stage,
                stage_identity,
                ObjectKind::PrivateFile,
            )?;
        }
        Ok(())
    }

    pub(super) fn reconcile_provisioning_stages(&self) -> Result<u32> {
        let mut names = inventory_bounded(
            &self.parent,
            MAX_PARENT_INVENTORY_ENTRIES,
            MAX_PARENT_INVENTORY_NAME_BYTES,
            Instant::now() + PARENT_INVENTORY_TIMEOUT,
        )?;
        names.retain(|name| is_provisioning_stage_name(name));
        names.sort_unstable();
        let mut unproven = 0_u32;
        for name in names {
            let Some(stage) = inspect_provisioning_stage(&self.parent, &name)? else {
                unproven = unproven.checked_add(1).ok_or_else(unsafe_coordinator)?;
                continue;
            };
            match FileExt::try_lock(&stage.lock.0) {
                Ok(()) => {
                    let removal = remove_provisioning_stage(
                        &self.parent,
                        &name,
                        &stage.directory,
                        stage.identity,
                        &stage.marker,
                        &stage.lock,
                    );
                    FileExt::unlock(&stage.lock.0).map_err(|_| {
                        error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
                    })?;
                    removal?;
                }
                Err(TryLockError::WouldBlock) => {
                    unproven = unproven.checked_add(1).ok_or_else(unsafe_coordinator)?;
                }
                Err(TryLockError::Error(_)) => return Err(unavailable()),
            }
        }
        Ok(unproven)
    }
}

fn lock_file_until(file: &File, deadline: Instant) -> Result<()> {
    loop {
        if Instant::now() >= deadline {
            return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
        }
        match FileExt::try_lock(file) {
            Ok(()) if Instant::now() >= deadline => {
                let _ = FileExt::unlock(file);
                return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
            }
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(error_kind(AppDataResetCoordinatorErrorKind::Busy));
                }
                std::thread::sleep(LOCK_RETRY.min(deadline.duration_since(now)));
            }
            Err(TryLockError::Error(_)) => return Err(unavailable()),
        }
    }
}

struct ProvisioningStage {
    directory: File,
    identity: Identity,
    marker: (File, Identity),
    lock: (File, Identity),
}

#[derive(Clone, Copy)]
enum ObjectKind {
    ParentDirectory,
    PrivateDirectory,
    PrivateFile,
}

fn validate_data_root(data_root: &Path) -> Result<()> {
    if !data_root.is_absolute()
        || data_root.file_name().is_none()
        || data_root
            .parent()
            .is_none_or(|parent| parent.parent().is_none())
        || data_root.components().any(|component| {
            matches!(
                component,
                Component::CurDir | Component::ParentDir | Component::Prefix(_)
            )
        })
    {
        return Err(error_kind(
            AppDataResetCoordinatorErrorKind::InvalidConfiguration,
        ));
    }
    Ok(())
}

fn open_absolute_directory(path: &Path) -> Result<File> {
    if !path.is_absolute() {
        return Err(error_kind(
            AppDataResetCoordinatorErrorKind::InvalidConfiguration,
        ));
    }
    let root = nix::fcntl::open(
        Path::new("/"),
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| unsafe_parent())?;
    let mut current = root;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = openat(
                    &current,
                    name,
                    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
                    Mode::empty(),
                )
                .map(File::from)
                .map_err(|error| match error {
                    Errno::ELOOP | Errno::ENOTDIR => unsafe_parent(),
                    _ => unavailable(),
                })?;
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(error_kind(
                    AppDataResetCoordinatorErrorKind::InvalidConfiguration,
                ));
            }
        }
    }
    Ok(current)
}

fn open_private_directory(parent: &File, name: &str) -> Result<File> {
    openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(map_coordinator_open_error)
}

fn open_existing_private_directory(parent: &File, name: &str) -> Result<Option<File>> {
    match openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(descriptor) => {
            let directory = File::from(descriptor);
            let object_identity = identity(&directory, ObjectKind::PrivateDirectory)?;
            validate_retained(
                &directory,
                object_identity,
                ObjectKind::PrivateDirectory,
                false,
            )?;
            Ok(Some(directory))
        }
        Err(Errno::ENOENT) => Ok(None),
        Err(error) => Err(map_coordinator_open_error(error)),
    }
}

fn random_stage_name() -> Result<String> {
    let mut random = [0_u8; PROVISIONING_STAGE_HEX_LENGTH / 2];
    getrandom::fill(&mut random).map_err(|_| unavailable())?;
    let mut name = String::with_capacity(PROVISIONING_STAGE_PREFIX.len() + 32);
    name.push_str(PROVISIONING_STAGE_PREFIX);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut name, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(name)
}

fn publish_directory_no_replace(
    parent: &File,
    source: &str,
    destination: &str,
) -> Result<Publication> {
    match rename_no_replace(parent, source, destination) {
        Ok(()) => Ok(Publication::Published),
        Err(Errno::EEXIST) => Ok(Publication::Collision),
        Err(Errno::ENOENT) => Err(error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown)),
        Err(_) => Err(unavailable()),
    }
}

#[cfg(target_os = "linux")]
fn rename_no_replace(
    parent: &File,
    source: &str,
    destination: &str,
) -> std::result::Result<(), Errno> {
    let source = std::ffi::CString::new(source).map_err(|_| Errno::EINVAL)?;
    let destination = std::ffi::CString::new(destination).map_err(|_| Errno::EINVAL)?;
    // SAFETY: both names are generated/fixed single components beneath the
    // retained private parent. RENAME_NOREPLACE forbids replacement.
    let result = unsafe {
        nix::libc::syscall(
            // DUX-DESTRUCTIVE: allow=app-data-reset-linux-coordinator-publish -- atomically publish only one retained marker-complete reset coordinator without replacing an existing entry
            nix::libc::SYS_renameat2,
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
            nix::libc::RENAME_NOREPLACE,
        )
    };
    Errno::result(result).map(drop)
}

#[cfg(target_os = "macos")]
fn rename_no_replace(
    parent: &File,
    source: &str,
    destination: &str,
) -> std::result::Result<(), Errno> {
    use std::ffi::{c_char, c_int, c_uint};
    const RENAME_EXCL: c_uint = 0x0000_0004;
    const RENAME_NOFOLLOW_ANY: c_uint = 0x0000_0010;
    unsafe extern "C" {
        fn renameatx_np(
            from_fd: c_int,
            from: *const c_char,
            to_fd: c_int,
            to: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }
    let source = std::ffi::CString::new(source).map_err(|_| Errno::EINVAL)?;
    let destination = std::ffi::CString::new(destination).map_err(|_| Errno::EINVAL)?;
    // SAFETY: both names are generated/fixed single components beneath the
    // retained private parent. EXCL forbids replacement and NOFOLLOW_ANY
    // rejects aliases.
    let result = unsafe {
        // DUX-DESTRUCTIVE: allow=app-data-reset-macos-coordinator-publish -- atomically publish only one retained marker-complete reset coordinator without replacing an existing entry
        renameatx_np(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
            RENAME_EXCL | RENAME_NOFOLLOW_ANY,
        )
    };
    Errno::result(result).map(drop)
}

fn create_control(directory: &File, name: &str, contents: &[u8]) -> Result<(File, Identity)> {
    let (mut file, identity) = create_private_file(directory, name)?;
    if file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        return Err(unavailable());
    }
    validate_control(directory, name, &file, identity, contents)?;
    directory.sync_all().map_err(|_| unavailable())?;
    Ok((file, identity))
}

fn open_control(directory: &File, name: &str, contents: &[u8]) -> Result<Option<(File, Identity)>> {
    let Some((file, identity)) = open_private_file(directory, name, true)? else {
        return Ok(None);
    };
    validate_control(directory, name, &file, identity, contents)?;
    Ok(Some((file, identity)))
}

fn validate_control(
    directory: &File,
    name: &str,
    file: &File,
    identity: Identity,
    contents: &[u8],
) -> Result<()> {
    validate_named(directory, name, file, identity, ObjectKind::PrivateFile)?;
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if metadata.len() != u64::try_from(contents.len()).unwrap() {
        return Err(unsafe_object());
    }
    let mut observed = vec![0_u8; contents.len()];
    std::os::unix::fs::FileExt::read_exact_at(file, &mut observed, 0)
        .map_err(|_| unsafe_object())?;
    if observed != contents {
        return Err(unsafe_object());
    }
    Ok(())
}

fn create_private_file(directory: &File, name: &str) -> Result<(File, Identity)> {
    let file = openat(
        directory,
        name,
        OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        FILE_MODE,
    )
    .map(File::from)
    .map_err(|error| match error {
        Errno::EEXIST | Errno::ELOOP | Errno::EISDIR | Errno::ENOTDIR => unsafe_object(),
        _ => unavailable(),
    })?;
    fchmod(&file, FILE_MODE).map_err(|_| unavailable())?;
    let identity = identity(&file, ObjectKind::PrivateFile)?;
    validate_named(directory, name, &file, identity, ObjectKind::PrivateFile)?;
    Ok((file, identity))
}

fn open_private_file(
    directory: &File,
    name: &str,
    writable: bool,
) -> Result<Option<(File, Identity)>> {
    let access = if writable {
        OFlag::O_RDWR
    } else {
        OFlag::O_RDONLY
    };
    match openat(
        directory,
        name,
        access | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(descriptor) => {
            let file = File::from(descriptor);
            let identity = identity(&file, ObjectKind::PrivateFile)?;
            validate_named(directory, name, &file, identity, ObjectKind::PrivateFile)?;
            Ok(Some((file, identity)))
        }
        Err(Errno::ENOENT) => Ok(None),
        Err(Errno::ELOOP | Errno::EISDIR | Errno::ENOTDIR) => Err(unsafe_object()),
        Err(_) => Err(unavailable()),
    }
}

fn identity(file: &File, kind: ObjectKind) -> Result<Identity> {
    let status = fstat(file).map_err(|_| unavailable())?;
    let expected = match kind {
        ObjectKind::ParentDirectory | ObjectKind::PrivateDirectory => SFlag::S_IFDIR,
        ObjectKind::PrivateFile => SFlag::S_IFREG,
    };
    if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != expected {
        return Err(unsafe_for(kind));
    }
    Ok(Identity {
        device: status.st_dev as u64,
        inode: status.st_ino as u64,
    })
}

fn validate_retained(
    file: &File,
    expected: Identity,
    kind: ObjectKind,
    require_one_link: bool,
) -> Result<()> {
    let status = fstat(file).map_err(|_| unavailable())?;
    let mode = status.st_mode & 0o7777;
    let mode_valid = match kind {
        ObjectKind::ParentDirectory => mode & 0o022 == 0,
        ObjectKind::PrivateDirectory => mode == 0o700,
        ObjectKind::PrivateFile => mode == 0o600,
    };
    if identity(file, kind)? != expected
        || status.st_uid != geteuid().as_raw()
        || !mode_valid
        || (require_one_link && status.st_nlink != 1)
    {
        return Err(unsafe_for(kind));
    }
    match kind {
        ObjectKind::ParentDirectory => reject_granting_acl(file),
        ObjectKind::PrivateDirectory | ObjectKind::PrivateFile => reject_extended_acl(file),
    }
}

fn validate_named(
    directory: &File,
    name: &str,
    retained: &File,
    expected: Identity,
    kind: ObjectKind,
) -> Result<()> {
    let flags = match kind {
        ObjectKind::ParentDirectory | ObjectKind::PrivateDirectory => {
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
        }
        ObjectKind::PrivateFile => {
            OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
        }
    };
    let opened = openat(directory, name, flags, Mode::empty())
        .map(File::from)
        .map_err(|_| unsafe_for(kind))?;
    validate_retained(
        retained,
        expected,
        kind,
        matches!(kind, ObjectKind::PrivateFile),
    )?;
    validate_retained(
        &opened,
        expected,
        kind,
        matches!(kind, ObjectKind::PrivateFile),
    )
}

fn inventory(directory: &File) -> Result<Vec<String>> {
    inventory_bounded(
        directory,
        MAX_INVENTORY_ENTRIES,
        MAX_INVENTORY_NAME_BYTES,
        Instant::now() + Duration::from_millis(250),
    )
}

fn inventory_bounded(
    directory: &File,
    maximum_entries: usize,
    maximum_name_bytes: usize,
    deadline: Instant,
) -> Result<Vec<String>> {
    let clone = directory.try_clone().map_err(|_| unavailable())?;
    let owned: OwnedFd = clone.into();
    let mut entries = Dir::from_fd(owned).map_err(|_| unavailable())?;
    let mut names = Vec::new();
    let mut name_bytes = 0_usize;
    for entry in entries.iter() {
        if Instant::now() > deadline || names.len() >= maximum_entries {
            return Err(unsafe_coordinator());
        }
        let entry = entry.map_err(|_| unavailable())?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let name = std::str::from_utf8(bytes).map_err(|_| unsafe_object())?;
        name_bytes = name_bytes
            .checked_add(name.len())
            .ok_or_else(unsafe_coordinator)?;
        if name_bytes > maximum_name_bytes {
            return Err(unsafe_coordinator());
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

fn is_provisioning_stage_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(PROVISIONING_STAGE_PREFIX) else {
        return false;
    };
    suffix.len() == PROVISIONING_STAGE_HEX_LENGTH
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn inspect_provisioning_stage(parent: &File, name: &str) -> Result<Option<ProvisioningStage>> {
    let inspected = (|| {
        let directory = open_private_directory(parent, name)?;
        let identity = identity(&directory, ObjectKind::PrivateDirectory)?;
        validate_retained(&directory, identity, ObjectKind::PrivateDirectory, false)?;
        validate_named(
            parent,
            name,
            &directory,
            identity,
            ObjectKind::PrivateDirectory,
        )?;
        let mut children = inventory(&directory)?;
        children.sort_unstable();
        let mut expected = vec![LOCK_NAME.to_owned(), MARKER_NAME.to_owned()];
        expected.sort_unstable();
        if children != expected {
            return Err(unsafe_coordinator());
        }
        let marker =
            open_control(&directory, MARKER_NAME, STORE_MARKER)?.ok_or_else(unsafe_coordinator)?;
        let lock =
            open_control(&directory, LOCK_NAME, LOCK_MARKER)?.ok_or_else(unsafe_coordinator)?;
        Ok(ProvisioningStage {
            directory,
            identity,
            marker,
            lock,
        })
    })();
    match inspected {
        Ok(stage) => Ok(Some(stage)),
        Err(error)
            if matches!(
                error.kind(),
                AppDataResetCoordinatorErrorKind::UnsafeCoordinator
                    | AppDataResetCoordinatorErrorKind::UnsafeObject
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn remove_provisioning_stage(
    parent: &File,
    name: &str,
    directory: &File,
    directory_identity: Identity,
    marker: &(File, Identity),
    lock: &(File, Identity),
) -> Result<()> {
    validate_named(
        parent,
        name,
        directory,
        directory_identity,
        ObjectKind::PrivateDirectory,
    )?;
    validate_control(directory, MARKER_NAME, &marker.0, marker.1, STORE_MARKER)?;
    validate_control(directory, LOCK_NAME, &lock.0, lock.1, LOCK_MARKER)?;
    remove_named_file(directory, MARKER_NAME, &marker.0, marker.1)
        .map_err(|_| error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
    remove_named_file(directory, LOCK_NAME, &lock.0, lock.1)
        .map_err(|_| error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
    directory
        .sync_all()
        .map_err(|_| error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
    validate_named(
        parent,
        name,
        directory,
        directory_identity,
        ObjectKind::PrivateDirectory,
    )
    .map_err(|_| error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
    let name = std::ffi::CString::new(name).map_err(|_| unsafe_coordinator())?;
    // SAFETY: the canonical generated stage name and retained empty private
    // directory were fully identity-validated under their retained parent.
    let result = unsafe {
        // DUX-DESTRUCTIVE: allow=app-data-reset-provisioning-stage-reconcile -- remove only one exact empty marker-owned reset-coordinator provisioning stage after removing its two validated controls
        nix::libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), nix::libc::AT_REMOVEDIR)
    };
    if result != 0 {
        return Err(error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
    }
    parent
        .sync_all()
        .map_err(|_| error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
}

fn rename_stage_over_journal(directory: &File) -> Result<()> {
    let source = std::ffi::CString::new(JOURNAL_STAGE_NAME).expect("constant has no NUL");
    let destination = std::ffi::CString::new(JOURNAL_NAME).expect("constant has no NUL");
    // SAFETY: both names are fixed single components and the retained private
    // directory descriptor is the source and destination parent.
    let result = unsafe {
        // DUX-DESTRUCTIVE: allow=app-data-reset-journal-publish -- atomically replace only the fixed reset journal with its fully written fixed private stage
        nix::libc::renameat(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(error_kind(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
    }
}

fn remove_named_file(directory: &File, name: &str, file: &File, expected: Identity) -> Result<()> {
    validate_named(directory, name, file, expected, ObjectKind::PrivateFile)?;
    let name = std::ffi::CString::new(name).map_err(|_| unsafe_object())?;
    // SAFETY: the name is one validated fixed component and the retained
    // private coordinator directory is its exact parent.
    let result = unsafe {
        // DUX-DESTRUCTIVE: allow=app-data-reset-exact-file-unlink -- remove only an exact retained fixed journal stage or marker-owned provisioning-stage control after complete identity validation
        nix::libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0)
    };
    if result == 0 {
        Ok(())
    } else {
        Err(unavailable())
    }
}

#[cfg(target_os = "macos")]
fn reject_extended_acl(file: &File) -> Result<()> {
    use std::ffi::{c_int, c_void};
    const ACL_TYPE_EXTENDED: c_int = 0x100;
    const ACL_FIRST_ENTRY: c_int = 0;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
        fn acl_free(object: *mut c_void) -> c_int;
    }
    // SAFETY: Darwin returns an independently allocated ACL for this live fd.
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        return if Errno::last() == Errno::ENOENT {
            Ok(())
        } else {
            Err(unsafe_object())
        };
    }
    let mut entry = std::ptr::null_mut();
    // SAFETY: the ACL is live and the output pointer is writable.
    let status = unsafe { acl_get_entry(acl, ACL_FIRST_ENTRY, &raw mut entry) };
    // SAFETY: this frees the independently allocated ACL exactly once.
    let freed = unsafe { acl_free(acl) };
    if status < 0 || freed != 0 || !entry.is_null() {
        return Err(unsafe_object());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_extended_acl(file: &File) -> Result<()> {
    reject_linux_acl(file).map_err(|_| unsafe_object())
}

#[cfg(target_os = "macos")]
fn reject_granting_acl(file: &File) -> Result<()> {
    use std::ffi::{c_int, c_void};
    const ACL_TYPE_EXTENDED: c_int = 0x100;
    const ACL_FIRST_ENTRY: c_int = 0;
    const ACL_NEXT_ENTRY: c_int = -1;
    const ACL_EXTENDED_DENY: c_int = 2;
    const ACL_MAX_ENTRIES: usize = 128;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
        fn acl_free(object: *mut c_void) -> c_int;
    }
    // SAFETY: Darwin returns an independently allocated ACL for this live fd.
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        return if Errno::last() == Errno::ENOENT {
            Ok(())
        } else {
            Err(unsafe_parent())
        };
    }
    let inspection = (|| {
        let mut selector = ACL_FIRST_ENTRY;
        for _ in 0..ACL_MAX_ENTRIES {
            let mut entry = std::ptr::null_mut();
            // SAFETY: the ACL is live and the output pointer is writable.
            if unsafe { acl_get_entry(acl, selector, &raw mut entry) } < 0 {
                if selector == ACL_NEXT_ENTRY && Errno::last() == Errno::EINVAL {
                    return Ok(());
                }
                return Err(unsafe_parent());
            }
            if entry.is_null() {
                return Ok(());
            }
            let mut tag = 0;
            // SAFETY: entry belongs to the live ACL and tag is writable.
            if unsafe { acl_get_tag_type(entry, &raw mut tag) } != 0 || tag != ACL_EXTENDED_DENY {
                return Err(unsafe_parent());
            }
            selector = ACL_NEXT_ENTRY;
        }
        Err(unsafe_parent())
    })();
    // SAFETY: this frees the independently allocated ACL exactly once.
    if unsafe { acl_free(acl) } != 0 {
        return Err(unsafe_parent());
    }
    inspection
}

#[cfg(target_os = "linux")]
fn reject_granting_acl(file: &File) -> Result<()> {
    reject_linux_acl(file).map_err(|_| unsafe_parent())
}

#[cfg(target_os = "linux")]
fn reject_linux_acl(file: &File) -> std::result::Result<(), ()> {
    for name in [
        b"system.posix_acl_access\0".as_slice(),
        b"system.posix_acl_default\0".as_slice(),
    ] {
        // SAFETY: each name is NUL terminated and the live descriptor is only queried.
        let length = unsafe {
            nix::libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr().cast(),
                std::ptr::null_mut(),
                0,
            )
        };
        if length >= 0 {
            return Err(());
        }
        match Errno::last() {
            Errno::ENODATA | Errno::ENOTSUP => {}
            _ => return Err(()),
        }
    }
    Ok(())
}

fn map_coordinator_open_error(error: Errno) -> AppDataResetCoordinatorError {
    match error {
        Errno::ELOOP | Errno::ENOTDIR | Errno::EISDIR => unsafe_coordinator(),
        _ => unavailable(),
    }
}

fn unsafe_for(kind: ObjectKind) -> AppDataResetCoordinatorError {
    match kind {
        ObjectKind::ParentDirectory => unsafe_parent(),
        ObjectKind::PrivateDirectory => unsafe_coordinator(),
        ObjectKind::PrivateFile => unsafe_object(),
    }
}

fn error_kind(kind: AppDataResetCoordinatorErrorKind) -> AppDataResetCoordinatorError {
    error(kind)
}

fn unsafe_parent() -> AppDataResetCoordinatorError {
    error_kind(AppDataResetCoordinatorErrorKind::UnsafeParent)
}

fn unsafe_coordinator() -> AppDataResetCoordinatorError {
    error_kind(AppDataResetCoordinatorErrorKind::UnsafeCoordinator)
}

fn unsafe_object() -> AppDataResetCoordinatorError {
    error_kind(AppDataResetCoordinatorErrorKind::UnsafeObject)
}

fn unavailable() -> AppDataResetCoordinatorError {
    error_kind(AppDataResetCoordinatorErrorKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::sync::{Arc, Mutex};

    use tempfile::TempDir;

    use super::*;

    fn storage(temp: &TempDir) -> ResetCoordinatorStorage {
        ResetCoordinatorStorage::open_or_create(&temp.path().canonicalize().unwrap().join("Dux"))
            .unwrap()
    }

    #[test]
    fn controls_are_exact_private_single_link_files() {
        let temp = TempDir::new().unwrap();
        let storage = storage(&temp);
        storage.validate().unwrap();

        let root = temp
            .path()
            .canonicalize()
            .unwrap()
            .join(COORDINATOR_DIRECTORY_NAME);
        for (name, contents) in [(MARKER_NAME, STORE_MARKER), (LOCK_NAME, LOCK_MARKER)] {
            let path = root.join(name);
            let metadata = fs::metadata(&path).unwrap();
            assert_eq!(metadata.mode() & 0o7777, 0o600);
            assert_eq!(metadata.nlink(), 1);
            assert_eq!(fs::read(path).unwrap(), contents);
        }
    }

    #[test]
    fn uncommitted_stage_is_discarded_without_changing_journal() {
        let temp = TempDir::new().unwrap();
        let storage = storage(&temp);
        storage.write_journal(b"first").unwrap();
        let root = temp
            .path()
            .canonicalize()
            .unwrap()
            .join(COORDINATOR_DIRECTORY_NAME);
        fs::write(root.join(JOURNAL_STAGE_NAME), b"second").unwrap();
        fs::set_permissions(
            root.join(JOURNAL_STAGE_NAME),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        assert_eq!(
            storage.with_lock(|storage| storage.read_journal()).unwrap(),
            Some(b"first".to_vec())
        );
        assert!(!root.join(JOURNAL_STAGE_NAME).exists());
    }

    #[test]
    fn journal_write_faults_preserve_the_atomic_publication_certainty_boundary() {
        let before_case = TempDir::new().unwrap();
        let before = storage(&before_case);
        set_test_journal_write_fault(TestJournalWriteFault::BeforeRename);
        assert_eq!(
            before.write_journal(b"before").unwrap_err().kind(),
            AppDataResetCoordinatorErrorKind::Unavailable
        );
        assert_eq!(
            before.with_lock(|storage| storage.read_journal()).unwrap(),
            None
        );

        for fault in [
            TestJournalWriteFault::AfterRename,
            TestJournalWriteFault::AfterDirectorySync,
            TestJournalWriteFault::DuringReadback,
        ] {
            let after_case = TempDir::new().unwrap();
            let after = storage(&after_case);
            set_test_journal_write_fault(fault);
            assert_eq!(
                after.write_journal(b"after").unwrap_err().kind(),
                AppDataResetCoordinatorErrorKind::OutcomeUnknown
            );
            assert_eq!(
                after.with_lock(|storage| storage.read_journal()).unwrap(),
                Some(b"after".to_vec())
            );
        }
    }

    #[test]
    fn unknown_and_hard_linked_objects_fail_closed() {
        let unknown_case = TempDir::new().unwrap();
        let unknown = storage(&unknown_case);
        fs::write(
            unknown_case
                .path()
                .canonicalize()
                .unwrap()
                .join(COORDINATOR_DIRECTORY_NAME)
                .join("unknown"),
            b"x",
        )
        .unwrap();
        assert_eq!(
            unknown.validate().unwrap_err().kind(),
            AppDataResetCoordinatorErrorKind::UnsafeObject
        );

        let link_case = TempDir::new().unwrap();
        let linked = storage(&link_case);
        let canonical = link_case.path().canonicalize().unwrap();
        let root = canonical.join(COORDINATOR_DIRECTORY_NAME);
        fs::hard_link(
            root.join(MARKER_NAME),
            canonical.join("outside-marker-link"),
        )
        .unwrap();
        assert_eq!(
            linked.validate().unwrap_err().kind(),
            AppDataResetCoordinatorErrorKind::UnsafeObject
        );
    }

    #[test]
    fn abandoned_partial_stage_cannot_poison_fixed_namespace() {
        let temp = TempDir::new().unwrap();
        let canonical = temp.path().canonicalize().unwrap();
        let partial = canonical.join(format!(
            "{PROVISIONING_STAGE_PREFIX}{}",
            "0".repeat(PROVISIONING_STAGE_HEX_LENGTH)
        ));
        fs::create_dir(&partial).unwrap();
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o700)).unwrap();

        let opened = storage(&temp);
        opened.validate().unwrap();
        assert_eq!(opened.reconcile_provisioning_stages().unwrap(), 1);
        assert!(partial.is_dir());
        assert!(
            canonical
                .join(COORDINATOR_DIRECTORY_NAME)
                .join(MARKER_NAME)
                .is_file()
        );
    }

    #[test]
    fn coordinator_provisioning_stage_lock_obeys_the_existing_deadline() {
        let temp = TempDir::new().unwrap();
        let canonical = temp.path().canonicalize().unwrap();
        let data_root = canonical.join("Dux");
        let held = Arc::new(Mutex::new(None::<File>));
        let held_for_hook = Arc::clone(&held);
        let parent_for_hook = canonical.clone();

        let Err(error) = ResetCoordinatorStorage::open_or_create_with_stage_lock_hook_until(
            &data_root,
            Instant::now() + Duration::from_millis(20),
            move || {
                let stage = fs::read_dir(&parent_for_hook)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with(PROVISIONING_STAGE_PREFIX))
                    })
                    .expect("provisioning hook must observe the private stage");
                let file = File::open(stage.join(LOCK_NAME)).unwrap();
                FileExt::lock(&file).unwrap();
                *held_for_hook.lock().unwrap() = Some(file);
            },
        ) else {
            panic!("contended provisioning stage unexpectedly published");
        };

        assert_eq!(error.kind(), AppDataResetCoordinatorErrorKind::Busy);
        drop(held.lock().unwrap().take());
        assert!(!canonical.join(COORDINATOR_DIRECTORY_NAME).exists());

        let opened = ResetCoordinatorStorage::open_or_create(&data_root).unwrap();
        opened.validate().unwrap();
        assert!(
            fs::read_dir(&canonical)
                .unwrap()
                .map(|entry| entry.unwrap())
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(PROVISIONING_STAGE_PREFIX))
        );
    }

    #[test]
    fn publication_collision_reopens_exact_winner() {
        let temp = TempDir::new().unwrap();
        let canonical = temp.path().canonicalize().unwrap();
        let data_root = canonical.join("Dux");
        let collision_root = data_root.clone();
        let opened = ResetCoordinatorStorage::open_or_create_with_hook(&data_root, move || {
            drop(ResetCoordinatorStorage::open_or_create(&collision_root).unwrap());
        })
        .unwrap();

        opened.validate().unwrap();
        assert!(opened.read_journal().unwrap().is_none());
        let stages = fs::read_dir(canonical)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(is_provisioning_stage_name)
            })
            .count();
        assert_eq!(stages, 0);
    }

    #[test]
    fn marker_complete_interrupted_stage_is_reconciled() {
        let temp = TempDir::new().unwrap();
        let canonical = temp.path().canonicalize().unwrap();
        let parent = open_absolute_directory(&canonical).unwrap();
        let stage_name = format!(
            "{PROVISIONING_STAGE_PREFIX}{}",
            "1".repeat(PROVISIONING_STAGE_HEX_LENGTH)
        );
        mkdirat(&parent, stage_name.as_str(), DIRECTORY_MODE).unwrap();
        let stage = open_private_directory(&parent, &stage_name).unwrap();
        fchmod(&stage, DIRECTORY_MODE).unwrap();
        create_control(&stage, LOCK_NAME, LOCK_MARKER).unwrap();
        create_control(&stage, MARKER_NAME, STORE_MARKER).unwrap();
        stage.sync_all().unwrap();
        parent.sync_all().unwrap();
        drop(stage);
        drop(parent);

        let opened = storage(&temp);
        opened.validate().unwrap();
        assert!(!canonical.join(stage_name).exists());
        assert_eq!(opened.reconcile_provisioning_stages().unwrap(), 0);
    }

    #[test]
    fn symlink_ancestor_is_rejected() {
        let temp = TempDir::new().unwrap();
        let canonical = temp.path().canonicalize().unwrap();
        let real = canonical.join("real");
        fs::create_dir(&real).unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&real, canonical.join("alias")).unwrap();

        let error = match ResetCoordinatorStorage::open_or_create(&canonical.join("alias/Dux")) {
            Ok(_) => panic!("symlink ancestor unexpectedly opened"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), AppDataResetCoordinatorErrorKind::UnsafeParent);
    }
}
