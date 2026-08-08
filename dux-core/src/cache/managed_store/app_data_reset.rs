use super::*;

impl AppDataResetManagedCacheDrainBatch {
    pub(crate) const fn removed_objects(self) -> u8 {
        self.removed_objects
    }

    pub(crate) const fn cache_payload_has_more(self) -> bool {
        self.cache_payload_has_more
    }
}

impl AppDataResetManagedCacheStageRetirementBatch {
    pub(crate) const fn removed_structural_objects(self) -> u8 {
        self.removed_structural_objects
    }

    pub(crate) const fn cache_stage_has_more(self) -> bool {
        self.cache_stage_has_more
    }
}

impl<'scope> AppDataResetManagedCacheRecoveryAdmission<'scope> {
    pub(crate) const fn location(&self) -> AppDataResetManagedCacheRecoveryLocation {
        self.location
    }

    pub(crate) fn revalidate(&self) -> Result<()> {
        self.revalidate_until(self.deadline)
    }

    fn revalidate_until(&self, deadline: Instant) -> Result<()> {
        let deadline = deadline.min(self.deadline);
        if Instant::now() >= deadline {
            return Err(busy());
        }
        self.publication.revalidate()?;
        match self.location {
            AppDataResetManagedCacheRecoveryLocation::Canonical
            | AppDataResetManagedCacheRecoveryLocation::Detached => {
                let store = self.store.ok_or_else(internal_state)?;
                let expected_inventory = self
                    .expected_inventory
                    .as_ref()
                    .ok_or_else(internal_state)?;
                let expected_identity = self.expected_identity.ok_or_else(internal_state)?;
                if platform::identity_parts(store.inner.directory_identity) != expected_identity {
                    return Err(changed());
                }
                let store_name = match self.location {
                    AppDataResetManagedCacheRecoveryLocation::Canonical => STORE_DIRECTORY_NAME,
                    AppDataResetManagedCacheRecoveryLocation::Detached => self.cache_stage.as_str(),
                    AppDataResetManagedCacheRecoveryLocation::ProvenAbsent => {
                        return Err(internal_state());
                    }
                };
                self.publication.validate_exact_recovery_location(
                    store,
                    store_name,
                    self.cache_stage,
                    self.location,
                    self.deadline,
                )?;
                let current = store
                    .inventory_locked_at_name_until(store_name, self.deadline)?
                    .facts();
                if current != *expected_inventory {
                    return Err(changed());
                }
            }
            AppDataResetManagedCacheRecoveryLocation::ProvenAbsent => {
                if self.store.is_some()
                    || self.expected_inventory.is_some()
                    || self._writer_lock.is_some()
                    || self.expected_identity.is_some()
                {
                    return Err(internal_state());
                }
                self.publication
                    .validate_exact_recovery_absence(self.cache_stage, self.deadline)?;
            }
        }
        if Instant::now() >= self.deadline {
            Err(busy())
        } else {
            Ok(())
        }
    }

    /// Consume this observation and detach only when the journaled store is
    /// still canonical. A previously detached store and proven prior absence
    /// are revalidated no-effect successes. Any rename-attempt uncertainty
    /// consumes the witness and is returned as an error for outer recovery.
    pub(crate) fn detach_if_canonical(mut self) -> Result<Self> {
        self.revalidate()?;
        if self.location == AppDataResetManagedCacheRecoveryLocation::Canonical {
            let store = self.store.ok_or_else(internal_state)?;
            let expected_inventory = self
                .expected_inventory
                .as_ref()
                .ok_or_else(internal_state)?;
            self.publication.detach_store(
                store,
                expected_inventory,
                self.cache_stage,
                self.deadline,
            )?;
            self.location = AppDataResetManagedCacheRecoveryLocation::Detached;
            self.revalidate()?;
        }
        Ok(self)
    }

