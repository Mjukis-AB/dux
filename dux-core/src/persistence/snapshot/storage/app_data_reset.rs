use super::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetSnapshotRecovery {
    /// Open the detached old snapshot store without provisioning it. Structural
    /// tail states are accepted only for an already-durable `Draining` pass.
    pub(crate) fn open_until(
        database_root_path: &Path,
        database_root: File,
        allow_structural_tail: bool,
        deadline: Instant,
    ) -> Result<Self> {
        if Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        validate_app_data_reset_database_root_path(database_root_path)?;
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
        let Some(directory) =
            platform::open_existing_directory(&database_root, database_root_path, DIRECTORY_NAME)?
        else {
            if !allow_structural_tail {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::UnrecognizedStore,
                ));
            }
            let recovery = Self {
                inner: AppDataResetSnapshotRecoveryInner::Absent(AppDataResetSnapshotAbsentStore {
                    database_root_path: database_root_path.to_path_buf(),
                    database_root,
                    database_root_identity,
                }),
                state: Some(AppDataResetSnapshotStoreRetirementState::Absent),
                deadline,
            };
            recovery.revalidate_until(deadline)?;
            return Ok(recovery);
        };

        let directory_identity =
            Identity(platform::identity(&directory, platform::Kind::Directory)?);
        validate_app_data_reset_snapshot_directory(
            &database_root,
            database_root_identity,
            &directory,
            directory_identity,
            deadline,
        )?;
        let names = app_data_reset_snapshot_retirement_inventory_names(&directory, deadline)?;
        let has_marker = names.iter().any(|name| name == MARKER_NAME);
        let has_writer = names.iter().any(|name| name == WRITER_LOCK_NAME);

        let recovery = match (has_marker, has_writer) {
            (true, true) => {
                let store = SecureSnapshotStore::from_open_directory(
                    database_root_path.to_path_buf(),
                    database_root,
                    database_root_identity,
                    directory_path,
                    directory,
                )?;
                let inventory = store.inventory_with_writer_lease_until(deadline)?;
                inventory.revalidate_complete_for_app_data_reset_until(deadline)?;
                let state = inventory
                    .entries
                    .is_empty()
                    .then_some(AppDataResetSnapshotStoreRetirementState::FullControlsEmpty);
                Self {
                    inner: AppDataResetSnapshotRecoveryInner::Full(inventory),
                    state,
                    deadline,
                }
            }
            (false, true) if allow_structural_tail && names.as_slice() == [WRITER_LOCK_NAME] => {
                let (writer, writer_identity) = open_control(
                    &directory,
                    &directory_path,
                    WRITER_LOCK_NAME,
                    WRITER_MARKER,
                    true,
                )?
                .ok_or_else(unsafe_inventory_object)?;
                acquire_app_data_reset_snapshot_retirement_writer_lock(&writer, deadline)?;
                if !platform::same_filesystem(directory_identity.0, writer_identity.0) {
                    return Err(unsafe_inventory_object());
                }
                Self {
                    inner: AppDataResetSnapshotRecoveryInner::Partial(
                        AppDataResetSnapshotPartialRetirementStore {
                            database_root_path: database_root_path.to_path_buf(),
                            database_root,
                            database_root_identity,
                            directory,
                            directory_identity,
                            writer: Some(AppDataResetSnapshotRetirementControl {
                                file: writer,
                                identity: writer_identity,
                            }),
                        },
                    ),
                    state: Some(AppDataResetSnapshotStoreRetirementState::WriterOnly),
                    deadline,
                }
            }
            (false, false) if allow_structural_tail && names.is_empty() => Self {
                inner: AppDataResetSnapshotRecoveryInner::Partial(
                    AppDataResetSnapshotPartialRetirementStore {
                        database_root_path: database_root_path.to_path_buf(),
                        database_root,
                        database_root_identity,
                        directory,
                        directory_identity,
                        writer: None,
                    },
                ),
                state: Some(AppDataResetSnapshotStoreRetirementState::EmptyDirectory),
                deadline,
            },
            // Marker-only and every other partial or payload-without-controls
            // shape are not states this monotonic protocol can produce.
            _ => return Err(unsafe_inventory_object()),
        };
        recovery.revalidate_until(deadline)?;
        Ok(recovery)
    }

    pub(crate) const fn retirement_state(
        &self,
    ) -> Option<AppDataResetSnapshotStoreRetirementState> {
        self.state
    }

    pub(crate) fn payload_drain_candidate_until(
        &self,
        deadline: Instant,
    ) -> Result<Option<AppDataResetSnapshotPayloadDrainCandidate>> {
        match &self.inner {
            AppDataResetSnapshotRecoveryInner::Full(inventory) => {
                inventory.app_data_reset_payload_drain_candidate_until(deadline)
            }
            AppDataResetSnapshotRecoveryInner::Partial(_)
            | AppDataResetSnapshotRecoveryInner::Absent(_) => Ok(None),
        }
    }

    pub(crate) fn revalidate_payload_candidate_until(
        &self,
        candidate: &AppDataResetSnapshotPayloadDrainCandidate,
        deadline: Instant,
    ) -> Result<()> {
        match &self.inner {
            AppDataResetSnapshotRecoveryInner::Full(inventory) => {
                candidate.revalidate_against_until(inventory, deadline)
            }
            AppDataResetSnapshotRecoveryInner::Partial(_)
            | AppDataResetSnapshotRecoveryInner::Absent(_) => Err(unsafe_inventory_object()),
        }
    }

    pub(crate) fn drain_one_payload(
        &mut self,
        candidate: AppDataResetSnapshotPayloadDrainCandidate,
        authority: AppDataResetSnapshotPayloadDrainAuthority,
    ) -> std::result::Result<
        AppDataResetSnapshotPayloadDrainCompletion,
        AppDataResetSnapshotPayloadDrainError,
    > {
        let completion = match &mut self.inner {
            AppDataResetSnapshotRecoveryInner::Full(inventory) => {
                inventory.drain_one_app_data_reset_payload(candidate, authority)?
            }
            AppDataResetSnapshotRecoveryInner::Partial(_)
            | AppDataResetSnapshotRecoveryInner::Absent(_) => {
                return Err(AppDataResetSnapshotPayloadDrainError::BeforeEffect(
                    SnapshotStorageErrorKind::InternalState,
                ));
            }
        };

        // The inventory mutates only after unlink and directory sync both
        // succeed. Advance the retained recovery typestate at that same
        // certainty boundary so removing the final payload can be read back as
        // `FullControlsEmpty` instead of a false outcome-unknown result.
        self.state = match &self.inner {
            AppDataResetSnapshotRecoveryInner::Full(inventory) => inventory
                .entries
                .is_empty()
                .then_some(AppDataResetSnapshotStoreRetirementState::FullControlsEmpty),
            AppDataResetSnapshotRecoveryInner::Partial(_)
            | AppDataResetSnapshotRecoveryInner::Absent(_) => unreachable!(),
        };
        Ok(completion)
    }

    pub(crate) fn revalidate_until(&self, deadline: Instant) -> Result<()> {
        let deadline = deadline.min(self.deadline);
        self.revalidate_at_until(deadline)
    }

    /// Repeat the exact post-payload state under the newly minted certainty
    /// budget. This deliberately does not clip that budget to the admission
    /// deadline, which has already authorized and bounded the completed
    /// effect; it grants no further mutation authority.
    pub(crate) fn revalidate_after_payload_effect_until(&self, deadline: Instant) -> Result<()> {
        self.revalidate_at_until(deadline)
    }

    fn revalidate_at_until(&self, deadline: Instant) -> Result<()> {
        if Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        match &self.inner {
            AppDataResetSnapshotRecoveryInner::Full(inventory) => {
                inventory.revalidate_complete_for_app_data_reset_until(deadline)?;
                let expected_state = inventory
                    .entries
                    .is_empty()
                    .then_some(AppDataResetSnapshotStoreRetirementState::FullControlsEmpty);
                if self.state != expected_state {
                    return Err(unsafe_inventory_object());
                }
            }
            AppDataResetSnapshotRecoveryInner::Partial(store) => {
                validate_app_data_reset_snapshot_partial_store(store, self.state, deadline)?;
            }
            AppDataResetSnapshotRecoveryInner::Absent(store) => {
                if self.state != Some(AppDataResetSnapshotStoreRetirementState::Absent) {
                    return Err(unsafe_inventory_object());
                }
                validate_app_data_reset_snapshot_absence(store, deadline)?;
            }
        }
        if Instant::now() >= deadline {
            Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy))
        } else {
            Ok(())
        }
    }

    #[allow(clippy::disallowed_methods)]
    fn revalidate_structural_final_gate_until(&self, deadline: Instant) -> Result<()> {
        #[cfg(test)]
        if take_test_reset_store_retire_fault(
            TEST_FAULT_RESET_STORE_RETIRE_CASE_ALIAS_AT_FINAL_GATE,
        ) {
            let database_root_path = match &self.inner {
                AppDataResetSnapshotRecoveryInner::Full(inventory) => {
                    &inventory.store.database_root_path
                }
                AppDataResetSnapshotRecoveryInner::Partial(store) => &store.database_root_path,
                AppDataResetSnapshotRecoveryInner::Absent(store) => &store.database_root_path,
            };
            let intermediate = database_root_path.join(".dux-snapshot-case-alias-test");
            // DUX-DESTRUCTIVE: allow=test-reset-snapshot-case-alias-intermediate-rename -- test-only final-gate seam moves the exact retained snapshots directory through a private intermediate spelling so case-folding filesystems cannot collapse the interposition to a no-op
            std::fs::rename(database_root_path.join(DIRECTORY_NAME), &intermediate)
                .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
            // DUX-DESTRUCTIVE: allow=test-reset-snapshot-case-alias-final-rename -- test-only final-gate seam installs the case-only alias for the same retained snapshots directory before exact raw-name revalidation and never grants production rename authority
            std::fs::rename(&intermediate, database_root_path.join("Snapshots"))
                .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        }
        if take_test_reset_store_retire_fault(
            TEST_FAULT_RESET_STORE_RETIRE_FOREIGN_FILESYSTEM_AT_FINAL_GATE,
        ) {
            return Err(unsafe_inventory_object());
        }
        self.revalidate_until(deadline)?;
        #[cfg(test)]
        if take_test_reset_store_retire_fault(
            TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_DEADLINE_AT_FINAL_GATE,
        ) {
            exhaust_test_deadline(deadline);
        }
        if Instant::now() >= deadline {
            Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy))
        } else {
            Ok(())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetSnapshotRecovery {
    /// Retire exactly one structure from the old snapshot store. The opaque
    /// authority is minted only after the coordinator re-joins the exact
    /// `Draining` journal, old/fresh roots, snapshot state, and cache absence.
    pub(crate) fn retire_one_structure(
        &mut self,
        authority: AppDataResetSnapshotStoreRetireAuthority,
    ) -> std::result::Result<
        AppDataResetSnapshotStoreRetirementCompletion,
        AppDataResetSnapshotStoreRetirementError,
    > {
        let pre_effect_deadline = authority.pre_effect_deadline();
        let before_effect = |error: SnapshotStorageError| {
            AppDataResetSnapshotStoreRetirementError::BeforeEffect(error.kind())
        };
        if pre_effect_deadline != self.deadline {
            return Err(AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                SnapshotStorageErrorKind::InternalState,
            ));
        }
        #[cfg(test)]
        if take_test_reset_store_retire_fault(
            TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_PRE_EFFECT_DEADLINE,
        ) {
            exhaust_test_deadline(pre_effect_deadline);
        }
        self.revalidate_until(pre_effect_deadline)
            .map_err(before_effect)?;
        if take_test_reset_store_retire_fault(TEST_FAULT_RESET_STORE_RETIRE_BEFORE_EFFECT) {
            return Err(AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                SnapshotStorageErrorKind::Unavailable,
            ));
        }
        let state = self
            .state
            .ok_or(AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                SnapshotStorageErrorKind::InternalState,
            ))?;
        if state == AppDataResetSnapshotStoreRetirementState::Absent {
            return Err(AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                SnapshotStorageErrorKind::InternalState,
            ));
        }

        match &self.inner {
            AppDataResetSnapshotRecoveryInner::Full(inventory)
                if state == AppDataResetSnapshotStoreRetirementState::FullControlsEmpty =>
            {
                let marker = inventory.store.marker.try_clone().map_err(|_| {
                    AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                        SnapshotStorageErrorKind::Unavailable,
                    )
                })?;
                platform::remove_app_data_reset_snapshot_control_with_before_unlink(
                    &inventory.store.directory,
                    MARKER_NAME,
                    marker,
                    inventory.store.marker_identity.0,
                    || self.revalidate_structural_final_gate_until(pre_effect_deadline),
                )
                .map_err(before_effect)?;
                let post_effect_deadline =
                    app_data_reset_snapshot_retirement_post_effect_deadline()?;
                if take_test_reset_store_retire_fault(TEST_FAULT_RESET_STORE_RETIRE_AFTER_EFFECT) {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                platform::sync_directory(&inventory.store.directory)
                    .map_err(|_| AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)?;
                if take_test_reset_store_retire_fault(
                    TEST_FAULT_RESET_STORE_RETIRE_AFTER_DIRECTORY_SYNC,
                ) {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                if take_test_reset_store_retire_fault(TEST_FAULT_RESET_STORE_RETIRE_DURING_READBACK)
                {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                #[cfg(test)]
                if take_test_reset_store_retire_fault(
                    TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_POST_EFFECT_DEADLINE,
                ) {
                    exhaust_test_deadline(post_effect_deadline);
                }
                validate_app_data_reset_snapshot_writer_only_after_marker(
                    inventory,
                    post_effect_deadline,
                )
                .map_err(|_| AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)?;
                Ok(AppDataResetSnapshotStoreRetirementCompletion {
                    progress: AppDataResetSnapshotStoreRetirementBatch {
                        removed_structural_objects: 1,
                        snapshot_store_has_more: true,
                    },
                    post_effect_deadline,
                })
            }
            AppDataResetSnapshotRecoveryInner::Partial(store)
                if state == AppDataResetSnapshotStoreRetirementState::WriterOnly =>
            {
                let writer = store.writer.as_ref().ok_or(
                    AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                        SnapshotStorageErrorKind::InternalState,
                    ),
                )?;
                let writer_file = writer.file.try_clone().map_err(|_| {
                    AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                        SnapshotStorageErrorKind::Unavailable,
                    )
                })?;
                platform::remove_app_data_reset_snapshot_control_with_before_unlink(
                    &store.directory,
                    WRITER_LOCK_NAME,
                    writer_file,
                    writer.identity.0,
                    || self.revalidate_structural_final_gate_until(pre_effect_deadline),
                )
                .map_err(before_effect)?;
                let post_effect_deadline =
                    app_data_reset_snapshot_retirement_post_effect_deadline()?;
                if take_test_reset_store_retire_fault(TEST_FAULT_RESET_STORE_RETIRE_AFTER_EFFECT) {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                platform::sync_directory(&store.directory)
                    .map_err(|_| AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)?;
                if take_test_reset_store_retire_fault(
                    TEST_FAULT_RESET_STORE_RETIRE_AFTER_DIRECTORY_SYNC,
                ) {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                if take_test_reset_store_retire_fault(TEST_FAULT_RESET_STORE_RETIRE_DURING_READBACK)
                {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                #[cfg(test)]
                if take_test_reset_store_retire_fault(
                    TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_POST_EFFECT_DEADLINE,
                ) {
                    exhaust_test_deadline(post_effect_deadline);
                }
                validate_app_data_reset_snapshot_empty_after_writer(store, post_effect_deadline)
                    .map_err(|_| AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)?;
                Ok(AppDataResetSnapshotStoreRetirementCompletion {
                    progress: AppDataResetSnapshotStoreRetirementBatch {
                        removed_structural_objects: 1,
                        snapshot_store_has_more: true,
                    },
                    post_effect_deadline,
                })
            }
            AppDataResetSnapshotRecoveryInner::Partial(store)
                if state == AppDataResetSnapshotStoreRetirementState::EmptyDirectory =>
            {
                let directory = store.directory.try_clone().map_err(|_| {
                    AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                        SnapshotStorageErrorKind::Unavailable,
                    )
                })?;
                platform::remove_app_data_reset_snapshot_directory_with_before_unlink(
                    &store.database_root,
                    DIRECTORY_NAME,
                    directory,
                    store.directory_identity.0,
                    || self.revalidate_structural_final_gate_until(pre_effect_deadline),
                )
                .map_err(before_effect)?;
                let post_effect_deadline =
                    app_data_reset_snapshot_retirement_post_effect_deadline()?;
                if take_test_reset_store_retire_fault(TEST_FAULT_RESET_STORE_RETIRE_AFTER_EFFECT) {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                platform::sync_directory(&store.database_root)
                    .map_err(|_| AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)?;
                if take_test_reset_store_retire_fault(
                    TEST_FAULT_RESET_STORE_RETIRE_AFTER_DIRECTORY_SYNC,
                ) {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                if take_test_reset_store_retire_fault(TEST_FAULT_RESET_STORE_RETIRE_DURING_READBACK)
                {
                    return Err(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown);
                }
                #[cfg(test)]
                if take_test_reset_store_retire_fault(
                    TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_POST_EFFECT_DEADLINE,
                ) {
                    exhaust_test_deadline(post_effect_deadline);
                }
                let absent = AppDataResetSnapshotAbsentStore {
                    database_root_path: store.database_root_path.clone(),
                    database_root: store
                        .database_root
                        .try_clone()
                        .map_err(|_| AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)?,
                    database_root_identity: store.database_root_identity,
                };
                validate_app_data_reset_snapshot_absence(&absent, post_effect_deadline)
                    .map_err(|_| AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)?;
                Ok(AppDataResetSnapshotStoreRetirementCompletion {
                    progress: AppDataResetSnapshotStoreRetirementBatch {
                        removed_structural_objects: 1,
                        snapshot_store_has_more: false,
                    },
                    post_effect_deadline,
                })
            }
            AppDataResetSnapshotRecoveryInner::Full(_)
            | AppDataResetSnapshotRecoveryInner::Partial(_)
            | AppDataResetSnapshotRecoveryInner::Absent(_) => {
                Err(AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                    SnapshotStorageErrorKind::InternalState,
                ))
            }
        }
    }
}
