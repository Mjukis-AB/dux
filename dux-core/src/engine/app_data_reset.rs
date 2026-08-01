#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the private reset composition is consumed by the namespace-witness slice"
    )
)]

#[cfg(test)]
use std::cell::Cell;
use std::sync::Arc;
use std::time::Instant;

use crate::app_data_reset_transaction::{
    AppDataResetTransaction, AppDataResetTransactionErrorKind,
};
use crate::cache::ManagedCacheStoreErrorKind;
use crate::persistence::snapshot::{
    AppDataResetSnapshotAdmission, SnapshotRepository, SnapshotRepositoryErrorKind,
};
use crate::persistence::{
    AppDataResetAdmittedStoreOutcome, AppDataResetCoordinator, AppDataResetCoordinatorError,
    AppDataResetCoordinatorErrorKind, AppDataResetCoordinatorSession,
    AppDataResetDataNamespaceAdmission, AppDataResetJournal, AppDataResetPhase,
    AppDataResetStoreBlockers, AppDataResetStoreGuard, HistoryError, HistoryErrorKind,
    StoreCoordinator,
};

use super::managed_scan_cache::{
    AppDataResetManagedScanCacheAdmission, AppDataResetManagedScanCacheError, ManagedScanCache,
};
use super::registry::{AppDataResetAdmissionOutcome, AppDataResetQuiesced, EngineHandle};

#[cfg(test)]
std::thread_local! {
    static TEST_PANIC_AFTER_CACHE_DETACH: Cell<bool> = const { Cell::new(false) };
    static TEST_PANIC_AFTER_DATA_DETACH: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_test_panic_after_cache_detach() {
    TEST_PANIC_AFTER_CACHE_DETACH.with(|armed| armed.set(true));
}

#[cfg(test)]
pub(crate) fn set_test_panic_after_data_detach() {
    TEST_PANIC_AFTER_DATA_DETACH.with(|armed| armed.set(true));
}

fn maybe_panic_after_cache_detach() {
    #[cfg(test)]
    TEST_PANIC_AFTER_CACHE_DETACH.with(|armed| {
        assert!(!armed.replace(false), "injected panic after cache detach");
    });
}

fn maybe_panic_after_data_detach() {
    #[cfg(test)]
    TEST_PANIC_AFTER_DATA_DETACH.with(|armed| {
        assert!(!armed.replace(false), "injected panic after data detach");
    });
}

/// A reset-shutdown capability was consumed without proving worker
/// quiescence. The old engine remains terminal and the reset effect must not
/// run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppDataResetShutdownError {
    #[error("engine workers did not quiesce within the bounded reset deadline")]
    ShutdownIncomplete,
    #[error("the reset terminal lifecycle claim is internally inconsistent")]
    InternalState,
}

/// Whether this private reset composition changed the engine lifecycle.
///
/// A refusal before terminal arbitration leaves the prior lifecycle untouched;
/// that prior state is not necessarily open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetEngineDisposition {
    UnchangedByAttempt,
    Terminal,
}

/// Refusal observed before this call claimed terminal reset.
///
/// The engine lifecycle is unchanged by this attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetPreTerminalRefusal {
    AdmissionDeadlineExceeded,
    #[allow(
        dead_code,
        reason = "retained for stable internal refusal classification while reset coordination moves after quiescence"
    )]
    Coordinator(AppDataResetCoordinatorErrorKind),
    Store(HistoryErrorKind),
    Transaction(AppDataResetTransactionErrorKind),
    LifecycleBusy,
    ProvisioningDebt {
        unproven_stage_count: u32,
    },
    RecoveryRequired {
        phase: AppDataResetPhase,
    },
}

/// Terminal lifecycle already belongs to another operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetTerminalOwner {
    OrdinaryClose,
    AppDataReset,
}

/// Path-free, process-local work that remains outside engine workers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AppDataResetRuntimeBlockers {
    active_cleanup_operation: bool,
    process_cleanup_quarantine: bool,
}

impl AppDataResetRuntimeBlockers {
    pub(crate) const fn new(
        active_cleanup_operation: bool,
        process_cleanup_quarantine: bool,
    ) -> Self {
        Self {
            active_cleanup_operation,
            process_cleanup_quarantine,
        }
    }