    /// Convert an exact detached-or-absent recovery observation into bounded
    /// drain authority only when it still matches the journal-derived binding
    /// supplied by the coordinator layer.
    pub(crate) fn into_drain_candidate(
        self,
        expected_identity: Option<(u64, u64)>,
        expected_stage: &AppDataResetCacheStageName,
    ) -> Result<AppDataResetManagedCacheDrainCandidate<'scope>> {
        self.revalidate()?;
        if !self.has_drain_binding(expected_identity, expected_stage) {
            return Err(internal_state());
        }
        Ok(AppDataResetManagedCacheDrainCandidate { inner: self })
    }

    fn has_drain_binding(
        &self,
        expected_identity: Option<(u64, u64)>,
        expected_stage: &AppDataResetCacheStageName,
    ) -> bool {
        if self.cache_stage.as_str() != expected_stage.as_str()
            || self.expected_identity != expected_identity
        {
            return false;
        }
        match (expected_identity, self.location) {
            (Some(identity), AppDataResetManagedCacheRecoveryLocation::Detached) => {
                self.store.is_some_and(|store| {
                    platform::identity_parts(store.inner.directory_identity) == identity
                }) && self.expected_inventory.is_some()
                    && self._writer_lock.is_some()
            }
            (None, AppDataResetManagedCacheRecoveryLocation::ProvenAbsent) => {
                self.store.is_none()
                    && self.expected_inventory.is_none()
                    && self._writer_lock.is_none()
            }
            (Some(_), AppDataResetManagedCacheRecoveryLocation::Canonical)
            | (Some(_), AppDataResetManagedCacheRecoveryLocation::ProvenAbsent)
            | (None, AppDataResetManagedCacheRecoveryLocation::Canonical)
            | (None, AppDataResetManagedCacheRecoveryLocation::Detached) => false,
        }
    }
}

