use super::*;

impl SecureSnapshotStore {
    /// Descriptor-only open used while resuming an already-journaled app-data
    /// reset. The caller retains the database-root publication fence and
    /// cleanup/writer exclusion. This function never provisions or repairs a
    /// missing snapshot store and never resolves the database root by path.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn open_existing_for_app_data_reset_root(
        database_root_path: &Path,
        database_root: File,
    ) -> Result<Self> {
        if !database_root_path.is_absolute()
            || database_root_path.file_name().is_none()
            || database_root_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::InvalidConfiguration,
            ));
        }
        let database_root_identity = Identity(platform::identity(
            &database_root,
            platform::Kind::Directory,
        )?);
        platform::validate_retained(
            &database_root,
            database_root_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        let directory_path = database_root_path.join(DIRECTORY_NAME);
        let directory =
            platform::open_existing_directory(&database_root, database_root_path, DIRECTORY_NAME)?
                .ok_or_else(|| {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::UnrecognizedStore)
                })?;
        Self::from_open_directory(
            database_root_path.to_path_buf(),
            database_root,
            database_root_identity,
            directory_path,
            directory,
        )
    }

    /// Opens exactly `<database parent>/snapshots`.
    ///
    /// `database_path` is configuration, not a discovered path. The containing
    /// SQLite storage layer must remain open for at least this store's lifetime
    /// so its own retained root-replacement guards remain effective.
    /// Returns `None` when a read-only database has no snapshot store yet.
    /// Read-only access never provisions or repairs storage.
    pub(crate) fn open_for_database(
        database_path: &Path,
        access: SnapshotStoreAccess,
    ) -> Result<Option<Self>> {
        if !database_path.is_absolute()
            || database_path.file_name().is_none()
            || database_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::InvalidConfiguration,
            ));
        }
        let parent_path = database_path.parent().ok_or_else(|| {
            SnapshotStorageError::new(SnapshotStorageErrorKind::InvalidConfiguration)
        })?;
        let parent = platform::open_private_directory(parent_path)?;
        let parent_identity = Identity(platform::identity(&parent, platform::Kind::Directory)?);

        let directory_path = parent_path.join(DIRECTORY_NAME);
        platform::validate_retained(&parent, parent_identity.0, platform::Kind::Directory, false)?;

        let store = match platform::open_existing_directory(&parent, parent_path, DIRECTORY_NAME)? {
            Some(directory) => Self::from_open_directory(
                parent_path.to_path_buf(),
                parent,
                parent_identity,
                directory_path,
                directory,
            )?,
            None if access == SnapshotStoreAccess::ReadOnly => return Ok(None),
            None => Self::provision(parent_path, &parent, parent_identity, directory_path)?,
        };
        // Inventory is meaningful only while the permanent writer lock excludes
        // a legitimate publisher's current temporary file.
        let lock = store.acquire_writer_lock(OPEN_LOCK_TIMEOUT)?;
        store.validate_inventory(None)?;
        drop(lock);
        Ok(Some(store))
    }

    fn provision(
        database_root_path: &Path,
        database_root: &File,
        database_root_identity: Identity,
        directory_path: PathBuf,
    ) -> Result<Self> {
        Self::provision_with_before_publish(
            database_root_path,
            database_root,
            database_root_identity,
            directory_path,
            || {},
        )
    }

    pub(super) fn provision_with_before_publish(
        database_root_path: &Path,
        database_root: &File,
        database_root_identity: Identity,
        directory_path: PathBuf,
        before_publish: impl Fn(),
    ) -> Result<Self> {
        for _ in 0..RANDOM_ATTEMPTS {
            let stage_name = random_directory_stage_name()?;
            let Some(directory) = platform::create_private_directory_exclusive(
                database_root,
                database_root_path,
                &stage_name,
            )?
            else {
                continue;
            };
            let stage_path = database_root_path.join(&stage_name);
            let directory_identity =
                Identity(platform::identity(&directory, platform::Kind::Directory)?);
            let (marker, marker_identity) =
                create_control(&directory, &stage_path, MARKER_NAME, STORE_MARKER)?;
            let (writer_lock, writer_lock_identity) =
                create_control(&directory, &stage_path, WRITER_LOCK_NAME, WRITER_MARKER)?;
            platform::sync_directory(&directory)?;
            platform::validate_retained(
                database_root,
                database_root_identity.0,
                platform::Kind::Directory,
                false,
            )?;
            before_publish();
            match platform::publish_directory_no_replace(
                database_root,
                &stage_name,
                &directory,
                directory_identity.0,
                database_root,
                DIRECTORY_NAME,
            )? {
                platform::Publication::Published => {
                    platform::sync_directory(database_root)?;
                    platform::validate_named(
                        database_root,
                        DIRECTORY_NAME,
                        &directory,
                        directory_identity.0,
                        platform::Kind::Directory,
                    )?;
                    return Self::from_parts(
                        database_root_path.to_path_buf(),
                        database_root.try_clone().map_err(|_| {
                            SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable)
                        })?,
                        database_root_identity,
                        directory_path,
                        directory,
                        directory_identity,
                        marker,
                        marker_identity,
                        writer_lock,
                        writer_lock_identity,
                    );
                }
                platform::Publication::Collision => {
                    // The bounded marker-complete loser is deliberately not
                    // scavenged. It remains inside the database root that owns
                    // it and cannot be mistaken for a published snapshot store.
                    let directory = platform::open_existing_directory(
                        database_root,
                        database_root_path,
                        DIRECTORY_NAME,
                    )?
                    .ok_or_else(|| {
                        SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeRoot)
                    })?;
                    return Self::from_open_directory(
                        database_root_path.to_path_buf(),
                        database_root.try_clone().map_err(|_| {
                            SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable)
                        })?,
                        database_root_identity,
                        directory_path,
                        directory,
                    );
                }
            }
        }
        Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        ))
    }

    pub(super) fn from_open_directory(
        database_root_path: PathBuf,
        database_root: File,
        database_root_identity: Identity,
        path: PathBuf,
        directory: File,
    ) -> Result<Self> {
        let directory_identity =
            Identity(platform::identity(&directory, platform::Kind::Directory)?);
        platform::validate_retained(
            &directory,
            directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        let (marker, marker_identity) =
            open_control(&directory, &path, MARKER_NAME, STORE_MARKER, false)?.ok_or_else(
                || SnapshotStorageError::new(SnapshotStorageErrorKind::UnrecognizedStore),
            )?;
        let (writer_lock, writer_lock_identity) =
            open_control(&directory, &path, WRITER_LOCK_NAME, WRITER_MARKER, true)?
                .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject))?;
        Self::from_parts(
            database_root_path,
            database_root,
            database_root_identity,
            path,
            directory,
            directory_identity,
            marker,
            marker_identity,
            writer_lock,
            writer_lock_identity,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        database_root_path: PathBuf,
        database_root: File,
        database_root_identity: Identity,
        path: PathBuf,
        directory: File,
        directory_identity: Identity,
        marker: File,
        marker_identity: Identity,
        writer_lock: File,
        writer_lock_identity: Identity,
    ) -> Result<Self> {
        platform::validate_retained(
            &database_root,
            database_root_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                database_root_path,
                database_root,
                database_root_identity,
                path,
                directory,
                directory_identity,
                marker,
                marker_identity,
                writer_lock,
                writer_lock_identity,
                writer_in_use: AtomicBool::new(false),
            }),
        })
    }

    pub(crate) fn open(&self, name: &SnapshotFileName) -> Result<Option<RetainedSnapshot>> {
        Ok(self
            .open_with_writer_lease(name, OPEN_LOCK_TIMEOUT)?
            .map(SnapshotOpenLease::into_retained))
    }

    /// Simulate a malicious same-user replacement that ignores the advisory
    /// writer lease so higher-layer retention tests can exercise fail-closed
    /// post-validation behavior without exposing a production capability.
    #[cfg(test)]
    pub(in crate::persistence::snapshot) fn replace_final_for_test(
        &self,
        name: &SnapshotFileName,
        bytes: &[u8],
    ) -> Result<()> {
        let Some((current, identity)) = platform::open_named_final_for_removal(
            &self.inner.directory,
            &self.inner.path,
            name.as_str(),
        )?
        else {
            return Err(unsafe_inventory_object());
        };
        platform::remove_retained_final(&self.inner.directory, name.as_str(), current, identity)?;
        platform::sync_directory(&self.inner.directory)?;
        let Some((mut replacement, _)) = platform::create_private_file_exclusive(
            &self.inner.directory,
            &self.inner.path,
            name.as_str(),
        )?
        else {
            return Err(unsafe_inventory_object());
        };
        replacement
            .write_all(bytes)
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        replacement
            .sync_all()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        platform::sync_directory(&self.inner.directory)
    }

    /// Open one immutable snapshot while retaining the store-wide writer
    /// exclusion. Database-backed review pins use this to keep validation,
    /// pin insertion, and the retention tombstone boundary in one
    /// database-before-snapshot critical section.
    pub(crate) fn open_with_writer_lease(
        &self,
        name: &SnapshotFileName,
        timeout: Duration,
    ) -> Result<Option<SnapshotOpenLease>> {
        let lock = self.acquire_writer_lock(timeout)?;
        self.validate_inventory(None)?;
        let Some((file, identity)) = platform::open_named_regular(
            &self.inner.directory,
            &self.inner.path,
            name.as_str(),
            false,
        )?
        else {
            drop(lock);
            return Ok(None);
        };
        let retained = RetainedSnapshot {
            store: Arc::clone(&self.inner),
            name: name.clone(),
            file,
            identity: Identity(identity),
        };
        retained.revalidate()?;
        Ok(Some(SnapshotOpenLease {
            retained,
            _writer_lock: lock,
        }))
    }

    /// Observe every final and recognized temporary object in one bounded
    /// directory pass while retaining the store-wide writer exclusion.
    pub(crate) fn inventory_with_writer_lease(
        &self,
        timeout: Duration,
    ) -> Result<SnapshotStoreInventoryLease> {
        let writer_lock = self.acquire_writer_lock(timeout)?;
        self.inventory_with_retained_writer_lock(writer_lock)
    }

    pub(crate) fn inventory_with_writer_lease_until(
        &self,
        deadline: Instant,
    ) -> Result<SnapshotStoreInventoryLease> {
        let writer_lock = self.acquire_writer_lock_until(deadline)?;
        self.inventory_with_retained_writer_lock_until(writer_lock, deadline)
    }

    fn inventory_with_retained_writer_lock(
        &self,
        writer_lock: SnapshotWriterLock,
    ) -> Result<SnapshotStoreInventoryLease> {
        let deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))?;
        self.inventory_with_retained_writer_lock_until(writer_lock, deadline)
    }

    fn inventory_with_retained_writer_lock_until(
        &self,
        writer_lock: SnapshotWriterLock,
        deadline: Instant,
    ) -> Result<SnapshotStoreInventoryLease> {
        let inventory = self.inventory_locked_until(None, deadline)?;
        Ok(SnapshotStoreInventoryLease {
            store: Arc::clone(&self.inner),
            entries: inventory.entries,
            entries_usage: inventory.entries_usage,
            controls: inventory.controls,
            total_usage: inventory.total_usage,
            _writer_lock: writer_lock,
        })
    }

    /// Completely inventory the retained database root and durably remove at
    /// most one exact marker-owned provisioning stage. The database/root
    /// coordination lock that excludes a compliant provisioner is owned by
    /// the repository layer; this primitive contributes only bounded physical
    /// observation, exact ownership proof, and descriptor-relative mutation.
    pub(crate) fn reconcile_provisioning_stage(
        &self,
    ) -> std::result::Result<
        SnapshotProvisioningStageReconciliation,
        SnapshotProvisioningStageRemovalError,
    > {
        let before_effect = SnapshotProvisioningStageRemovalError::BeforeEffect;
        let inventory = self
            .inventory_provisioning_stages()
            .map_err(before_effect)?;
        let total_before = inventory.total_stage_count;
        let marker_before = inventory.marker_owned_count;
        let unproven_before = inventory.unproven_count;
        let usage_before = inventory.control_usage;
        let Some(stage) = inventory.first_marker_owned else {
            return Ok(SnapshotProvisioningStageReconciliation {
                outcome: if unproven_before == 0 {
                    SnapshotProvisioningStageRemoval::NoStage
                } else {
                    SnapshotProvisioningStageRemoval::DeferredUnproven
                },
                total_stage_count_before: total_before,
                total_stage_count_after: total_before,
                marker_owned_count_before: marker_before,
                marker_owned_count_after: marker_before,
                unproven_count_before: unproven_before,
                unproven_count_after: unproven_before,
                control_usage_before: usage_before,
                control_usage_after: usage_before,
                removed_control_usage: None,
                has_more: marker_before > 0,
            });
        };

        let outcome = match stage.shape {
            ProvisioningStageShape::MarkerOnly => {
                SnapshotProvisioningStageRemoval::RemovedMarkerOnly
            }
            ProvisioningStageShape::MarkerComplete => {
                SnapshotProvisioningStageRemoval::RemovedMarkerComplete
            }
        };
        // Freeze every reportable postcondition before the first unlink.
        let total_after = total_before.checked_sub(1).ok_or_else(|| {
            before_effect(SnapshotStorageError::new(
                SnapshotStorageErrorKind::InternalState,
            ))
        })?;
        let marker_after = marker_before.checked_sub(1).ok_or_else(|| {
            before_effect(SnapshotStorageError::new(
                SnapshotStorageErrorKind::InternalState,
            ))
        })?;
        let usage_after = usage_before
            .checked_sub(stage.control_usage)
            .map_err(before_effect)?;
        let removed_usage = stage.control_usage;
        self.remove_provisioning_stage(stage)?;
        Ok(SnapshotProvisioningStageReconciliation {
            outcome,
            total_stage_count_before: total_before,
            total_stage_count_after: total_after,
            marker_owned_count_before: marker_before,
            marker_owned_count_after: marker_after,
            unproven_count_before: unproven_before,
            unproven_count_after: unproven_before,
            control_usage_before: usage_before,
            control_usage_after: usage_after,
            removed_control_usage: Some(removed_usage),
            has_more: marker_after > 0,
        })
    }

    pub(super) fn inventory_provisioning_stages(&self) -> Result<ProvisioningStageInventory> {
        platform::validate_retained(
            &self.inner.database_root,
            self.inner.database_root_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        let deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))?;
        let mut names = platform::inventory_provisioning_stage_names(
            &self.inner.database_root,
            MAX_INVENTORY_ENTRIES,
            MAX_INVENTORY_NAME_BYTES,
            MAX_PROVISIONING_STAGES,
            deadline,
        )?;
        names.sort_unstable();

        let mut marker_owned_count = 0_u64;
        let mut unproven_count = 0_u64;
        let mut control_usage = SnapshotFileUsage::default();
        let mut first_marker_owned = None;
        for name in &names {
            if Instant::now() > deadline {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
            let Some(opened) = platform::open_provisioning_stage_for_removal(
                &self.inner.database_root,
                &self.inner.database_root_path,
                name,
            )?
            else {
                return Err(unsafe_inventory_object());
            };
            let ProvisioningStageOpen::Private(directory, directory_identity) = opened else {
                unproven_count = unproven_count
                    .checked_add(1)
                    .ok_or_else(unsafe_inventory_object)?;
                continue;
            };
            let children = platform::inventory(
                &directory,
                3,
                MARKER_NAME.len() + WRITER_LOCK_NAME.len() + 1,
                deadline,
            )?;
            if children.is_empty() {
                unproven_count = unproven_count
                    .checked_add(1)
                    .ok_or_else(unsafe_inventory_object)?;
                continue;
            }
            if children.len() > 2
                || children
                    .iter()
                    .any(|child| child != MARKER_NAME && child != WRITER_LOCK_NAME)
                || !children.iter().any(|child| child == MARKER_NAME)
            {
                return Err(unsafe_inventory_object());
            }

            let Some((marker, marker_identity)) = platform::open_stage_control_for_removal(
                &directory,
                &self.inner.database_root_path.join(name),
                MARKER_NAME,
            )?
            else {
                return Err(unsafe_inventory_object());
            };
            platform::validate_named(
                &directory,
                MARKER_NAME,
                &marker,
                marker_identity,
                platform::Kind::RegularFile,
            )?;
            prove_marker(&marker, STORE_MARKER)?;
            let marker_usage = snapshot_file_usage(&marker)?;

            let writer_lock = if children.iter().any(|child| child == WRITER_LOCK_NAME) {
                let Some((writer, writer_identity)) = platform::open_stage_control_for_removal(
                    &directory,
                    &self.inner.database_root_path.join(name),
                    WRITER_LOCK_NAME,
                )?
                else {
                    return Err(unsafe_inventory_object());
                };
                platform::validate_named(
                    &directory,
                    WRITER_LOCK_NAME,
                    &writer,
                    writer_identity,
                    platform::Kind::RegularFile,
                )?;
                prove_marker(&writer, WRITER_MARKER)?;
                Some((writer, writer_identity))
            } else {
                None
            };
            let usage = if let Some((writer, _)) = &writer_lock {
                marker_usage.checked_add(snapshot_file_usage(writer)?)?
            } else {
                marker_usage
            };
            control_usage = control_usage.checked_add(usage)?;
            marker_owned_count = marker_owned_count
                .checked_add(1)
                .ok_or_else(unsafe_inventory_object)?;
            if first_marker_owned.is_none() {
                first_marker_owned = Some(RetainedProvisioningStage {
                    name: name.clone(),
                    directory,
                    directory_identity,
                    marker,
                    marker_identity,
                    writer_lock,
                    shape: if children.len() == 2 {
                        ProvisioningStageShape::MarkerComplete
                    } else {
                        ProvisioningStageShape::MarkerOnly
                    },
                    control_usage: usage,
                });
            }
        }
        platform::validate_retained(
            &self.inner.database_root,
            self.inner.database_root_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        Ok(ProvisioningStageInventory {
            total_stage_count: u64::try_from(names.len()).map_err(|_| unsafe_inventory_object())?,
            marker_owned_count,
            unproven_count,
            control_usage,
            first_marker_owned,
        })
    }

    fn remove_provisioning_stage(
        &self,
        stage: RetainedProvisioningStage,
    ) -> std::result::Result<(), SnapshotProvisioningStageRemovalError> {
        self.remove_provisioning_stage_with_hooks(
            stage,
            || Ok(()),
            platform::sync_directory,
            platform::sync_directory,
        )
    }

    pub(super) fn remove_provisioning_stage_with_hooks(
        &self,
        stage: RetainedProvisioningStage,
        before_first_remove: impl FnOnce() -> Result<()>,
        mut sync_stage: impl FnMut(&File) -> Result<()>,
        sync_root: impl FnOnce(&File) -> Result<()>,
    ) -> std::result::Result<(), SnapshotProvisioningStageRemovalError> {
        let before_effect = SnapshotProvisioningStageRemovalError::BeforeEffect;
        platform::validate_retained(
            &self.inner.database_root,
            self.inner.database_root_identity.0,
            platform::Kind::Directory,
            false,
        )
        .map_err(before_effect)?;
        platform::validate_named(
            &self.inner.database_root,
            &stage.name,
            &stage.directory,
            stage.directory_identity,
            platform::Kind::Directory,
        )
        .map_err(before_effect)?;
        before_first_remove().map_err(before_effect)?;

        let mut effect_started = false;
        if let Some((writer, writer_identity)) = stage.writer_lock {
            platform::remove_retained_stage_control(
                &stage.directory,
                WRITER_LOCK_NAME,
                writer,
                writer_identity,
            )
            .map_err(before_effect)?;
            effect_started = true;
            sync_stage(&stage.directory)
                .map_err(|_| SnapshotProvisioningStageRemovalError::OutcomeUnknown)?;
        }
        let marker_result = platform::remove_retained_stage_control(
            &stage.directory,
            MARKER_NAME,
            stage.marker,
            stage.marker_identity,
        );
        if let Err(error) = marker_result {
            return Err(if effect_started {
                SnapshotProvisioningStageRemovalError::OutcomeUnknown
            } else {
                before_effect(error)
            });
        }
        sync_stage(&stage.directory)
            .map_err(|_| SnapshotProvisioningStageRemovalError::OutcomeUnknown)?;
        platform::remove_retained_provisioning_stage(
            &self.inner.database_root,
            &stage.name,
            stage.directory,
            stage.directory_identity,
        )
        .map_err(|_| SnapshotProvisioningStageRemovalError::OutcomeUnknown)?;
        sync_root(&self.inner.database_root)
            .map_err(|_| SnapshotProvisioningStageRemovalError::OutcomeUnknown)
    }

    pub(crate) fn reserve_stage(
        &self,
        name: SnapshotFileName,
        timeout: Duration,
    ) -> Result<SnapshotStageReservation> {
        let lock = self.acquire_writer_lock(timeout)?;
        let inventory = self.inventory_locked(None)?;
        let existing_temps = inventory
            .entries
            .iter()
            .filter(|entry| matches!(entry.kind, SnapshotInventoryEntryKind::RecognizedTemp))
            .count();
        if existing_temps >= MAX_RECOGNIZED_TEMPS {
            return Err(unsafe_inventory_object());
        }
        for _ in 0..RANDOM_ATTEMPTS {
            let temp_name = random_temp_name(&name)?;
            if inventory
                .entries
                .iter()
                .any(|entry| entry.name == temp_name)
            {
                continue;
            }
            return Ok(SnapshotStageReservation {
                store: Arc::clone(&self.inner),
                final_name: name,
                temp_name,
                lock_timeout: timeout,
                created: false,
                _writer_lock: lock,
            });
        }
        Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        ))
    }

    #[cfg(test)]
    pub(crate) fn stage(
        &self,
        name: SnapshotFileName,
        timeout: Duration,
    ) -> Result<StagedSnapshot> {
        let mut reservation = self.reserve_stage(name, timeout)?;
        reservation.create()
    }

    pub(super) fn acquire_writer_lock(&self, timeout: Duration) -> Result<SnapshotWriterLock> {
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            SnapshotStorageError::new(SnapshotStorageErrorKind::InvalidConfiguration)
        })?;
        self.acquire_writer_lock_until_mode(deadline, true)
    }

    fn acquire_writer_lock_until(&self, deadline: Instant) -> Result<SnapshotWriterLock> {
        self.acquire_writer_lock_until_mode(deadline, false)
    }

    fn acquire_writer_lock_until_mode(
        &self,
        deadline: Instant,
        allow_expired_initial_try: bool,
    ) -> Result<SnapshotWriterLock> {
        if !allow_expired_initial_try && Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        if self
            .inner
            .writer_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        if let Err(error) = self.validate_controls() {
            self.inner.writer_in_use.store(false, Ordering::Release);
            return Err(error);
        }
        let file = self
            .inner
            .writer_lock
            .try_clone()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable));
        let file = match file {
            Ok(file) => file,
            Err(error) => {
                self.inner.writer_in_use.store(false, Ordering::Release);
                return Err(error);
            }
        };
        let mut first_attempt = true;
        loop {
            if Instant::now() >= deadline && !(allow_expired_initial_try && first_attempt) {
                self.inner.writer_in_use.store(false, Ordering::Release);
                return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
            }
            first_attempt = false;
            match FileExt::try_lock(&file) {
                Ok(()) if !allow_expired_initial_try && Instant::now() >= deadline => {
                    record_writer_unlock(&self.inner.writer_in_use, FileExt::unlock(&file).is_ok());
                    return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
                }
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(
                        LOCK_RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(TryLockError::WouldBlock) => {
                    self.inner.writer_in_use.store(false, Ordering::Release);
                    return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
                }
                Err(TryLockError::Error(_)) => {
                    self.inner.writer_in_use.store(false, Ordering::Release);
                    return Err(SnapshotStorageError::new(
                        SnapshotStorageErrorKind::Unavailable,
                    ));
                }
            }
        }
        if let Err(error) = self.validate_controls() {
            record_writer_unlock(&self.inner.writer_in_use, FileExt::unlock(&file).is_ok());
            return Err(error);
        }
        if !allow_expired_initial_try && Instant::now() >= deadline {
            record_writer_unlock(&self.inner.writer_in_use, FileExt::unlock(&file).is_ok());
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        Ok(SnapshotWriterLock {
            store: Arc::clone(&self.inner),
            file,
        })
    }

    pub(super) fn validate_controls(&self) -> Result<()> {
        platform::validate_retained(
            &self.inner.database_root,
            self.inner.database_root_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        platform::validate_named(
            &self.inner.database_root,
            DIRECTORY_NAME,
            &self.inner.directory,
            self.inner.directory_identity.0,
            platform::Kind::Directory,
        )?;
        platform::validate_retained(
            &self.inner.directory,
            self.inner.directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        for (name, file, identity, marker) in [
            (
                MARKER_NAME,
                &self.inner.marker,
                self.inner.marker_identity,
                STORE_MARKER,
            ),
            (
                WRITER_LOCK_NAME,
                &self.inner.writer_lock,
                self.inner.writer_lock_identity,
                WRITER_MARKER,
            ),
        ] {
            platform::validate_retained(file, identity.0, platform::Kind::RegularFile, true)?;
            platform::validate_named(
                &self.inner.directory,
                name,
                file,
                identity.0,
                platform::Kind::RegularFile,
            )?;
            prove_marker(file, marker)?;
        }
        Ok(())
    }

    pub(super) fn validate_inventory(&self, allowed_temp: Option<&str>) -> Result<()> {
        self.inventory_locked(allowed_temp).map(drop)
    }

    fn inventory_locked(&self, allowed_temp: Option<&str>) -> Result<SnapshotStorageInventory> {
        let deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))?;
        self.inventory_locked_until(allowed_temp, deadline)
    }

    pub(super) fn inventory_locked_until(
        &self,
        allowed_temp: Option<&str>,
        deadline: Instant,
    ) -> Result<SnapshotStorageInventory> {
        self.inventory_locked_until_with_locked_temp(allowed_temp, None, deadline)
    }

    pub(super) fn inventory_locked_until_with_locked_temp(
        &self,
        allowed_temp: Option<&str>,
        locked_temp: Option<&SnapshotInventoryEntry>,
        caller_deadline: Instant,
    ) -> Result<SnapshotStorageInventory> {
        if Instant::now() >= caller_deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        self.validate_controls()
            .map_err(|error| prefer_snapshot_inventory_deadline(error, caller_deadline))?;
        let local_deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))?;
        let deadline = caller_deadline.min(local_deadline);
        if Instant::now() >= deadline {
            return Err(snapshot_inventory_deadline_error(caller_deadline));
        }
        let marker_usage = snapshot_file_usage(&self.inner.marker)
            .map_err(|error| prefer_snapshot_inventory_deadline(error, caller_deadline))?;
        let writer_usage = snapshot_file_usage(&self.inner.writer_lock)
            .map_err(|error| prefer_snapshot_inventory_deadline(error, caller_deadline))?;
        let controls_total = marker_usage.checked_add(writer_usage)?;
        let controls = SnapshotControlUsage {
            store_marker: marker_usage,
            writer_lock: writer_usage,
            total: controls_total,
        };
        let mut entries = Vec::new();
        let mut entries_usage = SnapshotFileUsage::default();
        let mut recognized_temps = 0_usize;
        let inventory = platform::inventory(
            &self.inner.directory,
            MAX_INVENTORY_ENTRIES,
            MAX_INVENTORY_NAME_BYTES,
            deadline,
        )
        .map_err(|error| {
            if Instant::now() >= caller_deadline {
                snapshot_inventory_deadline_error(caller_deadline)
            } else {
                error
            }
        })?;
        let mut locked_temp_seen = locked_temp.is_none();
        for name in inventory {
            if Instant::now() >= deadline {
                return Err(snapshot_inventory_deadline_error(caller_deadline));
            }
            if name == MARKER_NAME || name == WRITER_LOCK_NAME {
                continue;
            }
            let kind = if is_recognized_temp_name(&name) {
                recognized_temps = recognized_temps.checked_add(1).ok_or_else(|| {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
                })?;
                if recognized_temps > MAX_RECOGNIZED_TEMPS {
                    return Err(SnapshotStorageError::new(
                        SnapshotStorageErrorKind::UnsafeObject,
                    ));
                }
                SnapshotInventoryEntryKind::RecognizedTemp
            } else {
                SnapshotInventoryEntryKind::Final(SnapshotFileName::parse(&name)?)
            };
            if allowed_temp.is_some_and(|allowed| name == allowed) {
                if !matches!(kind, SnapshotInventoryEntryKind::RecognizedTemp) {
                    return Err(unsafe_inventory_object());
                }
                continue;
            }
            let Some((file, identity)) =
                platform::open_named_regular(&self.inner.directory, &self.inner.path, &name, false)
                    .map_err(|error| prefer_snapshot_inventory_deadline(error, caller_deadline))?
            else {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::UnsafeObject,
                ));
            };
            platform::validate_named(
                &self.inner.directory,
                &name,
                &file,
                identity,
                platform::Kind::RegularFile,
            )
            .map_err(|error| prefer_snapshot_inventory_deadline(error, caller_deadline))?;
            let usage = snapshot_file_usage(&file)
                .map_err(|error| prefer_snapshot_inventory_deadline(error, caller_deadline))?;
            let temp_kernel_state = if matches!(kind, SnapshotInventoryEntryKind::RecognizedTemp) {
                if let Some(locked) = locked_temp.filter(|locked| locked.name == name) {
                    if locked.kind != SnapshotInventoryEntryKind::RecognizedTemp
                        || locked.identity != Identity(identity)
                        || locked.usage != usage
                        || locked.temp_kernel_state != Some(SnapshotTempKernelState::Quiescent)
                    {
                        return Err(unsafe_inventory_object());
                    }
                    locked_temp_seen = true;
                    Some(SnapshotTempKernelState::Quiescent)
                } else {
                    Some(probe_temp_kernel_state(&file).map_err(|error| {
                        prefer_snapshot_inventory_deadline(error, caller_deadline)
                    })?)
                }
            } else {
                None
            };
            if matches!(kind, SnapshotInventoryEntryKind::Final(_))
                && usage.logical_bytes() > MAX_SNAPSHOT_FILE_BYTES
            {
                return Err(unsafe_inventory_object());
            }
            entries_usage = entries_usage.checked_add(usage)?;
            entries.push(SnapshotInventoryEntry {
                name,
                kind,
                identity: Identity(identity),
                usage,
                temp_kernel_state,
            });
            if Instant::now() >= deadline {
                return Err(snapshot_inventory_deadline_error(caller_deadline));
            }
        }
        if !locked_temp_seen {
            return Err(unsafe_inventory_object());
        }
        if Instant::now() >= deadline {
            return Err(snapshot_inventory_deadline_error(caller_deadline));
        }
        let total_usage = controls.total().checked_add(entries_usage)?;
        Ok(SnapshotStorageInventory {
            entries,
            entries_usage,
            controls,
            total_usage,
        })
    }
}
