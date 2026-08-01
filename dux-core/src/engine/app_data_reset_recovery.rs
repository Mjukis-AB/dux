//! Private pre-open roll-forward recovery for an incomplete app-data reset.
//!
//! Recovery runs before ordinary database, snapshot, cache, or worker
//! publication. It consumes the exact shared-lease observation, transfers the
//! original coordinator to an exclusive session, and derives every namespace
//! name from the sealed journal. It may publish only the transaction-bound,
//! pre-SQLite fresh data bootstrap; it never repairs, migrates, opens SQLite,
//! creates snapshots/cache, deletes a detached stage, or claims reclaimed
//! bytes.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::cache::{
    AppDataResetManagedCacheRecoveryAdmission, AppDataResetManagedCacheRecoveryLocation,
    ManagedCacheStore,
};
use crate::persistence::{
    AppDataResetCoordinatorErrorKind, AppDataResetCoordinatorSession, AppDataResetFreshNamespace,
    AppDataResetFreshNamespaceLocation, AppDataResetJournal, AppDataResetPhase,
    AppDataResetRecoveryDataLocation, AppDataResetRecoveryDataNamespace,
    AppDataResetRecoveryIntent,
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
/// publish or adopt the exact fresh bootstrap; and `FreshNamespaceReady` is
/// validation-only. Every path remains recovery-required because draining and
/// completed-state admission belong to later checkpoints.
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

            let journal_for_fresh = journal.clone();
            let fresh_result = session.with_fresh_data_namespace_until(
                canonical_database_path,
                &journal,
                deadline,
                |session, fresh| {
                    ManagedCacheStore::with_app_data_reset_recovery_admission_until(
                        cache_directory,
                        cache_identity,
                        transaction.cache_stage(),
                        deadline,
                        |cache| reconcile_fresh_namespace(session, journal_for_fresh, fresh, cache),
                    )
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
            if fresh.location() == AppDataResetFreshNamespaceLocation::Canonical
                && exact_identity == journal.fresh_data_identity()
                && fresh.revalidate().is_ok()
                && cache.revalidate().is_ok()
            {
                pending(AppDataResetPhase::FreshNamespaceReady)
            } else {
                pending(journal.phase())
            }
        }
        _ => pending(journal.phase()),
    }
}

const fn pending(phase: AppDataResetPhase) -> AppDataResetPreOpenRecoveryOutcome {
    AppDataResetPreOpenRecoveryOutcome::RecoveryRequired { phase }
}