impl AppDataResetManagedCacheDrainCandidate<'_> {
    pub(crate) fn revalidate(&self) -> Result<()> {
        self.inner.revalidate()
    }

    pub(crate) fn is_bound_to(
        &self,
        expected_identity: Option<(u64, u64)>,
        expected_stage: &AppDataResetCacheStageName,
    ) -> bool {
        self.inner
            .has_drain_binding(expected_identity, expected_stage)
    }

    /// Remove the lexicographically first non-control object, if one exists.
    ///
    /// Production orchestration must invoke this only through its
    /// coordinator-issued `Draining` authority. The consumed candidate makes
    /// an unlink attempt non-repeatable; any uncertainty after a successful
    /// unlink is reported without revealing a path, object name, or byte count.
    pub(crate) fn drain_one_detached_payload(
        self,
        _authority: AppDataResetCacheDrainAuthority,
    ) -> std::result::Result<AppDataResetManagedCacheDrainBatch, AppDataResetManagedCacheDrainError>
    {
        let before_effect = |error: ManagedCacheStoreError| {
            AppDataResetManagedCacheDrainError::BeforeEffect(error.kind())
        };
        self.revalidate().map_err(before_effect)?;

        if self.inner.location == AppDataResetManagedCacheRecoveryLocation::ProvenAbsent {
            return Ok(AppDataResetManagedCacheDrainBatch {
                removed_objects: 0,
                cache_payload_has_more: false,
            });
        }
        if self.inner.location != AppDataResetManagedCacheRecoveryLocation::Detached {
            return Err(AppDataResetManagedCacheDrainError::BeforeEffect(
                ManagedCacheStoreErrorKind::InternalState,
            ));
        }

        let store = self.inner.store.ok_or({
            AppDataResetManagedCacheDrainError::BeforeEffect(
                ManagedCacheStoreErrorKind::InternalState,
            )
        })?;
        let expected_inventory = self.inner.expected_inventory.as_ref().ok_or({
            AppDataResetManagedCacheDrainError::BeforeEffect(
                ManagedCacheStoreErrorKind::InternalState,
            )
        })?;
        let mut current = store
            .inventory_locked_at_name_until(self.inner.cache_stage.as_str(), self.inner.deadline)
            .map_err(before_effect)?;
        if current.facts() != *expected_inventory {
            return Err(AppDataResetManagedCacheDrainError::BeforeEffect(
                ManagedCacheStoreErrorKind::ChangedSinceSnapshot,
            ));
        }
        if current.objects.is_empty() {
            return Ok(AppDataResetManagedCacheDrainBatch {
                removed_objects: 0,
                cache_payload_has_more: false,
            });
        }

        // `inventory_locked_at_name_until` sorts the complete bounded name
        // inventory and omits both retained controls from `objects`.
        let object = current.objects.remove(0);
        let mut expected_after = expected_inventory.clone();
        expected_after.objects.remove(0);
        let cache_payload_has_more = !expected_after.objects.is_empty();
        if Instant::now() >= self.inner.deadline {
            return Err(AppDataResetManagedCacheDrainError::BeforeEffect(
                ManagedCacheStoreErrorKind::Busy,
            ));
        }
        if take_test_fault(TEST_FAULT_RESET_DRAIN_BEFORE_UNLINK) {
            return Err(AppDataResetManagedCacheDrainError::BeforeEffect(
                ManagedCacheStoreErrorKind::Unavailable,
            ));
        }

        platform::remove_retained_file(
            &store.inner.directory,
            &object.name,
            object.file,
            object.facts.identity,
        )
        .map_err(before_effect)?;
        if take_test_fault(TEST_FAULT_RESET_DRAIN_AFTER_UNLINK) {
            return Err(AppDataResetManagedCacheDrainError::OutcomeUnknown);
        }
        platform::sync_directory(&store.inner.directory)
            .map_err(|_| AppDataResetManagedCacheDrainError::OutcomeUnknown)?;
        if take_test_fault(TEST_FAULT_RESET_DRAIN_AFTER_DIRECTORY_SYNC) {
            return Err(AppDataResetManagedCacheDrainError::OutcomeUnknown);
        }

        let post_effect_deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or(AppDataResetManagedCacheDrainError::OutcomeUnknown)?;
        self.inner
            .publication
            .validate_exact_recovery_location(
                store,
                self.inner.cache_stage.as_str(),
                self.inner.cache_stage,
                AppDataResetManagedCacheRecoveryLocation::Detached,
                post_effect_deadline,
            )
            .map_err(|_| AppDataResetManagedCacheDrainError::OutcomeUnknown)?;
        let after = store
            .inventory_locked_at_name_until(self.inner.cache_stage.as_str(), post_effect_deadline)
            .map_err(|_| AppDataResetManagedCacheDrainError::OutcomeUnknown)?;
        if take_test_fault(TEST_FAULT_RESET_DRAIN_DURING_READBACK) {
            return Err(AppDataResetManagedCacheDrainError::OutcomeUnknown);
        }
        if Instant::now() >= post_effect_deadline || after.facts() != expected_after {
            return Err(AppDataResetManagedCacheDrainError::OutcomeUnknown);
        }
        Ok(AppDataResetManagedCacheDrainBatch {
            removed_objects: 1,
            cache_payload_has_more,
        })
    }
}