    pub(crate) const fn is_empty(self) -> bool {
        !self.active_cleanup_operation && !self.process_cleanup_quarantine
    }

    pub(crate) const fn has_active_cleanup_operation(self) -> bool {
        self.active_cleanup_operation
    }

    pub(crate) const fn has_process_cleanup_quarantine(self) -> bool {
        self.process_cleanup_quarantine
    }
}

/// Refusal discovered only after this call made the old engine terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetPostTerminalRefusal {
    Shutdown(AppDataResetShutdownError),
    AdmissionDeadlineExceeded,
    RuntimeInspectionBusy,
    RuntimeBlocked(AppDataResetRuntimeBlockers),
    DataNamespace(HistoryErrorKind),
    StoreBlocked(AppDataResetStoreBlockers),
    Store(HistoryErrorKind),
    Snapshot(SnapshotRepositoryErrorKind),
    ManagedCache(ManagedCacheStoreErrorKind),
    Coordinator(AppDataResetCoordinatorErrorKind),
    ProvisioningDebt { unproven_stage_count: u32 },
    RecoveryRequired { phase: AppDataResetPhase },
    LifecycleInternalState,
}

/// Private result of composing the reset lock/lifecycle layers implemented by
/// this checkpoint.
///
/// No variant carries a path, journal identifier, namespace witness, or
/// reset-target effect authority. Independent coordinator provisioning or
/// reconciliation may occur before admission. Only `Admitted` ran the
/// higher-ranked callback while every retained proof was live.
pub(crate) enum AppDataResetCompositionOutcome<T> {
    Admitted(T),
    PreTerminalRefused(AppDataResetPreTerminalRefusal),
    TerminalOwnedElsewhere(AppDataResetTerminalOwner),
    TerminalWithoutAdmission(AppDataResetPostTerminalRefusal),
}

/// Result of attempting the first durable reset-journal transition while all
/// reset admission proofs remain retained.
///
/// `RecoveryRequired` is emitted only when journal publication may have
/// committed. The caller must stop and let a later recovery session inspect
/// the coordinator; it must never retry the intent or perform an effect.
#[must_use = "Prepared may have committed; inspect the outcome and never retry uncertainty"]
pub(crate) enum AppDataResetPreparedIntentOutcome<T> {
    Committed(T),
    RefusedBeforeIntent(AppDataResetPostTerminalRefusal),
    RecoveryRequired,
}

/// Result of advancing a committed reset through exact managed-cache
/// detachment. After `Prepared`, no failure permits cancellation or retry.
#[must_use = "cache detachment may have occurred; every failure requires recovery"]
pub(crate) enum AppDataResetCacheDetachOutcome<T> {
    Committed(T),
    RecoveryRequired,
}

/// Result of advancing a committed reset through exact data-root detachment.
/// No failure after `CacheDetached` permits cancellation or retry.
#[must_use = "data-root detachment may have occurred; every failure requires recovery"]
pub(crate) enum AppDataResetDataDetachOutcome<T> {
    Committed(T),
    RecoveryRequired,
}

impl<T> AppDataResetCompositionOutcome<T> {
    pub(crate) const fn engine_disposition(&self) -> AppDataResetEngineDisposition {
        match self {
            Self::PreTerminalRefused(_) => AppDataResetEngineDisposition::UnchangedByAttempt,
            Self::Admitted(_)
            | Self::TerminalOwnedElsewhere(_)
            | Self::TerminalWithoutAdmission(_) => AppDataResetEngineDisposition::Terminal,
        }
    }
}

/// Callback-scoped proof of coordinator ownership, core-worker quiescence,
/// data-namespace publication fencing, cleanup/database exclusion, snapshot
/// exclusion, and present-or-absent cache-namespace publication fencing.
///
/// Its fields are private and its lifetimes are higher-ranked at the call
/// site, so none of the retained proofs can escape. The consume-once internal
/// operation may commit only the coordinator's durable `Prepared` intent. It
/// exposes no reset-target namespace operation.
/// Coordinator-only provisioning reconciliation may still occur.
pub(crate) struct AppDataResetCoreAdmission<
    'session,
    'storage,
    'guard,
    'store,
    'quiesced,
    'data,
    'snapshot,
    'cache,
    'runtime,
    'transaction,
