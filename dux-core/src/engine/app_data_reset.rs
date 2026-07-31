#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the private reset composition is consumed by the namespace-witness slice"
    )
)]

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cache::ManagedCacheStoreErrorKind;
use crate::persistence::snapshot::{
    AppDataResetSnapshotAdmission, SnapshotRepository, SnapshotRepositoryErrorKind,
};
use crate::persistence::{
    AppDataResetAdmittedStoreOutcome, AppDataResetCoordinator, AppDataResetCoordinatorError,
    AppDataResetCoordinatorErrorKind, AppDataResetCoordinatorSession, AppDataResetPhase,
    AppDataResetStoreBlockers, AppDataResetStoreGuard, HistoryErrorKind, StoreCoordinator,
};

use super::managed_scan_cache::{
    AppDataResetManagedScanCacheAdmission, AppDataResetManagedScanCacheError, ManagedScanCache,
};
use super::registry::{AppDataResetAdmissionOutcome, AppDataResetQuiesced, EngineHandle};

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
    Coordinator(AppDataResetCoordinatorErrorKind),
    Store(HistoryErrorKind),
    LifecycleBusy,
    ProvisioningDebt { unproven_stage_count: u32 },
    RecoveryRequired { phase: AppDataResetPhase },
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
    StoreBlocked(AppDataResetStoreBlockers),
    Store(HistoryErrorKind),
    Snapshot(SnapshotRepositoryErrorKind),
    ManagedCache(ManagedCacheStoreErrorKind),
    ManagedCacheAbsenceUnfenced,
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
/// cleanup/database exclusion, snapshot exclusion, and present-cache
/// exclusion.
///
/// Its fields are private and its lifetimes are higher-ranked at the call
/// site, so none of the retained proofs can escape. This checkpoint exposes
/// validation only and no journal transition or reset-target namespace
/// operation. Coordinator-only provisioning reconciliation may still occur.
pub(crate) struct AppDataResetCoreAdmission<
    'session,
    'storage,
    'guard,
    'store,
    'quiesced,
    'snapshot,
    'cache,
    'runtime,
> {
    store: &'store mut AppDataResetStoreGuard<'guard>,
    snapshot: AppDataResetSnapshotAdmission<'snapshot>,
    managed_cache: AppDataResetManagedScanCacheAdmission<'cache>,
    _quiesced: &'quiesced AppDataResetQuiesced,
    coordinator: &'session mut AppDataResetCoordinatorSession<'storage>,
    deadline: Instant,
    inspect_runtime_blockers: &'runtime dyn Fn(Instant) -> Result<AppDataResetRuntimeBlockers, ()>,
}

