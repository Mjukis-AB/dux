use super::*;

impl SnapshotRepository {
    pub(crate) fn coordinates_store(&self, store: &Arc<StoreCoordinator>) -> bool {
        Arc::ptr_eq(&self.database, store)
    }

    /// Retain the complete snapshot-store writer observation during one
    /// higher-ranked reset callback.
    ///
    /// The matching database admission is a required input and is returned
    /// only as a callback-scoped borrow. This encodes database-before-snapshot
    /// acquisition and prevents either owned guard from escaping.
    pub(crate) fn with_app_data_reset_snapshot_admission<'guard, T>(
        &self,
        database_admission: &mut AppDataResetStoreGuard<'guard>,
        deadline: Instant,
        admitted: impl for<'database, 'snapshot> FnOnce(
            &'database mut AppDataResetStoreGuard<'guard>,
            AppDataResetSnapshotAdmission<'snapshot>,
        ) -> T,
    ) -> Result<T, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        if !database_admission.coordinates_store(&self.database) {
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InternalState,
            )));
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let inventory = store
            .inventory_with_writer_lease_until(deadline)
            .map_err(map_storage)?;
        inventory
            .revalidate_complete_for_app_data_reset_until(deadline)
            .map_err(map_storage)?;
        Ok(admitted(
            database_admission,
            AppDataResetSnapshotAdmission {
                inventory: &inventory,
                deadline,
            },
        ))
    }

    pub(crate) fn open(
        database: Arc<StoreCoordinator>,
        access: SnapshotStoreAccess,
    ) -> Result<Self, SnapshotRepositoryError> {
        let review = match access {
            SnapshotStoreAccess::ReadWrite => Some(SnapshotReviewContext {
                owner: OnceLock::new(),
                active_slots: Arc::new(AtomicUsize::new(0)),
                decoded_slots: Arc::new(AtomicUsize::new(0)),
                decoded_bytes: Arc::new(AtomicU64::new(0)),
            }),
            SnapshotStoreAccess::ReadOnly => None,
        };
        let store = match access {
            SnapshotStoreAccess::ReadWrite => {
                let _database_guard = database
                    .lock_current_history_connection()
                    .map_err(map_history)?;
                let database_path = database.validated_database_path().map_err(map_history)?;
                SecureSnapshotStore::open_for_database(&database_path, access)
                    .map_err(map_storage)?
            }
            SnapshotStoreAccess::ReadOnly => {
                let database_path = database.validated_database_path().map_err(map_history)?;
                SecureSnapshotStore::open_for_database(&database_path, access)
                    .map_err(map_storage)?
            }
        };
        Ok(Self {
            database,
            store,
            access,
            review,
        })
    }

    /// Reconcile the complete bounded physical snapshot store with exact
    /// current-schema history and review-pin facts.
    ///
    /// The returned value is metadata-only and cannot authorize a tombstone or
    /// unlink. Read-only/newer-schema repositories are rejected because this
    /// is a retention-maintenance prerequisite, not a generic history query.
    #[allow(
        dead_code,
        reason = "sealed inventory diagnostics remain internal before app/FFI presentation"
    )]
    pub(crate) fn inspect_retention_inventory(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotRetentionInventory, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let (inventory, storage) =
            self.build_retention_inventory_with_guard(&database_guard, observed_at)?;
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;
        Ok(inventory)
    }

    /// Observe only marker-owned database/control and snapshot storage.
    ///
    /// The database writer guard remains held before the snapshot writer lease
    /// is acquired, preserving the permanent database -> snapshot lock order.
    /// Returned aggregates contain no path, scan ID, file name, or mutation
    /// capability. Embedded AI content is a logical subset of SQLite.
    #[cfg(test)]
    pub(crate) fn inspect_owned_storage_footprint(
        &self,
        observed_at: SystemTime,
    ) -> Result<DuxOwnedStorageFootprint, SnapshotRepositoryError> {
        match self.inspect_owned_storage_footprint_with(observed_at, || {
            Ok::<(), std::convert::Infallible>(())
        })? {
            Ok((footprint, ())) => Ok(footprint),
            Err(error) => match error {},
        }
    }

    /// Observe database and snapshot usage while one additional read acquires
    /// its lock after the permanent database -> snapshot order.
    ///
    /// The callback must only acquire the final managed-cache inventory lock.
    /// Both earlier stores are revalidated after it returns, including when
    /// the callback reports an error.
    pub(crate) fn inspect_owned_storage_footprint_with<T, E>(
        &self,
        observed_at: SystemTime,
        inspect_managed_cache: impl FnOnce() -> Result<T, E>,
    ) -> Result<Result<(DuxOwnedStorageFootprint, T), E>, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let (database, embedded_ai_cache) = self
            .database
            .inspect_owned_database_footprint_with_guard(&database_guard, observed_at)
            .map_err(map_history)?;
        let (inventory, storage) =
            self.build_retention_inventory_with_guard(&database_guard, observed_at)?;
        let snapshots = owned_snapshot_storage_footprint(&inventory)?;
        let physical_total = database.checked_add(snapshots.total).map_err(map_history)?;
        let managed_cache = inspect_managed_cache();
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;
        let footprint = DuxOwnedStorageFootprint {
            observed_at,
            database,
            snapshots,
            embedded_ai_cache,
            physical_total,
        };
        Ok(managed_cache.map(|managed_cache| (footprint, managed_cache)))
    }

    fn build_retention_inventory_with_guard(
        &self,
        database_guard: &HistoryConnectionGuard<'_>,
        observed_at: SystemTime,
    ) -> Result<(SnapshotRetentionInventory, SnapshotStoreInventoryLease), SnapshotRepositoryError>
    {
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        let cap_bytes = self
            .database
            .load_snapshot_retention_cap_with_guard(database_guard)
            .map_err(map_history)?
            .cap_bytes;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        // Lock order is permanently database -> snapshot. The inventory lease
        // captures each exact final/temp identity and usage sequentially while
        // keeping the store-wide exclusion live until SQLite reconciliation;
        // every name is reopened and revalidated before handoff.
        let storage = store
            .inventory_with_writer_lease(PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?;
        let pins = inspect_snapshot_review_pin_population(&database_guard.connection, observed_at)
            .map_err(map_history)?;
        let temp_leases =
            inspect_snapshot_temp_leases(&database_guard.connection).map_err(map_history)?;
        let inventory = build_snapshot_retention_inventory(
            &database_guard.connection,
            &storage,
            pins,
            temp_leases,
            observed_at,
            cap_bytes,
        )
        .map_err(map_history)?;
        Ok((inventory, storage))
    }

    /// Prepare one bounded, path-free witness for every exact removable
    /// retained final in the current stable snapshot-store population.
    ///
    /// The witness includes all available/tombstoned finals and all excluded
    /// physical maintenance objects so consumption can reject any drift. It
    /// never includes scan roots and grants no generic storage capability.
    pub(crate) fn prepare_snapshot_storage_clear(
        &self,
        observed_at: SystemTime,
    ) -> Result<Option<PreparedSnapshotStorageClear>, SnapshotStorageClearError> {
        let observed_at = unix_ms_to_system_time(
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)
                .map_err(map_snapshot_storage_clear_repository_error)?,
        )
        .map_err(map_history)
        .map_err(map_snapshot_storage_clear_repository_error)?;
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(snapshot_storage_clear_error(
                SnapshotStorageClearErrorKind::ReadOnlyStore,
            ));
        }
        let (inventory, storage) = self
            .build_retention_inventory_with_guard(&database_guard, observed_at)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        storage
            .revalidate()
            .map_err(map_storage)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        if inventory.accounting_unstable {
            return Err(snapshot_storage_clear_error(
                SnapshotStorageClearErrorKind::Busy,
            ));
        }
        let prepared = prepared_snapshot_storage_clear(&inventory)?;
        if prepared.clearable_count() == Some(0) {
            Ok(None)
        } else if prepared.clearable_count().is_none() {
            Err(snapshot_storage_clear_error(
                SnapshotStorageClearErrorKind::BudgetExceeded,
            ))
        } else {
            Ok(Some(prepared))
        }
    }

    /// Consume one exact prepared witness, atomically tombstone every still
    /// available eligible final, then durably remove all confirmed eligible
    /// and already-tombstoned residual files by retained handle.
    pub(crate) fn clear_snapshot_storage(
        &self,
        prepared: PreparedSnapshotStorageClear,
        observed_at: SystemTime,
    ) -> Result<SnapshotStorageClearResult, SnapshotStorageClearError> {
        self.clear_snapshot_storage_with_hooks(
            prepared,
            observed_at,
            || Ok(()),
            |storage, retained| storage.remove_observed_final_reconciled(retained),
        )
    }

    pub(super) fn clear_snapshot_storage_with_hooks(
        &self,
        prepared: PreparedSnapshotStorageClear,
        observed_at: SystemTime,
        after_tombstone_commit: impl FnOnce() -> Result<(), HistoryError>,
        mut remove: impl FnMut(
            &mut SnapshotStoreInventoryLease,
            &RetainedSnapshot,
        )
            -> std::result::Result<SnapshotFileUsage, SnapshotFinalRemovalError>,
    ) -> Result<SnapshotStorageClearResult, SnapshotStorageClearError> {
        let observed_at = unix_ms_to_system_time(
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)
                .map_err(map_snapshot_storage_clear_repository_error)?,
        )
        .map_err(map_history)
        .map_err(map_snapshot_storage_clear_repository_error)?;
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(snapshot_storage_clear_error(
                SnapshotStorageClearErrorKind::ReadOnlyStore,
            ));
        }
        let (inventory, mut storage) = self
            .build_retention_inventory_with_guard(&database_guard, observed_at)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        storage
            .revalidate()
            .map_err(map_storage)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        if inventory.accounting_unstable {
            return Err(snapshot_storage_clear_error(
                SnapshotStorageClearErrorKind::ChangedSincePreview,
            ));
        }
        let current = prepared_snapshot_storage_clear(&inventory)?;
        if current != prepared {
            return Err(snapshot_storage_clear_error(
                SnapshotStorageClearErrorKind::ChangedSincePreview,
            ));
        }

        // Validate every complete document before the first logical or
        // physical effect. The inventory writer lease keeps all exact final
        // identities stable while handles are opened sequentially.
        for final_witness in prepared.finals.iter().filter(|final_witness| {
            matches!(
                final_witness.state,
                SnapshotStorageClearFinalState::Eligible
                    | SnapshotStorageClearFinalState::TombstonedResidual
            )
        }) {
            let retained = storage
                .retain_observed_final(final_witness.reference.file_name())
                .map_err(map_storage)
                .map_err(map_snapshot_storage_clear_repository_error)?;
            decode_reference(&retained, &final_witness.reference)
                .map_err(map_snapshot_storage_clear_repository_error)?;
        }
        storage
            .revalidate()
            .map_err(map_storage)
            .map_err(map_snapshot_storage_clear_repository_error)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)
            .map_err(map_snapshot_storage_clear_repository_error)?;

        let mut tombstones = Vec::new();
        tombstones
            .try_reserve_exact(prepared.eligible_snapshot_count as usize)
            .map_err(|_| {
                snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded)
            })?;
        for final_witness in prepared
            .finals
            .iter()
            .filter(|final_witness| final_witness.state == SnapshotStorageClearFinalState::Eligible)
        {
            tombstones.push(
                PreparedSnapshotRetentionTombstone::prepare(
                    &final_witness.reference,
                    final_witness.completed_at,
                    observed_at,
                )
                .map_err(map_history)
                .map_err(map_snapshot_storage_clear_repository_error)?,
            );
        }

        if !tombstones.is_empty() {
            let transaction = database_guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_write_sql_error)
                .map_err(map_history)
                .map_err(map_snapshot_storage_clear_repository_error)?;
            for tombstone in &tombstones {
                insert_snapshot_retention_tombstone(&transaction, tombstone)
                    .map_err(map_history)
                    .map_err(map_snapshot_storage_clear_repository_error)?;
            }
            let commit_result = transaction
                .commit()
                .map_err(map_write_sql_error)
                .and_then(|()| after_tombstone_commit())
                .and_then(|()| {
                    self.database
                        .revalidate_current_history_guard(&database_guard)
                });
            match commit_result {
                Ok(()) => {
                    for tombstone in &tombstones {
                        reconcile_snapshot_retention_tombstone_insert(
                            &database_guard.connection,
                            tombstone,
                            HistoryError::new(HistoryErrorKind::OutcomeUnknown),
                        )
                        .map_err(|_| {
                            snapshot_storage_clear_error(
                                SnapshotStorageClearErrorKind::OutcomeUnknown,
                            )
                        })?;
                    }
                }
                Err(failure) => {
                    if self
                        .database
                        .revalidate_current_history_guard(&database_guard)
                        .is_err()
                    {
                        return Err(snapshot_storage_clear_error(
                            SnapshotStorageClearErrorKind::OutcomeUnknown,
                        ));
                    }
                    let mut exact = 0_usize;
                    let mut first_error = None;
                    for tombstone in &tombstones {
                        match reconcile_snapshot_retention_tombstone_insert(
                            &database_guard.connection,
                            tombstone,
                            failure,
                        ) {
                            Ok(()) => exact += 1,
                            Err(error) => {
                                first_error.get_or_insert(error);
                            }
                        }
                    }
                    if exact == tombstones.len() {
                        // The atomic transaction committed despite the
                        // adjacent failure; all exact rows are now durable.
                    } else if exact == 0 {
                        return Err(map_snapshot_storage_clear_repository_error(map_history(
                            first_error.unwrap_or(failure),
                        )));
                    } else {
                        return Err(snapshot_storage_clear_error(
                            SnapshotStorageClearErrorKind::OutcomeUnknown,
                        ));
                    }
                }
            }
        }

        let mut cleared_eligible_snapshot_count = 0_u32;
        let mut cleared_tombstoned_residual_count = 0_u32;
        let mut cleared_usage = OwnedStorageUsage::default();
        let mut effect_started = !tombstones.is_empty();
        for state in [
            SnapshotStorageClearFinalState::TombstonedResidual,
            SnapshotStorageClearFinalState::Eligible,
        ] {
            for final_witness in prepared
                .finals
                .iter()
                .filter(|final_witness| final_witness.state == state)
            {
                let retained = match storage
                    .retain_observed_final(final_witness.reference.file_name())
                    .map_err(map_storage)
                {
                    Ok(retained) => retained,
                    Err(error) if !effect_started => {
                        return Err(map_snapshot_storage_clear_repository_error(error));
                    }
                    Err(_) => {
                        return Err(snapshot_storage_clear_error(
                            SnapshotStorageClearErrorKind::OutcomeUnknown,
                        ));
                    }
                };
                let removed = match remove(&mut storage, &retained) {
                    Ok(removed) => removed,
                    Err(SnapshotFinalRemovalError::BeforeEffect(error)) if !effect_started => {
                        return Err(map_snapshot_storage_clear_repository_error(map_storage(
                            error,
                        )));
                    }
                    Err(
                        SnapshotFinalRemovalError::BeforeEffect(_)
                        | SnapshotFinalRemovalError::OutcomeUnknown,
                    ) => {
                        return Err(snapshot_storage_clear_error(
                            SnapshotStorageClearErrorKind::OutcomeUnknown,
                        ));
                    }
                };
                drop(retained);
                effect_started = true;
                if removed != final_witness.usage {
                    return Err(snapshot_storage_clear_error(
                        SnapshotStorageClearErrorKind::OutcomeUnknown,
                    ));
                }
                cleared_usage = checked_owned_storage_usage_add_file(cleared_usage, removed)
                    .ok_or_else(|| {
                        snapshot_storage_clear_error(SnapshotStorageClearErrorKind::OutcomeUnknown)
                    })?;
                match state {
                    SnapshotStorageClearFinalState::Eligible => {
                        cleared_eligible_snapshot_count = cleared_eligible_snapshot_count
                            .checked_add(1)
                            .ok_or_else(|| {
                                snapshot_storage_clear_error(
                                    SnapshotStorageClearErrorKind::OutcomeUnknown,
                                )
                            })?;
                    }
                    SnapshotStorageClearFinalState::TombstonedResidual => {
                        cleared_tombstoned_residual_count = cleared_tombstoned_residual_count
                            .checked_add(1)
                            .ok_or_else(|| {
                                snapshot_storage_clear_error(
                                    SnapshotStorageClearErrorKind::OutcomeUnknown,
                                )
                            })?;
                    }
                    SnapshotStorageClearFinalState::Protected => {
                        return Err(snapshot_storage_clear_error(
                            SnapshotStorageClearErrorKind::InternalState,
                        ));
                    }
                }
                storage.revalidate().map_err(|_| {
                    snapshot_storage_clear_error(SnapshotStorageClearErrorKind::OutcomeUnknown)
                })?;
                self.database
                    .revalidate_current_history_guard(&database_guard)
                    .map_err(|_| {
                        snapshot_storage_clear_error(SnapshotStorageClearErrorKind::OutcomeUnknown)
                    })?;
            }
        }

        let result = SnapshotStorageClearResult {
            cleared_eligible_snapshot_count,
            cleared_tombstoned_residual_count,
            cleared_usage,
        };
        if result.cleared_count() != prepared.clearable_count()
            || result.cleared_usage != prepared.clearable
            || result.cleared_eligible_snapshot_count != prepared.eligible_snapshot_count
            || result.cleared_tombstoned_residual_count != prepared.tombstoned_residual_count
        {
            return Err(snapshot_storage_clear_error(
                SnapshotStorageClearErrorKind::OutcomeUnknown,
            ));
        }
        Ok(result)
    }

    /// Reconcile at most one exact marker-owned provisioning stage beneath
    /// the retained database root.
    ///
    /// The caller supplies no path, name, root, identity, or victim. The
    /// current-schema database guard is retained across the storage operation
    /// so a compliant provisioner cannot race classification or removal. This
    /// physical-only operation never reads or mutates SQLite rows.
    pub(crate) fn reconcile_snapshot_provisioning_stage(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotProvisioningStageReconciliationBatchResult, SnapshotRepositoryError> {
        self.reconcile_snapshot_provisioning_stage_with_reconciler(observed_at, |store| {
            store.reconcile_provisioning_stage()
        })
    }

    pub(super) fn reconcile_snapshot_provisioning_stage_with_reconciler(
        &self,
        observed_at: SystemTime,
        reconcile: impl FnOnce(
            &SecureSnapshotStore,
        ) -> std::result::Result<
            StorageProvisioningStageReconciliation,
            SnapshotProvisioningStageRemovalError,
        >,
    ) -> Result<SnapshotProvisioningStageReconciliationBatchResult, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let observed_at = unix_ms_to_system_time(
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)?,
        )
        .map_err(map_history)?;
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        self.database
            .validate_history_guard(&database_guard)
            .map_err(map_history)?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;

        let reconciliation = match reconcile(store) {
            Ok(reconciliation) => reconciliation,
            Err(SnapshotProvisioningStageRemovalError::BeforeEffect(error)) => {
                return Err(map_storage(error));
            }
            Err(SnapshotProvisioningStageRemovalError::OutcomeUnknown) => {
                return Err(outcome_unknown());
            }
        };
        let removed = reconciliation.removed_control_usage();
        let physical_effect = matches!(
            reconciliation.outcome(),
            StorageProvisioningStageRemoval::RemovedMarkerOnly
                | StorageProvisioningStageRemoval::RemovedMarkerComplete
        );
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(|error| {
                if physical_effect {
                    outcome_unknown()
                } else {
                    map_history(error)
                }
            })?;

        let outcome = match (reconciliation.outcome(), removed) {
            (StorageProvisioningStageRemoval::NoStage, None) => {
                SnapshotProvisioningStageReconciliationBatchOutcome::NoStage
            }
            (StorageProvisioningStageRemoval::DeferredUnproven, None) => {
                SnapshotProvisioningStageReconciliationBatchOutcome::DeferredUnproven
            }
            (StorageProvisioningStageRemoval::RemovedMarkerOnly, Some(usage)) => {
                SnapshotProvisioningStageReconciliationBatchOutcome::RemovedMarkerOnly {
                    bytes: usage.charged_bytes(),
                }
            }
            (StorageProvisioningStageRemoval::RemovedMarkerComplete, Some(usage)) => {
                SnapshotProvisioningStageReconciliationBatchOutcome::RemovedMarkerComplete {
                    bytes: usage.charged_bytes(),
                }
            }
            (StorageProvisioningStageRemoval::NoStage, Some(_))
            | (StorageProvisioningStageRemoval::DeferredUnproven, Some(_)) => {
                return Err(repository_error(SnapshotRepositoryErrorKind::Storage(
                    SnapshotStorageErrorKind::InternalState,
                )));
            }
            (StorageProvisioningStageRemoval::RemovedMarkerOnly, None)
            | (StorageProvisioningStageRemoval::RemovedMarkerComplete, None) => {
                return Err(outcome_unknown());
            }
        };

        Ok(SnapshotProvisioningStageReconciliationBatchResult {
            observed_at,
            outcome,
            total_stage_count_before: reconciliation.total_stage_count_before(),
            total_stage_count_after: reconciliation.total_stage_count_after(),
            marker_owned_count_before: reconciliation.marker_owned_count_before(),
            marker_owned_count_after: reconciliation.marker_owned_count_after(),
            unproven_count_before: reconciliation.unproven_count_before(),
            unproven_count_after: reconciliation.unproven_count_after(),
            control_charged_bytes_before: reconciliation.control_usage_before().charged_bytes(),
            control_charged_bytes_after: reconciliation.control_usage_after().charged_bytes(),
            has_more: reconciliation.has_more(),
        })
    }

    /// Reconcile at most one exact snapshot-temp lease whose fully decoded
    /// parent scan is failed, cancelled, or interrupted.
    ///
    /// The caller supplies no name or identity. Authority is derived from the
    /// complete bounded lease population and physical inventory while holding
    /// the permanent database-before-snapshot lock order. Running rows,
    /// unleased temps, and provisioning stages are outside this operation.
    pub(crate) fn reconcile_terminal_snapshot_temp_residual(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotTerminalTempReconciliationBatchResult, SnapshotRepositoryError> {
        self.reconcile_terminal_snapshot_temp_residual_with_hooks(
            observed_at,
            |storage, name| storage.remove_observed_quiescent_temp_reconciled(name),
            || Ok(()),
        )
    }

    pub(super) fn reconcile_terminal_snapshot_temp_residual_with_hooks(
        &self,
        observed_at: SystemTime,
        remove: impl FnOnce(
            &mut SnapshotStoreInventoryLease,
            &str,
        ) -> std::result::Result<SnapshotFileUsage, SnapshotTempRemovalError>,
        after_row_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<SnapshotTerminalTempReconciliationBatchResult, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let observed_at = unix_ms_to_system_time(
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)?,
        )
        .map_err(map_history)?;
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        self.database
            .validate_history_guard(&database_guard)
            .map_err(map_history)?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let mut storage = store
            .inventory_with_writer_lease(PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?;
        let leases =
            inspect_snapshot_temp_leases(&database_guard.connection).map_err(map_history)?;
        let inventory =
            build_snapshot_terminal_temp_inventory(&storage, leases).map_err(map_history)?;
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;

        let terminal_lease_count_before = u32::try_from(inventory.entries.len()).map_err(|_| {
            repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::QueryLimitExceeded,
            ))
        })?;
        let active_terminal_lease_count_before = inventory.active_count;
        let terminal_charged_bytes_before = inventory.charged_bytes;
        let Some(candidate) = inventory.first_actionable() else {
            let outcome = if terminal_lease_count_before == 0 {
                SnapshotTerminalTempReconciliationBatchOutcome::NoTerminalResidual
            } else {
                SnapshotTerminalTempReconciliationBatchOutcome::DeferredActive
            };
            return Ok(SnapshotTerminalTempReconciliationBatchResult {
                observed_at,
                outcome,
                terminal_lease_count_before,
                terminal_lease_count_after: terminal_lease_count_before,
                active_terminal_lease_count_before,
                active_terminal_lease_count_after: active_terminal_lease_count_before,
                terminal_charged_bytes_before,
                terminal_charged_bytes_after: terminal_charged_bytes_before,
                has_more: terminal_lease_count_before > 0,
            });
        };

        // The joined status used for bounded classification is not mutation
        // authority. Fully decode the exact selected parent and require its
        // complete terminal/no-snapshot shape before any physical or SQL
        // effect.
        let parent = self
            .database
            .load_scan_with_guard(&database_guard, candidate.lease.scan_id())
            .map_err(map_history)?
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::CorruptData,
                ))
            })?;
        if !matches!(
            parent.status(),
            ScanStatus::Failed | ScanStatus::Cancelled | ScanStatus::Interrupted
        ) || parent.snapshot().is_some()
        {
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::CorruptData,
            )));
        }
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;

        // Freeze all aggregate postconditions before either the unlink or the
        // exact-row transaction begins.
        let terminal_lease_count_after = terminal_lease_count_before
            .checked_sub(1)
            .ok_or_else(|| history_repository_error(HistoryErrorKind::InternalState))?;
        let active_terminal_lease_count_after = active_terminal_lease_count_before;
        let (terminal_charged_bytes_after, removed_bytes, physical_effect) =
            match candidate.physical {
                SnapshotTerminalTempPhysicalState::Missing => {
                    // Row-before-file creation plus both retained locks proves
                    // that a compliant creator cannot still publish this name.
                    // Revalidate and durably flush the complete directory
                    // immediately before the metadata-only effect.
                    storage.sync_directory_state().map_err(map_storage)?;
                    (terminal_charged_bytes_before, None, false)
                }
                SnapshotTerminalTempPhysicalState::Active => {
                    return Err(history_repository_error(HistoryErrorKind::InternalState));
                }
                SnapshotTerminalTempPhysicalState::Quiescent(expected_usage) => {
                    let terminal_charged_bytes_after = terminal_charged_bytes_before
                        .checked_sub(expected_usage.charged_bytes())
                        .ok_or_else(|| history_repository_error(HistoryErrorKind::InternalState))?;
                    let removed = match remove(&mut storage, candidate.lease.temp_name()) {
                        Ok(removed) => removed,
                        Err(SnapshotTempRemovalError::BeforeEffect(error)) => {
                            return Err(map_storage(error));
                        }
                        Err(SnapshotTempRemovalError::OutcomeUnknown) => {
                            return Err(outcome_unknown());
                        }
                    };
                    if removed != expected_usage {
                        return Err(outcome_unknown());
                    }
                    storage.revalidate().map_err(|_| outcome_unknown())?;
                    (
                        terminal_charged_bytes_after,
                        Some(removed.charged_bytes()),
                        true,
                    )
                }
            };

        let expected = candidate.lease.as_prepared();
        if let Err(error) = self.delete_temp_lease_with_guard_and_hook(
            &mut database_guard,
            &expected,
            after_row_commit,
        ) {
            return if physical_effect {
                Err(outcome_unknown())
            } else {
                Err(error)
            };
        }
        if snapshot_temp_lease_state(&database_guard.connection, &expected)
            .map_err(|_| outcome_unknown())?
            != SnapshotTempLeaseState::Missing
        {
            return Err(outcome_unknown());
        }
        storage.revalidate().map_err(|_| {
            if physical_effect {
                outcome_unknown()
            } else {
                repository_error(SnapshotRepositoryErrorKind::Storage(
                    SnapshotStorageErrorKind::UnsafeObject,
                ))
            }
        })?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(|_| outcome_unknown())?;

        let scan_id = parent.id().clone();
        let outcome = match removed_bytes {
            Some(bytes) => SnapshotTerminalTempReconciliationBatchOutcome::RemovedTempAndLease {
                scan_id,
                bytes,
            },
            None => SnapshotTerminalTempReconciliationBatchOutcome::ReconciledRowOnly { scan_id },
        };
        Ok(SnapshotTerminalTempReconciliationBatchResult {
            observed_at,
            outcome,
            terminal_lease_count_before,
            terminal_lease_count_after,
            active_terminal_lease_count_before,
            active_terminal_lease_count_after,
            terminal_charged_bytes_before,
            terminal_charged_bytes_after,
            has_more: terminal_lease_count_after > 0,
        })
    }

    /// Reconcile at most one marker-owned recognized snapshot temp whose exact
    /// name is absent from the complete bounded durable lease population.
    ///
    /// The caller supplies no name or identity. This physical-only boundary
    /// retains the current-schema database guard before the snapshot writer
    /// lease, skips active names, and never adopts the temp or mutates SQLite.
    /// Row-bound temps, provisioning stages, scans, and final snapshots are
    /// outside this operation.
    pub(crate) fn reconcile_unleased_snapshot_temp(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotUnleasedTempReconciliationBatchResult, SnapshotRepositoryError> {
        self.reconcile_unleased_snapshot_temp_with_remover(observed_at, |storage, name| {
            storage.remove_observed_unleased_temp_reconciled(name)
        })
    }

    pub(super) fn reconcile_unleased_snapshot_temp_with_remover(
        &self,
        observed_at: SystemTime,
        remove: impl FnOnce(
            &mut SnapshotStoreInventoryLease,
            &str,
        ) -> std::result::Result<SnapshotFileUsage, SnapshotTempRemovalError>,
    ) -> Result<SnapshotUnleasedTempReconciliationBatchResult, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let observed_at = unix_ms_to_system_time(
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)?,
        )
        .map_err(map_history)?;
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        self.database
            .validate_history_guard(&database_guard)
            .map_err(map_history)?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let mut storage = store
            .inventory_with_writer_lease(PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?;
        let leases =
            inspect_snapshot_temp_leases(&database_guard.connection).map_err(map_history)?;
        let inventory =
            build_snapshot_unleased_temp_inventory(&storage, &leases).map_err(map_history)?;
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;

        let unleased_temp_count_before = u32::try_from(inventory.entries.len())
            .map_err(|_| history_repository_error(HistoryErrorKind::QueryLimitExceeded))?;
        let active_unleased_temp_count_before = inventory.active_count;
        let unleased_charged_bytes_before = inventory.charged_bytes;
        let Some(candidate) = inventory.first_actionable() else {
            let outcome = if unleased_temp_count_before == 0 {
                SnapshotUnleasedTempReconciliationBatchOutcome::NoUnleasedTemp
            } else {
                SnapshotUnleasedTempReconciliationBatchOutcome::DeferredActive
            };
            return Ok(SnapshotUnleasedTempReconciliationBatchResult {
                observed_at,
                outcome,
                unleased_temp_count_before,
                unleased_temp_count_after: unleased_temp_count_before,
                active_unleased_temp_count_before,
                active_unleased_temp_count_after: active_unleased_temp_count_before,
                unleased_charged_bytes_before,
                unleased_charged_bytes_after: unleased_charged_bytes_before,
                has_more: unleased_temp_count_before > 0,
            });
        };

        let expected_usage = match candidate.state {
            SnapshotUnleasedTempPhysicalState::Quiescent(usage) => usage,
            SnapshotUnleasedTempPhysicalState::Active(_) => {
                return Err(history_repository_error(HistoryErrorKind::InternalState));
            }
        };
        let unleased_temp_count_after = unleased_temp_count_before
            .checked_sub(1)
            .ok_or_else(|| history_repository_error(HistoryErrorKind::InternalState))?;
        let active_unleased_temp_count_after = active_unleased_temp_count_before;
        let unleased_charged_bytes_after = unleased_charged_bytes_before
            .checked_sub(expected_usage.charged_bytes())
            .ok_or_else(|| history_repository_error(HistoryErrorKind::InternalState))?;

        let removed = match remove(&mut storage, &candidate.name) {
            Ok(removed) => removed,
            Err(SnapshotTempRemovalError::BeforeEffect(error)) => {
                return Err(map_storage(error));
            }
            Err(SnapshotTempRemovalError::OutcomeUnknown) => {
                return Err(outcome_unknown());
            }
        };
        if removed != expected_usage {
            return Err(outcome_unknown());
        }
        storage.revalidate().map_err(|_| outcome_unknown())?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(|_| outcome_unknown())?;

        Ok(SnapshotUnleasedTempReconciliationBatchResult {
            observed_at,
            outcome: SnapshotUnleasedTempReconciliationBatchOutcome::Removed {
                bytes: removed.charged_bytes(),
            },
            unleased_temp_count_before,
            unleased_temp_count_after,
            active_unleased_temp_count_before,
            active_unleased_temp_count_after,
            unleased_charged_bytes_before,
            unleased_charged_bytes_after,
            has_more: unleased_temp_count_after > 0,
        })
    }

    /// Remove at most one fully proven physical orphan.
    ///
    /// Authority comes only from a complete physical-final inventory, absence
    /// of any exact snapshot-path reference, a checksum-valid document whose
    /// hashed scan ID equals its physical name, and an exact parent scan row
    /// with no snapshot reference while the writer lease proves no publication
    /// is currently in flight. Cap policy, pins, temporary files, and
    /// tombstones are intentionally outside this decision.
    pub(crate) fn reconcile_physical_orphan(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotOrphanReconciliationBatchResult, SnapshotRepositoryError> {
        self.reconcile_physical_orphan_with_remover(observed_at, |storage, retained| {
            storage.remove_observed_final_reconciled(retained)
        })
    }

    pub(super) fn reconcile_physical_orphan_with_remover(
        &self,
        observed_at: SystemTime,
        remove: impl FnOnce(
            &mut SnapshotStoreInventoryLease,
            &RetainedSnapshot,
        )
            -> std::result::Result<SnapshotFileUsage, SnapshotFinalRemovalError>,
    ) -> Result<SnapshotOrphanReconciliationBatchResult, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let observed_at = unix_ms_to_system_time(
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)?,
        )
        .map_err(map_history)?;
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        self.database
            .validate_history_guard(&database_guard)
            .map_err(map_history)?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        // Lock order is permanently database -> snapshot. Publication uses
        // the same snapshot writer lock, so no orphan can be adopted or
        // replaced between classification and exact retained-handle unlink.
        let mut storage = store
            .inventory_with_writer_lease(PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?;
        let inventory =
            build_snapshot_physical_orphan_inventory(&database_guard.connection, &storage)
                .map_err(map_history)?;
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;

        let orphan_count_before = u32::try_from(inventory.finals.len()).map_err(|_| {
            repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::QueryLimitExceeded,
            ))
        })?;
        let orphan_charged_bytes_before = inventory.usage.charged_bytes;
        let Some(candidate) = inventory.finals.first() else {
            return Ok(SnapshotOrphanReconciliationBatchResult {
                observed_at,
                outcome: SnapshotOrphanReconciliationBatchOutcome::NoOrphan,
                orphan_count_before: 0,
                orphan_count_after: 0,
                orphan_charged_bytes_before: 0,
                orphan_charged_bytes_after: 0,
                has_more: false,
            });
        };

        let retained = storage
            .retain_observed_final(&candidate.file_name)
            .map_err(map_storage)?;
        let (document, _digest) = decode_retained(&retained)?;
        if SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes())
            != candidate.file_name
        {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        let parent = self
            .database
            .load_scan_with_guard(&database_guard, &document.metadata.scan_id)
            .map_err(map_history)?
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::CorruptData,
                ))
            })?;
        if !document.metadata.root.matches_path(parent.root())
            || parent.snapshot().is_some()
            || !matches!(
                parent.status(),
                ScanStatus::Running
                    | ScanStatus::Failed
                    | ScanStatus::Cancelled
                    | ScanStatus::Interrupted
            )
        {
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::CorruptData,
            )));
        }
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;

        // Freeze every accounting postcondition before the unlink boundary.
        let orphan_count_after = orphan_count_before.checked_sub(1).ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InternalState,
            ))
        })?;
        let orphan_charged_bytes_after = orphan_charged_bytes_before
            .checked_sub(candidate.usage.charged_bytes())
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::InternalState,
                ))
            })?;
        let removed = match remove(&mut storage, &retained) {
            Ok(removed) => removed,
            Err(SnapshotFinalRemovalError::BeforeEffect(error)) => return Err(map_storage(error)),
            Err(SnapshotFinalRemovalError::OutcomeUnknown) => return Err(outcome_unknown()),
        };
        drop(retained);
        if removed != candidate.usage {
            return Err(outcome_unknown());
        }
        storage.revalidate().map_err(|_| outcome_unknown())?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(|_| outcome_unknown())?;
        let bytes = removed.charged_bytes();
        Ok(SnapshotOrphanReconciliationBatchResult {
            observed_at,
            outcome: SnapshotOrphanReconciliationBatchOutcome::Removed {
                scan_id: document.metadata.scan_id,
                bytes,
            },
            orphan_count_before,
            orphan_count_after,
            orphan_charged_bytes_before,
            orphan_charged_bytes_after,
            has_more: orphan_count_after > 0,
        })
    }

    /// Apply at most one exact physical snapshot-retention mutation.
    ///
    /// Existing tombstoned residuals are retried before a new victim is
    /// selected. A new tombstone is permitted only from a stable complete
    /// inventory whose oldest candidate is neither latest-two nor actively
    /// pinned. The database guard and snapshot writer lease remain held from
    /// that proof through append-only tombstone commit and retained-handle
    /// unlink.
    pub(crate) fn enforce_retention_cap(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotRetentionBatchResult, SnapshotRepositoryError> {
        self.enforce_retention_cap_with_hook(observed_at, || Ok(()))
    }

    pub(super) fn enforce_retention_cap_with_hook(
        &self,
        observed_at: SystemTime,
        after_tombstone_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<SnapshotRetentionBatchResult, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let observed_at = unix_ms_to_system_time(
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)?,
        )
        .map_err(map_history)?;
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let (inventory, mut storage) =
            self.build_retention_inventory_with_guard(&database_guard, observed_at)?;
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;

        let cap_bytes = inventory.cap_bytes;
        let charged_bytes_before = inventory.totals.store_total.charged_bytes;

        // Logical unavailability is already durable for this exact identity,
        // so residual retry does not depend on a fresh cap calculation.
        if let Some(residual) = inventory.oldest_tombstoned_residual() {
            let scan_id = residual.scan_id.clone();
            let file_name = residual.reference.file_name().clone();
            let expected_usage = residual.usage;
            let charged_bytes_after =
                charged_bytes_after_removal(charged_bytes_before, expected_usage.charged_bytes())?;
            let has_more = inventory.has_additional_work_after(&scan_id);
            let retained = storage
                .retain_observed_final(&file_name)
                .map_err(map_storage)?;
            decode_reference(&retained, &residual.reference)?;
            let removed = storage
                .remove_observed_final(&retained)
                .map_err(map_storage)?;
            drop(retained);
            if removed != expected_usage {
                return Err(repository_error(SnapshotRepositoryErrorKind::Storage(
                    SnapshotStorageErrorKind::InternalState,
                )));
            }
            storage.revalidate().map_err(map_storage)?;
            self.database
                .revalidate_current_history_guard(&database_guard)
                .map_err(map_history)?;
            let bytes = removed.charged_bytes();
            return Ok(SnapshotRetentionBatchResult {
                observed_at,
                outcome: SnapshotRetentionBatchOutcome::RemovedTombstonedResidual {
                    scan_id,
                    bytes,
                },
                cap_bytes,
                charged_bytes_before,
                charged_bytes_after,
                has_more,
            });
        }

        if charged_bytes_before <= cap_bytes {
            return Ok(SnapshotRetentionBatchResult {
                observed_at,
                outcome: SnapshotRetentionBatchOutcome::UnderCap,
                cap_bytes,
                charged_bytes_before,
                charged_bytes_after: charged_bytes_before,
                has_more: false,
            });
        }
        if inventory.accounting_unstable {
            return Ok(SnapshotRetentionBatchResult {
                observed_at,
                outcome: SnapshotRetentionBatchOutcome::DeferredUnstable,
                cap_bytes,
                charged_bytes_before,
                charged_bytes_after: charged_bytes_before,
                has_more: true,
            });
        }
        let Some(candidate) = inventory.oldest_eviction_candidate() else {
            return Ok(SnapshotRetentionBatchResult {
                observed_at,
                outcome: SnapshotRetentionBatchOutcome::DeferredNoEligibleSnapshot,
                cap_bytes,
                charged_bytes_before,
                charged_bytes_after: charged_bytes_before,
                has_more: false,
            });
        };
        if !candidate.is_eviction_observation() {
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InternalState,
            )));
        }

        let scan_id = candidate.scan_id.clone();
        let file_name = candidate.reference.file_name().clone();
        let expected_usage = candidate.usage;
        let charged_bytes_after =
            charged_bytes_after_removal(charged_bytes_before, expected_usage.charged_bytes())?;
        let has_more = inventory.has_additional_work_after(&scan_id);
        let tombstone = PreparedSnapshotRetentionTombstone::prepare(
            &candidate.reference,
            candidate.completed_at,
            observed_at,
        )
        .map_err(map_history)?;

        let retained = storage
            .retain_observed_final(&file_name)
            .map_err(map_storage)?;
        decode_reference(&retained, &candidate.reference)?;
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;

        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)
            .map_err(map_history)?;
        insert_snapshot_retention_tombstone(&transaction, &tombstone).map_err(map_history)?;
        let commit_result = transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_tombstone_commit())
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(&database_guard)
            });
        if let Err(failure) = commit_result {
            if self
                .database
                .revalidate_current_history_guard(&database_guard)
                .is_err()
            {
                return Err(outcome_unknown());
            }
            reconcile_snapshot_retention_tombstone_insert(
                &database_guard.connection,
                &tombstone,
                failure,
            )
            .map_err(map_history)?;
        } else {
            reconcile_snapshot_retention_tombstone_insert(
                &database_guard.connection,
                &tombstone,
                HistoryError::new(HistoryErrorKind::OutcomeUnknown),
            )
            .map_err(map_history)?;
        }

        let removed = storage
            .remove_observed_final(&retained)
            .map_err(map_storage)?;
        drop(retained);
        if removed != expected_usage {
            return Err(repository_error(SnapshotRepositoryErrorKind::Storage(
                SnapshotStorageErrorKind::InternalState,
            )));
        }
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;
        let bytes = removed.charged_bytes();
        Ok(SnapshotRetentionBatchResult {
            observed_at,
            outcome: SnapshotRetentionBatchOutcome::TombstonedAndRemoved { scan_id, bytes },
            cap_bytes,
            charged_bytes_before,
            charged_bytes_after,
            has_more,
        })
    }

    /// Acquire an explicit cross-process lease for one snapshot-backed UI
    /// review. Candidate and cleanup-session state deliberately do not call
    /// this implicitly.
    #[allow(
        dead_code,
        reason = "review leases are wired to Explorer/FFI in a later milestone slice"
    )]
    pub(crate) fn acquire_review_lease(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        self.acquire_review_lease_with_hook(reference, purpose, observed_at, || Ok(()))
    }

    #[cfg(test)]
    pub(super) fn acquire_review_lease_after_commit_failure_for_test(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        self.acquire_review_lease_with_hook(reference, purpose, observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    fn acquire_review_lease_with_hook(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        self.acquire_review_lease_with_pin_id_source(
            reference,
            purpose,
            observed_at,
            || {
                (0..SNAPSHOT_REVIEW_PIN_ID_ATTEMPTS)
                    .map(|_| SnapshotReviewPinId::random())
                    .collect::<Result<Vec<_>, _>>()
            },
            after_commit,
        )
    }

    #[cfg(test)]
    pub(super) fn acquire_review_lease_with_pin_ids_for_test(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
        pin_ids: Vec<SnapshotReviewPinId>,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        self.acquire_review_lease_with_pin_id_source(
            reference,
            purpose,
            observed_at,
            || Ok(pin_ids),
            || Ok(()),
        )
    }

    fn acquire_review_lease_with_pin_id_source(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
        pin_id_source: impl FnOnce() -> Result<Vec<SnapshotReviewPinId>, HistoryError>,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        if reference.version() != SNAPSHOT_FORMAT_VERSION {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::IncompatibleVersion,
            ));
        }
        let context = self
            .review
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReadOnly))?;
        let owner = context.owner()?;
        let slot = context.reserve()?;
        // Generate fallible process-local material before either persistence
        // lock. The stable owner is created once with the repository.
        let pin_ids = pin_id_source().map_err(map_history)?;

        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let scan = self
            .database
            .load_scan_with_guard(&database_guard, reference.scan_id())
            .map_err(map_history)?
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::NotFound,
                ))
            })?;
        let completed_at = scan
            .completed_at()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch))?;
        if scan.status() != ScanStatus::Succeeded || scan.snapshot() != Some(reference) {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        match load_snapshot_retention_state(&database_guard.connection, reference)
            .map_err(map_history)?
        {
            SnapshotRetentionState::Available => {}
            SnapshotRetentionState::Tombstoned { .. } => {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::SnapshotUnavailable,
                ));
            }
        }

        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let opened = store
            .open_with_writer_lease(reference.file_name(), PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingSnapshot))?;
        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(crate::persistence::history::map_write_sql_error)
            .map_err(map_history)?;
        let pins = pin_ids
            .into_iter()
            .map(|pin_id| {
                PreparedSnapshotReviewPin::prepare(
                    pin_id,
                    reference,
                    completed_at,
                    owner.clone(),
                    purpose,
                    observed_at,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_history)?;
        let pin = insert_snapshot_review_pin_candidates(&transaction, &pins)
            .map_err(map_history)?
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::InternalState,
                ))
            })?;
        let write = transaction
            .commit()
            .map_err(crate::persistence::history::map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(&database_guard)
            });
        if let Err(failure) = write
            && !reconcile_review_pin(&self.database, &database_guard, &pin)?
        {
            return Err(map_history(failure));
        }
        opened.retained().revalidate().map_err(map_storage)?;
        let retained = opened.into_retained();
        drop(database_guard);
        Ok(SnapshotReviewLease {
            database: Arc::clone(&self.database),
            retained,
            reference: reference.clone(),
            pin,
            decoded_slots: Arc::clone(&context.decoded_slots),
            decoded_bytes: Arc::clone(&context.decoded_bytes),
            _slot: slot,
            _not_sync: PhantomData,
            #[cfg(test)]
            test_fault: Cell::new(SnapshotReviewTestFault::None),
        })
    }

    pub(super) fn stage_document(
        &self,
        document: &SnapshotDocument,
    ) -> Result<(StagedSnapshot, SnapshotDigest, PreparedSnapshotTempLease), SnapshotRepositoryError>
    {
        self.stage_document_with_create(document, |reservation| {
            reservation.create().map_err(map_storage)
        })
    }

    pub(super) fn stage_document_with_create(
        &self,
        document: &SnapshotDocument,
        create: impl FnOnce(
            &mut SnapshotStageReservation,
        ) -> Result<StagedSnapshot, SnapshotRepositoryError>,
    ) -> Result<(StagedSnapshot, SnapshotDigest, PreparedSnapshotTempLease), SnapshotRepositoryError>
    {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        // Generate every fallible process-local identity before entering the
        // permanent database-to-snapshot critical section.
        let lease_id = SnapshotTempLeaseId::random().map_err(map_history)?;
        let owner = current_process_instance().map_err(map_process_identity)?;
        let created_at = SystemTime::now();
        let file_name =
            SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes());
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        self.reconcile_existing_temp_lease_with_guard(
            &mut database_guard,
            store,
            &document.metadata.scan_id,
        )?;
        let mut reservation = store
            .reserve_stage(file_name.clone(), PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?;
        let temp_lease = PreparedSnapshotTempLease::prepare(
            lease_id,
            document.metadata.scan_id.clone(),
            file_name,
            reservation.temp_name().to_owned(),
            owner,
            created_at,
        )
        .map_err(map_history)?;
        self.insert_temp_lease_with_guard(&mut database_guard, &temp_lease)?;
        let mut staged = create(&mut reservation)?;
        drop(reservation);
        drop(database_guard);
        let expected_digest = match encode_snapshot(document, &mut staged) {
            Ok(digest) => digest,
            Err(error) => {
                let mut database_guard = match self.database.lock_current_history_connection() {
                    Ok(guard) => guard,
                    Err(history) => {
                        staged.abandon();
                        return Err(map_history(history));
                    }
                };
                self.abort_staged_with_guard(&mut database_guard, staged, &temp_lease)?;
                drop(database_guard);
                return Err(map_codec(error));
            }
        };
        Ok((staged, expected_digest, temp_lease))
    }

    fn reconcile_existing_temp_lease_with_guard(
        &self,
        database_guard: &mut HistoryConnectionGuard<'_>,
        store: &SecureSnapshotStore,
        scan_id: &ScanId,
    ) -> Result<(), SnapshotRepositoryError> {
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        let population =
            inspect_snapshot_temp_leases(&database_guard.connection).map_err(map_history)?;
        let Some(existing) = population
            .rows()
            .iter()
            .find(|row| row.scan_id() == scan_id)
        else {
            return Ok(());
        };
        let expected = existing.as_prepared();
        let mut storage = store
            .inventory_with_writer_lease(PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?;
        if let Some(entry) = storage
            .entries()
            .iter()
            .find(|entry| entry.name() == existing.temp_name())
        {
            if !matches!(entry.kind(), SnapshotInventoryEntryKind::RecognizedTemp) {
                return Err(repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::CorruptData,
                )));
            }
            match entry.temp_kernel_state() {
                Some(SnapshotTempKernelState::Active) => {
                    return Err(repository_error(SnapshotRepositoryErrorKind::Storage(
                        SnapshotStorageErrorKind::Busy,
                    )));
                }
                Some(SnapshotTempKernelState::Quiescent) => {
                    storage
                        .remove_quiescent_temp(existing.temp_name())
                        .map_err(map_storage)?;
                }
                None => {
                    return Err(repository_error(SnapshotRepositoryErrorKind::History(
                        HistoryErrorKind::CorruptData,
                    )));
                }
            }
        }
        self.delete_temp_lease_with_guard(database_guard, &expected)?;
        drop(storage);
        Ok(())
    }

    pub(super) fn insert_temp_lease_with_guard(
        &self,
        database_guard: &mut HistoryConnectionGuard<'_>,
        lease: &PreparedSnapshotTempLease,
    ) -> Result<(), SnapshotRepositoryError> {
        self.insert_temp_lease_with_guard_and_hook(database_guard, lease, || Ok(()))
    }

    pub(super) fn insert_temp_lease_with_guard_and_hook(
        &self,
        database_guard: &mut HistoryConnectionGuard<'_>,
        lease: &PreparedSnapshotTempLease,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), SnapshotRepositoryError> {
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(crate::persistence::history::map_write_sql_error)
            .map_err(map_history)?;
        insert_snapshot_temp_lease(&transaction, lease).map_err(map_history)?;
        let write = transaction
            .commit()
            .map_err(crate::persistence::history::map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(database_guard)
            });
        if let Err(failure) = write {
            self.revalidate_temp_lease_reconciliation(database_guard)?;
            reconcile_snapshot_temp_lease_insert(&database_guard.connection, lease, failure)
                .map_err(map_history)?;
        }
        Ok(())
    }

    fn delete_temp_lease_with_guard(
        &self,
        database_guard: &mut HistoryConnectionGuard<'_>,
        lease: &PreparedSnapshotTempLease,
    ) -> Result<(), SnapshotRepositoryError> {
        self.delete_temp_lease_with_guard_and_hook(database_guard, lease, || Ok(()))
    }

    pub(super) fn delete_temp_lease_with_guard_and_hook(
        &self,
        database_guard: &mut HistoryConnectionGuard<'_>,
        lease: &PreparedSnapshotTempLease,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), SnapshotRepositoryError> {
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(crate::persistence::history::map_write_sql_error)
            .map_err(map_history)?;
        delete_snapshot_temp_lease(&transaction, lease).map_err(map_history)?;
        let write = transaction
            .commit()
            .map_err(crate::persistence::history::map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(database_guard)
            });
        if let Err(failure) = write {
            self.revalidate_temp_lease_reconciliation(database_guard)?;
            reconcile_snapshot_temp_lease_delete(&database_guard.connection, lease, failure)
                .map_err(map_history)?;
        }
        Ok(())
    }

    fn revalidate_temp_lease_reconciliation(
        &self,
        database_guard: &HistoryConnectionGuard<'_>,
    ) -> Result<(), SnapshotRepositoryError> {
        self.database
            .revalidate_current_history_guard(database_guard)
            .map_err(|_| outcome_unknown())
    }

    pub(super) fn abort_staged_with_guard(
        &self,
        database_guard: &mut HistoryConnectionGuard<'_>,
        staged: StagedSnapshot,
        lease: &PreparedSnapshotTempLease,
    ) -> Result<(), SnapshotRepositoryError> {
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        if snapshot_temp_lease_state(&database_guard.connection, lease).map_err(map_history)?
            != SnapshotTempLeaseState::Exact
        {
            let mutation = staged.abort().map_err(map_storage)?;
            drop(mutation);
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::CorruptData,
            )));
        }
        let mutation: SnapshotTempMutationLease = staged.abort().map_err(map_storage)?;
        self.delete_temp_lease_with_guard(database_guard, lease)?;
        drop(mutation);
        Ok(())
    }

    fn publish_staged(
        &self,
        database_guard: &HistoryConnectionGuard<'_>,
        staged: StagedSnapshot,
        document: &SnapshotDocument,
        expected_digest: SnapshotDigest,
        temp_lease: PreparedSnapshotTempLease,
    ) -> Result<PublishedSnapshot, SnapshotRepositoryError> {
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        if snapshot_temp_lease_state(&database_guard.connection, &temp_lease)
            .map_err(map_history)?
            != SnapshotTempLeaseState::Exact
        {
            let mutation = staged.abort().map_err(map_storage)?;
            drop(mutation);
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::CorruptData,
            )));
        }
        let file_name =
            SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes());
        let publication = match staged.publish_no_replace().map_err(map_storage)? {
            SnapshotPublication::Published(publication)
            | SnapshotPublication::Existing(publication) => publication,
        };
        validate_retained_document(publication.retained(), document, expected_digest)?;
        let reference = SnapshotReference {
            scan_id: document.metadata.scan_id.clone(),
            version: NonZeroU32::new(SNAPSHOT_FORMAT_VERSION)
                .expect("snapshot format version is nonzero"),
            file_name,
            digest: expected_digest,
        };
        Ok(PublishedSnapshot {
            reference,
            publication,
            temp_lease,
        })
    }

    /// Leave one exact schema-v8 row-only or quiescent row-bound temp for
    /// engine/repository maintenance tests. The caller must have recorded the
    /// matching running scan and may terminalize it after this helper returns.
    #[cfg(test)]
    pub(crate) fn leave_snapshot_temp_residual_for_test(
        &self,
        document: &SnapshotDocument,
        create_file: bool,
    ) -> Result<(), SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let lease_id = SnapshotTempLeaseId::random().map_err(map_history)?;
        let owner = current_process_instance().map_err(map_process_identity)?;
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        self.reconcile_existing_temp_lease_with_guard(
            &mut database_guard,
            store,
            &document.metadata.scan_id,
        )?;
        let file_name =
            SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes());
        let mut reservation = store
            .reserve_stage(file_name.clone(), PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?;
        let lease = PreparedSnapshotTempLease::prepare(
            lease_id,
            document.metadata.scan_id.clone(),
            file_name,
            reservation.temp_name().to_owned(),
            owner,
            SystemTime::now(),
        )
        .map_err(map_history)?;
        self.insert_temp_lease_with_guard(&mut database_guard, &lease)?;
        if create_file {
            let mut staged = reservation.create().map_err(map_storage)?;
            std::io::Write::write_all(&mut staged, b"terminal-temp-residual").map_err(|_| {
                repository_error(SnapshotRepositoryErrorKind::Storage(
                    SnapshotStorageErrorKind::Unavailable,
                ))
            })?;
            staged.sync_all().map_err(map_storage)?;
            staged.abandon();
        }
        drop(reservation);
        drop(database_guard);
        Ok(())
    }

    /// Leave one physical recognized temp with no durable lease row for
    /// repository/engine maintenance tests. Returning the staged handle keeps
    /// its kernel lock active; a quiescent fixture closes the handle here.
    #[cfg(test)]
    pub(crate) fn leave_unleased_snapshot_temp_for_test(
        &self,
        seed: &[u8],
        active: bool,
    ) -> Result<Option<StagedSnapshot>, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let mut staged = store
            .stage(
                SnapshotFileName::from_scan_id(seed),
                PUBLICATION_LOCK_TIMEOUT,
            )
            .map_err(map_storage)?;
        std::io::Write::write_all(&mut staged, b"unleased-temp-fixture").map_err(|_| {
            repository_error(SnapshotRepositoryErrorKind::Storage(
                SnapshotStorageErrorKind::Unavailable,
            ))
        })?;
        staged.sync_all().map_err(map_storage)?;
        if active {
            Ok(Some(staged))
        } else {
            staged.abandon();
            Ok(None)
        }
    }

    #[cfg(test)]
    pub(crate) fn publish_orphan_for_test(
        &self,
        document: &SnapshotDocument,
    ) -> Result<PublishedSnapshot, SnapshotRepositoryError> {
        let (staged, expected_digest, temp_lease) = self.stage_document(document)?;
        let database_guard = match self.database.lock_current_history_connection() {
            Ok(guard) => guard,
            Err(error) => {
                staged.abandon();
                return Err(map_history(error));
            }
        };
        self.publish_staged(
            &database_guard,
            staged,
            document,
            expected_digest,
            temp_lease,
        )
    }

    pub(crate) fn load(
        &self,
        reference: &SnapshotReference,
    ) -> Result<SnapshotDocument, SnapshotRepositoryError> {
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let retained = self.open_available_with_guard(&database_guard, reference)?;
        drop(database_guard);
        decode_reference(&retained, reference)
    }

    pub(crate) fn load_for_candidate_recovery(
        &self,
        reference: &SnapshotReference,
    ) -> Result<SnapshotReviewDocument, SnapshotRepositoryError> {
        SnapshotReviewDocument::from_document_for_recovery(self.load(reference)?)
    }

    pub(super) fn load_with_guard(
        &self,
        database_guard: &HistoryConnectionGuard<'_>,
        reference: &SnapshotReference,
    ) -> Result<SnapshotDocument, SnapshotRepositoryError> {
        let retained = self.open_available_with_guard(database_guard, reference)?;
        decode_reference(&retained, reference)
    }

    /// Check logical availability under the database fence before acquiring a
    /// retained file handle. Retention takes the same database-first order
    /// before its snapshot writer lock and unlink.
    fn open_available_with_guard(
        &self,
        database_guard: &HistoryConnectionGuard<'_>,
        reference: &SnapshotReference,
    ) -> Result<RetainedSnapshot, SnapshotRepositoryError> {
        if reference.version() != SNAPSHOT_FORMAT_VERSION {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::IncompatibleVersion,
            ));
        }
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        match load_snapshot_retention_state(&database_guard.connection, reference)
            .map_err(map_history)?
        {
            SnapshotRetentionState::Available => {}
            SnapshotRetentionState::Tombstoned { .. } => {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::SnapshotUnavailable,
                ));
            }
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        store
            .open(reference.file_name())
            .map_err(map_storage)?
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingSnapshot))
    }

    /// Publish the immutable file before exact-CASing the durable scan summary.
    /// A published file without a DB row is a harmless retention orphan; the
    /// reverse ordering is forbidden.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "preserved for callers that explicitly opt out of candidate evaluation"
        )
    )]
    pub(crate) fn complete_scan(
        &self,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: &ScanCoverage,
        document: &SnapshotDocument,
    ) -> Result<SnapshotReference, SnapshotRepositoryError> {
        self.complete_scan_inner(completed_at, counts, coverage, document, None)
    }

    /// Publish a snapshot and atomically commit the succeeded scan plus the
    /// evaluator's complete terminal discovery output.
    pub(crate) fn complete_scan_with_candidate_evaluation(
        &self,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: &ScanCoverage,
        document: &SnapshotDocument,
        identity: &CandidateEvaluationIdentity,
        evaluation: &CandidateEvaluationCompletion,
    ) -> Result<SnapshotReference, SnapshotRepositoryError> {
        self.complete_scan_inner(
            completed_at,
            counts,
            coverage,
            document,
            Some((identity, evaluation)),
        )
    }

    fn complete_scan_inner(
        &self,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: &ScanCoverage,
        document: &SnapshotDocument,
        evaluation: Option<(&CandidateEvaluationIdentity, &CandidateEvaluationCompletion)>,
    ) -> Result<SnapshotReference, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        counts.validate_for_storage().map_err(map_history)?;
        if coverage.status() == crate::ScanCoverageStatus::Unknown {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        if counts.directory_count != document.metadata.totals.directory_count
            || counts.file_count != document.metadata.totals.file_count
            || counts.logical_bytes != document.metadata.totals.logical_bytes
            || counts.allocated_bytes != document.metadata.totals.allocated_bytes
        {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        if let Some((_, terminal)) = evaluation {
            terminal
                .validate_for_scan(&document.metadata.scan_id, completed_at)
                .map_err(map_history)?;
        }
        let current = self
            .database
            .load_scan(&document.metadata.scan_id)
            .map_err(map_history)?
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::NotFound,
                ))
            })?;
        if !document.metadata.root.matches_path(current.root()) {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        if coverage
            .issues()
            .iter()
            .filter_map(|issue| issue.path())
            .any(|path| !path.starts_with(current.root()))
        {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }

        if current.status() == ScanStatus::Succeeded {
            let reference = current
                .snapshot()
                .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch))?;
            let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
                document.metadata.scan_id.clone(),
                completed_at,
                counts,
                coverage.clone(),
                reference.clone(),
            )
            .map_err(map_history)?;
            if !current.exactly_matches_completion(&completion)
                || &self.load(reference)? != document
            {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::ReferenceMismatch,
                ));
            }
            if let Some((identity, terminal)) = evaluation {
                let request =
                    NewCandidateEvaluation::try_new(identity.clone(), reference, completed_at)
                        .map_err(map_history)?;
                let stored = self
                    .database
                    .load_candidate_evaluation(request.scan_id())
                    .map_err(map_history)?
                    .ok_or_else(|| {
                        repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch)
                    })?;
                if !terminal.exactly_matches_record(&stored, &request) {
                    return Err(repository_error(
                        SnapshotRepositoryErrorKind::ReferenceMismatch,
                    ));
                }
            }
            return Ok(reference.clone());
        }
        if current.status() != ScanStatus::Running || current.snapshot().is_some() {
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InvalidTransition,
            )));
        }

        let (staged, expected_digest, temp_lease) = self.stage_document(document)?;
        let mut database_guard = match self.database.lock_current_history_connection() {
            Ok(guard) => guard,
            Err(error) => {
                staged.abandon();
                return Err(map_history(error));
            }
        };
        let loaded = match self
            .database
            .load_scan_with_guard(&database_guard, &document.metadata.scan_id)
        {
            Ok(loaded) => loaded,
            Err(error) => {
                self.abort_staged_with_guard(&mut database_guard, staged, &temp_lease)?;
                return Err(map_history(error));
            }
        };
        let current = match loaded {
            Some(current) => current,
            None => {
                self.abort_staged_with_guard(&mut database_guard, staged, &temp_lease)?;
                return Err(repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::NotFound,
                )));
            }
        };
        if !document.metadata.root.matches_path(current.root()) {
            self.abort_staged_with_guard(&mut database_guard, staged, &temp_lease)?;
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        if current.status() == ScanStatus::Succeeded {
            self.abort_staged_with_guard(&mut database_guard, staged, &temp_lease)?;
            let reference = current
                .snapshot()
                .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch))?;
            let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
                document.metadata.scan_id.clone(),
                completed_at,
                counts,
                coverage.clone(),
                reference.clone(),
            )
            .map_err(map_history)?;
            if !current.exactly_matches_completion(&completion)
                || &self.load_with_guard(&database_guard, reference)? != document
            {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::ReferenceMismatch,
                ));
            }
            if let Some((identity, terminal)) = evaluation {
                let request =
                    NewCandidateEvaluation::try_new(identity.clone(), reference, completed_at)
                        .map_err(map_history)?;
                let stored = self
                    .database
                    .load_candidate_evaluation_with_guard(&database_guard, request.scan_id())
                    .map_err(map_history)?
                    .ok_or_else(|| {
                        repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch)
                    })?;
                if !terminal.exactly_matches_record(&stored, &request) {
                    return Err(repository_error(
                        SnapshotRepositoryErrorKind::ReferenceMismatch,
                    ));
                }
            }
            return Ok(reference.clone());
        }
        if current.status() != ScanStatus::Running || current.snapshot().is_some() {
            self.abort_staged_with_guard(&mut database_guard, staged, &temp_lease)?;
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InvalidTransition,
            )));
        }

        let published = self.publish_staged(
            &database_guard,
            staged,
            document,
            expected_digest,
            temp_lease,
        )?;
        let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
            document.metadata.scan_id.clone(),
            completed_at,
            counts,
            coverage.clone(),
            published.reference().clone(),
        )
        .map_err(map_history)?;
        if let Some((identity, terminal)) = evaluation {
            let request = NewCandidateEvaluation::try_new(
                identity.clone(),
                published.reference(),
                completed_at,
            )
            .map_err(map_history)?;
            self.database
                .record_scan_finished_with_evaluation_and_temp_lease_reconciled_with_guard(
                    &mut database_guard,
                    &completion,
                    &request,
                    terminal,
                    &published.temp_lease,
                )
                .map_err(map_history)?;
        } else {
            self.database
                .record_scan_finished_with_temp_lease_reconciled_with_guard(
                    &mut database_guard,
                    &completion,
                    &published.temp_lease,
                )
                .map_err(map_history)?;
        }
        published.revalidate()?;
        Ok(published.reference().clone())
    }
}