> {
    store: &'store mut AppDataResetStoreGuard<'guard>,
    data_namespace: AppDataResetDataNamespaceAdmission<'data>,
    snapshot: AppDataResetSnapshotAdmission<'snapshot>,
    managed_cache: AppDataResetManagedScanCacheAdmission<'cache>,
    _quiesced: &'quiesced AppDataResetQuiesced,
    coordinator: &'session mut AppDataResetCoordinatorSession<'storage>,
    deadline: Instant,
    inspect_runtime_blockers: &'runtime dyn Fn(Instant) -> Result<AppDataResetRuntimeBlockers, ()>,
    transaction: &'transaction AppDataResetTransaction,
}

/// A durably committed `Prepared` intent that still retains every proof needed
/// by the future namespace-detachment operation.
///
/// The higher-ranked callback prevents this value from escaping reset
/// composition. This checkpoint exposes only revalidation and no namespace
/// mutation.
#[must_use = "a committed Prepared intent must be revalidated or handed to recovery"]
pub(crate) struct AppDataResetPreparedIntent<
    'session,
    'storage,
    'guard,
    'store,
    'quiesced,
    'data,
    'snapshot,
    'cache,
    'runtime,
    'transaction,
> {
    admission: AppDataResetCoreAdmission<
        'session,
        'storage,
        'guard,
        'store,
        'quiesced,
        'data,
        'snapshot,
        'cache,
        'runtime,
        'transaction,
    >,
    journal: AppDataResetJournal,
}

/// A durably committed `CacheDetached` checkpoint retaining every proof for
/// the subsequent data-namespace detachment.
#[must_use = "a committed CacheDetached checkpoint must continue or be recovered"]
pub(crate) struct AppDataResetCacheDetached<
    'session,
    'storage,
    'guard,
    'store,
    'quiesced,
    'data,
    'snapshot,
    'cache,
    'runtime,
    'transaction,
> {
    admission: AppDataResetCoreAdmission<
        'session,
        'storage,
        'guard,
        'store,
        'quiesced,
        'data,
        'snapshot,
        'cache,
        'runtime,
        'transaction,
    >,
    journal: AppDataResetJournal,
}

/// A durably committed `DataDetached` checkpoint retaining every proof needed
/// by later fresh-namespace provisioning. It exposes no ordinary store or
/// snapshot operation through the now-stale canonical paths.
#[must_use = "a committed DataDetached checkpoint must continue or be recovered"]
pub(crate) struct AppDataResetDataDetached<
    'session,
    'storage,
    'guard,
    'store,
    'quiesced,
    'data,
    'snapshot,
    'cache,
    'runtime,
    'transaction,
> {
    admission: AppDataResetCoreAdmission<
        'session,
        'storage,
        'guard,
        'store,
        'quiesced,
        'data,
        'snapshot,
        'cache,
        'runtime,
        'transaction,
    >,
    journal: AppDataResetJournal,
}

impl<
    'session,
    'storage,
    'guard,
    'store,
    'quiesced,
    'data,
    'snapshot,
    'cache,
    'runtime,
    'transaction,
