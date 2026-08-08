use super::*;

impl AppDataResetCoordinatorSession<'_> {
    /// Reopen the exact canonical-or-detached data namespace named by a
    /// validated journal while retaining this exclusive coordinator session.
    /// The recovery opener is descriptor-only and cannot provision, repair,
    /// migrate, or open SQLite.
    pub(crate) fn with_recovery_data_namespace_until<T>(
        &mut self,
        database_path: &Path,
        journal: &AppDataResetJournal,
        deadline: Instant,
        operation: impl for<'session, 'data> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetRecoveryDataNamespace<'data>,
        ) -> T,
    ) -> std::result::Result<T, HistoryError> {
        let current = self
            .storage
            .read_journal_exact_until(deadline)
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?
            .ok_or_else(|| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            })?;
        let (_, canonical_root_name) = self.storage.data_root_binding();
        if current != *journal
            || (journal.has_canonical_root_name_binding()
                && !journal.is_bound_to_canonical_root_name(canonical_root_name))
            || database_path.parent().and_then(Path::file_name) != Some(canonical_root_name)
        {
            return Err(HistoryError::new(
                super::history::HistoryErrorKind::InternalState,
            ));
        }
        let transaction = journal
            .validated_transaction()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?;
        StoreCoordinator::with_app_data_reset_recovery_data_namespace_until(
            database_path,
            &transaction,
            journal.data_identity(),
            deadline,
            |data_namespace| operation(self, data_namespace),
        )
    }

    /// Acquire the same ordered old-store guards for the phase-specific fresh
    /// bootstrap publisher. Every path/name/provenance input is derived from
    /// the validated journal transaction inside this persistence boundary.
    pub(crate) fn with_fresh_data_namespace_until<T>(
        &mut self,
        database_path: &Path,
        journal: &AppDataResetJournal,
        deadline: Instant,
        operation: impl for<'session, 'data> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetFreshNamespace<'data>,
        ) -> T,
    ) -> std::result::Result<T, HistoryError> {
        let current = self
            .storage
            .read_journal()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?
            .ok_or_else(|| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            })?;
        let (_, canonical_root_name) = self.storage.data_root_binding();
        if current != *journal
            || !journal.is_bound_to_canonical_root_name(canonical_root_name)
            || database_path.parent().and_then(Path::file_name) != Some(canonical_root_name)
        {
            return Err(HistoryError::new(
                super::history::HistoryErrorKind::InternalState,
            ));
        }
        let transaction = journal
            .validated_transaction()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?;
        StoreCoordinator::with_app_data_reset_fresh_namespace_until(
            database_path,
            &transaction,
            journal.data_identity(),
            journal.phase() == AppDataResetPhase::Draining,
            deadline,
            |fresh_namespace| operation(self, fresh_namespace),
        )
    }

    /// Try the reset-only old-SQLite opener while retaining this exact
    /// coordinator session. Callers use this path only for a durable
    /// `Draining` journal; if snapshot storage still exists the strict old-root
    /// inventory refuses it and the engine may continue through the earlier
    /// cache/snapshot pipeline without any effect.
    pub(crate) fn with_draining_old_database_until<T>(
        &mut self,
        database_path: &Path,
        journal: &AppDataResetJournal,
        deadline: Instant,
        operation: impl for<'session, 'data> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetOldDatabaseDrainingAdmission<'data>,
        ) -> T,
    ) -> std::result::Result<T, HistoryError> {
        let current = self
            .storage
            .read_journal()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?
            .ok_or_else(|| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            })?;
        let (_, canonical_root_name) = self.storage.data_root_binding();
        if current != *journal
            || journal.phase() != AppDataResetPhase::Draining
            || !journal.is_bound_to_canonical_root_name(canonical_root_name)
            || database_path.parent().and_then(Path::file_name) != Some(canonical_root_name)
        {
            return Err(HistoryError::new(
                super::history::HistoryErrorKind::InternalState,
            ));
        }
        let transaction = journal
            .validated_transaction()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?;
        let fresh_identity = journal
            .fresh_data_identity()
            .ok_or_else(|| HistoryError::new(super::history::HistoryErrorKind::InternalState))?;
        StoreCoordinator::with_app_data_reset_draining_old_database_until(
            database_path,
            &transaction,
            journal.data_identity(),
            fresh_identity,
            deadline,
            |data| operation(self, data),
        )
    }

    /// Seal the exact published fresh identity and advance only the matching
    /// `DataDetached` journal. No raw identity can enter from the engine layer.
    pub(crate) fn commit_fresh_namespace(
        &mut self,
        expected: &AppDataResetJournal,
        published: &AppDataResetPublishedFreshNamespace<'_>,
    ) -> Result<AppDataResetJournal> {
        let transaction = expected.validated_transaction()?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        published
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if !published.is_bound_to(
            &transaction,
            expected.data_identity(),
            publication_parent_identity,
            canonical_root_name,
        ) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let identity = published
            .fresh_identity()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        self.advance_fresh_namespace_identity(expected, identity)
    }

    /// Admit one bounded cache payload batch only after the exact
    /// `FreshNamespaceReady` journal has durably advanced to `Draining`, or
    /// after an already-`Draining` restart has re-proved the same namespaces.
    /// Journal-write uncertainty returns no batch token, so unlink authority
    /// is unreachable until durability is known.
    pub(crate) fn admit_draining_cache_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetReadyToDrainNamespace<'data>,
        cache: AppDataResetManagedCacheDrainCandidate<'cache>,
    ) -> Result<AppDataResetDrainingCacheBatch<'data, 'cache>> {
        expected.validate()?;
        if !matches!(
            expected.phase(),
            AppDataResetPhase::FreshNamespaceReady | AppDataResetPhase::Draining
        ) {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }

        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let journal = if expected.phase() == AppDataResetPhase::FreshNamespaceReady {
            let next = expected.advanced_draining()?;
            self.storage.write_journal(&encode_journal(&next)?)?;
            next
        } else {
            expected.clone()
        };

        data.revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        let durable = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != journal || durable.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingCacheBatch {
            journal,
            data,
            cache,
        })
    }

    /// Consume the coordinator-issued token for exactly one cache object.
    /// No retry is possible from this token after an unlink attempt.
    pub(crate) fn run_draining_cache_batch(
        &mut self,
        batch: AppDataResetDrainingCacheBatch<'_, '_>,
    ) -> Result<AppDataResetManagedCacheDrainBatch> {
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let authority = AppDataResetCacheDrainAuthority { _private: () };
        let progress = batch
            .cache
            .drain_one_detached_payload(authority)
            .map_err(map_cache_drain_error)?;

        // The data namespace is unaffected by the cache-only batch and must
        // still match exactly. Any later uncertainty remains recovery debt.
        batch
            .data
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        let durable = self
            .storage
            .read_journal()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(progress)
    }

    /// Join the exact fresh/old namespace proof to one structural cache-tail
    /// candidate. This boundary deliberately accepts only a journal that was
    /// already `Draining` when the recovery pass began; the transition from
    /// `FreshNamespaceReady` cannot retire ownership controls in the same pass.
    pub(crate) fn admit_draining_cache_stage_retirement_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetReadyToDrainNamespace<'data>,
        cache: AppDataResetManagedCacheStageRetirementCandidate<'cache>,
    ) -> Result<AppDataResetDrainingCacheStageRetirementBatch<'data, 'cache>> {
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }

        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let durable = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected || durable.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingCacheStageRetirementBatch {
            journal: durable,
            data,
            cache,
        })
    }

    /// Consume the coordinator-issued structural capability for exactly one
    /// marker, writer control, or empty detached stage shell. Post-effect
    /// uncertainty remains reset recovery debt and is never retried here.
    pub(crate) fn run_draining_cache_stage_retirement_batch(
        &mut self,
        batch: AppDataResetDrainingCacheStageRetirementBatch<'_, '_>,
    ) -> Result<AppDataResetManagedCacheStageRetirementBatch> {
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let authority = AppDataResetCacheStageRetireAuthority { _private: () };
        let progress = batch
            .cache
            .retire_one_structure(authority)
            .map_err(map_cache_stage_retirement_error)?;

        batch
            .data
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        let durable = self
            .storage
            .read_journal()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(progress)
    }

    /// Join exact cache absence to the first retained old snapshot payload.
    /// This is available only after an already-durable `Draining` journal;
    /// cache payload/control work and snapshot payload work therefore cannot
    /// occur in the same recovery pass.
    pub(crate) fn admit_draining_snapshot_payload_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetOldSnapshotPayloadDrainCandidate<'data>,
        cache: AppDataResetManagedCacheAbsentWitness<'cache>,
        pre_effect_deadline: Instant,
    ) -> Result<AppDataResetDrainingSnapshotPayloadBatch<'data, 'cache>> {
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }

        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
            || data.deadline() != pre_effect_deadline
            || cache.deadline() != pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let durable = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected
            || durable.phase() != AppDataResetPhase::Draining
            || Instant::now() >= pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingSnapshotPayloadBatch {
            journal: durable,
            data,
            cache,
            pre_effect_deadline,
        })
    }

    /// Consume the sole production snapshot-reset capability for one exact
    /// old-store final or quiescent temporary. Every post-effect ambiguity is
    /// durable recovery debt; this batch is never retried.
    pub(crate) fn run_draining_snapshot_payload_batch(
        &mut self,
        batch: AppDataResetDrainingSnapshotPayloadBatch<'_, '_>,
    ) -> Result<AppDataResetSnapshotPayloadDrainBatch> {
        let pre_effect_deadline = batch.pre_effect_deadline;
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }

        let authority = AppDataResetSnapshotPayloadDrainAuthority {
            pre_effect_deadline,
        };
        let completion = batch
            .data
            .drain_one_snapshot_payload(authority)
            .map_err(map_snapshot_payload_drain_error)?;
        let post_effect_deadline = completion.post_effect_deadline();

        #[cfg(test)]
        if take_test_app_data_reset_coordinator_postcheck_fault(
            TestAppDataResetCoordinatorPostcheckFault::ExhaustBeforeCacheReadback,
        ) {
            exhaust_coordinator_postcheck_deadline(post_effect_deadline);
        }
        batch
            .cache
            .revalidate_after_effect_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        #[cfg(test)]
        if take_test_app_data_reset_coordinator_postcheck_fault(
            TestAppDataResetCoordinatorPostcheckFault::ExhaustBeforeJournalReadback,
        ) {
            exhaust_coordinator_postcheck_deadline(post_effect_deadline);
        }
        let durable = self
            .storage
            .read_journal_exact_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal || Instant::now() >= post_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(completion.into_progress())
    }

    /// Join exact cache absence to one monotonic old snapshot-store structural
    /// state. This is available only on a pass already observing durable
    /// `Draining`, after payload selection has returned no candidate.
    pub(crate) fn admit_draining_snapshot_store_retirement_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetOldSnapshotStoreRetirementCandidate<'data>,
        cache: AppDataResetManagedCacheAbsentWitness<'cache>,
        pre_effect_deadline: Instant,
    ) -> Result<AppDataResetDrainingSnapshotStoreRetirementBatch<'data, 'cache>> {
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }

        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
            || data.deadline() != pre_effect_deadline
            || cache.deadline() != pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let durable = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected
            || durable.phase() != AppDataResetPhase::Draining
            || Instant::now() >= pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingSnapshotStoreRetirementBatch {
            journal: durable,
            data,
            cache,
            pre_effect_deadline,
        })
    }

    /// Consume the sole production structural capability for one exact old
    /// snapshot marker, locked writer control, or empty directory. Every
    /// post-effect ambiguity remains durable recovery debt.
    pub(crate) fn run_draining_snapshot_store_retirement_batch(
        &mut self,
        batch: AppDataResetDrainingSnapshotStoreRetirementBatch<'_, '_>,
    ) -> Result<AppDataResetSnapshotStoreRetirementBatch> {
        let pre_effect_deadline = batch.pre_effect_deadline;
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }

        let authority = AppDataResetSnapshotStoreRetireAuthority {
            pre_effect_deadline,
        };
        let completion = batch
            .data
            .retire_one_snapshot_store_structure(authority)
            .map_err(map_snapshot_store_retirement_error)?;
        let post_effect_deadline = completion.post_effect_deadline();

        #[cfg(test)]
        if take_test_app_data_reset_coordinator_postcheck_fault(
            TestAppDataResetCoordinatorPostcheckFault::ExhaustBeforeCacheReadback,
        ) {
            exhaust_coordinator_postcheck_deadline(post_effect_deadline);
        }
        batch
            .cache
            .revalidate_after_effect_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        #[cfg(test)]
        if take_test_app_data_reset_coordinator_postcheck_fault(
            TestAppDataResetCoordinatorPostcheckFault::ExhaustBeforeJournalReadback,
        ) {
            exhaust_coordinator_postcheck_deadline(post_effect_deadline);
        }
        let durable = self
            .storage
            .read_journal_exact_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal || Instant::now() >= post_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(completion.into_progress())
    }

    /// Join exact cache absence to one old SQLite sidecar or, only after all
    /// sidecars are gone, the main database. The database candidate already
    /// carries exact snapshot absence plus old/fresh namespace binding; no
    /// single observation can mint this batch.
    pub(crate) fn admit_draining_old_database_payload_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetOldDatabasePayloadDrainCandidate<'data>,
        cache: AppDataResetManagedCacheAbsentWitness<'cache>,
        pre_effect_deadline: Instant,
    ) -> Result<AppDataResetDrainingOldDatabasePayloadBatch<'data, 'cache>> {
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining
            || data.state() != AppDataResetOldDatabasePayloadState::DatabasePresent
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to_transaction(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
            || data.deadline() != pre_effect_deadline
            || cache.deadline() != pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        let durable = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected
            || durable.phase() != AppDataResetPhase::Draining
            || Instant::now() >= pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingOldDatabasePayloadBatch {
            journal: durable,
            data,
            cache,
            pre_effect_deadline,
        })
    }

    /// Consume the sole coordinator-issued old-database payload capability.
    /// Local old/fresh/snapshot read-back occurs inside the storage completion;
    /// cache and journal read-back share that exact fresh deadline here.
    pub(crate) fn run_draining_old_database_payload_batch(
        &mut self,
        batch: AppDataResetDrainingOldDatabasePayloadBatch<'_, '_>,
    ) -> Result<AppDataResetOldDatabasePayloadDrainBatch> {
        let pre_effect_deadline = batch.pre_effect_deadline;
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }

        let completion = batch
            .data
            .drain_one(AppDataResetOldDatabasePayloadDrainAuthority {
                pre_effect_deadline,
            })
            .map_err(map_old_database_payload_drain_error)?;
        let post_effect_deadline = completion.post_effect_deadline();
        #[cfg(test)]
        if take_test_app_data_reset_coordinator_postcheck_fault(
            TestAppDataResetCoordinatorPostcheckFault::ExhaustBeforeCacheReadback,
        ) {
            exhaust_coordinator_postcheck_deadline(post_effect_deadline);
        }
        batch
            .cache
            .revalidate_after_effect_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        #[cfg(test)]
        if take_test_app_data_reset_coordinator_postcheck_fault(
            TestAppDataResetCoordinatorPostcheckFault::ExhaustBeforeJournalReadback,
        ) {
            exhaust_coordinator_postcheck_deadline(post_effect_deadline);
        }
        let durable = self
            .storage
            .read_journal_exact_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal || Instant::now() >= post_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(completion.into_progress())
    }

    /// Join exact cache absence to one protocol-ordered old-store control or
    /// empty detached root shell. No payload observation can mint this batch.
    pub(crate) fn admit_draining_old_database_store_retirement_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetOldDatabaseStoreRetirementCandidate<'data>,
        cache: AppDataResetManagedCacheAbsentWitness<'cache>,
        pre_effect_deadline: Instant,
    ) -> Result<AppDataResetDrainingOldDatabaseStoreRetirementBatch<'data, 'cache>> {
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to_transaction(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
            || data.deadline() != pre_effect_deadline
            || cache.deadline() != pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        let durable = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected
            || durable.phase() != AppDataResetPhase::Draining
            || Instant::now() >= pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingOldDatabaseStoreRetirementBatch {
            journal: durable,
            data,
            cache,
            pre_effect_deadline,
        })
    }

    /// Consume the sole coordinator-issued old-store structural capability.
    /// Old/fresh read-back occurs inside storage; cache and journal read-back
    /// share that exact fresh deadline here.
    pub(crate) fn run_draining_old_database_store_retirement_batch(
        &mut self,
        batch: AppDataResetDrainingOldDatabaseStoreRetirementBatch<'_, '_>,
    ) -> Result<AppDataResetOldDatabaseStoreRetirementBatch> {
        let pre_effect_deadline = batch.pre_effect_deadline;
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }

        let completion = batch
            .data
            .retire_one(AppDataResetOldDatabaseStoreRetireAuthority {
                pre_effect_deadline,
            })
            .map_err(map_old_database_store_retirement_error)?;
        let post_effect_deadline = completion.post_effect_deadline();
        #[cfg(test)]
        if take_test_app_data_reset_coordinator_postcheck_fault(
            TestAppDataResetCoordinatorPostcheckFault::ExhaustBeforeCacheReadback,
        ) {
            exhaust_coordinator_postcheck_deadline(post_effect_deadline);
        }
        batch
            .cache
            .revalidate_after_effect_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        #[cfg(test)]
        if take_test_app_data_reset_coordinator_postcheck_fault(
            TestAppDataResetCoordinatorPostcheckFault::ExhaustBeforeJournalReadback,
        ) {
            exhaust_coordinator_postcheck_deadline(post_effect_deadline);
        }
        let durable = self
            .storage
            .read_journal_exact_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal || Instant::now() >= post_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(completion.into_progress())
    }

    /// Join exact old-root and managed-cache absence to the durable V2
    /// `Draining` journal. The transaction-origin record is the final physical
    /// reset control: if present this batch can remove only that record; only
    /// a later origin-absent batch can publish `Complete`.
    pub(crate) fn admit_completion_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetOldDatabaseStoreAbsentWitness<'data>,
        cache: AppDataResetManagedCacheAbsentWitness<'cache>,
        pre_effect_deadline: Instant,
    ) -> Result<AppDataResetCompletionBatch<'data, 'cache>> {
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if current != *expected
            || !expected.is_bound_to_canonical_root_name(canonical_root_name)
            || !data.is_bound_to_transaction(
                &transaction,
                expected.data_identity(),
                fresh_identity,
                publication_parent_identity,
                canonical_root_name,
            )
            || !cache.is_bound_to(cache_identity, transaction.cache_stage())
            || data.deadline() != pre_effect_deadline
            || cache.deadline() != pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        let durable = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected
            || durable.phase() != AppDataResetPhase::Draining
            || Instant::now() >= pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetCompletionBatch {
            journal: durable,
            data,
            cache,
            pre_effect_deadline,
        })
    }

    pub(crate) fn run_completion_batch(
        &mut self,
        batch: AppDataResetCompletionBatch<'_, '_>,
    ) -> Result<AppDataResetCompletionBatchOutcome> {
        let AppDataResetCompletionBatch {
            journal,
            data,
            cache,
            pre_effect_deadline: deadline,
        } = batch;
        let current = self
            .storage
            .read_journal_exact_until(deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate_until(deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate_until(deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if Instant::now() >= deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }

        if data.fresh_origin_is_present() {
            let completion = data
                .retire_fresh_origin(
                    AppDataResetFreshOriginRetireAuthority {
                        pre_effect_deadline: deadline,
                    },
                    || {
                        cache.revalidate_until(deadline).map_err(|error| {
                            completed_reset_envelope_as_database_error(
                                map_managed_cache_before_effect_error(error.kind()),
                            )
                        })?;
                        let current = self
                            .storage
                            .read_journal_exact_until(deadline)
                            .map_err(|_| {
                                DatabaseOpenError::new(
                                    DatabaseOpenErrorKind::StorageRootUnavailable,
                                )
                            })?
                            .ok_or_else(|| {
                                DatabaseOpenError::new(DatabaseOpenErrorKind::UnsafeStorageObject)
                            })
                            .and_then(|bytes| {
                                decode_journal(&bytes).map_err(|_| {
                                    DatabaseOpenError::new(
                                        DatabaseOpenErrorKind::UnsafeStorageObject,
                                    )
                                })
                            })?;
                        if current != journal || Instant::now() >= deadline {
                            return Err(DatabaseOpenError::new(if Instant::now() >= deadline {
                                DatabaseOpenErrorKind::Busy
                            } else {
                                DatabaseOpenErrorKind::UnsafeStorageObject
                            }));
                        }
                        Ok(())
                    },
                )
                .map_err(map_fresh_origin_retirement_error)?;
            let post_effect_deadline = completion.post_effect_deadline();
            completion
                .revalidate_until(post_effect_deadline)
                .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
            cache
                .revalidate_after_effect_until(post_effect_deadline)
                .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
            let durable = self
                .storage
                .read_journal_exact_until(post_effect_deadline)
                .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
                .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
                .and_then(|bytes| {
                    decode_journal(&bytes)
                        .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
                })?;
            if durable != journal || Instant::now() >= post_effect_deadline {
                return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
            }
            return Ok(AppDataResetCompletionBatchOutcome::OriginRetired);
        }

        let complete = journal.advanced_complete()?;
        self.storage.write_journal(&encode_journal(&complete)?)?;
        data.revalidate_until(deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        cache
            .revalidate_until(deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        let durable = self
            .storage
            .read_journal_exact_until(deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != complete || Instant::now() >= deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(AppDataResetCompletionBatchOutcome::Complete(complete))
    }

    #[cfg(test)]
    pub(crate) fn replace_journal_for_fresh_binding_test(
        &mut self,
        journal: &AppDataResetJournal,
    ) -> Result<()> {
        self.storage.write_journal(&encode_journal(journal)?)
    }

    /// Acquire the data-root publication fence strictly inside this retained
    /// coordinator session and before database-side reset admission.
    ///
    /// The StoreCoordinator primitive is persistence-private; this is the only
    /// production route that lends its validation-only witness.
    pub(crate) fn with_data_namespace_admission_until<T>(
        &mut self,
        store: &StoreCoordinator,
        transaction: &AppDataResetTransaction,
        deadline: Instant,
        operation: impl for<'session, 'data> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetDataNamespaceAdmission<'data>,
        ) -> T,
    ) -> std::result::Result<T, HistoryError> {
        store.with_app_data_reset_data_namespace_admission_until(
            transaction,
            deadline,
            |data_namespace| operation(self, data_namespace),
        )
    }

    /// Acquire database-side reset admission strictly inside this retained
    /// coordinator session.
    ///
    /// The higher-ranked callback prevents an admitted store guard from
    /// escaping this call. Consequently the coordinator lock always outlives
    /// cleanup and database exclusion, and callers cannot invert that order.
    pub(crate) fn with_admitted_store<T>(
        &mut self,
        store: &StoreCoordinator,
        operation: impl for<'session, 'guard> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetStoreGuard<'guard>,
        ) -> T,
    ) -> std::result::Result<AppDataResetAdmittedStoreOutcome<T>, HistoryError> {
        match store.begin_app_data_reset_store_admission()? {
            AppDataResetStoreAdmission::Blocked(blockers) => {
                Ok(AppDataResetAdmittedStoreOutcome::Blocked(blockers))
            }
            AppDataResetStoreAdmission::Admitted(guard) => Ok(
                AppDataResetAdmittedStoreOutcome::Admitted(operation(self, guard)),
            ),
        }
    }

    /// Acquire database-side reset admission using the same absolute deadline
    /// as every earlier and later reset lock.
    pub(crate) fn with_admitted_store_until<T>(
        &mut self,
        store: &StoreCoordinator,
        deadline: Instant,
        operation: impl for<'session, 'guard> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetStoreGuard<'guard>,
        ) -> T,
    ) -> std::result::Result<AppDataResetAdmittedStoreOutcome<T>, HistoryError> {
        match store.begin_app_data_reset_store_admission_until(deadline)? {
            AppDataResetStoreAdmission::Blocked(blockers) => {
                Ok(AppDataResetAdmittedStoreOutcome::Blocked(blockers))
            }
            AppDataResetStoreAdmission::Admitted(guard) => Ok(
                AppDataResetAdmittedStoreOutcome::Admitted(operation(self, guard)),
            ),
        }
    }

    /// Return the exact current journal without releasing the retained lock.
    pub(crate) fn recover(&mut self) -> Result<Option<AppDataResetJournal>> {
        let Some(bytes) = self.storage.read_journal()? else {
            return Ok(None);
        };
        let journal = decode_journal(&bytes)?;
        if journal.has_canonical_root_name_binding()
            && !journal.is_bound_to_canonical_root_name(self.storage.data_root_binding().1)
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(Some(journal))
    }

    pub(crate) fn provisioning_debt(&mut self) -> Result<AppDataResetProvisioningDebt> {
        self.storage
            .reconcile_provisioning_stages()
            .map(|unproven_stage_count| AppDataResetProvisioningDebt {
                unproven_stage_count,
            })
    }

    /// Commit the first durable reset intent or replace one completed intent
    /// without releasing the retained lock.
    pub(crate) fn begin(&mut self, prepared: &AppDataResetJournal) -> Result<()> {
        prepared.validate()?;
        if prepared.phase != AppDataResetPhase::Prepared
            || !prepared.is_bound_to_canonical_root_name(self.storage.data_root_binding().1)
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        if let Some(current) = self
            .storage
            .read_journal()?
            .map(|bytes| decode_journal(&bytes))
            && current?.phase != AppDataResetPhase::Complete
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        self.storage.write_journal(&encode_journal(prepared)?)
    }

    /// Upgrade a legacy V1 `Prepared`/`CacheDetached` record only after the
    /// exact canonical old root has been reopened by identity. The V2 binding
    /// is durable before any namespace effect may follow.
    pub(crate) fn bind_legacy_canonical_root(
        &mut self,
        expected: &AppDataResetJournal,
        binding: AppDataResetCanonicalRootBinding<'_>,
    ) -> Result<AppDataResetJournal> {
        if binding.root_name() != self.storage.data_root_binding().1 {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let next = expected.bound_canonical_root(binding)?;
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != *expected {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        self.storage.write_journal(&encode_journal(&next)?)?;
        Ok(next)
    }

    /// Exact compare-and-advance while retaining the same exclusive lock.
    pub(crate) fn advance(
        &mut self,
        expected: &AppDataResetJournal,
        next_phase: AppDataResetPhase,
    ) -> Result<AppDataResetJournal> {
        expected.validate()?;
        if !expected.is_bound_to_canonical_root_name(self.storage.data_root_binding().1) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let next = expected.advanced(next_phase)?;
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != *expected {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        self.storage.write_journal(&encode_journal(&next)?)?;
        Ok(next)
    }

    /// Exact compare-and-advance for the only transition that must seal a new
    /// filesystem identity into the durable coordinator.
    pub(super) fn advance_fresh_namespace_identity(
        &mut self,
        expected: &AppDataResetJournal,
        fresh_identity: AppDataResetStoreIdentity,
    ) -> Result<AppDataResetJournal> {
        expected.validate()?;
        if !expected.is_bound_to_canonical_root_name(self.storage.data_root_binding().1) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let next = expected.advanced_fresh_namespace(fresh_identity)?;
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != *expected {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        self.storage.write_journal(&encode_journal(&next)?)?;
        Ok(next)
    }

    #[cfg(test)]
    pub(super) fn advance_late_phase_for_test(
        &mut self,
        expected: &AppDataResetJournal,
        phase: AppDataResetPhase,
    ) -> Result<AppDataResetJournal> {
        let next = expected.advanced_late_phase_for_test(phase)?;
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != *expected {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        self.storage.write_journal(&encode_journal(&next)?)?;
        Ok(next)
    }
}
