use super::*;

impl ManagedCacheStore {
    /// Open the fixed marker-owned child beneath the conventional cache
    /// container. Read-only access never creates either directory.
    pub(crate) fn open(
        conventional_container: &Path,
        access: ManagedCacheStoreAccess,
    ) -> Result<Option<Self>> {
        let deadline = Instant::now().checked_add(LOCK_TIMEOUT).ok_or_else(|| {
            ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InternalState)
        })?;
        Self::open_until(conventional_container, access, deadline)
    }

    pub(crate) fn open_until(
        conventional_container: &Path,
        access: ManagedCacheStoreAccess,
        deadline: Instant,
    ) -> Result<Option<Self>> {
        let publication = PublicationFence::acquire(conventional_container, access, deadline)?;
        let Some(store) = Self::open_under_publication_fence(&publication, access)? else {
            return Ok(None);
        };
        let lock = store.acquire_writer_lock_until(deadline)?;
        store.inventory_locked()?;
        drop(lock);
        Ok(Some(store))
    }

    fn open_under_publication_fence(
        publication: &PublicationFence,
        access: ManagedCacheStoreAccess,
    ) -> Result<Option<Self>> {
        publication.revalidate()?;
        let Some(container) = publication.container.as_ref() else {
            return Ok(None);
        };
        let path = publication.container_path.join(STORE_DIRECTORY_NAME);
        let store = match platform::open_existing_private_directory(
            &container.file,
            &publication.container_path,
            STORE_DIRECTORY_NAME,
        )? {
            Some(directory) => Self::from_open_directory(publication, path, directory, access)?,
            None if access == ManagedCacheStoreAccess::ReadOnly => return Ok(None),
            None => Self::provision(publication, path, access)?,
        };
        publication.validate_store(&store)?;
        Ok(Some(store))
    }

    pub(super) fn provision(
        publication: &PublicationFence,
        path: PathBuf,
        access: ManagedCacheStoreAccess,
    ) -> Result<Self> {
        let container = publication.container.as_ref().ok_or_else(internal_state)?;
        for _ in 0..RANDOM_ATTEMPTS {
            let stage_name = random_stage_name()?;
            let Some(directory) = platform::create_private_directory_exclusive(
                &container.file,
                &publication.container_path,
                &stage_name,
            )?
            else {
                continue;
            };
            let stage_path = publication.container_path.join(&stage_name);
            let directory_identity =
                platform::identity(&directory, platform::Kind::PrivateDirectory)?;
            let mut stage = ProvisioningStage {
                name: stage_name,
                directory,
                identity: directory_identity,
                controls: Vec::with_capacity(2),
            };
            let prepared = (|| {
                let (marker, marker_identity) =
                    stage.create_control(&stage_path, MARKER_NAME, STORE_MARKER)?;
                let (writer_lock, writer_lock_identity) =
                    stage.create_control(&stage_path, WRITER_LOCK_NAME, WRITER_MARKER)?;
                platform::sync_directory(&stage.directory)?;
                publication.revalidate()?;
                Ok((marker, marker_identity, writer_lock, writer_lock_identity))
            })();
            let (marker, marker_identity, writer_lock, writer_lock_identity) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    return if stage.cleanup(&container.file).is_ok() {
                        Err(error)
                    } else {
                        Err(unsafe_store())
                    };
                }
            };
            let published = platform::publish_directory_no_replace(
                &container.file,
                &stage.name,
                &stage.directory,
                stage.identity,
                STORE_DIRECTORY_NAME,
            );
            match published {
                Err(error) => {
                    return if stage.cleanup(&container.file).is_ok() {
                        Err(error)
                    } else {
                        Err(unsafe_store())
                    };
                }
                Ok(platform::Publication::Published) => {
                    platform::sync_directory(&container.file).map_err(|_| outcome_unknown())?;
                    platform::validate_named(
                        &container.file,
                        STORE_DIRECTORY_NAME,
                        &stage.directory,
                        stage.identity,
                        platform::Kind::PrivateDirectory,
                    )
                    .map_err(|_| outcome_unknown())?;
                    return Ok(Self {
                        inner: Arc::new(StoreInner {
                            parent_path: publication.parent_path.clone(),
                            parent: publication
                                .parent
                                .try_clone()
                                .map_err(|_| outcome_unknown())?,
                            parent_identity: publication.parent_identity,
                            container_path: publication.container_path.clone(),
                            container: container.file.try_clone().map_err(|_| outcome_unknown())?,
                            container_identity: container.identity,
                            path,
                            directory: stage.directory,
                            directory_identity: stage.identity,
                            marker,
                            marker_identity,
                            writer_lock,
                            writer_lock_identity,
                            writer_in_use: AtomicBool::new(false),
                        }),
                        access,
                    });
                }
                Ok(platform::Publication::Collision) => {
                    stage.cleanup(&container.file)?;
                    let directory = platform::open_existing_private_directory(
                        &container.file,
                        &publication.container_path,
                        STORE_DIRECTORY_NAME,
                    )?
                    .ok_or_else(unsafe_store)?;
                    return Self::from_open_directory(publication, path, directory, access);
                }
            }
        }
        Err(unavailable())
    }

    fn from_open_directory(
        publication: &PublicationFence,
        path: PathBuf,
        directory: File,
        access: ManagedCacheStoreAccess,
    ) -> Result<Self> {
        let container = publication.container.as_ref().ok_or_else(internal_state)?;
        let directory_identity = platform::identity(&directory, platform::Kind::PrivateDirectory)?;
        platform::validate_retained(
            &directory,
            directory_identity,
            platform::Kind::PrivateDirectory,
            false,
        )?;
        let (marker, marker_identity) =
            open_control(&directory, &path, MARKER_NAME, STORE_MARKER, false)?
                .ok_or_else(unrecognized)?;
        let (writer_lock, writer_lock_identity) =
            open_control(&directory, &path, WRITER_LOCK_NAME, WRITER_MARKER, true)?
                .ok_or_else(unsafe_object)?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                parent_path: publication.parent_path.clone(),
                parent: publication.parent.try_clone().map_err(|_| unavailable())?,
                parent_identity: publication.parent_identity,
                container_path: publication.container_path.clone(),
                container: container.file.try_clone().map_err(|_| unavailable())?,
                container_identity: container.identity,
                path,
                directory,
                directory_identity,
                marker,
                marker_identity,
                writer_lock,
                writer_lock_identity,
                writer_in_use: AtomicBool::new(false),
            }),
            access,
        })
    }

    pub(crate) fn load(
        &self,
        canonical_root: &Path,
        config: &CachedScanConfig,
    ) -> Result<Option<ManagedCacheDocument>> {
        let key = managed_cache_entry_key(canonical_root, config).map_err(|_| corrupt())?;
        let name = final_name(&key);
        let lock = self.acquire_writer_lock(LOCK_TIMEOUT)?;
        let inventory = self.inventory_locked()?;
        let Some(object) = inventory
            .objects
            .iter()
            .find(|object| object.kind == InventoryKind::Entry && object.name == name)
        else {
            drop(lock);
            return Ok(None);
        };
        platform::validate_named(
            &self.inner.directory,
            &object.name,
            &object.file,
            object.facts.identity,
            platform::Kind::PrivateFile,
        )?;
        let document = decode_validated_entry(&object.file, object.facts, canonical_root, config)?;
        platform::validate_named(
            &self.inner.directory,
            &object.name,
            &object.file,
            object.facts.identity,
            platform::Kind::PrivateFile,
        )?;
        drop(lock);
        Ok(Some(document))
    }

    pub(crate) fn save(
        &self,
        canonical_root: &Path,
        config: &CachedScanConfig,
        metadata: &CacheMetadata,
        tree: &DiskTree,
    ) -> std::result::Result<(), ManagedCacheSaveError> {
        if self.access != ManagedCacheStoreAccess::ReadWrite {
            return Err(ManagedCacheSaveError::BeforePublication(
                ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::ReadOnly),
            ));
        }
        let key = managed_cache_entry_key(canonical_root, config)
            .map_err(|_| ManagedCacheSaveError::BeforePublication(corrupt()))?;
        let bytes = encode_managed_cache(canonical_root, config, metadata, tree)
            .map_err(|_| ManagedCacheSaveError::BeforePublication(corrupt()))?;
        if u64::try_from(bytes.len()).ok().is_none_or(|length| {
            length > MAX_MANAGED_CACHE_FILE_BYTES
                || length < u64::try_from(MANAGED_CACHE_HEADER_BYTES).unwrap_or(u64::MAX)
        }) {
            return Err(ManagedCacheSaveError::BeforePublication(budget()));
        }
        let lock = self
            .acquire_writer_lock(LOCK_TIMEOUT)
            .map_err(ManagedCacheSaveError::BeforePublication)?;
        let final_name = final_name(&key);
        let inventory = self
            .inventory_locked()
            .map_err(ManagedCacheSaveError::BeforePublication)?;
        inventory
            .reserve_save_capacity(&final_name)
            .map_err(ManagedCacheSaveError::BeforePublication)?;
        let mut temp = self
            .create_temp(&key)
            .map_err(ManagedCacheSaveError::BeforePublication)?;
        let prepublication = (|| -> Result<()> {
            if take_test_fault(TEST_FAULT_PRE_PUBLICATION) {
                return Err(unavailable());
            }
            let file = temp.file_mut()?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| unavailable())?;
            let file = temp.file()?;
            let temp_facts = ObjectFacts {
                identity: temp.identity,
                change: platform::change_token(file)?,
                usage: ManagedCacheStorageUsage::from_file(file)?,
            };
            if temp_facts.usage.logical_bytes != u64::try_from(bytes.len()).unwrap_or(u64::MAX) {
                return Err(unsafe_object());
            }
            platform::validate_named(
                &self.inner.directory,
                &temp.name,
                file,
                temp.identity,
                platform::Kind::PrivateFile,
            )?;
            decode_validated_entry(file, temp_facts, canonical_root, config)?;
            let publication = platform::publish_file_replace(
                &self.inner.directory,
                &temp.name,
                file,
                temp.identity,
                &final_name,
            )?;
            if publication != platform::Publication::Published {
                return Err(internal_state());
            }
            Ok(())
        })();
        if let Err(error) = prepublication {
            let cleaned = temp.cleanup(&self.inner.directory).is_ok();
            drop(lock);
            return Err(ManagedCacheSaveError::BeforePublication(if cleaned {
                error
            } else {
                unsafe_object()
            }));
        }
        let published_identity = temp.identity;
        let published = temp
            .into_published_file()
            .map_err(|_| ManagedCacheSaveError::OutcomeUnknown)?;
        if take_test_fault(TEST_FAULT_POST_PUBLICATION) {
            drop(lock);
            return Err(ManagedCacheSaveError::OutcomeUnknown);
        }
        platform::sync_directory(&self.inner.directory)
            .map_err(|_| ManagedCacheSaveError::OutcomeUnknown)?;
        platform::validate_named(
            &self.inner.directory,
            &final_name,
            &published,
            published_identity,
            platform::Kind::PrivateFile,
        )
        .map_err(|_| ManagedCacheSaveError::OutcomeUnknown)?;
        self.inventory_locked()
            .map_err(|_| ManagedCacheSaveError::OutcomeUnknown)?;
        drop(lock);
        Ok(())
    }

    pub(crate) fn footprint(&self) -> Result<ManagedCacheStoreFootprint> {
        let lock = self.acquire_writer_lock(LOCK_TIMEOUT)?;
        let footprint = self.inventory_locked()?.footprint()?;
        drop(lock);
        Ok(footprint)
    }

    /// Retain the present store's writer lock and complete bounded inventory
    /// during one reset callback without exposing either owned capability.
    #[cfg(test)]
    pub(crate) fn with_app_data_reset_writer_admission<T>(
        &self,
        timeout: Duration,
        admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheAdmission<'scope>) -> T,
    ) -> Result<T> {
        let transaction = AppDataResetTransaction::for_test("00112233445566778899aabbccddeeff")
            .ok_or_else(internal_state)?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InternalState)
        })?;
        let publication = PublicationFence::acquire(
            &self.inner.container_path,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        publication.validate_store(self)?;
        let writer_lock = self.acquire_writer_lock_until_mode(deadline, true)?;
        self.with_retained_app_data_reset_writer(
            &publication,
            transaction.cache_stage(),
            writer_lock,
            deadline,
            admitted,
        )
    }

    /// Test-only probe for the child writer independent of publication fences.
    #[cfg(test)]
    pub(crate) fn with_test_child_writer_until<T>(
        &self,
        deadline: Instant,
        operation: impl FnOnce() -> T,
    ) -> Result<T> {
        let writer = self.acquire_writer_lock_until(deadline)?;
        let result = operation();
        drop(writer);
        Ok(result)
    }

    /// Test-only probe for publication exclusion independent of the child
    /// writer.
    #[cfg(test)]
    pub(crate) fn with_test_publication_fence_until<T>(
        conventional_container: &Path,
        deadline: Instant,
        operation: impl FnOnce() -> T,
    ) -> Result<T> {
        let publication = PublicationFence::acquire(
            conventional_container,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        publication.revalidate()?;
        let result = operation();
        drop(publication);
        Ok(result)
    }

    /// Inspect only the canonical managed-cache child and the exact
    /// journal-derived detached stage under retained publication fences.
    ///
    /// A journaled identity requires exactly one matching marker-owned store;
    /// a journaled absence requires both names to be proven absent. The
    /// callback retains a writer lease and complete inventory for a present
    /// store. This path never provisions or repairs either namespace.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn with_app_data_reset_recovery_admission_until<T>(
        conventional_container: &Path,
        expected_identity: Option<(u64, u64)>,
        cache_stage: &AppDataResetCacheStageName,
        deadline: Instant,
        admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheRecoveryAdmission<'scope>) -> T,
    ) -> Result<T> {
        let publication = PublicationFence::acquire(
            conventional_container,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        let canonical =
            publication.open_exact_recovery_directory(STORE_DIRECTORY_NAME, deadline)?;
        let detached = publication.open_exact_recovery_directory(cache_stage.as_str(), deadline)?;

        let Some(expected_identity) = expected_identity else {
            if canonical.is_some() || detached.is_some() {
                return Err(changed());
            }
            let admission = AppDataResetManagedCacheRecoveryAdmission {
                store: None,
                expected_inventory: None,
                _writer_lock: None,
                publication: &publication,
                cache_stage,
                expected_identity: None,
                location: AppDataResetManagedCacheRecoveryLocation::ProvenAbsent,
                deadline,
            };
            admission.revalidate()?;
            return Ok(admitted(admission));
        };

        let (directory, location, store_name) = match (canonical, detached) {
            (Some(directory), None) => (
                directory,
                AppDataResetManagedCacheRecoveryLocation::Canonical,
                STORE_DIRECTORY_NAME,
            ),
            (None, Some(directory)) => (
                directory,
                AppDataResetManagedCacheRecoveryLocation::Detached,
                cache_stage.as_str(),
            ),
            (Some(_), Some(_)) | (None, None) => return Err(changed()),
        };
        let store = Self::from_open_directory(
            &publication,
            publication.container_path.join(store_name),
            directory,
            ManagedCacheStoreAccess::ReadOnly,
        )?;
        if platform::identity_parts(store.inner.directory_identity) != expected_identity {
            return Err(changed());
        }
        let writer_lock = store.acquire_writer_lock_at_name_until(store_name, deadline)?;
        let expected_inventory = store
            .inventory_locked_at_name_until(store_name, deadline)?
            .facts();
        let admission = AppDataResetManagedCacheRecoveryAdmission {
            store: Some(&store),
            expected_inventory: Some(expected_inventory),
            _writer_lock: Some(&writer_lock),
            publication: &publication,
            cache_stage,
            expected_identity: Some(expected_identity),
            location,
            deadline,
        };
        admission.revalidate()?;
        Ok(admitted(admission))
    }

    /// Inspect the exact detached managed-cache tail for an already-durable
    /// `Draining` recovery pass. This boundary never falls back from an unsafe
    /// structural shape: a full store with payloads is returned as the existing
    /// payload candidate, while only the exact monotonic structural states can
    /// produce a retirement candidate.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn with_app_data_reset_draining_cache_admission_until<T>(
        conventional_container: &Path,
        expected_identity: Option<(u64, u64)>,
        cache_stage: &AppDataResetCacheStageName,
        deadline: Instant,
        admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheDrainingAdmission<'scope>) -> T,
    ) -> Result<T> {
        let publication = PublicationFence::acquire(
            conventional_container,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        let canonical =
            publication.open_exact_recovery_directory(STORE_DIRECTORY_NAME, deadline)?;
        let detached = publication.open_exact_recovery_directory(cache_stage.as_str(), deadline)?;
        if canonical.is_some() {
            return Err(changed());
        }

        let Some(directory) = detached else {
            publication.validate_exact_recovery_absence(cache_stage, deadline)?;
            let candidate = AppDataResetManagedCacheStageRetirementCandidate {
                inner: AppDataResetManagedCacheStageRetirementInner::Absent,
                publication: &publication,
                cache_stage,
                expected_identity,
                state: AppDataResetManagedCacheStageRetirementState::Absent,
                deadline,
            };
            candidate.revalidate()?;
            return Ok(admitted(AppDataResetManagedCacheDrainingAdmission::Absent(
                AppDataResetManagedCacheAbsentWitness { inner: candidate },
            )));
        };

        let expected_identity = expected_identity.ok_or_else(changed)?;
        let identity = platform::identity(&directory, platform::Kind::PrivateDirectory)?;
        if platform::identity_parts(identity) != expected_identity {
            return Err(changed());
        }
        publication.validate_exact_retirement_stage(&directory, identity, cache_stage, deadline)?;
        let names = retirement_inventory_names(&directory, deadline)?;
        let has_marker = names.iter().any(|name| name == MARKER_NAME);
        let has_writer = names.iter().any(|name| name == WRITER_LOCK_NAME);
        let stage_path = publication.container_path.join(cache_stage.as_str());

        match (has_marker, has_writer) {
            (true, true) => {
                let store = Self::from_open_directory(
                    &publication,
                    stage_path,
                    directory,
                    ManagedCacheStoreAccess::ReadOnly,
                )?;
                if platform::identity_parts(store.inner.directory_identity) != expected_identity {
                    return Err(changed());
                }
                let writer_lock =
                    store.acquire_writer_lock_at_name_until(cache_stage.as_str(), deadline)?;
                let inventory =
                    store.inventory_locked_at_name_until(cache_stage.as_str(), deadline)?;
                let payloads_remain = !inventory.objects.is_empty();
                let expected_inventory = inventory.facts();
                if payloads_remain {
                    let recovery = AppDataResetManagedCacheRecoveryAdmission {
                        store: Some(&store),
                        expected_inventory: Some(expected_inventory),
                        _writer_lock: Some(&writer_lock),
                        publication: &publication,
                        cache_stage,
                        expected_identity: Some(expected_identity),
                        location: AppDataResetManagedCacheRecoveryLocation::Detached,
                        deadline,
                    };
                    recovery.revalidate()?;
                    let candidate = AppDataResetManagedCacheDrainCandidate { inner: recovery };
                    return Ok(admitted(
                        AppDataResetManagedCacheDrainingAdmission::PayloadsRemain(candidate),
                    ));
                }

                let candidate = AppDataResetManagedCacheStageRetirementCandidate {
                    inner: AppDataResetManagedCacheStageRetirementInner::FullControlsEmpty {
                        store: &store,
                        expected_inventory,
                        _writer_lock: &writer_lock,
                    },
                    publication: &publication,
                    cache_stage,
                    expected_identity: Some(expected_identity),
                    state: AppDataResetManagedCacheStageRetirementState::FullControlsEmpty,
                    deadline,
                };
                candidate.revalidate()?;
                Ok(admitted(
                    AppDataResetManagedCacheDrainingAdmission::Retirement(candidate),
                ))
            }
            (false, true) if names.len() == 1 => {
                let (writer, writer_identity) = open_control(
                    &directory,
                    &stage_path,
                    WRITER_LOCK_NAME,
                    WRITER_MARKER,
                    true,
                )?
                .ok_or_else(unsafe_object)?;
                acquire_retirement_writer_lock(&writer, deadline)?;
                platform::validate_named(
                    &directory,
                    WRITER_LOCK_NAME,
                    &writer,
                    writer_identity,
                    platform::Kind::PrivateFile,
                )?;
                prove_control(&writer, WRITER_MARKER)?;
                let partial = AppDataResetManagedCachePartialRetirementStage {
                    directory,
                    identity,
                    writer: Some(AppDataResetManagedCacheRetirementControl {
                        file: writer,
                        identity: writer_identity,
                    }),
                };
                let candidate = AppDataResetManagedCacheStageRetirementCandidate {
                    inner: AppDataResetManagedCacheStageRetirementInner::Partial(&partial),
                    publication: &publication,
                    cache_stage,
                    expected_identity: Some(expected_identity),
                    state: AppDataResetManagedCacheStageRetirementState::WriterOnly,
                    deadline,
                };
                candidate.revalidate()?;
                Ok(admitted(
                    AppDataResetManagedCacheDrainingAdmission::Retirement(candidate),
                ))
            }
            (false, false) if names.is_empty() => {
                let partial = AppDataResetManagedCachePartialRetirementStage {
                    directory,
                    identity,
                    writer: None,
                };
                let candidate = AppDataResetManagedCacheStageRetirementCandidate {
                    inner: AppDataResetManagedCacheStageRetirementInner::Partial(&partial),
                    publication: &publication,
                    cache_stage,
                    expected_identity: Some(expected_identity),
                    state: AppDataResetManagedCacheStageRetirementState::EmptyStage,
                    deadline,
                };
                candidate.revalidate()?;
                Ok(admitted(
                    AppDataResetManagedCacheDrainingAdmission::Retirement(candidate),
                ))
            }
            // Marker-only is never a state produced by this protocol. Every
            // other partial-control or payload-without-controls shape is also
            // unsafe and must not be reinterpreted as retirement progress.
            _ => Err(unsafe_object()),
        }
    }

    /// Read-only validation for a durable V2 completed reset. The transaction
    /// stage must remain absent forever. A canonical cache may be absent on
    /// the first ordinary open or may be a later valid store whose identity is
    /// distinct from the detached cache recorded by the reset. Once stage
    /// absence is proven, an exact set of optional canonical-object failures
    /// is delegated to `ManagedScanCache` as ordinary evolved cache state.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn validate_app_data_reset_completed_namespace_until(
        conventional_container: &Path,
        retired_identity: Option<(u64, u64)>,
        cache_stage: &AppDataResetCacheStageName,
        allow_canonical: bool,
        deadline: Instant,
    ) -> Result<AppDataResetCompletedCacheFence> {
        // Even after ordinary store initialization, reset's exact transaction
        // stage must be proven absent under the retained publication fence.
        // Optional-cache policy therefore cannot swallow fence acquisition,
        // configuration, contention, or namespace-drift errors.
        let publication = PublicationFence::acquire(
            conventional_container,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        if publication
            .open_exact_recovery_directory(cache_stage.as_str(), deadline)?
            .is_some()
        {
            return Err(changed());
        }
        let exact_canonical_before =
            publication.exact_recovery_name_exists(STORE_DIRECTORY_NAME, deadline)?;
        let canonical =
            match publication.open_exact_recovery_directory(STORE_DIRECTORY_NAME, deadline) {
                Ok(canonical) => canonical,
                Err(error)
                    if allow_canonical
                        && exact_canonical_before
                        && Instant::now() < deadline
                        && is_optional_completed_cache_object_error(error.kind()) =>
                {
                    // A malformed ordinary canonical cache is nonfatal after the
                    // first completed-store initialization. ManagedScanCache will
                    // retain and report its typed failure on explicit use. The
                    // transaction stage was still proven exactly absent above.
                    if !publication.exact_recovery_name_exists(STORE_DIRECTORY_NAME, deadline)? {
                        return Err(changed());
                    }
                    publication.revalidate()?;
                    return Ok(AppDataResetCompletedCacheFence {
                        _publication: publication,
                    });
                }
                Err(error) => return Err(error),
            };
        if !allow_canonical {
            return if canonical.is_some() {
                Err(changed())
            } else {
                publication.revalidate()?;
                if Instant::now() < deadline {
                    Ok(AppDataResetCompletedCacheFence {
                        _publication: publication,
                    })
                } else {
                    Err(busy())
                }
            };
        }
        let Some(directory) = canonical else {
            publication.revalidate()?;
            return if Instant::now() < deadline {
                Ok(AppDataResetCompletedCacheFence {
                    _publication: publication,
                })
            } else {
                Err(busy())
            };
        };
        let identity = platform::identity(&directory, platform::Kind::PrivateDirectory)?;
        if retired_identity == Some(platform::identity_parts(identity)) {
            return Err(changed());
        }
        publication.revalidate()?;
        if Instant::now() < deadline {
            Ok(AppDataResetCompletedCacheFence {
                _publication: publication,
            })
        } else {
            Err(busy())
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn with_app_data_reset_recovery_admission_until<T>(
        _conventional_container: &Path,
        _expected_identity: Option<(u64, u64)>,
        _cache_stage: &AppDataResetCacheStageName,
        _deadline: Instant,
        _admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheRecoveryAdmission<'scope>) -> T,
    ) -> Result<T> {
        Err(unsupported())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn with_app_data_reset_draining_cache_admission_until<T>(
        _conventional_container: &Path,
        _expected_identity: Option<(u64, u64)>,
        _cache_stage: &AppDataResetCacheStageName,
        _deadline: Instant,
        _admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheDrainingAdmission<'scope>) -> T,
    ) -> Result<T> {
        Err(unsupported())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn validate_app_data_reset_completed_namespace_until(
        _conventional_container: &Path,
        _retired_identity: Option<(u64, u64)>,
        _cache_stage: &AppDataResetCacheStageName,
        _allow_canonical: bool,
        _deadline: Instant,
    ) -> Result<AppDataResetCompletedCacheFence> {
        Err(unsupported())
    }

    /// Inspect the exact conventional namespace under retained publication
    /// fences without provisioning it, then lend either a present or absent
    /// witness to one higher-ranked callback.
    pub(crate) fn with_app_data_reset_admission_until<T>(
        conventional_container: &Path,
        transaction: &AppDataResetTransaction,
        deadline: Instant,
        admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheAdmission<'scope>) -> T,
    ) -> Result<T> {
        let publication = PublicationFence::acquire(
            conventional_container,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        let Some(store) =
            Self::open_under_publication_fence(&publication, ManagedCacheStoreAccess::ReadOnly)?
        else {
            let admission = AppDataResetManagedCacheAdmission {
                state: AppDataResetManagedCacheAdmissionState::Absent {
                    publication: &publication,
                    cache_stage: transaction.cache_stage(),
                },
                deadline,
                detached: false,
            };
            admission.revalidate()?;
            if Instant::now() >= deadline {
                return Err(busy());
            }
            return Ok(admitted(admission));
        };
        let writer_lock = store.acquire_writer_lock_until(deadline)?;
        store.with_retained_app_data_reset_writer(
            &publication,
            transaction.cache_stage(),
            writer_lock,
            deadline,
            admitted,
        )
    }

    fn with_retained_app_data_reset_writer<T>(
        &self,
        publication: &PublicationFence,
        cache_stage: &AppDataResetCacheStageName,
        writer_lock: WriterLock,
        deadline: Instant,
        admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheAdmission<'scope>) -> T,
    ) -> Result<T> {
        let expected = self.inventory_locked_until(deadline)?.facts();
        let admission = AppDataResetManagedCacheAdmission {
            state: AppDataResetManagedCacheAdmissionState::Present {
                store: self,
                expected,
                _writer_lock: &writer_lock,
                publication,
                cache_stage,
            },
            deadline,
            detached: false,
        };
        admission.revalidate()?;
        if Instant::now() >= deadline {
            return Err(busy());
        }
        Ok(admitted(admission))
    }

    pub(crate) fn prepare_clear(&self) -> Result<Option<ManagedCacheClearSnapshot>> {
        if self.access != ManagedCacheStoreAccess::ReadWrite {
            return Err(ManagedCacheStoreError::new(
                ManagedCacheStoreErrorKind::ReadOnly,
            ));
        }
        let lock = self.acquire_writer_lock(LOCK_TIMEOUT)?;
        let inventory = self.inventory_locked()?;
        let footprint = inventory.footprint()?;
        if footprint.entry_count == 0 && footprint.temporary_count == 0 {
            drop(lock);
            return Ok(None);
        }
        let snapshot = ManagedCacheClearSnapshot {
            store: Arc::clone(&self.inner),
            facts: inventory.facts(),
            footprint,
        };
        drop(lock);
        Ok(Some(snapshot))
    }

    pub(crate) fn clear(
        &self,
        snapshot: ManagedCacheClearSnapshot,
    ) -> std::result::Result<ManagedCacheClearResult, ManagedCacheClearError> {
        if self.access != ManagedCacheStoreAccess::ReadWrite {
            return Err(ManagedCacheClearError::BeforeEffect(
                ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::ReadOnly),
            ));
        }
        if !Arc::ptr_eq(&snapshot.store, &self.inner) {
            return Err(ManagedCacheClearError::BeforeEffect(
                ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InvalidConfiguration),
            ));
        }
        let lock = self
            .acquire_writer_lock(LOCK_TIMEOUT)
            .map_err(ManagedCacheClearError::BeforeEffect)?;
        let mut current = self
            .inventory_locked()
            .map_err(ManagedCacheClearError::BeforeEffect)?;
        if current.facts() != snapshot.facts {
            return Err(ManagedCacheClearError::BeforeEffect(
                ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::ChangedSinceSnapshot),
            ));
        }
        let cleared_usage = current
            .clearable_usage()
            .map_err(ManagedCacheClearError::BeforeEffect)?;
        let mut mutated = false;
        for object in current.objects.drain(..) {
            let removal = platform::remove_retained_file(
                &self.inner.directory,
                &object.name,
                object.file,
                object.facts.identity,
            );
            if removal.is_err() {
                return if mutated {
                    Err(ManagedCacheClearError::OutcomeUnknown)
                } else {
                    Err(ManagedCacheClearError::BeforeEffect(unsafe_object()))
                };
            }
            mutated = true;
        }
        if platform::sync_directory(&self.inner.directory).is_err() {
            return Err(ManagedCacheClearError::OutcomeUnknown);
        }
        let after = self
            .inventory_locked()
            .map_err(|_| ManagedCacheClearError::OutcomeUnknown)?;
        if !after.objects.is_empty() {
            return Err(ManagedCacheClearError::OutcomeUnknown);
        }
        drop(lock);
        Ok(ManagedCacheClearResult {
            cleared_entries: snapshot.footprint.entry_count,
            cleared_temporary: snapshot.footprint.temporary_count,
            cleared_usage,
        })
    }

    pub(super) fn create_temp(&self, key: &ManagedCacheEntryKey) -> Result<PendingTemp> {
        for _ in 0..RANDOM_ATTEMPTS {
            let name = random_temp_name(key)?;
            if let Some((file, identity)) = platform::create_private_file_exclusive(
                &self.inner.directory,
                &self.inner.path,
                &name,
            )? {
                return Ok(PendingTemp {
                    name,
                    file: Some(file),
                    identity,
                });
            }
        }
        Err(unavailable())
    }

    pub(super) fn acquire_writer_lock(&self, timeout: Duration) -> Result<WriterLock> {
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InvalidConfiguration)
        })?;
        self.acquire_writer_lock_until_mode(deadline, true)
    }

    fn acquire_writer_lock_until(&self, deadline: Instant) -> Result<WriterLock> {
        self.acquire_writer_lock_until_mode(deadline, false)
    }

    fn acquire_writer_lock_at_name_until(
        &self,
        store_name: &str,
        deadline: Instant,
    ) -> Result<WriterLock> {
        self.acquire_writer_lock_until_mode_at_name(store_name, deadline, false)
    }

    fn acquire_writer_lock_until_mode(
        &self,
        deadline: Instant,
        allow_expired_initial_try: bool,
    ) -> Result<WriterLock> {
        self.acquire_writer_lock_until_mode_at_name(
            STORE_DIRECTORY_NAME,
            deadline,
            allow_expired_initial_try,
        )
    }

    fn acquire_writer_lock_until_mode_at_name(
        &self,
        store_name: &str,
        deadline: Instant,
        allow_expired_initial_try: bool,
    ) -> Result<WriterLock> {
        if !allow_expired_initial_try && Instant::now() >= deadline {
            return Err(ManagedCacheStoreError::new(
                ManagedCacheStoreErrorKind::Busy,
            ));
        }
        if self
            .inner
            .writer_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ManagedCacheStoreError::new(
                ManagedCacheStoreErrorKind::Busy,
            ));
        }
        if let Err(error) = self.validate_controls_at_name(store_name) {
            self.inner.writer_in_use.store(false, Ordering::Release);
            return Err(error);
        }
        let file = match self.inner.writer_lock.try_clone() {
            Ok(file) => file,
            Err(_) => {
                self.inner.writer_in_use.store(false, Ordering::Release);
                return Err(unavailable());
            }
        };
        let mut first_attempt = true;
        loop {
            if Instant::now() >= deadline && !(allow_expired_initial_try && first_attempt) {
                self.inner.writer_in_use.store(false, Ordering::Release);
                return Err(ManagedCacheStoreError::new(
                    ManagedCacheStoreErrorKind::Busy,
                ));
            }
            first_attempt = false;
            match FileExt::try_lock(&file) {
                Ok(()) if !allow_expired_initial_try && Instant::now() >= deadline => {
                    let unlocked = FileExt::unlock(&file).is_ok();
                    if unlocked {
                        self.inner.writer_in_use.store(false, Ordering::Release);
                    }
                    return Err(ManagedCacheStoreError::new(
                        ManagedCacheStoreErrorKind::Busy,
                    ));
                }
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(
                        LOCK_RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(TryLockError::WouldBlock) => {
                    self.inner.writer_in_use.store(false, Ordering::Release);
                    return Err(ManagedCacheStoreError::new(
                        ManagedCacheStoreErrorKind::Busy,
                    ));
                }
                Err(TryLockError::Error(_)) => {
                    self.inner.writer_in_use.store(false, Ordering::Release);
                    return Err(unavailable());
                }
            }
        }
        if let Err(error) = self.validate_controls_at_name(store_name) {
            let unlocked = FileExt::unlock(&file).is_ok();
            if unlocked {
                self.inner.writer_in_use.store(false, Ordering::Release);
            }
            return Err(error);
        }
        if !allow_expired_initial_try && Instant::now() >= deadline {
            let unlocked = FileExt::unlock(&file).is_ok();
            if unlocked {
                self.inner.writer_in_use.store(false, Ordering::Release);
            }
            return Err(ManagedCacheStoreError::new(
                ManagedCacheStoreErrorKind::Busy,
            ));
        }
        Ok(WriterLock {
            store: Arc::clone(&self.inner),
            file,
        })
    }

    fn validate_controls_at_name(&self, store_name: &str) -> Result<()> {
        platform::validate_path(
            &self.inner.parent_path,
            &self.inner.parent,
            self.inner.parent_identity,
            platform::Kind::ContainerDirectory,
        )?;
        platform::validate_named(
            &self.inner.parent,
            "Dux",
            &self.inner.container,
            self.inner.container_identity,
            platform::Kind::ContainerDirectory,
        )?;
        platform::validate_path(
            &self.inner.container_path,
            &self.inner.container,
            self.inner.container_identity,
            platform::Kind::ContainerDirectory,
        )?;
        validate_same_filesystem(self.inner.container_identity, self.inner.directory_identity)?;
        platform::validate_retained(
            &self.inner.container,
            self.inner.container_identity,
            platform::Kind::ContainerDirectory,
            false,
        )?;
        platform::validate_named(
            &self.inner.container,
            store_name,
            &self.inner.directory,
            self.inner.directory_identity,
            platform::Kind::PrivateDirectory,
        )?;
        for (name, file, identity, expected) in [
            (
                MARKER_NAME,
                &self.inner.marker,
                self.inner.marker_identity,
                STORE_MARKER.as_slice(),
            ),
            (
                WRITER_LOCK_NAME,
                &self.inner.writer_lock,
                self.inner.writer_lock_identity,
                WRITER_MARKER.as_slice(),
            ),
        ] {
            platform::validate_named(
                &self.inner.directory,
                name,
                file,
                identity,
                platform::Kind::PrivateFile,
            )?;
            prove_control(file, expected)?;
        }
        Ok(())
    }

    fn inventory_locked(&self) -> Result<ManagedCacheInventory> {
        let deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(budget)?;
        self.inventory_locked_until(deadline)
    }

    pub(super) fn inventory_locked_until(
        &self,
        outer_deadline: Instant,
    ) -> Result<ManagedCacheInventory> {
        self.inventory_locked_at_name_until(STORE_DIRECTORY_NAME, outer_deadline)
    }

    pub(super) fn inventory_locked_at_name_until(
        &self,
        store_name: &str,
        outer_deadline: Instant,
    ) -> Result<ManagedCacheInventory> {
        if Instant::now() >= outer_deadline {
            return Err(busy());
        }
        std::thread::sleep(take_test_inventory_delay());
        self.validate_controls_at_name(store_name)?;
        if Instant::now() >= outer_deadline {
            return Err(busy());
        }
        let inventory_deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(budget)?
            .min(outer_deadline);
        let names = platform::inventory(
            &self.inner.directory,
            RECOVERY_MAX_NON_CONTROL_OBJECTS + 3,
            MAX_INVENTORY_NAME_BYTES,
            inventory_deadline,
        );
        let mut names = match names {
            Err(_) if Instant::now() >= outer_deadline => return Err(busy()),
            result => result?,
        };
        names.sort_unstable();
        if names
            .iter()
            .filter(|name| name.as_str() == MARKER_NAME)
            .count()
            != 1
            || names
                .iter()
                .filter(|name| name.as_str() == WRITER_LOCK_NAME)
                .count()
                != 1
        {
            return Err(unrecognized());
        }
        let controls = ManagedCacheStorageUsage::from_file(&self.inner.marker)?.checked_add(
            ManagedCacheStorageUsage::from_file(&self.inner.writer_lock)?,
        )?;
        let mut entries = ManagedCacheStorageUsage::default();
        let mut temporary = ManagedCacheStorageUsage::default();
        let mut objects = Vec::new();
        let mut temporary_count = 0_usize;
        for name in names {
            if Instant::now() >= inventory_deadline {
                return if Instant::now() >= outer_deadline {
                    Err(busy())
                } else {
                    Err(budget())
                };
            }
            if name == MARKER_NAME || name == WRITER_LOCK_NAME {
                continue;
            }
            if objects.len() >= RECOVERY_MAX_NON_CONTROL_OBJECTS {
                return Err(budget());
            }
            let kind = if is_final_name(&name) {
                InventoryKind::Entry
            } else if is_temp_name(&name) {
                temporary_count = temporary_count.checked_add(1).ok_or_else(budget)?;
                if temporary_count > RECOVERY_MAX_TEMPORARY_OBJECTS {
                    return Err(budget());
                }
                InventoryKind::Temporary
            } else {
                return Err(unsafe_object());
            };
            let (file, identity) = platform::open_named_private_file(
                &self.inner.directory,
                &self.inner.path,
                &name,
                false,
            )?
            .ok_or_else(unsafe_object)?;
            platform::validate_named(
                &self.inner.directory,
                &name,
                &file,
                identity,
                platform::Kind::PrivateFile,
            )?;
            let usage = ManagedCacheStorageUsage::from_file(&file)?;
            if usage.logical_bytes > MAX_MANAGED_CACHE_FILE_BYTES
                || (kind == InventoryKind::Entry
                    && usage.logical_bytes
                        < u64::try_from(MANAGED_CACHE_HEADER_BYTES).unwrap_or(u64::MAX))
            {
                return Err(budget());
            }
            match kind {
                InventoryKind::Entry => entries = entries.checked_add(usage)?,
                InventoryKind::Temporary => temporary = temporary.checked_add(usage)?,
            }
            let change = platform::change_token(&file)?;
            objects.push(InventoryObject {
                name,
                kind,
                file,
                facts: ObjectFacts {
                    identity,
                    change,
                    usage,
                },
            });
        }
        platform::validate_retained(
            &self.inner.directory,
            self.inner.directory_identity,
            platform::Kind::PrivateDirectory,
            false,
        )?;
        if Instant::now() >= inventory_deadline {
            return if Instant::now() >= outer_deadline {
                Err(busy())
            } else {
                Err(budget())
            };
        }
        let total = controls.checked_add(entries)?.checked_add(temporary)?;
        Ok(ManagedCacheInventory {
            controls,
            entries,
            temporary,
            total,
            objects,
        })
    }
}

pub(super) const fn is_optional_completed_cache_object_error(
    kind: ManagedCacheStoreErrorKind,
) -> bool {
    matches!(
        kind,
        ManagedCacheStoreErrorKind::UnsafeStore
            | ManagedCacheStoreErrorKind::UnsafeObject
            | ManagedCacheStoreErrorKind::UnrecognizedStore
            | ManagedCacheStoreErrorKind::CorruptData
            | ManagedCacheStoreErrorKind::Unavailable
    )
}