>
    AppDataResetCoreAdmission<
        'session,
        'storage,
        'guard,
        'store,
        'quiesced,
        'data,
        'snapshot,
        'cache,
        'runtime,
        'transaction,
    >
{
    pub(crate) fn revalidate(&mut self) -> Result<(), AppDataResetPostTerminalRefusal> {
        self.revalidate_with_journal(None)
    }

    fn revalidate_with_journal(
        &mut self,
        expected_journal: Option<&AppDataResetJournal>,
    ) -> Result<(), AppDataResetPostTerminalRefusal> {
        if Instant::now() >= self.deadline {
            return Err(AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded);
        }
        self.revalidate_coordinator_state(expected_journal)?;
        if let Err(error) = self.data_namespace.revalidate(&*self.store) {
            return Err(if Instant::now() >= self.deadline {
                AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded
            } else {
                AppDataResetPostTerminalRefusal::DataNamespace(error.kind)
            });
        }
        let runtime_blockers = (self.inspect_runtime_blockers)(self.deadline)
            .map_err(|()| AppDataResetPostTerminalRefusal::RuntimeInspectionBusy)?;
        if !runtime_blockers.is_empty() {
            return Err(AppDataResetPostTerminalRefusal::RuntimeBlocked(
                runtime_blockers,
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded);
        }
        if self.data_namespace.is_detached() {
            self.store
                .revalidate_after_data_detach()
                .map_err(|error| AppDataResetPostTerminalRefusal::Store(error.kind))?;
        } else {
            let blockers = self
                .store
                .revalidate()
                .map_err(|error| AppDataResetPostTerminalRefusal::Store(error.kind))?;
            if !blockers.is_empty() {
                return Err(AppDataResetPostTerminalRefusal::StoreBlocked(blockers));
            }
        }
        self.snapshot
            .revalidate()
            .map_err(|error| AppDataResetPostTerminalRefusal::Snapshot(error.kind))?;
        self.managed_cache
            .revalidate()
            .map_err(map_managed_cache_refusal)?;
        if let Err(error) = self.data_namespace.revalidate(&*self.store) {
            return Err(if Instant::now() >= self.deadline {
                AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded
            } else {
                AppDataResetPostTerminalRefusal::DataNamespace(error.kind)
            });
        }
        self.revalidate_coordinator_state(expected_journal)?;
        if Instant::now() >= self.deadline {
            return Err(AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded);
        }
        Ok(())
    }

    fn revalidate_coordinator_state(
        &mut self,
        expected_journal: Option<&AppDataResetJournal>,
    ) -> Result<(), AppDataResetPostTerminalRefusal> {
        if let Some(expected) = expected_journal {
            let current = self
                .coordinator
                .recover()
                .map_err(|error| AppDataResetPostTerminalRefusal::Coordinator(error.kind()))?;
            if current.as_ref() != Some(expected) {
                return Err(AppDataResetPostTerminalRefusal::Coordinator(
                    AppDataResetCoordinatorErrorKind::ChangedSinceRead,
                ));
            }
            let debt = self
                .coordinator
                .provisioning_debt()
                .map_err(|error| AppDataResetPostTerminalRefusal::Coordinator(error.kind()))?;
            if debt.unproven_stage_count() != 0 {
                return Err(AppDataResetPostTerminalRefusal::ProvisioningDebt {
                    unproven_stage_count: debt.unproven_stage_count(),
                });
            }
        } else {
            validate_coordinator_preflight(self.coordinator)
                .map_err(|error| AppDataResetPostTerminalRefusal::Coordinator(error.kind()))?
                .map_or(Ok(()), |refusal| match refusal {
                    AppDataResetPreTerminalRefusal::Coordinator(kind) => {
                        Err(AppDataResetPostTerminalRefusal::Coordinator(kind))
                    }
                    AppDataResetPreTerminalRefusal::AdmissionDeadlineExceeded => {
                        Err(AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded)
                    }
                    AppDataResetPreTerminalRefusal::Store(kind) => {
                        Err(AppDataResetPostTerminalRefusal::Store(kind))
                    }
                    AppDataResetPreTerminalRefusal::Transaction(_) => {
                        Err(AppDataResetPostTerminalRefusal::LifecycleInternalState)
                    }
                    AppDataResetPreTerminalRefusal::LifecycleBusy => {
                        Err(AppDataResetPostTerminalRefusal::LifecycleInternalState)
                    }
                    AppDataResetPreTerminalRefusal::ProvisioningDebt {
                        unproven_stage_count,
                    } => Err(AppDataResetPostTerminalRefusal::ProvisioningDebt {
                        unproven_stage_count,
                    }),
                    AppDataResetPreTerminalRefusal::RecoveryRequired { phase } => {
                        Err(AppDataResetPostTerminalRefusal::RecoveryRequired { phase })
                    }
                })?;
        }
        Ok(())
    }

    fn detach_managed_cache(
        mut self,
        journal: &AppDataResetJournal,
    ) -> Result<Self, AppDataResetManagedScanCacheError> {
        let detached = self
            .managed_cache
            .detach(journal.cache_identity(), journal.cache_stage_name())?;
        self.managed_cache = detached;
        Ok(self)
    }

    fn detach_data_root(mut self, journal: &AppDataResetJournal) -> Result<Self, HistoryError> {
        let detached = self.data_namespace.detach(
            &*self.store,
            journal.data_identity(),
            journal.data_stage_name(),
        )?;
        self.data_namespace = detached;
        Ok(self)
    }

    pub(crate) fn commit_prepared_intent(
        mut self,
    ) -> AppDataResetPreparedIntentOutcome<
        AppDataResetPreparedIntent<
            'session,
            'storage,
            'guard,
            'store,
            'quiesced,
            'data,
            'snapshot,
            'cache,
            'runtime,
            'transaction,
        >,
    > {
        if let Err(error) = self.revalidate() {
            return AppDataResetPreparedIntentOutcome::RefusedBeforeIntent(error);
        }
        let data_identity = match self.data_namespace.journal_identity() {
            Ok(identity) => identity,
            Err(error) => {
                return AppDataResetPreparedIntentOutcome::RefusedBeforeIntent(
                    AppDataResetPostTerminalRefusal::DataNamespace(error.kind),
                );
            }
        };
        let cache_identity = match self.managed_cache.journal_identity() {
            Ok(identity) => identity,
            Err(error) => {
                return AppDataResetPreparedIntentOutcome::RefusedBeforeIntent(
                    map_managed_cache_refusal(error),
                );
            }
        };
        let canonical_root = match self.data_namespace.canonical_root_binding() {
            Ok(binding) => binding,
            Err(error) => {
                return AppDataResetPreparedIntentOutcome::RefusedBeforeIntent(
                    AppDataResetPostTerminalRefusal::DataNamespace(error.kind),
                );
            }
        };
        let journal = match AppDataResetJournal::prepared(
            self.transaction,
            data_identity,
            cache_identity,
            canonical_root,
        ) {
            Ok(journal) => journal,
            Err(error) => {
                return AppDataResetPreparedIntentOutcome::RefusedBeforeIntent(
                    AppDataResetPostTerminalRefusal::Coordinator(error.kind()),
                );
            }
        };
        if let Err(error) = self.revalidate_coordinator_state(None) {
            return AppDataResetPreparedIntentOutcome::RefusedBeforeIntent(error);
        }
        if Instant::now() >= self.deadline {
            return AppDataResetPreparedIntentOutcome::RefusedBeforeIntent(
                AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded,
            );
        }
        match self.coordinator.begin(&journal) {
            Ok(()) => AppDataResetPreparedIntentOutcome::Committed(AppDataResetPreparedIntent {
                admission: self,
                journal,
            }),
            Err(error) if error.kind() == AppDataResetCoordinatorErrorKind::OutcomeUnknown => {
                AppDataResetPreparedIntentOutcome::RecoveryRequired
            }
            Err(error) => AppDataResetPreparedIntentOutcome::RefusedBeforeIntent(
                AppDataResetPostTerminalRefusal::Coordinator(error.kind()),
            ),
        }
    }
}

impl<
    'session,
    'storage,
    'guard,
    'store,
    'quiesced,
    'data,
    'snapshot,
    'cache,
    'runtime,
    'transaction,
>
    AppDataResetPreparedIntent<
        'session,
        'storage,
        'guard,
        'store,
        'quiesced,
        'data,
        'snapshot,
        'cache,
        'runtime,
        'transaction,
    >
{
    pub(crate) fn revalidate(&mut self) -> Result<(), AppDataResetPostTerminalRefusal> {
        self.admission.revalidate_with_journal(Some(&self.journal))
    }

    #[cfg(test)]
    pub(crate) const fn journal(&self) -> &AppDataResetJournal {
        &self.journal
    }

    pub(crate) fn detach_managed_cache(
        mut self,
    ) -> AppDataResetCacheDetachOutcome<
        AppDataResetCacheDetached<
            'session,
            'storage,
            'guard,
            'store,
            'quiesced,
            'data,
            'snapshot,
            'cache,
            'runtime,
            'transaction,
        >,
    > {
        if self.revalidate().is_err() {
            return AppDataResetCacheDetachOutcome::RecoveryRequired;
        }
        let admission = match self.admission.detach_managed_cache(&self.journal) {
            Ok(admission) => admission,
            Err(_) => return AppDataResetCacheDetachOutcome::RecoveryRequired,
        };
        maybe_panic_after_cache_detach();
        let journal = match admission
            .coordinator
            .advance(&self.journal, AppDataResetPhase::CacheDetached)
        {
            Ok(journal) => journal,
            Err(_) => return AppDataResetCacheDetachOutcome::RecoveryRequired,
        };
        let mut detached = AppDataResetCacheDetached { admission, journal };
        if detached.revalidate().is_err() {
            AppDataResetCacheDetachOutcome::RecoveryRequired
        } else {
            AppDataResetCacheDetachOutcome::Committed(detached)
        }
    }
}

impl<
    'session,
    'storage,
    'guard,
    'store,
    'quiesced,
    'data,
    'snapshot,
    'cache,
    'runtime,
    'transaction,
>
    AppDataResetCacheDetached<
        'session,
        'storage,
        'guard,
        'store,
        'quiesced,
        'data,
        'snapshot,
        'cache,
        'runtime,
        'transaction,
    >
{
    pub(crate) fn revalidate(&mut self) -> Result<(), AppDataResetPostTerminalRefusal> {
        self.admission.revalidate_with_journal(Some(&self.journal))
    }

    #[cfg(test)]
    pub(crate) const fn journal(&self) -> &AppDataResetJournal {
        &self.journal
    }

    pub(crate) fn detach_data_root(
        mut self,
    ) -> AppDataResetDataDetachOutcome<
        AppDataResetDataDetached<
            'session,
            'storage,
            'guard,
            'store,
            'quiesced,
            'data,
            'snapshot,
            'cache,
            'runtime,
            'transaction,
        >,
    > {
        if self.revalidate().is_err() {
            return AppDataResetDataDetachOutcome::RecoveryRequired;
        }
        let admission = match self.admission.detach_data_root(&self.journal) {
            Ok(admission) => admission,
            Err(_) => return AppDataResetDataDetachOutcome::RecoveryRequired,
        };
        maybe_panic_after_data_detach();
        let journal = match admission
            .coordinator
            .advance(&self.journal, AppDataResetPhase::DataDetached)
        {
            Ok(journal) => journal,
            Err(_) => return AppDataResetDataDetachOutcome::RecoveryRequired,
        };
        let mut detached = AppDataResetDataDetached { admission, journal };
        if detached.revalidate().is_err() {
            AppDataResetDataDetachOutcome::RecoveryRequired
        } else {
            AppDataResetDataDetachOutcome::Committed(detached)
        }
    }
}

impl AppDataResetDataDetached<'_, '_, '_, '_, '_, '_, '_, '_, '_, '_> {
    pub(crate) fn revalidate(&mut self) -> Result<(), AppDataResetPostTerminalRefusal> {
        self.admission.revalidate_with_journal(Some(&self.journal))
    }

    #[cfg(test)]
    pub(crate) const fn journal(&self) -> &AppDataResetJournal {
        &self.journal
    }
}

/// Compose terminal worker quiescence, exclusive coordinator recovery,
/// data-namespace publication admission, cleanup/database admission, snapshot
/// admission, and present-or-absent cache-namespace admission. Quiescence must
/// release the ordinary engine's shared coordinator lease before this path can
/// acquire the exclusive reset session. The boundary itself does not write
/// intent; its retained callback may consume admission to commit `Prepared`
/// and continue through the currently implemented detach checkpoints.
pub(super) fn with_terminal_store_preflight_until<T>(
    engine: &EngineHandle,
    store: &Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    managed_scan_cache: &ManagedScanCache,
    deadline: Instant,
    inspect_runtime_blockers: impl Fn(Instant) -> Result<AppDataResetRuntimeBlockers, ()>,
    admitted: impl for<
        'session,
        'storage,
        'guard,
        'store,
        'quiesced,
        'data,
        'snapshot,
        'cache,
        'runtime,
        'transaction,
    > FnOnce(
        AppDataResetCoreAdmission<
            'session,
            'storage,
            'guard,
            'store,
            'quiesced,
            'data,
            'snapshot,
            'cache,
            'runtime,
            'transaction,
        >,
    )
        -> T,
) -> AppDataResetCompositionOutcome<T> {
    if Instant::now() >= deadline {
        return AppDataResetCompositionOutcome::PreTerminalRefused(
            AppDataResetPreTerminalRefusal::AdmissionDeadlineExceeded,
        );
    }
    if !snapshots.coordinates_store(store) {
        return AppDataResetCompositionOutcome::PreTerminalRefused(
            AppDataResetPreTerminalRefusal::Store(HistoryErrorKind::InternalState),
        );
    }
    let validated_database_path = match store.validated_database_path() {
        Ok(path) => path,
        Err(error) => {
            return AppDataResetCompositionOutcome::PreTerminalRefused(
                AppDataResetPreTerminalRefusal::Store(error.kind),
            );
        }
    };
    let Some(data_root) = validated_database_path.parent() else {
        return AppDataResetCompositionOutcome::PreTerminalRefused(
            AppDataResetPreTerminalRefusal::Store(HistoryErrorKind::InternalState),
        );
    };
    let transaction = match AppDataResetTransaction::generate() {
        Ok(transaction) => transaction,
        Err(error) => {
            return AppDataResetCompositionOutcome::PreTerminalRefused(
                AppDataResetPreTerminalRefusal::Transaction(error),
            );
        }
    };
    let shutdown = match engine.begin_app_data_reset_shutdown_until(deadline) {
        Err(()) => {
            return AppDataResetCompositionOutcome::PreTerminalRefused(
                AppDataResetPreTerminalRefusal::LifecycleBusy,
            );
        }
        Ok(AppDataResetAdmissionOutcome::Admitted(shutdown)) => shutdown,
        Ok(AppDataResetAdmissionOutcome::OrdinaryCloseWon) => {
            return AppDataResetCompositionOutcome::TerminalOwnedElsewhere(
                AppDataResetTerminalOwner::OrdinaryClose,
            );
        }
        Ok(AppDataResetAdmissionOutcome::AlreadyResetting) => {
            return AppDataResetCompositionOutcome::TerminalOwnedElsewhere(
                AppDataResetTerminalOwner::AppDataReset,
            );
        }
        Ok(AppDataResetAdmissionOutcome::InternalState) => {
            return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                AppDataResetPostTerminalRefusal::LifecycleInternalState,
            );
        }
    };
    let quiesced = match shutdown.wait_until_quiesced_until(deadline) {
        Ok(quiesced) => quiesced,
        Err(error) => {
            return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                AppDataResetPostTerminalRefusal::Shutdown(error),
            );
        }
    };

    let runtime_blockers = match inspect_runtime_blockers(deadline) {
        Ok(blockers) => blockers,
        Err(()) => {
            return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                AppDataResetPostTerminalRefusal::RuntimeInspectionBusy,
            );
        }
    };
    if !runtime_blockers.is_empty() {
        return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
            AppDataResetPostTerminalRefusal::RuntimeBlocked(runtime_blockers),
        );
    }

    let coordinator = match AppDataResetCoordinator::open_or_create_until(data_root, deadline) {
        Ok(coordinator) => coordinator,
        Err(error) => {
            return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                AppDataResetPostTerminalRefusal::Coordinator(error.kind()),
            );
        }
    };
    let mut admitted = Some(admitted);
    let outcome = coordinator.with_exclusive_session_until(deadline, |session| {
        if let Some(refusal) = validate_coordinator_preflight(session)? {
            let refusal = match refusal {
                AppDataResetPreTerminalRefusal::RecoveryRequired { phase } => {
                    AppDataResetPostTerminalRefusal::RecoveryRequired { phase }
                }
                AppDataResetPreTerminalRefusal::ProvisioningDebt {
                    unproven_stage_count,
                } => AppDataResetPostTerminalRefusal::ProvisioningDebt {
                    unproven_stage_count,
                },
                _ => AppDataResetPostTerminalRefusal::LifecycleInternalState,
            };
            return Ok(AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                refusal,
            ));
        }

        let data_outcome = session.with_data_namespace_admission_until(
            store,
            &transaction,
            deadline,
            |session, data_namespace| {
                let runtime_blockers = match inspect_runtime_blockers(deadline) {
                    Ok(blockers) => blockers,
                    Err(()) => {
                        return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                            AppDataResetPostTerminalRefusal::RuntimeInspectionBusy,
                        );
                    }
                };
                if !runtime_blockers.is_empty() {
                    return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                        AppDataResetPostTerminalRefusal::RuntimeBlocked(runtime_blockers),
                    );
                }
                let store_outcome =
                    session.with_admitted_store_until(store, deadline, |session, mut store| {
                        let runtime_blockers = match inspect_runtime_blockers(deadline) {
                            Ok(blockers) => blockers,
                            Err(()) => {
                                return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                                    AppDataResetPostTerminalRefusal::RuntimeInspectionBusy,
                                );
                            }
                        };
                        if !runtime_blockers.is_empty() {
                            return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                                AppDataResetPostTerminalRefusal::RuntimeBlocked(runtime_blockers),
                            );
                        }
                        let snapshot_outcome =
                            snapshots.with_app_data_reset_snapshot_admission(
                                &mut store,
                                deadline,
                                |store, snapshot| {
                                    managed_scan_cache.with_app_data_reset_admission(
                                        &transaction,
                                        deadline,
                                        |managed_cache| {
                                            let mut admission = AppDataResetCoreAdmission {
                                                store,
                                                data_namespace,
                                                snapshot,
                                                managed_cache,
                                                _quiesced: &quiesced,
                                                coordinator: session,
                                                deadline,
                                                inspect_runtime_blockers:
                                                    &inspect_runtime_blockers,
                                                transaction: &transaction,
                                            };
                                            if let Err(error) = admission.revalidate() {
                                                return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                                                    error,
                                                );
                                            }
                                            if Instant::now() >= deadline {
                                                return AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                                                    AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded,
                                                );
                                            }
                                            AppDataResetCompositionOutcome::Admitted(
                                                admitted.take().expect(
                                                    "reset admission callback is invoked at most once",
                                                )(admission),
                                            )
                                        },
                                    )
                                },
                            );
                        match snapshot_outcome {
                            Ok(Ok(outcome)) => outcome,
                            Ok(Err(error)) => {
                                AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                                    map_managed_cache_refusal(error),
                                )
                            }
                            Err(error) => {
                                AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                                    AppDataResetPostTerminalRefusal::Snapshot(error.kind),
                                )
                            }
                        }
                    });
                match store_outcome {
                    Ok(AppDataResetAdmittedStoreOutcome::Admitted(outcome)) => outcome,
                    Ok(AppDataResetAdmittedStoreOutcome::Blocked(blockers)) => {
                        AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                            AppDataResetPostTerminalRefusal::StoreBlocked(blockers),
                        )
                    }
                    Err(error) => AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                        AppDataResetPostTerminalRefusal::Store(error.kind),
                    ),
                }
            },
        );
        Ok(match data_outcome {
            Ok(outcome) => outcome,
            Err(error) => AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                AppDataResetPostTerminalRefusal::DataNamespace(error.kind),
            ),
        })
    });

    match outcome {
        Ok(outcome) => outcome,
        Err(error) => AppDataResetCompositionOutcome::TerminalWithoutAdmission(
            AppDataResetPostTerminalRefusal::Coordinator(error.kind()),
        ),
    }
}

fn map_managed_cache_refusal(
    error: AppDataResetManagedScanCacheError,
) -> AppDataResetPostTerminalRefusal {
    match error {
        AppDataResetManagedScanCacheError::Store(kind) => {
            AppDataResetPostTerminalRefusal::ManagedCache(kind)
        }
    }
}

fn validate_coordinator_preflight(
    session: &mut AppDataResetCoordinatorSession<'_>,
) -> Result<Option<AppDataResetPreTerminalRefusal>, AppDataResetCoordinatorError> {
    let journal = session.recover()?;
    if let Some(journal) = journal
        && journal.phase() != AppDataResetPhase::Complete
    {
        return Ok(Some(AppDataResetPreTerminalRefusal::RecoveryRequired {
            phase: journal.phase(),
        }));
    }
    let debt = session.provisioning_debt()?;
    if debt.unproven_stage_count() != 0 {
        return Ok(Some(AppDataResetPreTerminalRefusal::ProvisioningDebt {
            unproven_stage_count: debt.unproven_stage_count(),
        }));
    }
    Ok(None)
}
