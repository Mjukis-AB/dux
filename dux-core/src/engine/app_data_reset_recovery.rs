//! Private pre-open roll-forward recovery for an incomplete app-data reset.
//!
//! Recovery runs before ordinary database, snapshot, cache, or worker
//! publication. It consumes the exact shared-lease observation, transfers the
//! original coordinator to an exclusive session, and derives every namespace
//! name from the sealed journal. It may publish only the transaction-bound,
//! pre-SQLite fresh data bootstrap; it never repairs, migrates, opens SQLite,
//! creates snapshots/cache or claims reclaimed bytes. Once
//! `Draining` is durable it may remove at most one exact managed-cache payload
//! per pass, then on later passes retire one exact control or empty stage shell.
//! After exact cache absence, one later pass may remove the first recognized
//! old snapshot payload. Once payloads are empty, later passes retire exactly
//! one snapshot marker, locked writer control, or empty directory. The old
//! SQLite payload then drains sidecar-first and main-database-last. Finally,
//! later passes retire exactly one protocol-ordered old-store control or the
//! empty detached data-root shell. Exact old-root and cache absence admit one
//! final monotonic tail: one pass retires the fresh bootstrap's origin record,
//! a later namespace-effect-free pass publishes `Complete`, and both remain recovery-
//! required. Ordinary storage is admitted only by a subsequent engine open
//! after independent completed-state validation.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::cache::{
    AppDataResetManagedCacheDrainingAdmission, AppDataResetManagedCacheRecoveryAdmission,
    AppDataResetManagedCacheRecoveryLocation, ManagedCacheStore,
};
use crate::persistence::{
    AppDataResetCompletionBatchOutcome, AppDataResetCoordinatorErrorKind,
    AppDataResetCoordinatorSession, AppDataResetFreshNamespace, AppDataResetFreshNamespaceLocation,
    AppDataResetJournal, AppDataResetOldDatabaseDrainingAdmission, AppDataResetPhase,
    AppDataResetRecoveryDataLocation, AppDataResetRecoveryDataNamespace,
    AppDataResetRecoveryIntent, AppDataResetSnapshotDrainingAdmission,
};

const PRE_OPEN_RECOVERY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetPreOpenRecoveryOutcome {
    RecoveryRequired { phase: AppDataResetPhase },
    CoordinatorUnavailable,
}

pub(crate) fn deadline() -> Result<Instant, AppDataResetCoordinatorErrorKind> {
    Instant::now()
        .checked_add(PRE_OPEN_RECOVERY_TIMEOUT)
        .ok_or(AppDataResetCoordinatorErrorKind::InternalState)
}