impl AppDataResetManagedCacheStageRetirementCandidate<'_> {
    #[cfg(test)]
    pub(crate) const fn state(&self) -> AppDataResetManagedCacheStageRetirementState {
        self.state
    }

    pub(crate) fn is_bound_to(
        &self,
        expected_identity: Option<(u64, u64)>,
        expected_stage: &AppDataResetCacheStageName,
    ) -> bool {
        if self.expected_identity != expected_identity
            || self.cache_stage.as_str() != expected_stage.as_str()
        {
            return false;
        }
        match &self.inner {
            AppDataResetManagedCacheStageRetirementInner::FullControlsEmpty { store, .. } => {
                expected_identity.is_some_and(|identity| {
                    platform::identity_parts(store.inner.directory_identity) == identity
                })
            }
            AppDataResetManagedCacheStageRetirementInner::Partial(stage) => expected_identity
                .is_some_and(|identity| platform::identity_parts(stage.identity) == identity),
            AppDataResetManagedCacheStageRetirementInner::Absent => true,
        }
    }

    pub(crate) fn revalidate(&self) -> Result<()> {
        self.revalidate_until(self.deadline)
    }

    fn revalidate_until(&self, deadline: Instant) -> Result<()> {
        let deadline = deadline.min(self.deadline);
        if Instant::now() >= deadline {
            return Err(busy());
        }
        self.publication.revalidate()?;
        match &self.inner {
            AppDataResetManagedCacheStageRetirementInner::FullControlsEmpty {
                store,
                expected_inventory,
                _writer_lock: _,
            } => {
                let expected_identity = self.expected_identity.ok_or_else(internal_state)?;
                if self.state != AppDataResetManagedCacheStageRetirementState::FullControlsEmpty
                    || platform::identity_parts(store.inner.directory_identity) != expected_identity
                    || !expected_inventory.objects.is_empty()
                {
                    return Err(internal_state());
                }
                self.publication.validate_exact_recovery_location(
                    store,
                    self.cache_stage.as_str(),
                    self.cache_stage,
                    AppDataResetManagedCacheRecoveryLocation::Detached,
                    deadline,
                )?;
                let current =
                    store.inventory_locked_at_name_until(self.cache_stage.as_str(), deadline)?;
                if current.facts() != *expected_inventory || !current.objects.is_empty() {
                    return Err(changed());
                }
            }
            AppDataResetManagedCacheStageRetirementInner::Partial(stage) => {
                let expected_identity = self.expected_identity.ok_or_else(internal_state)?;
                if platform::identity_parts(stage.identity) != expected_identity {
                    return Err(internal_state());
                }
                self.publication.validate_exact_retirement_stage(
                    &stage.directory,
                    stage.identity,
                    self.cache_stage,
                    deadline,
                )?;
                let names = retirement_inventory_names(&stage.directory, deadline)?;
                match (self.state, stage.writer.as_ref()) {
                    (AppDataResetManagedCacheStageRetirementState::WriterOnly, Some(writer))
                        if names.as_slice() == [WRITER_LOCK_NAME] =>
                    {
                        platform::validate_named(
                            &stage.directory,
                            WRITER_LOCK_NAME,
                            &writer.file,
                            writer.identity,
                            platform::Kind::PrivateFile,
                        )?;
                        prove_control(&writer.file, WRITER_MARKER)?;
                    }
                    (AppDataResetManagedCacheStageRetirementState::EmptyStage, None)
                        if names.is_empty() => {}
                    _ => return Err(changed()),
                }
            }
            AppDataResetManagedCacheStageRetirementInner::Absent => {
                if self.state != AppDataResetManagedCacheStageRetirementState::Absent {
                    return Err(internal_state());
                }
                self.publication
                    .validate_exact_recovery_absence(self.cache_stage, deadline)?;
            }
        }
        if Instant::now() >= deadline {
            Err(busy())
        } else {
            Ok(())
        }
    }

    /// Retire exactly one structural object from the monotonic detached-cache
    /// tail. The opaque authority is distinct from payload drain authority and
    /// will be constructible in production only by the durable `Draining`
    /// coordinator join.
    pub(crate) fn retire_one_structure(
        self,
        _authority: AppDataResetCacheStageRetireAuthority,
    ) -> std::result::Result<
        AppDataResetManagedCacheStageRetirementBatch,
        AppDataResetManagedCacheStageRetirementError,
    > {
        let before_effect = |error: ManagedCacheStoreError| {
            AppDataResetManagedCacheStageRetirementError::BeforeEffect(error.kind())
        };
        self.revalidate().map_err(before_effect)?;
        if self.state == AppDataResetManagedCacheStageRetirementState::Absent {
            return Ok(AppDataResetManagedCacheStageRetirementBatch {
                removed_structural_objects: 0,
                cache_stage_has_more: false,
            });
        }
        if take_test_fault(TEST_FAULT_RESET_RETIRE_BEFORE_EFFECT) {
            return Err(AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                ManagedCacheStoreErrorKind::Unavailable,
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                ManagedCacheStoreErrorKind::Busy,
            ));
        }

        match &self.inner {
            AppDataResetManagedCacheStageRetirementInner::FullControlsEmpty { store, .. } => {
                let marker = store.inner.marker.try_clone().map_err(|_| {
                    AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                        ManagedCacheStoreErrorKind::Unavailable,
                    )
                })?;
                platform::remove_app_data_reset_stage_control(
                    &store.inner.directory,
                    MARKER_NAME,
                    marker,
                    store.inner.marker_identity,
                )
                .map_err(before_effect)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_AFTER_EFFECT) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                platform::sync_directory(&store.inner.directory)
                    .map_err(|_| AppDataResetManagedCacheStageRetirementError::OutcomeUnknown)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_AFTER_DIRECTORY_SYNC) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                let post_effect_deadline = retirement_post_effect_deadline()?;
                self.validate_writer_only_after_marker_removal(store, post_effect_deadline)
                    .map_err(|_| AppDataResetManagedCacheStageRetirementError::OutcomeUnknown)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_DURING_READBACK) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                Ok(AppDataResetManagedCacheStageRetirementBatch {
                    removed_structural_objects: 1,
                    cache_stage_has_more: true,
                })
            }
            AppDataResetManagedCacheStageRetirementInner::Partial(stage)
                if self.state == AppDataResetManagedCacheStageRetirementState::WriterOnly =>
            {
                let writer = stage.writer.as_ref().ok_or(
                    AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                        ManagedCacheStoreErrorKind::InternalState,
                    ),
                )?;
                let writer_file = writer.file.try_clone().map_err(|_| {
                    AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                        ManagedCacheStoreErrorKind::Unavailable,
                    )
                })?;
                platform::remove_app_data_reset_stage_control(
                    &stage.directory,
                    WRITER_LOCK_NAME,
                    writer_file,
                    writer.identity,
                )
                .map_err(before_effect)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_AFTER_EFFECT) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                platform::sync_directory(&stage.directory)
                    .map_err(|_| AppDataResetManagedCacheStageRetirementError::OutcomeUnknown)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_AFTER_DIRECTORY_SYNC) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                let post_effect_deadline = retirement_post_effect_deadline()?;
                self.validate_empty_after_writer_removal(stage, post_effect_deadline)
                    .map_err(|_| AppDataResetManagedCacheStageRetirementError::OutcomeUnknown)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_DURING_READBACK) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                Ok(AppDataResetManagedCacheStageRetirementBatch {
                    removed_structural_objects: 1,
                    cache_stage_has_more: true,
                })
            }
            AppDataResetManagedCacheStageRetirementInner::Partial(stage)
                if self.state == AppDataResetManagedCacheStageRetirementState::EmptyStage =>
            {
                let directory = stage.directory.try_clone().map_err(|_| {
                    AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                        ManagedCacheStoreErrorKind::Unavailable,
                    )
                })?;
                let container = self.publication.container.as_ref().ok_or(
                    AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                        ManagedCacheStoreErrorKind::InternalState,
                    ),
                )?;
                platform::remove_app_data_reset_retired_stage_directory(
                    &container.file,
                    self.cache_stage.as_str(),
                    directory,
                    stage.identity,
                )
                .map_err(before_effect)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_AFTER_EFFECT) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                platform::sync_directory(&container.file)
                    .map_err(|_| AppDataResetManagedCacheStageRetirementError::OutcomeUnknown)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_AFTER_DIRECTORY_SYNC) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                let post_effect_deadline = retirement_post_effect_deadline()?;
                self.publication
                    .validate_exact_recovery_absence(self.cache_stage, post_effect_deadline)
                    .map_err(|_| AppDataResetManagedCacheStageRetirementError::OutcomeUnknown)?;
                if take_test_fault(TEST_FAULT_RESET_RETIRE_DURING_READBACK) {
                    return Err(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown);
                }
                Ok(AppDataResetManagedCacheStageRetirementBatch {
                    removed_structural_objects: 1,
                    cache_stage_has_more: false,
                })
            }
            AppDataResetManagedCacheStageRetirementInner::Absent => {
                Err(AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                    ManagedCacheStoreErrorKind::InternalState,
                ))
            }
            AppDataResetManagedCacheStageRetirementInner::Partial(_) => {
                Err(AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                    ManagedCacheStoreErrorKind::InternalState,
                ))
            }
        }
    }

    fn validate_writer_only_after_marker_removal(
        &self,
        store: &ManagedCacheStore,
        deadline: Instant,
    ) -> Result<()> {
        self.publication.validate_exact_retirement_stage(
            &store.inner.directory,
            store.inner.directory_identity,
            self.cache_stage,
            deadline,
        )?;
        if retirement_inventory_names(&store.inner.directory, deadline)?.as_slice()
            != [WRITER_LOCK_NAME]
        {
            return Err(changed());
        }
        platform::validate_named(
            &store.inner.directory,
            WRITER_LOCK_NAME,
            &store.inner.writer_lock,
            store.inner.writer_lock_identity,
            platform::Kind::PrivateFile,
        )?;
        prove_control(&store.inner.writer_lock, WRITER_MARKER)
    }

    fn validate_empty_after_writer_removal(
        &self,
        stage: &AppDataResetManagedCachePartialRetirementStage,
        deadline: Instant,
    ) -> Result<()> {
        self.publication.validate_exact_retirement_stage(
            &stage.directory,
            stage.identity,
            self.cache_stage,
            deadline,
        )?;
        if retirement_inventory_names(&stage.directory, deadline)?.is_empty() {
            Ok(())
        } else {
            Err(changed())
        }
    }
}

