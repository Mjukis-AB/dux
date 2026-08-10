//! Sealed persistence for path-free automation schedules and activation cursors.
//!
//! Activation is saved consent and timing data only. This module has no method
//! that can prove eligibility, plan, enqueue, or execute cleanup.

use std::time::{Duration, SystemTime};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};

use crate::domain::{
    AUTOMATION_RECURRENCE_POLICY_REVISION, AutomationConfirmationMode, AutomationPeriodicCursor,
    AutomationScheduleAuthoringBinding, AutomationScheduleCadence, AutomationScheduleCursor,
    AutomationScheduleDraft, AutomationScheduleDraftConfig, AutomationScheduleId,
    AutomationSchedulePauseReason, AutomationScheduleScope, AutomationScheduleState,
    CandidateCategory, DEFAULT_AUTOMATION_PRE_RUN_NOTIFICATIONS, MAX_AUTOMATION_SCHEDULE_DRAFTS,
    MAX_AUTOMATION_SCHEDULE_EXCLUSIONS, MAX_AUTOMATION_UNIX_MS, RuleId, RuleRef, RuleRevision,
    first_automation_occurrence_after_unix_ms, materialize_automation_occurrence_unix_ms,
};

use super::history::{
    HistoryError, HistoryErrorKind, from_i64, map_query_sql_error, map_write_sql_error,
    run_bounded_query, stored_bool, system_time_to_unix_ms, to_i64, unix_ms_to_system_time,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

const DISABLED_STATE: &str = "disabled";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutomationScheduleDraftStoreUpdate {
    pub(crate) draft: AutomationScheduleDraft,
    pub(crate) changed: bool,
}

#[derive(Clone, Copy, Debug)]
enum ActivationMutation {
    EnablePeriodic,
    Pause(AutomationSchedulePauseReason),
    Resume,
    Disable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StoredDraft {
    draft: AutomationScheduleDraft,
    updated_at_unix_ms: i64,
}

#[derive(Debug)]
struct RawDraft {
    schedule_id: String,
    state: String,
    pause_reason: Option<String>,
    scope_kind: String,
    rule_id: Option<String>,
    rule_revision: Option<i64>,
    category: Option<String>,
    cadence: String,
    minimum_age_seconds: i64,
    minimum_reclaimable_bytes: i64,
    maximum_bytes_per_run: i64,
    notify_before_run: i64,
    confirmation_mode: String,
    authoring_policy_revision: Option<i64>,
    authoring_membership_sha256: Option<Vec<u8>>,
    pre_run_notifications_remaining: i64,
    revision: i64,
    cursor_revision: i64,
    recurrence_policy_revision: i64,
    recurrence_anchor_unix_ms: Option<i64>,
    next_occurrence_ordinal: Option<i64>,
    next_run_unix_ms: Option<i64>,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
}

impl StoreCoordinator {
    pub(crate) fn load_automation_schedule_drafts(
        &self,
    ) -> Result<Vec<AutomationScheduleDraft>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        Ok(load_stored_drafts(&guard.connection)?
            .into_iter()
            .map(|stored| stored.draft)
            .collect())
    }

    pub(crate) fn create_automation_schedule_draft(
        &self,
        id: AutomationScheduleId,
        config: AutomationScheduleDraftConfig,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.create_automation_schedule_draft_with_hook(id, config, observed_at, || Ok(()))
    }

    fn create_automation_schedule_draft_with_hook(
        &self,
        id: AutomationScheduleId,
        config: AutomationScheduleDraftConfig,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        if observed_at_unix_ms > MAX_AUTOMATION_UNIX_MS {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        if bounded_schedule_population(&transaction)? >= MAX_AUTOMATION_SCHEDULE_DRAFTS {
            return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
        }
        if load_stored_draft(&transaction, &id)?.is_some() {
            return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
        }
        let notifications_remaining = initial_notifications_remaining(&config);
        insert_draft(
            &transaction,
            &id,
            &config,
            1,
            observed_at_unix_ms,
            observed_at_unix_ms,
            notifications_remaining,
        )?;
        let expected = stored_draft(
            id.clone(),
            config,
            1,
            observed_at_unix_ms,
            observed_at_unix_ms,
            notifications_remaining,
        )?;
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(AutomationScheduleDraftStoreUpdate {
                    draft: expected.draft,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_create(self, &guard, &id, expected, failure)
    }

    pub(crate) fn replace_automation_schedule_draft(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        config: AutomationScheduleDraftConfig,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.replace_automation_schedule_draft_with_hook(
            id,
            expected_revision,
            config,
            observed_at,
            || Ok(()),
        )
    }

    fn replace_automation_schedule_draft_with_hook(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        config: AutomationScheduleDraftConfig,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        if observed_at_unix_ms > MAX_AUTOMATION_UNIX_MS {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let original = load_stored_draft(&transaction, id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
        if original.draft.revision() != expected_revision {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        if original.draft.state() != AutomationScheduleState::Disabled {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        if original.draft.config() == &config {
            return Ok(AutomationScheduleDraftStoreUpdate {
                draft: original.draft,
                changed: false,
            });
        }
        let revision = expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= i64::MAX as u64)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let updated_at_unix_ms = observed_at_unix_ms.max(original.updated_at_unix_ms);
        let notifications_remaining = initial_notifications_remaining(&config);
        let expected = stored_draft(
            id.clone(),
            config.clone(),
            revision,
            system_time_to_unix_ms(original.draft.created_at(), HistoryErrorKind::InternalState)?,
            updated_at_unix_ms,
            notifications_remaining,
        )?;
        let changed = update_draft_parent(
            &transaction,
            id,
            expected_revision,
            &config,
            revision,
            updated_at_unix_ms,
            notifications_remaining,
        )?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        transaction
            .execute(
                "DELETE FROM schedule_rule_exclusions WHERE schedule_id = ?1",
                [id.as_str()],
            )
            .map_err(map_write_sql_error)?;
        insert_exclusions(&transaction, id, config.excluded_rules())?;
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(AutomationScheduleDraftStoreUpdate {
                    draft: expected.draft,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_replace(self, &guard, id, &original, expected, failure)
    }

    pub(crate) fn delete_automation_schedule_draft(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
    ) -> Result<bool, HistoryError> {
        self.delete_automation_schedule_draft_with_hook(id, expected_revision, || Ok(()))
    }

    fn delete_automation_schedule_draft_with_hook(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<bool, HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let Some(original) = load_stored_draft(&transaction, id)? else {
            return Ok(false);
        };
        if original.draft.revision() != expected_revision {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let changed = transaction
            .execute(
                "DELETE FROM schedules WHERE schedule_id = ?1 AND revision = ?2",
                params![
                    id.as_str(),
                    to_i64(expected_revision, HistoryErrorKind::InvalidInput)?
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => return Ok(true),
            Err(error) => error,
        };
        reconcile_delete(self, &guard, id, &original, failure)
    }

    pub(crate) fn enable_automation_schedule_periodic(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.mutate_automation_schedule_activation(
            id,
            expected_revision,
            ActivationMutation::EnablePeriodic,
            observed_at,
            || Ok(()),
        )
    }

    pub(crate) fn pause_automation_schedule(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        reason: AutomationSchedulePauseReason,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.mutate_automation_schedule_activation(
            id,
            expected_revision,
            ActivationMutation::Pause(reason),
            observed_at,
            || Ok(()),
        )
    }

    pub(crate) fn resume_automation_schedule(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.mutate_automation_schedule_activation(
            id,
            expected_revision,
            ActivationMutation::Resume,
            observed_at,
            || Ok(()),
        )
    }

    pub(crate) fn disable_automation_schedule(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.mutate_automation_schedule_activation(
            id,
            expected_revision,
            ActivationMutation::Disable,
            observed_at,
            || Ok(()),
        )
    }

    fn mutate_automation_schedule_activation(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        mutation: ActivationMutation,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        if observed_at_unix_ms > MAX_AUTOMATION_UNIX_MS {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let original = load_stored_draft(&transaction, id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
        if original.draft.revision() != expected_revision {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let Some((state, cursor)) =
            activation_after_mutation(&original.draft, mutation, observed_at_unix_ms)?
        else {
            return Ok(AutomationScheduleDraftStoreUpdate {
                draft: original.draft,
                changed: false,
            });
        };
        let revision = expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= i64::MAX as u64)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let updated_at_unix_ms = observed_at_unix_ms.max(original.updated_at_unix_ms);
        let expected = stored_schedule(
            id.clone(),
            original.draft.config().clone(),
            state,
            cursor,
            revision,
            system_time_to_unix_ms(original.draft.created_at(), HistoryErrorKind::InternalState)?,
            updated_at_unix_ms,
            original.draft.pre_run_notifications_remaining(),
        )?;
        if update_activation_parent(
            &transaction,
            id,
            expected_revision,
            state,
            cursor,
            revision,
            updated_at_unix_ms,
        )? != 1
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(AutomationScheduleDraftStoreUpdate {
                    draft: expected.draft,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_replace(self, &guard, id, &original, expected, failure)
    }

    #[cfg(test)]
    pub(crate) fn advance_automation_periodic_cursor(
        &self,
        id: &AutomationScheduleId,
        expected_schedule_revision: u64,
        expected_cursor: AutomationPeriodicCursor,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.advance_automation_periodic_cursor_with_hook(
            id,
            expected_schedule_revision,
            expected_cursor,
            observed_at,
            || Ok(()),
        )
    }

    #[cfg(test)]
    fn advance_automation_periodic_cursor_with_hook(
        &self,
        id: &AutomationScheduleId,
        expected_schedule_revision: u64,
        expected_cursor: AutomationPeriodicCursor,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        if observed_at_unix_ms > MAX_AUTOMATION_UNIX_MS {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let mut guard = self.lock_current_history_connection()?;
        let original = load_stored_draft(&guard.connection, id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
        if original.draft.revision() != expected_schedule_revision
            || original.draft.state() != AutomationScheduleState::Enabled
            || original.draft.cursor() != Some(AutomationScheduleCursor::Periodic(expected_cursor))
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let cadence = original.draft.config().cadence();
        if !matches!(
            cadence,
            AutomationScheduleCadence::Weekly | AutomationScheduleCadence::Monthly
        ) {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let anchor_unix_ms = system_time_to_unix_ms(
            expected_cursor.recurrence_anchor(),
            HistoryErrorKind::InternalState,
        )?;
        let expected_next_unix_ms =
            system_time_to_unix_ms(expected_cursor.next_run(), HistoryErrorKind::InternalState)?;
        if observed_at_unix_ms < expected_next_unix_ms {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let (next_ordinal, next_run_unix_ms) =
            first_automation_occurrence_after_unix_ms(cadence, anchor_unix_ms, observed_at_unix_ms)
                .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        let cursor_revision = expected_cursor
            .cursor_revision()
            .checked_add(1)
            .filter(|revision| *revision <= i64::MAX as u64)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let next_cursor = AutomationPeriodicCursor::from_stored_parts(
            cursor_revision,
            AUTOMATION_RECURRENCE_POLICY_REVISION,
            expected_cursor.recurrence_anchor(),
            next_ordinal,
            unix_ms_to_system_time(next_run_unix_ms)?,
        );
        let expected = stored_schedule(
            id.clone(),
            original.draft.config().clone(),
            AutomationScheduleState::Enabled,
            Some(AutomationScheduleCursor::Periodic(next_cursor)),
            expected_schedule_revision,
            system_time_to_unix_ms(original.draft.created_at(), HistoryErrorKind::InternalState)?,
            original.updated_at_unix_ms,
            original.draft.pre_run_notifications_remaining(),
        )?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let changed = transaction
            .execute(
                "UPDATE schedules SET
                     cursor_revision = ?1, next_occurrence_ordinal = ?2,
                     next_run_unix_ms = ?3
                 WHERE schedule_id = ?4 AND state = 'enabled' AND revision = ?5
                   AND cursor_revision = ?6 AND recurrence_policy_revision = ?7
                   AND recurrence_anchor_unix_ms = ?8
                   AND next_occurrence_ordinal = ?9 AND next_run_unix_ms = ?10",
                params![
                    to_i64(cursor_revision, HistoryErrorKind::InvalidInput)?,
                    to_i64(next_ordinal, HistoryErrorKind::InvalidInput)?,
                    next_run_unix_ms,
                    id.as_str(),
                    to_i64(expected_schedule_revision, HistoryErrorKind::InvalidInput)?,
                    to_i64(
                        expected_cursor.cursor_revision(),
                        HistoryErrorKind::InvalidInput
                    )?,
                    i64::from(expected_cursor.recurrence_policy_revision()),
                    anchor_unix_ms,
                    to_i64(
                        expected_cursor.next_occurrence_ordinal(),
                        HistoryErrorKind::InvalidInput,
                    )?,
                    expected_next_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(AutomationScheduleDraftStoreUpdate {
                    draft: expected.draft,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_replace(self, &guard, id, &original, expected, failure)
    }

    #[cfg(test)]
    pub(super) fn create_automation_schedule_draft_after_commit_failure_for_test(
        &self,
        id: AutomationScheduleId,
        config: AutomationScheduleDraftConfig,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.create_automation_schedule_draft_with_hook(id, config, observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(test)]
    pub(super) fn replace_automation_schedule_draft_after_commit_failure_for_test(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        config: AutomationScheduleDraftConfig,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.replace_automation_schedule_draft_with_hook(
            id,
            expected_revision,
            config,
            observed_at,
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
        )
    }

    #[cfg(test)]
    pub(super) fn delete_automation_schedule_draft_after_commit_failure_for_test(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
    ) -> Result<bool, HistoryError> {
        self.delete_automation_schedule_draft_with_hook(id, expected_revision, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(test)]
    pub(super) fn enable_automation_schedule_periodic_after_commit_failure_for_test(
        &self,
        id: &AutomationScheduleId,
        expected_revision: u64,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.mutate_automation_schedule_activation(
            id,
            expected_revision,
            ActivationMutation::EnablePeriodic,
            observed_at,
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
        )
    }

    #[cfg(test)]
    pub(super) fn advance_automation_periodic_cursor_after_commit_failure_for_test(
        &self,
        id: &AutomationScheduleId,
        expected_schedule_revision: u64,
        expected_cursor: AutomationPeriodicCursor,
        observed_at: SystemTime,
    ) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
        self.advance_automation_periodic_cursor_with_hook(
            id,
            expected_schedule_revision,
            expected_cursor,
            observed_at,
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
        )
    }
}

fn reconcile_create(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    id: &AutomationScheduleId,
    expected: StoredDraft,
    failure: HistoryError,
) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_draft(&guard.connection, id) {
        Ok(Some(current)) if current == expected => Ok(AutomationScheduleDraftStoreUpdate {
            draft: expected.draft,
            changed: true,
        }),
        Ok(None) => Err(failure),
        Ok(Some(_)) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn reconcile_replace(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    id: &AutomationScheduleId,
    original: &StoredDraft,
    expected: StoredDraft,
    failure: HistoryError,
) -> Result<AutomationScheduleDraftStoreUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_draft(&guard.connection, id) {
        Ok(Some(current)) if current == expected => Ok(AutomationScheduleDraftStoreUpdate {
            draft: expected.draft,
            changed: true,
        }),
        Ok(Some(current)) if current == *original => Err(failure),
        Ok(None) | Ok(Some(_)) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn reconcile_delete(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    id: &AutomationScheduleId,
    original: &StoredDraft,
    failure: HistoryError,
) -> Result<bool, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_draft(&guard.connection, id) {
        Ok(None) => Ok(true),
        Ok(Some(current)) if current == *original => Err(failure),
        Ok(Some(_)) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn stored_draft(
    id: AutomationScheduleId,
    config: AutomationScheduleDraftConfig,
    revision: u64,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
    notifications_remaining: u8,
) -> Result<StoredDraft, HistoryError> {
    stored_schedule(
        id,
        config,
        AutomationScheduleState::Disabled,
        None,
        revision,
        created_at_unix_ms,
        updated_at_unix_ms,
        notifications_remaining,
    )
}

#[allow(clippy::too_many_arguments)]
fn stored_schedule(
    id: AutomationScheduleId,
    config: AutomationScheduleDraftConfig,
    state: AutomationScheduleState,
    cursor: Option<AutomationScheduleCursor>,
    revision: u64,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
    notifications_remaining: u8,
) -> Result<StoredDraft, HistoryError> {
    Ok(StoredDraft {
        draft: AutomationScheduleDraft::from_stored_parts(
            id,
            config,
            state,
            cursor,
            revision,
            unix_ms_to_system_time(created_at_unix_ms)?,
            unix_ms_to_system_time(updated_at_unix_ms)?,
            notifications_remaining,
        ),
        updated_at_unix_ms,
    })
}

fn activation_after_mutation(
    schedule: &AutomationScheduleDraft,
    mutation: ActivationMutation,
    observed_at_unix_ms: i64,
) -> Result<Option<(AutomationScheduleState, Option<AutomationScheduleCursor>)>, HistoryError> {
    match mutation {
        ActivationMutation::EnablePeriodic => {
            if schedule.state() != AutomationScheduleState::Disabled
                || !matches!(
                    schedule.config().cadence(),
                    AutomationScheduleCadence::Weekly | AutomationScheduleCadence::Monthly
                )
            {
                return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
            }
            let (ordinal, next_run_unix_ms) = first_automation_occurrence_after_unix_ms(
                schedule.config().cadence(),
                observed_at_unix_ms,
                observed_at_unix_ms,
            )
            .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
            Ok(Some((
                AutomationScheduleState::Enabled,
                Some(AutomationScheduleCursor::Periodic(
                    AutomationPeriodicCursor::from_stored_parts(
                        1,
                        AUTOMATION_RECURRENCE_POLICY_REVISION,
                        unix_ms_to_system_time(observed_at_unix_ms)?,
                        ordinal,
                        unix_ms_to_system_time(next_run_unix_ms)?,
                    ),
                )),
            )))
        }
        ActivationMutation::Pause(reason) => {
            if schedule.state() != AutomationScheduleState::Enabled {
                return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
            }
            Ok(Some((
                AutomationScheduleState::Paused(reason),
                schedule.cursor(),
            )))
        }
        ActivationMutation::Resume => {
            if !matches!(schedule.state(), AutomationScheduleState::Paused(_)) {
                return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
            }
            let cursor = match schedule.cursor() {
                Some(AutomationScheduleCursor::Periodic(cursor)) => {
                    let anchor_unix_ms = system_time_to_unix_ms(
                        cursor.recurrence_anchor(),
                        HistoryErrorKind::InternalState,
                    )?;
                    let (ordinal, next_run_unix_ms) = first_automation_occurrence_after_unix_ms(
                        schedule.config().cadence(),
                        anchor_unix_ms,
                        observed_at_unix_ms,
                    )
                    .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
                    let cursor_revision = cursor
                        .cursor_revision()
                        .checked_add(1)
                        .filter(|revision| *revision <= i64::MAX as u64)
                        .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
                    AutomationScheduleCursor::Periodic(AutomationPeriodicCursor::from_stored_parts(
                        cursor_revision,
                        AUTOMATION_RECURRENCE_POLICY_REVISION,
                        cursor.recurrence_anchor(),
                        ordinal,
                        unix_ms_to_system_time(next_run_unix_ms)?,
                    ))
                }
                None => return Err(HistoryError::new(HistoryErrorKind::InternalState)),
            };
            Ok(Some((AutomationScheduleState::Enabled, Some(cursor))))
        }
        ActivationMutation::Disable => match schedule.state() {
            AutomationScheduleState::Disabled => Ok(None),
            AutomationScheduleState::Enabled | AutomationScheduleState::Paused(_) => {
                Ok(Some((AutomationScheduleState::Disabled, None)))
            }
        },
    }
}

struct StoredActivationParts {
    state: &'static str,
    pause_reason: Option<&'static str>,
    cursor_revision: i64,
    recurrence_policy_revision: i64,
    recurrence_anchor_unix_ms: Option<i64>,
    next_occurrence_ordinal: Option<i64>,
    next_run_unix_ms: Option<i64>,
}

fn stored_activation_parts(
    state: AutomationScheduleState,
    cursor: Option<AutomationScheduleCursor>,
) -> Result<StoredActivationParts, HistoryError> {
    let (stored_state, pause_reason) = match state {
        AutomationScheduleState::Disabled => ("disabled", None),
        AutomationScheduleState::Enabled => ("enabled", None),
        AutomationScheduleState::Paused(AutomationSchedulePauseReason::User) => {
            ("paused", Some("user"))
        }
        AutomationScheduleState::Paused(AutomationSchedulePauseReason::Failure) => {
            ("paused", Some("failure"))
        }
    };
    match (state, cursor) {
        (AutomationScheduleState::Disabled, None) => Ok(StoredActivationParts {
            state: stored_state,
            pause_reason,
            cursor_revision: 0,
            recurrence_policy_revision: 0,
            recurrence_anchor_unix_ms: None,
            next_occurrence_ordinal: None,
            next_run_unix_ms: None,
        }),
        (
            AutomationScheduleState::Enabled | AutomationScheduleState::Paused(_),
            Some(AutomationScheduleCursor::Periodic(cursor)),
        ) => Ok(StoredActivationParts {
            state: stored_state,
            pause_reason,
            cursor_revision: to_i64(cursor.cursor_revision(), HistoryErrorKind::InvalidInput)?,
            recurrence_policy_revision: i64::from(cursor.recurrence_policy_revision()),
            recurrence_anchor_unix_ms: Some(system_time_to_unix_ms(
                cursor.recurrence_anchor(),
                HistoryErrorKind::InvalidInput,
            )?),
            next_occurrence_ordinal: Some(to_i64(
                cursor.next_occurrence_ordinal(),
                HistoryErrorKind::InvalidInput,
            )?),
            next_run_unix_ms: Some(system_time_to_unix_ms(
                cursor.next_run(),
                HistoryErrorKind::InvalidInput,
            )?),
        }),
        _ => Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
    }
}

fn update_activation_parent(
    transaction: &Transaction<'_>,
    id: &AutomationScheduleId,
    expected_revision: u64,
    state: AutomationScheduleState,
    cursor: Option<AutomationScheduleCursor>,
    revision: u64,
    updated_at_unix_ms: i64,
) -> Result<usize, HistoryError> {
    let parts = stored_activation_parts(state, cursor)?;
    transaction
        .execute(
            "UPDATE schedules SET
                 state = ?1, pause_reason = ?2, cursor_revision = ?3,
                 recurrence_policy_revision = ?4, recurrence_anchor_unix_ms = ?5,
                 next_occurrence_ordinal = ?6, next_run_unix_ms = ?7,
                 revision = ?8, updated_at_unix_ms = ?9
             WHERE schedule_id = ?10 AND revision = ?11",
            params![
                parts.state,
                parts.pause_reason,
                parts.cursor_revision,
                parts.recurrence_policy_revision,
                parts.recurrence_anchor_unix_ms,
                parts.next_occurrence_ordinal,
                parts.next_run_unix_ms,
                to_i64(revision, HistoryErrorKind::InvalidInput)?,
                updated_at_unix_ms,
                id.as_str(),
                to_i64(expected_revision, HistoryErrorKind::InvalidInput)?,
            ],
        )
        .map_err(map_write_sql_error)
}

fn bounded_schedule_population(connection: &Connection) -> Result<usize, HistoryError> {
    run_bounded_query(connection, || {
        let row_limit = i64::try_from(MAX_AUTOMATION_SCHEDULE_DRAFTS + 1)
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;
        let mut statement = connection
            .prepare("SELECT 1 FROM schedules LIMIT ?1")
            .map_err(map_query_sql_error)?;
        let mut rows = statement.query([row_limit]).map_err(map_query_sql_error)?;
        let mut count = 0_usize;
        while rows.next().map_err(map_query_sql_error)?.is_some() {
            count = count
                .checked_add(1)
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::InternalState))?;
            if count > MAX_AUTOMATION_SCHEDULE_DRAFTS {
                return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
            }
        }
        Ok(count)
    })
}

fn initial_notifications_remaining(config: &AutomationScheduleDraftConfig) -> u8 {
    if config.notify_before_run() {
        DEFAULT_AUTOMATION_PRE_RUN_NOTIFICATIONS
    } else {
        0
    }
}

fn stored_authoring_binding(
    binding: Option<AutomationScheduleAuthoringBinding>,
) -> (Option<i64>, Option<Vec<u8>>) {
    binding.map_or((None, None), |binding| {
        (
            Some(i64::from(binding.policy_revision())),
            Some(binding.digest().to_vec()),
        )
    })
}

fn insert_draft(
    transaction: &Transaction<'_>,
    id: &AutomationScheduleId,
    config: &AutomationScheduleDraftConfig,
    revision: u64,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
    notifications_remaining: u8,
) -> Result<(), HistoryError> {
    let scope = stored_scope(config.scope());
    let (authoring_policy_revision, authoring_membership_sha256) =
        stored_authoring_binding(config.authoring_binding());
    let changed = transaction
        .execute(
            "INSERT INTO schedules (
                 schedule_id, state, pause_reason, scope_kind, rule_id, rule_revision, category,
                 cadence, minimum_age_seconds, minimum_reclaimable_bytes,
                 maximum_bytes_per_run, notify_before_run, confirmation_mode,
                 authoring_policy_revision, authoring_membership_sha256,
                 pre_run_notifications_remaining, revision,
                 cursor_revision, recurrence_policy_revision,
                 recurrence_anchor_unix_ms, next_occurrence_ordinal, next_run_unix_ms,
                 created_at_unix_ms, updated_at_unix_ms
             ) VALUES (
                 ?1, 'disabled', NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                 ?10, ?11, ?12, ?13, ?14, ?15, 0, 0, NULL, NULL, NULL, ?16, ?17
             )",
            params![
                id.as_str(),
                scope.kind,
                scope.rule_id,
                scope.rule_revision,
                scope.category,
                stored_cadence(config.cadence()),
                to_i64(
                    config.minimum_age().as_secs(),
                    HistoryErrorKind::InvalidInput
                )?,
                to_i64(
                    config.minimum_reclaimable_bytes(),
                    HistoryErrorKind::InvalidInput
                )?,
                to_i64(
                    config.maximum_bytes_per_run(),
                    HistoryErrorKind::InvalidInput
                )?,
                i64::from(config.notify_before_run()),
                stored_confirmation(config.confirmation_mode()),
                authoring_policy_revision,
                authoring_membership_sha256,
                i64::from(notifications_remaining),
                to_i64(revision, HistoryErrorKind::InvalidInput)?,
                created_at_unix_ms,
                updated_at_unix_ms,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InternalState));
    }
    insert_exclusions(transaction, id, config.excluded_rules())
}

fn update_draft_parent(
    transaction: &Transaction<'_>,
    id: &AutomationScheduleId,
    expected_revision: u64,
    config: &AutomationScheduleDraftConfig,
    revision: u64,
    updated_at_unix_ms: i64,
    notifications_remaining: u8,
) -> Result<usize, HistoryError> {
    let scope = stored_scope(config.scope());
    let (authoring_policy_revision, authoring_membership_sha256) =
        stored_authoring_binding(config.authoring_binding());
    transaction
        .execute(
            "UPDATE schedules SET
                 scope_kind = ?1, rule_id = ?2, rule_revision = ?3, category = ?4,
                 cadence = ?5, minimum_age_seconds = ?6,
                 minimum_reclaimable_bytes = ?7, maximum_bytes_per_run = ?8,
                 notify_before_run = ?9, confirmation_mode = ?10,
                 authoring_policy_revision = ?11, authoring_membership_sha256 = ?12,
                 pre_run_notifications_remaining = ?13,
                 revision = ?14, updated_at_unix_ms = ?15
             WHERE schedule_id = ?16 AND state = 'disabled' AND revision = ?17",
            params![
                scope.kind,
                scope.rule_id,
                scope.rule_revision,
                scope.category,
                stored_cadence(config.cadence()),
                to_i64(
                    config.minimum_age().as_secs(),
                    HistoryErrorKind::InvalidInput
                )?,
                to_i64(
                    config.minimum_reclaimable_bytes(),
                    HistoryErrorKind::InvalidInput
                )?,
                to_i64(
                    config.maximum_bytes_per_run(),
                    HistoryErrorKind::InvalidInput
                )?,
                i64::from(config.notify_before_run()),
                stored_confirmation(config.confirmation_mode()),
                authoring_policy_revision,
                authoring_membership_sha256,
                i64::from(notifications_remaining),
                to_i64(revision, HistoryErrorKind::InvalidInput)?,
                updated_at_unix_ms,
                id.as_str(),
                to_i64(expected_revision, HistoryErrorKind::InvalidInput)?,
            ],
        )
        .map_err(map_write_sql_error)
}

fn insert_exclusions(
    transaction: &Transaction<'_>,
    id: &AutomationScheduleId,
    exclusions: &[RuleRef],
) -> Result<(), HistoryError> {
    if exclusions.len() > MAX_AUTOMATION_SCHEDULE_EXCLUSIONS {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    for (ordinal, exclusion) in exclusions.iter().enumerate() {
        let changed = transaction
            .execute(
                "INSERT INTO schedule_rule_exclusions (
                     schedule_id, exclusion_ordinal, rule_id, rule_revision
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    id.as_str(),
                    i64::try_from(ordinal)
                        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?,
                    exclusion.id().as_str(),
                    i64::from(exclusion.revision().get()),
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
    }
    Ok(())
}

fn load_stored_drafts(connection: &Connection) -> Result<Vec<StoredDraft>, HistoryError> {
    run_bounded_query(connection, || {
        let row_limit = i64::try_from(MAX_AUTOMATION_SCHEDULE_DRAFTS + 1)
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;
        let mut statement = connection
            .prepare(
                "SELECT schedule_id, state, pause_reason, scope_kind, rule_id, rule_revision, category,
                        cadence, minimum_age_seconds, minimum_reclaimable_bytes,
                        maximum_bytes_per_run, notify_before_run, confirmation_mode,
                        authoring_policy_revision, authoring_membership_sha256,
                        pre_run_notifications_remaining, revision,
                        cursor_revision, recurrence_policy_revision,
                        recurrence_anchor_unix_ms, next_occurrence_ordinal, next_run_unix_ms,
                        created_at_unix_ms, updated_at_unix_ms
                 FROM schedules
                 ORDER BY updated_at_unix_ms DESC, schedule_id ASC
                 LIMIT ?1",
            )
            .map_err(map_query_sql_error)?;
        let mut rows = statement.query([row_limit]).map_err(map_query_sql_error)?;
        let mut drafts = Vec::new();
        while let Some(row) = rows.next().map_err(map_query_sql_error)? {
            if drafts.len() >= MAX_AUTOMATION_SCHEDULE_DRAFTS {
                return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
            }
            let raw = raw_draft(row).map_err(map_query_sql_error)?;
            drafts.push(decode_draft(connection, raw)?);
        }
        Ok(drafts)
    })
}

fn load_stored_draft(
    connection: &Connection,
    id: &AutomationScheduleId,
) -> Result<Option<StoredDraft>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT schedule_id, state, pause_reason, scope_kind, rule_id, rule_revision, category,
                        cadence, minimum_age_seconds, minimum_reclaimable_bytes,
                        maximum_bytes_per_run, notify_before_run, confirmation_mode,
                        authoring_policy_revision, authoring_membership_sha256,
                        pre_run_notifications_remaining, revision,
                        cursor_revision, recurrence_policy_revision,
                        recurrence_anchor_unix_ms, next_occurrence_ordinal, next_run_unix_ms,
                        created_at_unix_ms, updated_at_unix_ms
                 FROM schedules WHERE schedule_id = ?1",
                [id.as_str()],
                raw_draft,
            )
            .optional()
            .map_err(map_query_sql_error)?;
        raw.map(|raw| decode_draft(connection, raw)).transpose()
    })
}

fn raw_draft(row: &Row<'_>) -> rusqlite::Result<RawDraft> {
    Ok(RawDraft {
        schedule_id: row.get(0)?,
        state: row.get(1)?,
        pause_reason: row.get(2)?,
        scope_kind: row.get(3)?,
        rule_id: row.get(4)?,
        rule_revision: row.get(5)?,
        category: row.get(6)?,
        cadence: row.get(7)?,
        minimum_age_seconds: row.get(8)?,
        minimum_reclaimable_bytes: row.get(9)?,
        maximum_bytes_per_run: row.get(10)?,
        notify_before_run: row.get(11)?,
        confirmation_mode: row.get(12)?,
        authoring_policy_revision: row.get(13)?,
        authoring_membership_sha256: row.get(14)?,
        pre_run_notifications_remaining: row.get(15)?,
        revision: row.get(16)?,
        cursor_revision: row.get(17)?,
        recurrence_policy_revision: row.get(18)?,
        recurrence_anchor_unix_ms: row.get(19)?,
        next_occurrence_ordinal: row.get(20)?,
        next_run_unix_ms: row.get(21)?,
        created_at_unix_ms: row.get(22)?,
        updated_at_unix_ms: row.get(23)?,
    })
}

fn decode_draft(connection: &Connection, raw: RawDraft) -> Result<StoredDraft, HistoryError> {
    if raw.revision <= 0
        || raw.created_at_unix_ms < 0
        || raw.created_at_unix_ms > MAX_AUTOMATION_UNIX_MS
        || raw.updated_at_unix_ms < raw.created_at_unix_ms
        || raw.updated_at_unix_ms > MAX_AUTOMATION_UNIX_MS
    {
        return Err(corrupt());
    }
    let cadence = cadence_from_stored(&raw.cadence)?;
    let (state, cursor) = decode_activation(&raw, cadence)?;
    let id = AutomationScheduleId::new(raw.schedule_id).map_err(|_| corrupt())?;
    let scope = match (
        raw.scope_kind.as_str(),
        raw.rule_id,
        raw.rule_revision,
        raw.category,
    ) {
        ("rule", Some(id), Some(revision), None) => AutomationScheduleScope::Rule(RuleRef::new(
            RuleId::new(id).map_err(|_| corrupt())?,
            RuleRevision::new(u32::try_from(revision).map_err(|_| corrupt())?)
                .map_err(|_| corrupt())?,
        )),
        ("category", None, None, Some(category)) => {
            AutomationScheduleScope::Category(category_from_stored(&category)?)
        }
        _ => return Err(corrupt()),
    };
    let minimum_age = Duration::from_secs(from_i64(raw.minimum_age_seconds)?);
    let minimum_reclaimable_bytes = from_i64(raw.minimum_reclaimable_bytes)?;
    let maximum_bytes_per_run = from_i64(raw.maximum_bytes_per_run)?;
    let notify_before_run = stored_bool(raw.notify_before_run)?;
    let confirmation_mode = confirmation_from_stored(&raw.confirmation_mode)?;
    let notifications_remaining = u8::try_from(raw.pre_run_notifications_remaining)
        .ok()
        .filter(|value| *value <= DEFAULT_AUTOMATION_PRE_RUN_NOTIFICATIONS)
        .ok_or_else(corrupt)?;
    if !notify_before_run && notifications_remaining != 0 {
        return Err(corrupt());
    }
    let exclusions = load_exclusions(connection, &id)?;
    let authoring_binding = decode_authoring_binding(
        raw.authoring_policy_revision,
        raw.authoring_membership_sha256,
    )?;
    let config = AutomationScheduleDraftConfig::try_new_with_optional_authoring_binding(
        scope,
        cadence,
        minimum_age,
        minimum_reclaimable_bytes,
        maximum_bytes_per_run,
        exclusions,
        notify_before_run,
        confirmation_mode,
        authoring_binding,
    )
    .map_err(|_| corrupt())?;
    let revision = from_i64(raw.revision)?;
    let created_at = unix_ms_to_system_time(raw.created_at_unix_ms)?;
    let updated_at = unix_ms_to_system_time(raw.updated_at_unix_ms)?;
    Ok(StoredDraft {
        draft: AutomationScheduleDraft::from_stored_parts(
            id,
            config,
            state,
            cursor,
            revision,
            created_at,
            updated_at,
            notifications_remaining,
        ),
        updated_at_unix_ms: raw.updated_at_unix_ms,
    })
}

fn decode_authoring_binding(
    policy_revision: Option<i64>,
    digest: Option<Vec<u8>>,
) -> Result<Option<AutomationScheduleAuthoringBinding>, HistoryError> {
    match (policy_revision, digest) {
        (None, None) => Ok(None),
        (Some(policy_revision), Some(digest)) => {
            let policy_revision = u32::try_from(policy_revision).map_err(|_| corrupt())?;
            let digest: [u8; 32] = digest.try_into().map_err(|_| corrupt())?;
            AutomationScheduleAuthoringBinding::try_new(policy_revision, digest)
                .map(Some)
                .map_err(|_| corrupt())
        }
        _ => Err(corrupt()),
    }
}

fn decode_activation(
    raw: &RawDraft,
    cadence: AutomationScheduleCadence,
) -> Result<(AutomationScheduleState, Option<AutomationScheduleCursor>), HistoryError> {
    if raw.state == DISABLED_STATE {
        if raw.pause_reason.is_some()
            || raw.cursor_revision != 0
            || raw.recurrence_policy_revision != 0
            || raw.recurrence_anchor_unix_ms.is_some()
            || raw.next_occurrence_ordinal.is_some()
            || raw.next_run_unix_ms.is_some()
        {
            return Err(corrupt());
        }
        return Ok((AutomationScheduleState::Disabled, None));
    }

    let state = match (raw.state.as_str(), raw.pause_reason.as_deref()) {
        ("enabled", None) => AutomationScheduleState::Enabled,
        ("paused", Some("user")) => {
            AutomationScheduleState::Paused(AutomationSchedulePauseReason::User)
        }
        ("paused", Some("failure")) => {
            AutomationScheduleState::Paused(AutomationSchedulePauseReason::Failure)
        }
        _ => return Err(corrupt()),
    };
    let cursor_revision = from_i64(raw.cursor_revision)?;
    if cursor_revision == 0
        || raw.recurrence_policy_revision != i64::from(AUTOMATION_RECURRENCE_POLICY_REVISION)
    {
        return Err(corrupt());
    }
    let cursor = match cadence {
        AutomationScheduleCadence::Weekly | AutomationScheduleCadence::Monthly => {
            let (Some(anchor), Some(ordinal), Some(next_run)) = (
                raw.recurrence_anchor_unix_ms,
                raw.next_occurrence_ordinal,
                raw.next_run_unix_ms,
            ) else {
                return Err(corrupt());
            };
            if !(0..=MAX_AUTOMATION_UNIX_MS).contains(&anchor)
                || !(0..=MAX_AUTOMATION_UNIX_MS).contains(&next_run)
            {
                return Err(corrupt());
            }
            let ordinal = from_i64(ordinal)?;
            if ordinal == 0
                || materialize_automation_occurrence_unix_ms(cadence, anchor, ordinal).ok()
                    != Some(next_run)
            {
                return Err(corrupt());
            }
            AutomationScheduleCursor::Periodic(AutomationPeriodicCursor::from_stored_parts(
                cursor_revision,
                AUTOMATION_RECURRENCE_POLICY_REVISION,
                unix_ms_to_system_time(anchor)?,
                ordinal,
                unix_ms_to_system_time(next_run)?,
            ))
        }
        AutomationScheduleCadence::LowDiskOnly => return Err(corrupt()),
    };
    Ok((state, Some(cursor)))
}

fn load_exclusions(
    connection: &Connection,
    id: &AutomationScheduleId,
) -> Result<Vec<RuleRef>, HistoryError> {
    let row_limit = i64::try_from(MAX_AUTOMATION_SCHEDULE_EXCLUSIONS + 1)
        .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;
    let mut statement = connection
        .prepare(
            "SELECT exclusion_ordinal, rule_id, rule_revision
             FROM schedule_rule_exclusions
             WHERE schedule_id = ?1
             ORDER BY exclusion_ordinal
             LIMIT ?2",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![id.as_str(), row_limit])
        .map_err(map_query_sql_error)?;
    let mut exclusions = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if exclusions.len() >= MAX_AUTOMATION_SCHEDULE_EXCLUSIONS {
            return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if usize::try_from(ordinal).ok() != Some(exclusions.len()) {
            return Err(corrupt());
        }
        let rule_id: String = row.get(1).map_err(map_query_sql_error)?;
        let revision: i64 = row.get(2).map_err(map_query_sql_error)?;
        exclusions.push(RuleRef::new(
            RuleId::new(rule_id).map_err(|_| corrupt())?,
            RuleRevision::new(u32::try_from(revision).map_err(|_| corrupt())?)
                .map_err(|_| corrupt())?,
        ));
    }
    if exclusions.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(corrupt());
    }
    Ok(exclusions)
}

struct StoredScope<'a> {
    kind: &'static str,
    rule_id: Option<&'a str>,
    rule_revision: Option<i64>,
    category: Option<&'static str>,
}

fn stored_scope(scope: &AutomationScheduleScope) -> StoredScope<'_> {
    match scope {
        AutomationScheduleScope::Rule(rule) => StoredScope {
            kind: "rule",
            rule_id: Some(rule.id().as_str()),
            rule_revision: Some(i64::from(rule.revision().get())),
            category: None,
        },
        AutomationScheduleScope::Category(category) => StoredScope {
            kind: "category",
            rule_id: None,
            rule_revision: None,
            category: Some(stored_category(*category)),
        },
    }
}

const fn stored_cadence(cadence: AutomationScheduleCadence) -> &'static str {
    match cadence {
        AutomationScheduleCadence::Weekly => "weekly",
        AutomationScheduleCadence::Monthly => "monthly",
        AutomationScheduleCadence::LowDiskOnly => "low_disk_only",
    }
}

fn cadence_from_stored(value: &str) -> Result<AutomationScheduleCadence, HistoryError> {
    match value {
        "weekly" => Ok(AutomationScheduleCadence::Weekly),
        "monthly" => Ok(AutomationScheduleCadence::Monthly),
        "low_disk_only" => Ok(AutomationScheduleCadence::LowDiskOnly),
        _ => Err(corrupt()),
    }
}

const fn stored_confirmation(mode: AutomationConfirmationMode) -> &'static str {
    match mode {
        AutomationConfirmationMode::RequireConfirmation => "require_confirmation",
        AutomationConfirmationMode::FullyAutomatic => "fully_automatic",
    }
}

fn confirmation_from_stored(value: &str) -> Result<AutomationConfirmationMode, HistoryError> {
    match value {
        "require_confirmation" => Ok(AutomationConfirmationMode::RequireConfirmation),
        "fully_automatic" => Ok(AutomationConfirmationMode::FullyAutomatic),
        _ => Err(corrupt()),
    }
}

const fn stored_category(category: CandidateCategory) -> &'static str {
    match category {
        CandidateCategory::DeveloperArtifact => "developer_artifact",
        CandidateCategory::ApplicationCache => "application_cache",
        CandidateCategory::BrowserCache => "browser_cache",
        CandidateCategory::LogAndDiagnostic => "log_and_diagnostic",
        CandidateCategory::InstallerAndDownload => "installer_and_download",
        CandidateCategory::DeviceAndSimulatorData => "device_and_simulator_data",
        CandidateCategory::CloudFile => "cloud_file",
        CandidateCategory::LargeReviewItem => "large_review_item",
        CandidateCategory::ProtectedSystemData => "protected_system_data",
        CandidateCategory::UnknownStorage => "unknown_storage",
    }
}

fn category_from_stored(value: &str) -> Result<CandidateCategory, HistoryError> {
    match value {
        "developer_artifact" => Ok(CandidateCategory::DeveloperArtifact),
        "application_cache" => Ok(CandidateCategory::ApplicationCache),
        "browser_cache" => Ok(CandidateCategory::BrowserCache),
        "log_and_diagnostic" => Ok(CandidateCategory::LogAndDiagnostic),
        "installer_and_download" => Ok(CandidateCategory::InstallerAndDownload),
        "device_and_simulator_data" => Ok(CandidateCategory::DeviceAndSimulatorData),
        "cloud_file" => Ok(CandidateCategory::CloudFile),
        "large_review_item" => Ok(CandidateCategory::LargeReviewItem),
        "protected_system_data" => Ok(CandidateCategory::ProtectedSystemData),
        "unknown_storage" => Ok(CandidateCategory::UnknownStorage),
        _ => Err(corrupt()),
    }
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;

    fn open(temp: &TempDir) -> Arc<StoreCoordinator> {
        StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap()
    }

    fn id(value: &str) -> AutomationScheduleId {
        AutomationScheduleId::new(value).unwrap()
    }

    fn rule(value: &str, revision: u32) -> RuleRef {
        RuleRef::new(
            RuleId::new(value).unwrap(),
            RuleRevision::new(revision).unwrap(),
        )
    }

    fn config(exclusions: Vec<RuleRef>, notify_before_run: bool) -> AutomationScheduleDraftConfig {
        AutomationScheduleDraftConfig::try_new_bound(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            AutomationScheduleCadence::Monthly,
            Duration::from_secs(30 * 24 * 60 * 60),
            50 * 1024 * 1024 * 1024,
            25 * 1024 * 1024 * 1024,
            exclusions,
            notify_before_run,
            AutomationConfirmationMode::RequireConfirmation,
            AutomationScheduleAuthoringBinding::try_new(1, [9; 32]).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn missing_registry_reads_empty_without_writing() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        assert!(store.load_automation_schedule_drafts().unwrap().is_empty());
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM schedules", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM schedule_rule_exclusions", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap(),
                0
            );
        });
    }

    #[test]
    fn create_reopen_and_storage_shape_are_disabled_and_path_free() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        let observed = UNIX_EPOCH + Duration::from_millis(1_750_000_000_123);
        let draft_id = id("automation:reopen");
        let settings = AutomationScheduleDraftConfig::default_for_scope(
            AutomationScheduleScope::Rule(rule("developer.rust.target", 3)),
        );
        let created = store
            .create_automation_schedule_draft_after_commit_failure_for_test(
                draft_id.clone(),
                settings.clone(),
                observed,
            )
            .unwrap();
        assert!(created.changed);
        assert_eq!(created.draft.id(), &draft_id);
        assert_eq!(created.draft.config(), &settings);
        assert_eq!(created.draft.revision(), 1);
        assert_eq!(created.draft.created_at(), observed);
        assert_eq!(created.draft.updated_at(), observed);
        assert_eq!(
            created.draft.pre_run_notifications_remaining(),
            DEFAULT_AUTOMATION_PRE_RUN_NOTIFICATIONS
        );
        store.with_connection(|connection| {
            let state: String = connection
                .query_row(
                    "SELECT state FROM schedules WHERE schedule_id = ?1",
                    [draft_id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(state, DISABLED_STATE);
            let columns = connection
                .prepare("SELECT name FROM pragma_table_info('schedules') ORDER BY cid")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(!columns.iter().any(|column| column.contains("path")));
            assert!(!columns.iter().any(|column| column == "enabled"));
            assert!(columns.iter().any(|column| column == "next_run_unix_ms"));
        });
        drop(store);

        let reopened = StoreCoordinator::open(&database).unwrap();
        assert_eq!(
            reopened.load_automation_schedule_drafts().unwrap(),
            vec![created.draft]
        );
    }

    #[test]
    fn replace_is_cas_idempotent_monotonic_and_preserves_independent_size_controls() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        let draft_id = id("automation:cas");
        let first_time = UNIX_EPOCH + Duration::from_millis(2_000);
        let first_config = config(Vec::new(), true);
        let created = store
            .create_automation_schedule_draft(draft_id.clone(), first_config.clone(), first_time)
            .unwrap();
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .execute(
                        "UPDATE schedules SET pre_run_notifications_remaining = 1
                         WHERE schedule_id = ?1",
                        [draft_id.as_str()],
                    )
                    .unwrap(),
                1
            );
        });
        let retry = store
            .replace_automation_schedule_draft(
                &draft_id,
                created.draft.revision(),
                first_config.clone(),
                first_time + Duration::from_secs(1),
            )
            .unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.draft.config(), &first_config);
        assert_eq!(retry.draft.revision(), created.draft.revision());
        assert_eq!(retry.draft.pre_run_notifications_remaining(), 1);

        let second_config = AutomationScheduleDraftConfig::try_new(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            AutomationScheduleCadence::LowDiskOnly,
            Duration::from_secs(7 * 24 * 60 * 60),
            60 * 1024 * 1024 * 1024,
            20 * 1024 * 1024 * 1024,
            vec![rule("developer.z", 2), rule("developer.a", 1)],
            true,
            AutomationConfirmationMode::FullyAutomatic,
        )
        .unwrap();
        let replaced = store
            .replace_automation_schedule_draft_after_commit_failure_for_test(
                &draft_id,
                1,
                second_config.clone(),
                UNIX_EPOCH + Duration::from_millis(1_000),
            )
            .unwrap();
        assert!(replaced.changed);
        assert_eq!(replaced.draft.revision(), 2);
        assert_eq!(replaced.draft.created_at(), first_time);
        assert_eq!(replaced.draft.updated_at(), first_time);
        assert_eq!(replaced.draft.config(), &second_config);
        assert_eq!(
            replaced.draft.pre_run_notifications_remaining(),
            DEFAULT_AUTOMATION_PRE_RUN_NOTIFICATIONS
        );
        assert_eq!(
            replaced.draft.config().excluded_rules(),
            [rule("developer.a", 1), rule("developer.z", 2)]
        );
        assert_eq!(
            store
                .replace_automation_schedule_draft(
                    &draft_id,
                    1,
                    second_config,
                    first_time + Duration::from_secs(2),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );

        let notifications_disabled = AutomationScheduleDraftConfig::try_new(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            AutomationScheduleCadence::LowDiskOnly,
            Duration::from_secs(7 * 24 * 60 * 60),
            60 * 1024 * 1024 * 1024,
            20 * 1024 * 1024 * 1024,
            vec![rule("developer.a", 1), rule("developer.z", 2)],
            false,
            AutomationConfirmationMode::FullyAutomatic,
        )
        .unwrap();
        let without_notifications = store
            .replace_automation_schedule_draft(
                &draft_id,
                replaced.draft.revision(),
                notifications_disabled.clone(),
                first_time + Duration::from_secs(2),
            )
            .unwrap();
        assert!(without_notifications.changed);
        assert_eq!(
            without_notifications.draft.config(),
            &notifications_disabled
        );
        assert_eq!(
            without_notifications
                .draft
                .pre_run_notifications_remaining(),
            0
        );

        drop(store);
        let reopened = StoreCoordinator::open(&database).unwrap();
        assert_eq!(
            reopened.load_automation_schedule_drafts().unwrap(),
            vec![without_notifications.draft]
        );
    }

    #[test]
    fn periodic_activation_transitions_are_revisioned_reopenable_and_exact_cas() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        let schedule_id = id("automation:activation");
        let created_at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let created = store
            .create_automation_schedule_draft(
                schedule_id.clone(),
                config(Vec::new(), true),
                created_at,
            )
            .unwrap();

        let enabled = store
            .enable_automation_schedule_periodic_after_commit_failure_for_test(
                &schedule_id,
                created.draft.revision(),
                created_at,
            )
            .unwrap();
        assert_eq!(enabled.draft.state(), AutomationScheduleState::Enabled);
        assert_eq!(enabled.draft.revision(), 2);
        let AutomationScheduleCursor::Periodic(first_cursor) = enabled.draft.cursor().unwrap();
        assert_eq!(first_cursor.cursor_revision(), 1);
        assert_eq!(first_cursor.recurrence_anchor(), created_at);
        assert!(first_cursor.next_run() > created_at);
        assert_eq!(
            store
                .replace_automation_schedule_draft(
                    &schedule_id,
                    enabled.draft.revision(),
                    enabled.draft.config().clone(),
                    created_at + Duration::from_secs(1),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );

        let advanced = store
            .advance_automation_periodic_cursor_after_commit_failure_for_test(
                &schedule_id,
                enabled.draft.revision(),
                first_cursor,
                first_cursor.next_run(),
            )
            .unwrap();
        assert_eq!(advanced.draft.revision(), enabled.draft.revision());
        assert_eq!(advanced.draft.updated_at(), enabled.draft.updated_at());
        let AutomationScheduleCursor::Periodic(advanced_cursor) = advanced.draft.cursor().unwrap();
        assert_eq!(advanced_cursor.cursor_revision(), 2);
        assert!(advanced_cursor.next_run() > first_cursor.next_run());
        assert_eq!(
            store
                .advance_automation_periodic_cursor(
                    &schedule_id,
                    enabled.draft.revision(),
                    first_cursor,
                    first_cursor.next_run(),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );

        let paused = store
            .pause_automation_schedule(
                &schedule_id,
                advanced.draft.revision(),
                AutomationSchedulePauseReason::User,
                advanced_cursor.next_run(),
            )
            .unwrap();
        assert_eq!(
            paused.draft.state(),
            AutomationScheduleState::Paused(AutomationSchedulePauseReason::User)
        );
        assert_eq!(paused.draft.cursor(), advanced.draft.cursor());
        let resumed_at = advanced_cursor.next_run() + Duration::from_secs(93 * 24 * 60 * 60);
        let resumed = store
            .resume_automation_schedule(&schedule_id, paused.draft.revision(), resumed_at)
            .unwrap();
        assert_eq!(resumed.draft.state(), AutomationScheduleState::Enabled);
        let AutomationScheduleCursor::Periodic(resumed_cursor) = resumed.draft.cursor().unwrap();
        assert_eq!(
            resumed_cursor.cursor_revision(),
            advanced_cursor.cursor_revision() + 1
        );
        assert_eq!(
            resumed_cursor.recurrence_anchor(),
            advanced_cursor.recurrence_anchor()
        );
        assert!(resumed_cursor.next_run() > resumed_at);

        let disabled = store
            .disable_automation_schedule(
                &schedule_id,
                resumed.draft.revision(),
                resumed_at + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(disabled.draft.state(), AutomationScheduleState::Disabled);
        assert_eq!(disabled.draft.cursor(), None);
        let retry = store
            .disable_automation_schedule(
                &schedule_id,
                disabled.draft.revision(),
                resumed_at + Duration::from_secs(2),
            )
            .unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.draft, disabled.draft);

        drop(store);
        let reopened = StoreCoordinator::open(&database).unwrap();
        assert_eq!(
            reopened.load_automation_schedule_drafts().unwrap(),
            vec![disabled.draft]
        );
    }

    #[test]
    fn low_disk_activation_is_unavailable_without_authoritative_episode_identity() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let schedule_id = id("automation:low-disk-deferred");
        let low_disk = AutomationScheduleDraftConfig::try_new(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            AutomationScheduleCadence::LowDiskOnly,
            Duration::ZERO,
            0,
            1,
            Vec::new(),
            false,
            AutomationConfirmationMode::RequireConfirmation,
        )
        .unwrap();
        store
            .create_automation_schedule_draft(
                schedule_id.clone(),
                low_disk,
                UNIX_EPOCH + Duration::from_millis(1),
            )
            .unwrap();
        assert_eq!(
            store
                .enable_automation_schedule_periodic(
                    &schedule_id,
                    1,
                    UNIX_EPOCH + Duration::from_millis(2),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
    }

    #[test]
    fn delete_is_revision_checked_idempotent_and_reconciles_committed_outcome() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let draft_id = id("automation:delete");
        store
            .create_automation_schedule_draft(
                draft_id.clone(),
                config(Vec::new(), true),
                UNIX_EPOCH + Duration::from_millis(1),
            )
            .unwrap();
        assert_eq!(
            store
                .delete_automation_schedule_draft(&draft_id, 2)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert!(
            store
                .delete_automation_schedule_draft_after_commit_failure_for_test(&draft_id, 1)
                .unwrap()
        );
        assert!(
            !store
                .delete_automation_schedule_draft(&draft_id, 1)
                .unwrap()
        );
        assert!(store.load_automation_schedule_drafts().unwrap().is_empty());
    }

    #[test]
    fn list_order_is_updated_descending_then_id_ascending() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let same_time = UNIX_EPOCH + Duration::from_millis(10);
        for value in ["automation:b", "automation:a"] {
            store
                .create_automation_schedule_draft(id(value), config(Vec::new(), true), same_time)
                .unwrap();
        }
        store
            .create_automation_schedule_draft(
                id("automation:c"),
                config(Vec::new(), true),
                same_time + Duration::from_millis(1),
            )
            .unwrap();
        let ids = store
            .load_automation_schedule_drafts()
            .unwrap()
            .into_iter()
            .map(|draft| draft.id().as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["automation:c", "automation:a", "automation:b"]);
    }

    #[test]
    fn noncanonical_or_malformed_rows_fail_closed_without_rewrite() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let draft_id = id("automation:corrupt-order");
        store
            .create_automation_schedule_draft(
                draft_id.clone(),
                config(vec![rule("developer.a", 1), rule("developer.z", 1)], true),
                UNIX_EPOCH + Duration::from_millis(1),
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE schedule_rule_exclusions SET exclusion_ordinal = 31
                     WHERE schedule_id = ?1 AND exclusion_ordinal = 0",
                    [draft_id.as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE schedule_rule_exclusions SET exclusion_ordinal = 0
                     WHERE schedule_id = ?1 AND exclusion_ordinal = 1",
                    [draft_id.as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE schedule_rule_exclusions SET exclusion_ordinal = 1
                     WHERE schedule_id = ?1 AND exclusion_ordinal = 31",
                    [draft_id.as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_automation_schedule_drafts().unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let draft_id = id("automation:corrupt-state");
        store
            .create_automation_schedule_draft(
                draft_id.clone(),
                config(Vec::new(), true),
                UNIX_EPOCH + Duration::from_millis(1),
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE schedules SET state = 'enabled' WHERE schedule_id = ?1",
                    [draft_id.as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store.load_automation_schedule_drafts().unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let schedule_id = id("automation:corrupt-active-shape");
        let low_disk = AutomationScheduleDraftConfig::try_new(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            AutomationScheduleCadence::LowDiskOnly,
            Duration::ZERO,
            0,
            1,
            Vec::new(),
            false,
            AutomationConfirmationMode::RequireConfirmation,
        )
        .unwrap();
        store
            .create_automation_schedule_draft(
                schedule_id.clone(),
                low_disk,
                UNIX_EPOCH + Duration::from_millis(1),
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE schedules SET state = 'enabled', cursor_revision = 1,
                         recurrence_policy_revision = 1, recurrence_anchor_unix_ms = 1,
                         next_occurrence_ordinal = 1, next_run_unix_ms = 604800001
                     WHERE schedule_id = ?1",
                    [schedule_id.as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store.load_automation_schedule_drafts().unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn every_authority_relevant_parent_field_is_revalidated_on_read() {
        let corrupting_updates = [
            "state = 'enabled'",
            "scope_kind = 'rule'",
            "category = 'protected'",
            "cadence = 'daily'",
            "minimum_age_seconds = -1",
            "minimum_reclaimable_bytes = -1",
            "maximum_bytes_per_run = 0",
            "notify_before_run = 2",
            "confirmation_mode = 'silent'",
            "authoring_policy_revision = NULL",
            "authoring_membership_sha256 = x'01'",
            "authoring_policy_revision = 0",
            "notify_before_run = 0, pre_run_notifications_remaining = 3",
            "pre_run_notifications_remaining = 4",
            "revision = 0",
            "created_at_unix_ms = -1",
            "updated_at_unix_ms = 0",
        ];
        for (index, update) in corrupting_updates.into_iter().enumerate() {
            let temp = TempDir::new().unwrap();
            let store = open(&temp);
            store
                .create_automation_schedule_draft(
                    id("automation:corrupt-parent"),
                    config(Vec::new(), true),
                    UNIX_EPOCH + Duration::from_millis(1),
                )
                .unwrap();
            store.with_connection(|connection| {
                connection
                    .pragma_update(None, "ignore_check_constraints", true)
                    .unwrap();
                connection
                    .execute(
                        &format!(
                            "UPDATE schedules SET {update}
                             WHERE schedule_id = 'automation:corrupt-parent'"
                        ),
                        [],
                    )
                    .unwrap();
                connection
                    .pragma_update(None, "ignore_check_constraints", false)
                    .unwrap();
            });
            assert_eq!(
                store.load_automation_schedule_drafts().unwrap_err().kind,
                HistoryErrorKind::CorruptData,
                "corruption fixture {index}: {update}"
            );
        }
    }

    #[test]
    fn hostile_population_is_bounded_for_reads_and_create() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        store.with_connection(|connection| {
            for ordinal in 0..=MAX_AUTOMATION_SCHEDULE_DRAFTS {
                connection
                    .execute(
                        "INSERT INTO schedules (
                             schedule_id, state, scope_kind, rule_id, rule_revision, category,
                             cadence, minimum_age_seconds, minimum_reclaimable_bytes,
                             maximum_bytes_per_run, notify_before_run, confirmation_mode,
                             pre_run_notifications_remaining, revision,
                             created_at_unix_ms, updated_at_unix_ms
                         ) VALUES (?1, 'disabled', 'category', NULL, NULL,
                             'developer_artifact', 'monthly', 0, 0, 1, 0,
                             'require_confirmation', 0, 1, 1, 1)",
                        [format!("automation:hostile:{ordinal:03}")],
                    )
                    .unwrap();
            }
        });
        assert_eq!(
            store.load_automation_schedule_drafts().unwrap_err().kind,
            HistoryErrorKind::QueryLimitExceeded
        );
        assert_eq!(
            store
                .create_automation_schedule_draft(
                    id("automation:overflow"),
                    config(Vec::new(), true),
                    UNIX_EPOCH + Duration::from_millis(2),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::QueryLimitExceeded
        );
    }
}