impl AppDataResetCoreAdmission<'_, '_, '_, '_, '_, '_, '_, '_> {
    pub(crate) fn revalidate(&mut self) -> Result<(), AppDataResetPostTerminalRefusal> {
        if Instant::now() >= self.deadline {
            return Err(AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded);
        }
        validate_coordinator_preflight(self.coordinator)
            .map_err(|error| AppDataResetPostTerminalRefusal::Coordinator(error.kind()))?
            .map_or(Ok(()), |refusal| match refusal {
                AppDataResetPreTerminalRefusal::Coordinator(kind) => {
                    Err(AppDataResetPostTerminalRefusal::Coordinator(kind))
                }
                AppDataResetPreTerminalRefusal::Store(kind) => {
                    Err(AppDataResetPostTerminalRefusal::Store(kind))
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
        let runtime_blockers = (self.inspect_runtime_blockers)(self.deadline)
            .map_err(|()| AppDataResetPostTerminalRefusal::RuntimeInspectionBusy)?;
        if !runtime_blockers.is_empty() {
            return Err(AppDataResetPostTerminalRefusal::RuntimeBlocked(
                runtime_blockers,
            ));
        }
        let blockers = self
            .store
            .revalidate()
            .map_err(|error| AppDataResetPostTerminalRefusal::Store(error.kind))?;
        if !blockers.is_empty() {
            return Err(AppDataResetPostTerminalRefusal::StoreBlocked(blockers));
        }
        self.snapshot
            .revalidate()
            .map_err(|error| AppDataResetPostTerminalRefusal::Snapshot(error.kind))?;
        self.managed_cache
            .revalidate()
            .map_err(map_managed_cache_refusal)?;
        if Instant::now() >= self.deadline {
            return Err(AppDataResetPostTerminalRefusal::AdmissionDeadlineExceeded);
        }
        Ok(())
    }
}

/// Compose coordinator-first preflight, terminal worker quiescence,
/// cleanup/database admission, snapshot admission, and present-cache
/// admission without creating durable intent or effects.
pub(super) fn with_terminal_store_preflight<T>(
    engine: &EngineHandle,
    store: &Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    managed_scan_cache: &ManagedScanCache,
    shutdown_timeout: Duration,
    inspect_runtime_blockers: impl Fn(Instant) -> Result<AppDataResetRuntimeBlockers, ()>,
    admitted: impl for<
        'session,
        'storage,
        'guard,
        'store,
        'quiesced,
        'snapshot,
        'cache,
        'runtime,
    > FnOnce(
        AppDataResetCoreAdmission<
            'session,
            'storage,
            'guard,
            'store,
            'quiesced,
            'snapshot,
            'cache,
            'runtime,
        >,
    )
        -> T,
) -> AppDataResetCompositionOutcome<T> {
    let Some(deadline) = Instant::now().checked_add(shutdown_timeout) else {
        return AppDataResetCompositionOutcome::PreTerminalRefused(
            AppDataResetPreTerminalRefusal::Store(HistoryErrorKind::InvalidInput),
        );
    };
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
    let coordinator = match AppDataResetCoordinator::open_or_create_until(data_root, deadline) {
        Ok(coordinator) => coordinator,
        Err(error) => {
            return AppDataResetCompositionOutcome::PreTerminalRefused(
                AppDataResetPreTerminalRefusal::Coordinator(error.kind()),
            );
        }
    };
    let disposition = Cell::new(AppDataResetEngineDisposition::UnchangedByAttempt);
    let mut admitted = Some(admitted);
    let outcome = coordinator.with_exclusive_session_until(deadline, |session| {
        if let Some(refusal) = validate_coordinator_preflight(session)? {
            return Ok(AppDataResetCompositionOutcome::PreTerminalRefused(refusal));
        }

        let shutdown = match engine.begin_app_data_reset_shutdown_until(deadline) {
            Err(()) => {
                return Ok(AppDataResetCompositionOutcome::PreTerminalRefused(
                    AppDataResetPreTerminalRefusal::LifecycleBusy,
                ));
            }
            Ok(AppDataResetAdmissionOutcome::Admitted(shutdown)) => {
                disposition.set(AppDataResetEngineDisposition::Terminal);
                shutdown
            }
            Ok(AppDataResetAdmissionOutcome::OrdinaryCloseWon) => {
                disposition.set(AppDataResetEngineDisposition::Terminal);
                return Ok(AppDataResetCompositionOutcome::TerminalOwnedElsewhere(
                    AppDataResetTerminalOwner::OrdinaryClose,
                ));
            }
            Ok(AppDataResetAdmissionOutcome::AlreadyResetting) => {
                disposition.set(AppDataResetEngineDisposition::Terminal);
                return Ok(AppDataResetCompositionOutcome::TerminalOwnedElsewhere(
                    AppDataResetTerminalOwner::AppDataReset,
                ));
            }
            Ok(AppDataResetAdmissionOutcome::InternalState) => {
                disposition.set(AppDataResetEngineDisposition::Terminal);
                return Ok(AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                    AppDataResetPostTerminalRefusal::LifecycleInternalState,
                ));
            }
        };
        let quiesced = match shutdown.wait_until_quiesced_until(deadline) {
            Ok(quiesced) => quiesced,
            Err(error) => {
                return Ok(AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                    AppDataResetPostTerminalRefusal::Shutdown(error),
                ));
            }
        };

        let runtime_blockers = match inspect_runtime_blockers(deadline) {
            Ok(blockers) => blockers,
            Err(()) => {
                return Ok(AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                    AppDataResetPostTerminalRefusal::RuntimeInspectionBusy,
                ));
            }
        };
        if !runtime_blockers.is_empty() {
            return Ok(AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                AppDataResetPostTerminalRefusal::RuntimeBlocked(runtime_blockers),
            ));
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
                            managed_scan_cache
                        .with_app_data_reset_admission(deadline, |managed_cache| {
                            let mut admission = AppDataResetCoreAdmission {
                                store,
                                snapshot,
                                managed_cache,
                                _quiesced: &quiesced,
                                coordinator: session,
                                deadline,
                                inspect_runtime_blockers: &inspect_runtime_blockers,
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
                            AppDataResetCompositionOutcome::Admitted(admitted
                                .take()
                                .expect("reset admission callback is invoked at most once")(
                                admission,
                            ))
                        })
                        },
                    );
                match snapshot_outcome {
                    Ok(Ok(outcome)) => outcome,
                    Ok(Err(error)) => AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                        map_managed_cache_refusal(error),
                    ),
                    Err(error) => AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                        AppDataResetPostTerminalRefusal::Snapshot(error.kind),
                    ),
                }
            });
        Ok(match store_outcome {
            Ok(AppDataResetAdmittedStoreOutcome::Admitted(outcome)) => outcome,
            Ok(AppDataResetAdmittedStoreOutcome::Blocked(blockers)) => {
                AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                    AppDataResetPostTerminalRefusal::StoreBlocked(blockers),
                )
            }
            Err(error) => AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                AppDataResetPostTerminalRefusal::Store(error.kind),
            ),
        })
    });

    match outcome {
        Ok(outcome) => outcome,
        Err(error) => match disposition.get() {
            AppDataResetEngineDisposition::UnchangedByAttempt => {
                AppDataResetCompositionOutcome::PreTerminalRefused(
                    AppDataResetPreTerminalRefusal::Coordinator(error.kind()),
                )
            }
            AppDataResetEngineDisposition::Terminal => {
                AppDataResetCompositionOutcome::TerminalWithoutAdmission(
                    AppDataResetPostTerminalRefusal::Coordinator(error.kind()),
                )
            }
        },
    }
}

fn map_managed_cache_refusal(
    error: AppDataResetManagedScanCacheError,
) -> AppDataResetPostTerminalRefusal {
    match error {
        AppDataResetManagedScanCacheError::Store(kind) => {
            AppDataResetPostTerminalRefusal::ManagedCache(kind)
        }
        AppDataResetManagedScanCacheError::UnfencedAbsence => {
            AppDataResetPostTerminalRefusal::ManagedCacheAbsenceUnfenced
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