impl AppDataResetManagedCacheAbsentWitness<'_> {
    pub(crate) const fn deadline(&self) -> Instant {
        self.inner.deadline
    }

    pub(crate) fn is_bound_to(
        &self,
        expected_identity: Option<(u64, u64)>,
        expected_stage: &AppDataResetCacheStageName,
    ) -> bool {
        self.inner.state == AppDataResetManagedCacheStageRetirementState::Absent
            && self.inner.is_bound_to(expected_identity, expected_stage)
    }

    #[cfg(test)]
    pub(crate) fn revalidate(&self) -> Result<()> {
        self.revalidate_until(self.inner.deadline)
    }

    pub(crate) fn revalidate_until(&self, deadline: Instant) -> Result<()> {
        if self.inner.state != AppDataResetManagedCacheStageRetirementState::Absent {
            return Err(internal_state());
        }
        self.inner.revalidate_until(deadline)
    }

    /// Re-read exact cache absence under a post-effect certainty deadline.
    /// Unlike ordinary admission revalidation, this no-effect check must not
    /// be clipped to the already-consumed pre-effect deadline.
    pub(crate) fn revalidate_after_effect_until(&self, deadline: Instant) -> Result<()> {
        if self.inner.state != AppDataResetManagedCacheStageRetirementState::Absent
            || !matches!(
                &self.inner.inner,
                AppDataResetManagedCacheStageRetirementInner::Absent
            )
            || Instant::now() >= deadline
        {
            return Err(internal_state());
        }
        self.inner.publication.revalidate()?;
        self.inner
            .publication
            .validate_exact_recovery_absence(self.inner.cache_stage, deadline)?;
        if Instant::now() >= deadline {
            Err(busy())
        } else {
            Ok(())
        }
    }
}