/// Reconcile the implemented detach effects and fresh-root bootstrap.
///
/// `Prepared` may advance through both detaches in one exclusive session;
/// `CacheDetached` may advance through the data detach; `DataDetached` may
/// publish or adopt the exact fresh bootstrap; and `FreshNamespaceReady` may
/// durably enter `Draining` before one bounded cache payload effect.
/// `Draining` routes recognized payloads first, then resumes the monotonic
/// cache-control/stage tail one exact effect per pass. After exact cache
/// absence it drains old snapshot payloads, then the monotonic snapshot-store
/// structural tail, then the old SQLite payload and old-root structural tail,
/// again one effect per pass. Every path that performs an effect or publishes
/// `Complete` remains recovery-required. Durable completed-state validation and
/// ordinary admission are deliberately performed only by a subsequent open.
pub(crate) fn recover_app_data_reset_before_open_until(
    canonical_database_path: &Path,
    cache_directory: &Path,
    intent: Box<AppDataResetRecoveryIntent>,
    deadline: Instant,
) -> Result<AppDataResetPreOpenRecoveryOutcome, AppDataResetCoordinatorErrorKind> {
    let intent = *intent;
    intent
        .with_exclusive_session_until(deadline, |session, journal| {
            let debt = session.provisioning_debt()?;
            if debt.unproven_stage_count() != 0 {
                return Ok(AppDataResetPreOpenRecoveryOutcome::CoordinatorUnavailable);
            }
            let transaction = journal.validated_transaction()?;
            if !matches!(
                journal.phase(),
                AppDataResetPhase::Prepared
                    | AppDataResetPhase::CacheDetached
                    | AppDataResetPhase::DataDetached
                    | AppDataResetPhase::FreshNamespaceReady
                    | AppDataResetPhase::Draining
            ) {
                return Ok(pending(journal.phase()));
            }

            let cache_identity = journal
                .cache_identity()
                .map(|identity| (identity.device(), identity.inode()));
            let mut journal = journal;
            if matches!(
                journal.phase(),
                AppDataResetPhase::Prepared | AppDataResetPhase::CacheDetached
            ) {
                let journal_for_admission = journal.clone();
                let journal_for_detach = journal.clone();
                let data_result = session.with_recovery_data_namespace_until(
                    canonical_database_path,
                    &journal_for_admission,
                    deadline,
                    |session, data| {
                        ManagedCacheStore::with_app_data_reset_recovery_admission_until(
                            cache_directory,
                            cache_identity,
                            transaction.cache_stage(),
                            deadline,
                            |cache| {
                                reconcile_detach_phases(session, journal_for_detach, data, cache)
                            },
                        )
                    },
                );
                journal = match data_result {
                    Ok(Ok(Some(journal))) => journal,
                    Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                        let Some(durable) = session.recover()? else {
                            return Ok(AppDataResetPreOpenRecoveryOutcome::CoordinatorUnavailable);
                        };
                        return Ok(pending(durable.phase()));
                    }
                };
            }

            if journal.phase() == AppDataResetPhase::Draining {
                let old_database_journal = journal.clone();
                let old_database_attempt = session.with_draining_old_database_until(
                    canonical_database_path,
                    &journal,
                    deadline,
                    |session, data| match data {
                        AppDataResetOldDatabaseDrainingAdmission::SnapshotStorePresent => Ok(None),
                        data => {
                            ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
                                cache_directory,
                                cache_identity,
                                transaction.cache_stage(),
                                deadline,
                                |cache| {
                                    reconcile_draining_old_database(
                                        session,
                                        old_database_journal,
                                        data,
                                        cache,
                                        deadline,
                                    )
                                },
                            )
                        }
                    },
                );
                match old_database_attempt {
                    Ok(Ok(Some(outcome))) => return Ok(outcome),
                    // Only the explicit, fully bounded snapshot-present state
                    // or a typed non-absent cache admission may resume the
                    // earlier pipeline. Every opener/admission error is a
                    // fail-closed recovery-required result, never fallback.
                    Ok(Ok(None)) => {}
                    Ok(Err(_)) | Err(_) => return Ok(pending(journal.phase())),
                }
            }

            let journal_for_fresh = journal.clone();
            let fresh_result = session.with_fresh_data_namespace_until(
                canonical_database_path,
                &journal,
                deadline,
                |session, fresh| {
                    if journal_for_fresh.phase() == AppDataResetPhase::Draining {
                        ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
                            cache_directory,
                            cache_identity,
                            transaction.cache_stage(),
                            deadline,
                            |cache| {
                                reconcile_draining_cache(
                                    session,
                                    journal_for_fresh,
                                    fresh,
                                    cache,
                                    deadline,
                                )
                            },
                        )
                    } else {
                        ManagedCacheStore::with_app_data_reset_recovery_admission_until(
                            cache_directory,
                            cache_identity,
                            transaction.cache_stage(),
                            deadline,
                            |cache| {
                                reconcile_fresh_namespace(session, journal_for_fresh, fresh, cache)
                            },
                        )
                    }
                },
            );
            match fresh_result {
                Ok(Ok(outcome)) => Ok(outcome),
                Ok(Err(_)) | Err(_) => Ok(pending(journal.phase())),
            }
        })
        .map_err(|error| error.kind())
}

