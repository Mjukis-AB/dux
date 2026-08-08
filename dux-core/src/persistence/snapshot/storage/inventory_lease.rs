use super::*;

impl SnapshotStoreInventoryLease {
    pub(crate) fn entries(&self) -> &[SnapshotInventoryEntry] {
        &self.entries
    }

    pub(crate) const fn entries_usage(&self) -> SnapshotFileUsage {
        self.entries_usage
    }

    pub(crate) const fn control_usage(&self) -> SnapshotControlUsage {
        self.controls
    }

    pub(crate) const fn total_usage(&self) -> SnapshotFileUsage {
        self.total_usage
    }

    /// Select the lexicographically first final or quiescent recognized temp
    /// from this complete retained inventory. An active temp can publish after
    /// the store writer is released, so any active observation refuses the
    /// whole reset batch. Empty is an explicit no-candidate result.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn app_data_reset_payload_drain_candidate_until(
        &self,
        deadline: Instant,
    ) -> Result<Option<AppDataResetSnapshotPayloadDrainCandidate>> {
        self.revalidate_complete_for_app_data_reset_until(deadline)?;
        let Some(selected) = self
            .entries
            .iter()
            .min_by(|left, right| left.name.cmp(&right.name))
        else {
            return Ok(None);
        };
        let candidate = AppDataResetSnapshotPayloadDrainCandidate {
            store: Arc::clone(&self.store),
            selected: selected.clone(),
        };
        if candidate.selected_is_safe() {
            Ok(Some(candidate))
        } else {
            Err(unsafe_inventory_object())
        }
    }

    /// Remove at most one exact payload selected from this lease, then sync
    /// and exactly re-inventory the snapshot directory. The candidate is
    /// consumed by every call. Production callers must also consume the
    /// coordinator-only authority minted from the durable `Draining` join.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn drain_one_app_data_reset_payload(
        &mut self,
        candidate: AppDataResetSnapshotPayloadDrainCandidate,
        authority: AppDataResetSnapshotPayloadDrainAuthority,
    ) -> std::result::Result<
        AppDataResetSnapshotPayloadDrainCompletion,
        AppDataResetSnapshotPayloadDrainError,
    > {
        let pre_effect_deadline = authority.pre_effect_deadline();
        let before_effect = |error: SnapshotStorageError| {
            AppDataResetSnapshotPayloadDrainError::BeforeEffect(error.kind())
        };
        #[cfg(test)]
        if take_test_reset_payload_drain_fault(
            TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_PRE_EFFECT_DEADLINE,
        ) {
            exhaust_test_deadline(pre_effect_deadline);
        }
        candidate
            .revalidate_against_until(self, pre_effect_deadline)
            .map_err(before_effect)?;
        if take_test_reset_payload_drain_fault(TEST_FAULT_RESET_PAYLOAD_DRAIN_BEFORE_EFFECT) {
            return Err(AppDataResetSnapshotPayloadDrainError::BeforeEffect(
                SnapshotStorageErrorKind::Unavailable,
            ));
        }

        let selected = candidate.selected.clone();
        let selected_name = selected.name.clone();
        let selected_kind = selected.kind.clone();
        let post_effect_deadline = std::cell::Cell::new(None);
        let validate_before_effect = |lease: &SnapshotStoreInventoryLease| {
            if Instant::now() >= pre_effect_deadline {
                return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
            }
            if take_test_reset_payload_drain_fault(
                TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_FILESYSTEM_AT_FINAL_GATE,
            ) {
                return Err(unsafe_inventory_object());
            }
            lease.revalidate_complete_for_app_data_reset_before_effect_until(
                &selected,
                pre_effect_deadline,
            )?;
            #[cfg(test)]
            if take_test_reset_payload_drain_fault(
                TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_DEADLINE_AT_FINAL_GATE,
            ) {
                exhaust_test_deadline(pre_effect_deadline);
            }
            if Instant::now() >= pre_effect_deadline {
                Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy))
            } else {
                Ok(())
            }
        };
        let sync_after_effect = |directory: &File| {
            let deadline = Instant::now()
                .checked_add(APP_DATA_RESET_POST_EFFECT_TIMEOUT)
                .ok_or_else(|| {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState)
                })?;
            post_effect_deadline.set(Some(deadline));
            if take_test_reset_payload_drain_fault(TEST_FAULT_RESET_PAYLOAD_DRAIN_AFTER_EFFECT) {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
            platform::sync_directory(directory)?;
            if Instant::now() >= deadline {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
            if take_test_reset_payload_drain_fault(
                TEST_FAULT_RESET_PAYLOAD_DRAIN_AFTER_DIRECTORY_SYNC,
            ) {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
            Ok(())
        };

        match selected_kind {
            SnapshotInventoryEntryKind::Final(name) => {
                let retained = self.retain_observed_final(&name).map_err(before_effect)?;
                self.remove_observed_final_with_callbacks(
                    &retained,
                    validate_before_effect,
                    sync_after_effect,
                )
                .map_err(map_app_data_reset_final_drain_error)?;
            }
            SnapshotInventoryEntryKind::RecognizedTemp => {
                self.remove_quiescent_temp_with_callbacks(
                    &selected_name,
                    validate_before_effect,
                    sync_after_effect,
                )
                .map_err(map_app_data_reset_temp_drain_error)?;
            }
        }

        let post_effect_deadline = post_effect_deadline
            .get()
            .ok_or(AppDataResetSnapshotPayloadDrainError::OutcomeUnknown)?;
        if take_test_reset_payload_drain_fault(TEST_FAULT_RESET_PAYLOAD_DRAIN_DURING_READBACK) {
            return Err(AppDataResetSnapshotPayloadDrainError::OutcomeUnknown);
        }
        #[cfg(test)]
        if take_test_reset_payload_drain_fault(
            TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_POST_EFFECT_DEADLINE,
        ) {
            exhaust_test_deadline(post_effect_deadline);
        }
        self.revalidate_complete_for_app_data_reset_until(post_effect_deadline)
            .map_err(|_| AppDataResetSnapshotPayloadDrainError::OutcomeUnknown)?;
        Ok(AppDataResetSnapshotPayloadDrainCompletion {
            progress: AppDataResetSnapshotPayloadDrainBatch {
                removed_objects: 1,
                snapshot_payload_has_more: !self.entries.is_empty(),
            },
            post_effect_deadline,
        })
    }

    /// Revalidate the exact controls and retained names after a higher layer
    /// has reconciled this point-in-time observation with SQLite state.
    pub(crate) fn revalidate(&self) -> Result<()> {
        let store = SecureSnapshotStore {
            inner: Arc::clone(&self.store),
        };
        store.validate_controls()?;
        if snapshot_file_usage(&self.store.marker)? != self.controls.store_marker()
            || snapshot_file_usage(&self.store.writer_lock)? != self.controls.writer_lock()
        {
            return Err(unsafe_inventory_object());
        }
        for entry in &self.entries {
            entry.revalidate(&self.store)?;
        }
        Ok(())
    }

    /// Revalidate the canonical store binding and repeat the complete bounded
    /// inventory while the writer exclusion remains held.
    ///
    /// The ordinary retained-entry check above is sufficient when a higher
    /// layer intentionally mutates one known inventory member. App-data reset
    /// has a stricter pre-intent requirement: an actor ignoring the advisory
    /// lock must not be able to add a new child or replace the canonical store
    /// without invalidating admission.
    #[cfg(test)]
    pub(crate) fn revalidate_complete(&self) -> Result<()> {
        let deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))?;
        self.revalidate_complete_until(deadline)
    }

    pub(crate) fn revalidate_complete_until(&self, deadline: Instant) -> Result<()> {
        if Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        let store = SecureSnapshotStore {
            inner: Arc::clone(&self.store),
        };
        let current = store.inventory_locked_until(None, deadline)?;
        self.require_matching_inventory_until(current, deadline)
    }

    pub(super) fn require_matching_inventory_until(
        &self,
        current: SnapshotStorageInventory,
        deadline: Instant,
    ) -> Result<()> {
        if Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        let mut expected_entries = self.entries.clone();
        expected_entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        let mut current_entries = current.entries;
        current_entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        if current_entries != expected_entries
            || current.entries_usage != self.entries_usage
            || current.controls != self.controls
            || current.total_usage != self.total_usage
        {
            return Err(unsafe_inventory_object());
        }
        if Instant::now() >= deadline {
            Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy))
        } else {
            Ok(())
        }
    }

    /// Require a stable, complete observation suitable for app-data reset.
    ///
    /// A staged snapshot retains its per-file kernel lock after releasing the
    /// store writer so it can continue encoding before publication. Such an
    /// active temporary can later publish into the namespace and therefore is
    /// a normal busy refusal, even when two point-in-time inventories happen
    /// to observe identical file usage.
    pub(crate) fn revalidate_complete_for_app_data_reset_until(
        &self,
        deadline: Instant,
    ) -> Result<()> {
        if Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        if self.entries.iter().any(|entry| {
            entry.kind == SnapshotInventoryEntryKind::RecognizedTemp
                && entry.temp_kernel_state == Some(SnapshotTempKernelState::Active)
        }) {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        validate_app_data_reset_snapshot_directory(
            &self.store.database_root,
            self.store.database_root_identity,
            &self.store.directory,
            self.store.directory_identity,
            deadline,
        )?;
        self.validate_app_data_reset_same_filesystem()?;
        self.revalidate_complete_until(deadline)
    }

    /// Repeat the complete reset inventory at the exact unlink boundary. A
    /// quiescent temporary is already exclusively locked by the remover at
    /// this point, so the inventory reuses that exact observed identity as
    /// quiescent instead of probing it through a second descriptor.
    pub(super) fn revalidate_complete_for_app_data_reset_before_effect_until(
        &self,
        selected: &SnapshotInventoryEntry,
        deadline: Instant,
    ) -> Result<()> {
        if Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        let locked_temp =
            matches!(selected.kind, SnapshotInventoryEntryKind::RecognizedTemp).then_some(selected);
        let store = SecureSnapshotStore {
            inner: Arc::clone(&self.store),
        };
        let current = store.inventory_locked_until_with_locked_temp(None, locked_temp, deadline)?;
        validate_app_data_reset_snapshot_directory(
            &self.store.database_root,
            self.store.database_root_identity,
            &self.store.directory,
            self.store.directory_identity,
            deadline,
        )?;
        self.validate_app_data_reset_store_control_filesystem()?;
        self.validate_app_data_reset_same_filesystem_entries(&current.entries)?;
        self.require_matching_inventory_until(current, deadline)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn validate_app_data_reset_same_filesystem(&self) -> Result<()> {
        self.validate_app_data_reset_store_control_filesystem()?;
        self.validate_app_data_reset_same_filesystem_entries(&self.entries)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn validate_app_data_reset_store_control_filesystem(&self) -> Result<()> {
        if take_test_reset_payload_drain_fault(
            TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_STORE_FILESYSTEM,
        ) || !platform::same_filesystem(
            self.store.database_root_identity.0,
            self.store.directory_identity.0,
        ) || !platform::same_filesystem(
            self.store.directory_identity.0,
            self.store.marker_identity.0,
        ) || !platform::same_filesystem(
            self.store.directory_identity.0,
            self.store.writer_lock_identity.0,
        ) {
            return Err(unsafe_inventory_object());
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn validate_app_data_reset_same_filesystem_entries(
        &self,
        entries: &[SnapshotInventoryEntry],
    ) -> Result<()> {
        if !entries.is_empty()
            && take_test_reset_payload_drain_fault(
                TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_PAYLOAD_FILESYSTEM,
            )
        {
            return Err(unsafe_inventory_object());
        }
        if entries.iter().any(|entry| {
            !platform::same_filesystem(self.store.directory_identity.0, entry.identity.0)
        }) {
            return Err(unsafe_inventory_object());
        }
        Ok(())
    }

    /// Revalidate the complete retained observation and durably confirm the
    /// current snapshot-directory namespace without mutating it.
    pub(crate) fn sync_directory_state(&self) -> Result<()> {
        self.revalidate()?;
        platform::sync_directory(&self.store.directory)
    }

    /// Reacquire and remove one exact row-bound temp after a nonblocking
    /// quiescence proof. The caller must keep the matching SQLite row and this
    /// writer lease live until it has durably consumed that row.
    pub(crate) fn remove_quiescent_temp(&mut self, name: &str) -> Result<()> {
        self.remove_observed_quiescent_temp_reconciled(name)
            .map(drop)
            .map_err(|error| match error {
                SnapshotTempRemovalError::BeforeEffect(error) => error,
                SnapshotTempRemovalError::OutcomeUnknown => {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable)
                }
            })
    }

    /// Remove one fully proven quiescent temporary file while preserving its
    /// exact physical effect boundary for durable row reconciliation.
    pub(crate) fn remove_observed_quiescent_temp_reconciled(
        &mut self,
        name: &str,
    ) -> std::result::Result<SnapshotFileUsage, SnapshotTempRemovalError> {
        self.remove_quiescent_temp_with_sync(name, platform::sync_directory)
    }

    /// Remove one observed quiescent temporary only after higher persistence
    /// has proved that its exact name is absent from the complete bounded
    /// durable lease population. This storage-only capability does not inspect
    /// SQLite or infer the absence of a lease itself.
    pub(crate) fn remove_observed_unleased_temp_reconciled(
        &mut self,
        name: &str,
    ) -> std::result::Result<SnapshotFileUsage, SnapshotTempRemovalError> {
        self.remove_quiescent_temp_with_sync(name, platform::sync_directory)
    }

    pub(super) fn remove_quiescent_temp_with_sync(
        &mut self,
        name: &str,
        sync_directory: impl FnOnce(&File) -> Result<()>,
    ) -> std::result::Result<SnapshotFileUsage, SnapshotTempRemovalError> {
        self.remove_quiescent_temp_with_callbacks(name, |_| Ok(()), sync_directory)
    }

    pub(super) fn remove_quiescent_temp_with_callbacks(
        &mut self,
        name: &str,
        validate_before_effect: impl FnOnce(&SnapshotStoreInventoryLease) -> Result<()>,
        sync_directory: impl FnOnce(&File) -> Result<()>,
    ) -> std::result::Result<SnapshotFileUsage, SnapshotTempRemovalError> {
        let before_effect = SnapshotTempRemovalError::BeforeEffect;
        let position = self
            .entries
            .iter()
            .position(|entry| entry.name == name)
            .ok_or_else(|| before_effect(unsafe_inventory_object()))?;
        let observed = &self.entries[position];
        if !matches!(observed.kind, SnapshotInventoryEntryKind::RecognizedTemp)
            || observed.temp_kernel_state != Some(SnapshotTempKernelState::Quiescent)
        {
            return Err(before_effect(unsafe_inventory_object()));
        }
        let Some((file, identity)) =
            platform::open_named_temp_for_removal(&self.store.directory, &self.store.path, name)
                .map_err(before_effect)?
        else {
            return Err(before_effect(unsafe_inventory_object()));
        };
        if Identity(identity) != observed.identity
            || snapshot_file_usage(&file).map_err(before_effect)? != observed.usage
        {
            return Err(before_effect(unsafe_inventory_object()));
        }
        match FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(before_effect(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Busy,
                )));
            }
            Err(TryLockError::Error(_)) => {
                return Err(before_effect(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                )));
            }
        }
        platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)
            .map_err(before_effect)?;
        platform::validate_named(
            &self.store.directory,
            name,
            &file,
            identity,
            platform::Kind::RegularFile,
        )
        .map_err(before_effect)?;
        if snapshot_file_usage(&file).map_err(before_effect)? != observed.usage {
            return Err(before_effect(unsafe_inventory_object()));
        }

        // Freeze every accounting postcondition before the physical effect so
        // no arithmetic failure can be mislabeled as an unlink failure.
        let removed = observed.usage;
        let entries_usage = self
            .entries_usage
            .checked_sub(removed)
            .map_err(before_effect)?;
        let total_usage = self
            .total_usage
            .checked_sub(removed)
            .map_err(before_effect)?;
        // The platform call consumes and closes the delete-capable, locked
        // handle before returning. From that success onward only directory
        // durability remains uncertain.
        platform::remove_retained_temp_with_before_unlink(
            &self.store.directory,
            name,
            file,
            identity,
            || validate_before_effect(self),
        )
        .map_err(before_effect)?;
        sync_directory(&self.store.directory)
            .map_err(|_| SnapshotTempRemovalError::OutcomeUnknown)?;

        self.entries.remove(position);
        self.entries_usage = entries_usage;
        self.total_usage = total_usage;
        Ok(removed)
    }

    /// Retain one exact final from this inventory observation for content
    /// validation while the caller continues to own writer exclusion.
    ///
    /// This grants no deletion authority. The returned handle is read-only and
    /// is admitted only when the typed name, identity, usage, retained object,
    /// and current directory entry all still match this lease's observation.
    pub(crate) fn retain_observed_final(
        &self,
        name: &SnapshotFileName,
    ) -> Result<RetainedSnapshot> {
        let observed = self
            .entries
            .iter()
            .find(|entry| {
                matches!(
                    &entry.kind,
                    SnapshotInventoryEntryKind::Final(observed) if observed == name
                )
            })
            .ok_or_else(unsafe_inventory_object)?;
        let Some((file, identity)) = platform::open_named_regular(
            &self.store.directory,
            &self.store.path,
            name.as_str(),
            false,
        )?
        else {
            return Err(unsafe_inventory_object());
        };
        if Identity(identity) != observed.identity || snapshot_file_usage(&file)? != observed.usage
        {
            return Err(unsafe_inventory_object());
        }
        let retained = RetainedSnapshot {
            store: Arc::clone(&self.store),
            name: name.clone(),
            file,
            identity: Identity(identity),
        };
        retained.revalidate()?;
        if snapshot_file_usage(&retained.file)? != observed.usage {
            return Err(unsafe_inventory_object());
        }
        Ok(retained)
    }

    /// Remove one exact final that was observed by this inventory lease.
    ///
    /// Higher persistence layers must establish either exact tombstone and
    /// retention-policy authority or a separately proven unreferenced-orphan
    /// authority before calling this storage-only capability. The typed final
    /// handle must match this lease's retained observation and remain live for
    /// the complete call. The entry is reopened no-follow with deletion access
    /// where the platform requires it, and its identity plus complete usage
    /// must remain unchanged before unlink.
    pub(crate) fn remove_observed_final(
        &mut self,
        retained: &RetainedSnapshot,
    ) -> Result<SnapshotFileUsage> {
        self.remove_observed_final_reconciled(retained)
            .map_err(|error| match error {
                SnapshotFinalRemovalError::BeforeEffect(error) => error,
                SnapshotFinalRemovalError::OutcomeUnknown => {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable)
                }
            })
    }

    /// Remove one fully proven final while preserving the physical effect
    /// boundary for callers that do not have a durable logical tombstone.
    pub(crate) fn remove_observed_final_reconciled(
        &mut self,
        retained: &RetainedSnapshot,
    ) -> std::result::Result<SnapshotFileUsage, SnapshotFinalRemovalError> {
        self.remove_observed_final_with_sync(retained, platform::sync_directory)
    }

    pub(super) fn remove_observed_final_with_sync(
        &mut self,
        retained: &RetainedSnapshot,
        sync_directory: impl FnOnce(&File) -> Result<()>,
    ) -> std::result::Result<SnapshotFileUsage, SnapshotFinalRemovalError> {
        self.remove_observed_final_with_callbacks(retained, |_| Ok(()), sync_directory)
    }

    pub(super) fn remove_observed_final_with_callbacks(
        &mut self,
        retained: &RetainedSnapshot,
        validate_before_effect: impl FnOnce(&SnapshotStoreInventoryLease) -> Result<()>,
        sync_directory: impl FnOnce(&File) -> Result<()>,
    ) -> std::result::Result<SnapshotFileUsage, SnapshotFinalRemovalError> {
        let before_effect = SnapshotFinalRemovalError::BeforeEffect;
        if !Arc::ptr_eq(&self.store, &retained.store) {
            return Err(before_effect(unsafe_inventory_object()));
        }
        let name = &retained.name;
        let position = self
            .entries
            .iter()
            .position(|entry| {
                matches!(
                    &entry.kind,
                    SnapshotInventoryEntryKind::Final(observed) if observed == name
                )
            })
            .ok_or_else(|| before_effect(unsafe_inventory_object()))?;
        let observed = &self.entries[position];
        if retained.identity != observed.identity
            || snapshot_file_usage(&retained.file).map_err(before_effect)? != observed.usage
        {
            return Err(before_effect(unsafe_inventory_object()));
        }
        retained.revalidate().map_err(before_effect)?;
        let Some((file, identity)) = platform::open_named_final_for_removal(
            &self.store.directory,
            &self.store.path,
            name.as_str(),
        )
        .map_err(before_effect)?
        else {
            return Err(before_effect(unsafe_inventory_object()));
        };
        if Identity(identity) != observed.identity
            || snapshot_file_usage(&file).map_err(before_effect)? != observed.usage
        {
            return Err(before_effect(unsafe_inventory_object()));
        }
        platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)
            .map_err(before_effect)?;
        platform::validate_named(
            &self.store.directory,
            name.as_str(),
            &file,
            identity,
            platform::Kind::RegularFile,
        )
        .map_err(before_effect)?;
        if snapshot_file_usage(&file).map_err(before_effect)? != observed.usage {
            return Err(before_effect(unsafe_inventory_object()));
        }

        // Freeze every accounting postcondition before the physical effect so
        // no arithmetic failure can be mislabeled as an unlink failure.
        let removed = observed.usage;
        let entries_usage = self
            .entries_usage
            .checked_sub(removed)
            .map_err(before_effect)?;
        let total_usage = self
            .total_usage
            .checked_sub(removed)
            .map_err(before_effect)?;
        // The platform function consumes and closes the delete-capable handle
        // before returning. On Windows the POSIX disposition is applied when
        // that handle closes, so directory sync must never run while it is
        // still live. From this return onward, only directory durability
        // remains uncertain.
        platform::remove_retained_final_with_before_unlink(
            &self.store.directory,
            name.as_str(),
            file,
            identity,
            || validate_before_effect(self),
        )
        .map_err(before_effect)?;
        sync_directory(&self.store.directory)
            .map_err(|_| SnapshotFinalRemovalError::OutcomeUnknown)?;

        self.entries.remove(position);
        self.entries_usage = entries_usage;
        self.total_usage = total_usage;
        Ok(removed)
    }
}