fn retirement_post_effect_deadline()
-> std::result::Result<Instant, AppDataResetManagedCacheStageRetirementError> {
    Instant::now()
        .checked_add(INVENTORY_DEADLINE)
        .ok_or(AppDataResetManagedCacheStageRetirementError::OutcomeUnknown)
}

impl AppDataResetManagedCacheAdmission<'_> {
    pub(crate) const fn is_present(&self) -> bool {
        matches!(
            self.state,
            AppDataResetManagedCacheAdmissionState::Present { .. }
        )
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn journal_identity_parts(&self) -> Option<(u64, u64)> {
        match &self.state {
            AppDataResetManagedCacheAdmissionState::Present { store, .. } => {
                Some(platform::identity_parts(store.inner.directory_identity))
            }
            AppDataResetManagedCacheAdmissionState::Absent { .. } => None,
        }
    }

    pub(crate) fn revalidate(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(busy());
        }
        match &self.state {
            AppDataResetManagedCacheAdmissionState::Present {
                store,
                expected,
                publication,
                cache_stage,
                ..
            } => {
                publication.revalidate()?;
                if self.detached {
                    publication.validate_detached_store(store, cache_stage)?;
                } else {
                    publication.validate_cache_stage_absent(cache_stage)?;
                    publication.validate_store(store)?;
                }
                let current = if self.detached {
                    store.inventory_locked_at_name_until(cache_stage.as_str(), self.deadline)?
                } else {
                    store.inventory_locked_until(self.deadline)?
                }
                .facts();
                if current == *expected {
                    if Instant::now() >= self.deadline {
                        Err(busy())
                    } else {
                        Ok(())
                    }
                } else {
                    Err(changed())
                }
            }
            AppDataResetManagedCacheAdmissionState::Absent {
                publication,
                cache_stage,
            } => {
                publication.revalidate()?;
                publication.validate_cache_stage_absent(cache_stage)?;
                if let Some(container) = publication.container.as_ref()
                    && platform::open_existing_private_directory(
                        &container.file,
                        &publication.container_path,
                        STORE_DIRECTORY_NAME,
                    )?
                    .is_some()
                {
                    return Err(changed());
                }
                if Instant::now() >= self.deadline {
                    Err(busy())
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Atomically detach the exact admitted cache child to the transaction's
    /// fixed stage name. Once the caller has persisted `Prepared`, every error
    /// from this method is recovery-required: validation can prove that an
    /// effect has not happened, but it cannot authorize abandoning or retrying
    /// the already committed transaction.
    pub(crate) fn detach(
        mut self,
        expected_identity: Option<(u64, u64)>,
        expected_stage_name: Option<&str>,
    ) -> Result<Self> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (expected_identity, expected_stage_name);
            return Err(unsupported());
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            if self.detached {
                return Err(internal_state());
            }
            let binding_matches = match &self.state {
                AppDataResetManagedCacheAdmissionState::Present {
                    store, cache_stage, ..
                } => {
                    expected_identity
                        == Some(platform::identity_parts(store.inner.directory_identity))
                        && expected_stage_name == Some(cache_stage.as_str())
                }
                AppDataResetManagedCacheAdmissionState::Absent { .. } => {
                    expected_identity.is_none() && expected_stage_name.is_none()
                }
            };
            if !binding_matches {
                return Err(internal_state());
            }
            self.revalidate()?;
            if let AppDataResetManagedCacheAdmissionState::Present {
                store,
                expected,
                publication,
                cache_stage,
                ..
            } = &self.state
            {
                publication.detach_store(store, expected, cache_stage, self.deadline)?;
            }
            self.detached = true;
            Ok(self)
        }
    }
}