fn reconcile_detach_phases(
    session: &mut AppDataResetCoordinatorSession<'_>,
    mut journal: AppDataResetJournal,
    mut data: AppDataResetRecoveryDataNamespace<'_>,
    mut cache: AppDataResetManagedCacheRecoveryAdmission<'_>,
) -> Option<AppDataResetJournal> {
    if !journal.has_canonical_root_name_binding() {
        if data.location() != AppDataResetRecoveryDataLocation::Canonical {
            return None;
        }
        let binding = data.canonical_root_binding().ok()?;
        journal = session.bind_legacy_canonical_root(&journal, binding).ok()?;
    }
    if journal.phase() == AppDataResetPhase::Prepared {
        // Data detachment is not authorized until CacheDetached is durable.
        if data.location() != AppDataResetRecoveryDataLocation::Canonical {
            return None;
        }
        cache = match cache.detach_if_canonical() {
            Ok(cache) => cache,
            Err(_) => return None,
        };
        if data.revalidate().is_err() || cache.revalidate().is_err() {
            return None;
        }
        journal = match session.advance(&journal, AppDataResetPhase::CacheDetached) {
            Ok(journal) => journal,
            Err(_) => return None,
        };
    }

    if journal.phase() == AppDataResetPhase::CacheDetached {
        // A canonical cache in this phase is an impossible rollback shape;
        // recovery must not reinterpret it as pending work.
        if cache.location() == AppDataResetManagedCacheRecoveryLocation::Canonical {
            return None;
        }
        data = match data.detach_if_canonical(journal.data_identity(), journal.data_stage_name()) {
            Ok(data) => data,
            Err(_) => return None,
        };
        if data.revalidate().is_err() || cache.revalidate().is_err() {
            return None;
        }
        journal = match session.advance(&journal, AppDataResetPhase::DataDetached) {
            Ok(journal) => journal,
            Err(_) => return None,
        };
    }

    if journal.phase() == AppDataResetPhase::DataDetached
        && data.location() == AppDataResetRecoveryDataLocation::Detached
        && cache.location() != AppDataResetManagedCacheRecoveryLocation::Canonical
        && data.journal_identity().ok() == Some(journal.data_identity())
        && data.revalidate().is_ok()
        && cache.revalidate().is_ok()
    {
        Some(journal)
    } else {
        None
    }
}

fn reconcile_fresh_namespace(
    session: &mut AppDataResetCoordinatorSession<'_>,
    journal: AppDataResetJournal,
    fresh: AppDataResetFreshNamespace<'_>,
    cache: AppDataResetManagedCacheRecoveryAdmission<'_>,
) -> AppDataResetPreOpenRecoveryOutcome {
    if cache.location() == AppDataResetManagedCacheRecoveryLocation::Canonical
        || cache.revalidate().is_err()
        || fresh.revalidate().is_err()
    {
        return pending(journal.phase());
    }

    match journal.phase() {
        AppDataResetPhase::DataDetached => {
            let published = match fresh.publish_if_needed() {
                Ok(published) => published,
                Err(_) => return pending(journal.phase()),
            };
            if published.revalidate().is_err() || cache.revalidate().is_err() {
                return pending(journal.phase());
            }
            match session.commit_fresh_namespace(&journal, &published) {
                Ok(journal) => pending(journal.phase()),
                Err(_) => match session.recover() {
                    Ok(Some(durable)) => pending(durable.phase()),
                    Ok(None) | Err(_) => AppDataResetPreOpenRecoveryOutcome::CoordinatorUnavailable,
                },
            }
        }
        AppDataResetPhase::FreshNamespaceReady => {
            let exact_identity = fresh.fresh_identity().ok().flatten();
            let Some(expected_fresh_identity) = journal.fresh_data_identity() else {
                return pending(journal.phase());
            };
            if fresh.location() != AppDataResetFreshNamespaceLocation::Canonical
                || exact_identity != Some(expected_fresh_identity)
                || fresh.revalidate().is_err()
                || cache.revalidate().is_err()
            {
                return pending(journal.phase());
            }
            let transaction = match journal.validated_transaction() {
                Ok(transaction) => transaction,
                Err(_) => return AppDataResetPreOpenRecoveryOutcome::CoordinatorUnavailable,
            };
            let cache_identity = journal
                .cache_identity()
                .map(|identity| (identity.device(), identity.inode()));
            let ready = match fresh.into_ready_to_drain(expected_fresh_identity) {
                Ok(ready) => ready,
                Err(_) => return pending(journal.phase()),
            };
            let cache = match cache.into_drain_candidate(cache_identity, transaction.cache_stage())
            {
                Ok(cache) => cache,
                Err(_) => return pending(journal.phase()),
            };
            let batch = match session.admit_draining_cache_batch(&journal, ready, cache) {
                Ok(batch) => batch,
                Err(_) => return durable_pending(session),
            };
            match session.run_draining_cache_batch(batch) {
                Ok(progress) => {
                    // Progress is deliberately private and path/byte-free. It
                    // does not imply completed reset or reclaimed capacity.
                    let _ = (
                        progress.removed_objects(),
                        progress.cache_payload_has_more(),
                    );
                    pending(AppDataResetPhase::Draining)
                }
                Err(_) => durable_pending(session),
            }
        }
        _ => pending(journal.phase()),
    }
}

