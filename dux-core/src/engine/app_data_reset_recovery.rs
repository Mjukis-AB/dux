//! Private pre-open roll-forward recovery for an incomplete app-data reset.
//!
//! Recovery runs before ordinary database, snapshot, cache, or worker
//! publication. It consumes the exact shared-lease observation, transfers the
//! original coordinator to an exclusive session, and derives every namespace
//! name from the sealed journal. It never provisions, repairs, migrates, opens
//! SQLite, deletes a detached stage, or claims reclaimed bytes.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::cache::{
    AppDataResetManagedCacheRecoveryAdmission, AppDataResetManagedCacheRecoveryLocation,
    ManagedCacheStore,
};
use crate::persistence::{
    AppDataResetCoordinatorErrorKind, AppDataResetCoordinatorSession, AppDataResetJournal,
    AppDataResetPhase, AppDataResetRecoveryDataLocation, AppDataResetRecoveryDataNamespace,
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

/// Reconcile only the already-implemented cache and data detach effects.
///
/// `Prepared` may advance through both detaches in one exclusive session;
/// `CacheDetached` may advance through the data detach; and `DataDetached` is
/// fully revalidated. Every path still returns recovery-required because fresh
/// namespace provisioning belongs to the next checkpoint.
pub(crate) fn recover_app_data_reset_before_open_until(
    canonical_database_path: &Path,
    cache_directory: &Path,
    intent: Box<AppDataResetRecoveryIntent>,
    deadline: Instant,
) -> Result<AppDataResetPreOpenRecoveryOutcome, AppDataResetCoordinatorErrorKind> {
    let intent = *intent;
    let observed_phase = intent.phase();
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
            ) {
                return Ok(pending(journal.phase()));
            }

            let cache_identity = journal
                .cache_identity()
                .map(|identity| (identity.device(), identity.inode()));
            let data_result = session.with_recovery_data_namespace_until(
                canonical_database_path,
                &transaction,
                journal.data_identity(),
                deadline,
                |session, data| {
                    ManagedCacheStore::with_app_data_reset_recovery_admission_until(
                        cache_directory,
                        cache_identity,
                        transaction.cache_stage(),
                        deadline,
                        |cache| reconcile_detach_phases(session, journal, data, cache),
                    )
                },
            );
            match data_result {
                Ok(Ok(outcome)) => Ok(outcome),
                Ok(Err(_)) | Err(_) => Ok(pending(observed_phase)),
            }
        })
        .map_err(|error| error.kind())
}

fn reconcile_detach_phases(
    session: &mut AppDataResetCoordinatorSession<'_>,
    mut journal: AppDataResetJournal,
    mut data: AppDataResetRecoveryDataNamespace<'_>,
    mut cache: AppDataResetManagedCacheRecoveryAdmission<'_>,
) -> AppDataResetPreOpenRecoveryOutcome {
    if journal.phase() == AppDataResetPhase::Prepared {
        // Data detachment is not authorized until CacheDetached is durable.
        if data.location() != AppDataResetRecoveryDataLocation::Canonical {
            return pending(journal.phase());
        }
        cache = match cache.detach_if_canonical() {
            Ok(cache) => cache,
            Err(_) => return pending(journal.phase()),
        };
        if data.revalidate().is_err() || cache.revalidate().is_err() {
            return pending(journal.phase());
        }
        journal = match session.advance(&journal, AppDataResetPhase::CacheDetached) {
            Ok(journal) => journal,
            Err(_) => return pending(journal.phase()),
        };
    }

    if journal.phase() == AppDataResetPhase::CacheDetached {
        // A canonical cache in this phase is an impossible rollback shape;
        // recovery must not reinterpret it as pending work.
        if cache.location() == AppDataResetManagedCacheRecoveryLocation::Canonical {
            return pending(journal.phase());
        }
        data = match data.detach_if_canonical(journal.data_identity(), journal.data_stage_name()) {
            Ok(data) => data,
            Err(_) => return pending(journal.phase()),
        };
        if data.revalidate().is_err() || cache.revalidate().is_err() {
            return pending(journal.phase());
        }
        journal = match session.advance(&journal, AppDataResetPhase::DataDetached) {
            Ok(journal) => journal,
            Err(_) => return pending(journal.phase()),
        };
    }

    if journal.phase() == AppDataResetPhase::DataDetached
        && data.location() == AppDataResetRecoveryDataLocation::Detached
        && cache.location() != AppDataResetManagedCacheRecoveryLocation::Canonical
        && data.journal_identity().ok() == Some(journal.data_identity())
        && data.revalidate().is_ok()
        && cache.revalidate().is_ok()
    {
        pending(AppDataResetPhase::DataDetached)
    } else {
        pending(journal.phase())
    }
}

const fn pending(phase: AppDataResetPhase) -> AppDataResetPreOpenRecoveryOutcome {
    AppDataResetPreOpenRecoveryOutcome::RecoveryRequired { phase }
}