fn reconcile_draining_cache(
    session: &mut AppDataResetCoordinatorSession<'_>,
    journal: AppDataResetJournal,
    fresh: AppDataResetFreshNamespace<'_>,
    cache: AppDataResetManagedCacheDrainingAdmission<'_>,
    deadline: Instant,
) -> AppDataResetPreOpenRecoveryOutcome {
    if journal.phase() != AppDataResetPhase::Draining {
        return pending(journal.phase());
    }
    let Some(expected_fresh_identity) = journal.fresh_data_identity() else {
        return AppDataResetPreOpenRecoveryOutcome::CoordinatorUnavailable;
    };
    if fresh.location() != AppDataResetFreshNamespaceLocation::Canonical
        || fresh.fresh_identity().ok().flatten() != Some(expected_fresh_identity)
        || fresh.revalidate().is_err()
    {
        return pending(journal.phase());
    }
    let ready = match fresh.into_ready_to_drain(expected_fresh_identity) {
        Ok(ready) => ready,
        Err(_) => return pending(journal.phase()),
    };

    match cache {
        AppDataResetManagedCacheDrainingAdmission::PayloadsRemain(cache) => {
            let batch = match session.admit_draining_cache_batch(&journal, ready, cache) {
                Ok(batch) => batch,
                Err(_) => return durable_pending(session),
            };
            match session.run_draining_cache_batch(batch) {
                Ok(progress) => {
                    let _ = (
                        progress.removed_objects(),
                        progress.cache_payload_has_more(),
                    );
                    pending(AppDataResetPhase::Draining)
                }
                Err(_) => durable_pending(session),
            }
        }
        AppDataResetManagedCacheDrainingAdmission::Retirement(cache) => {
            let batch =
                match session.admit_draining_cache_stage_retirement_batch(&journal, ready, cache) {
                    Ok(batch) => batch,
                    Err(_) => return durable_pending(session),
                };
            match session.run_draining_cache_stage_retirement_batch(batch) {
                Ok(progress) => {
                    let _ = (
                        progress.removed_structural_objects(),
                        progress.cache_stage_has_more(),
                    );
                    pending(AppDataResetPhase::Draining)
                }
                Err(_) => durable_pending(session),
            }
        }
        AppDataResetManagedCacheDrainingAdmission::Absent(cache) => {
            let snapshots = match ready.into_snapshot_draining_admission() {
                Ok(snapshots) => snapshots,
                Err(_) => return pending(journal.phase()),
            };
            match snapshots {
                AppDataResetSnapshotDrainingAdmission::PayloadsRemain(data) => {
                    let batch = match session
                        .admit_draining_snapshot_payload_batch(&journal, data, cache, deadline)
                    {
                        Ok(batch) => batch,
                        Err(_) => return durable_pending(session),
                    };
                    match session.run_draining_snapshot_payload_batch(batch) {
                        Ok(progress) => {
                            let _ = (
                                progress.removed_objects(),
                                progress.snapshot_payload_has_more(),
                            );
                            pending(AppDataResetPhase::Draining)
                        }
                        Err(_) => durable_pending(session),
                    }
                }
                AppDataResetSnapshotDrainingAdmission::Retirement(data) => {
                    let batch = match session.admit_draining_snapshot_store_retirement_batch(
                        &journal, data, cache, deadline,
                    ) {
                        Ok(batch) => batch,
                        Err(_) => return durable_pending(session),
                    };
                    match session.run_draining_snapshot_store_retirement_batch(batch) {
                        Ok(progress) => {
                            let _ = (
                                progress.removed_structural_objects(),
                                progress.snapshot_store_has_more(),
                            );
                            pending(AppDataResetPhase::Draining)
                        }
                        Err(_) => durable_pending(session),
                    }
                }
                AppDataResetSnapshotDrainingAdmission::Absent(data) => {
                    if data.revalidate_until(deadline).is_err()
                        || cache.revalidate_until(deadline).is_err()
                    {
                        return pending(journal.phase());
                    }
                    pending(AppDataResetPhase::Draining)
                }
            }
        }
    }
}

fn reconcile_draining_old_database(
    session: &mut AppDataResetCoordinatorSession<'_>,
    journal: AppDataResetJournal,
    data: AppDataResetOldDatabaseDrainingAdmission<'_>,
    cache: AppDataResetManagedCacheDrainingAdmission<'_>,
    deadline: Instant,
) -> Option<AppDataResetPreOpenRecoveryOutcome> {
    let AppDataResetManagedCacheDrainingAdmission::Absent(cache) = cache else {
        return None;
    };
    match data {
        AppDataResetOldDatabaseDrainingAdmission::SnapshotStorePresent => None,
        AppDataResetOldDatabaseDrainingAdmission::PayloadsRemain(data) => {
            let batch = match session
                .admit_draining_old_database_payload_batch(&journal, data, cache, deadline)
            {
                Ok(batch) => batch,
                Err(_) => return Some(durable_pending(session)),
            };
            match session.run_draining_old_database_payload_batch(batch) {
                Ok(progress) => {
                    let _ = (
                        progress.removed_objects(),
                        progress.old_database_payload_has_more(),
                    );
                    Some(pending(AppDataResetPhase::Draining))
                }
                Err(_) => Some(durable_pending(session)),
            }
        }
        AppDataResetOldDatabaseDrainingAdmission::Retirement(data) => {
            let batch = match session
                .admit_draining_old_database_store_retirement_batch(&journal, data, cache, deadline)
            {
                Ok(batch) => batch,
                Err(_) => return Some(durable_pending(session)),
            };
            match session.run_draining_old_database_store_retirement_batch(batch) {
                Ok(progress) => {
                    let _ = (
                        progress.removed_structural_objects(),
                        progress.old_store_has_more(),
                    );
                    Some(pending(AppDataResetPhase::Draining))
                }
                Err(_) => Some(durable_pending(session)),
            }
        }
        AppDataResetOldDatabaseDrainingAdmission::RootAbsent(data) => {
            let batch = match session.admit_completion_batch(&journal, data, cache, deadline) {
                Ok(batch) => batch,
                Err(_) => return Some(durable_pending(session)),
            };
            match session.run_completion_batch(batch) {
                Ok(AppDataResetCompletionBatchOutcome::OriginRetired) => {
                    Some(pending(AppDataResetPhase::Draining))
                }
                Ok(AppDataResetCompletionBatchOutcome::Complete(journal)) => {
                    Some(pending(journal.phase()))
                }
                Err(_) => Some(durable_pending(session)),
            }
        }
    }
}

fn durable_pending(
    session: &mut AppDataResetCoordinatorSession<'_>,
) -> AppDataResetPreOpenRecoveryOutcome {
    match session.recover() {
        Ok(Some(durable)) => pending(durable.phase()),
        Ok(None) | Err(_) => AppDataResetPreOpenRecoveryOutcome::CoordinatorUnavailable,
    }
}

const fn pending(phase: AppDataResetPhase) -> AppDataResetPreOpenRecoveryOutcome {
    AppDataResetPreOpenRecoveryOutcome::RecoveryRequired { phase }
}
