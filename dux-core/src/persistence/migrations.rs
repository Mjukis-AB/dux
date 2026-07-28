use std::borrow::Cow;
use std::time::{Duration, Instant};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, Transaction, params};
use sha2::{Digest, Sha256};

use super::status::{DATABASE_SCHEMA_VERSION, DatabaseOpenError, DatabaseOpenErrorKind};

pub(crate) const DUX_APPLICATION_ID: u32 = 0x4455_5831;
const MAX_LEDGER_ROWS: i64 = 128;
const MAX_SCHEMA_TYPE_BYTES: i64 = 16;
const MAX_SCHEMA_NAME_BYTES: i64 = 128;
const MAX_SCHEMA_SQL_BYTES: i64 = 64 * 1024;
const MAX_SCHEMA_FINGERPRINT_BYTES: usize = 512 * 1024;
const PROGRESS_OP_INTERVAL: i32 = 1_000;
const STARTUP_INSPECTION_BUDGET: InspectionBudget = InspectionBudget {
    max_callbacks: 250_000,
    max_elapsed: Duration::from_secs(10),
};
const STATUS_INSPECTION_BUDGET: InspectionBudget = InspectionBudget {
    max_callbacks: 1_000,
    max_elapsed: Duration::from_millis(250),
};
const MIGRATION_BUDGET: InspectionBudget = InspectionBudget {
    max_callbacks: 500_000,
    max_elapsed: Duration::from_secs(30),
};

#[derive(Clone, Copy)]
struct InspectionBudget {
    max_callbacks: u64,
    max_elapsed: Duration,
}

#[derive(Clone, Copy)]
struct InspectionClock {
    started_at: Instant,
    max_elapsed: Duration,
}

impl InspectionClock {
    fn checkpoint(self) -> Result<(), DatabaseOpenError> {
        if self.started_at.elapsed() >= self.max_elapsed {
            return Err(DatabaseOpenError::new(
                DatabaseOpenErrorKind::InspectionLimitExceeded,
            ));
        }
        Ok(())
    }
}

struct ProgressHandlerGuard<'connection> {
    connection: &'connection Connection,
    installed: bool,
}

impl ProgressHandlerGuard<'_> {
    fn remove(&mut self) -> rusqlite::Result<()> {
        self.connection.progress_handler(0, None::<fn() -> bool>)?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for ProgressHandlerGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        }
    }
}

struct MigrationAuthorizerGuard<'connection> {
    connection: &'connection Connection,
    installed: bool,
}

impl MigrationAuthorizerGuard<'_> {
    fn install(connection: &Connection) -> Result<MigrationAuthorizerGuard<'_>, DatabaseOpenError> {
        connection
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => {
                    Authorization::Deny
                }
                _ => Authorization::Allow,
            }))
            .map_err(map_migration_error)?;
        Ok(MigrationAuthorizerGuard {
            connection,
            installed: true,
        })
    }

    fn remove(&mut self) -> Result<(), DatabaseOpenError> {
        self.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(map_migration_error)?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for MigrationAuthorizerGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self
                .connection
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}

#[derive(Clone, Copy)]
enum InspectionDepth {
    FullIntegrity,
    Compatibility,
}

#[derive(Clone, Copy)]
pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) name: &'static str,
    pub(crate) checksum_sha256: [u8; 32],
    pub(crate) sql: &'static str,
}

const MIGRATIONS: [Migration; 12] = [
    Migration {
        version: 1,
        name: "initial-storage-schema",
        checksum_sha256: [
            0xb8, 0x0e, 0x49, 0x87, 0x60, 0x77, 0xda, 0x7f, 0xac, 0x81, 0x27, 0xb0, 0x09, 0xb4,
            0xe7, 0x2c, 0xb2, 0xd7, 0x77, 0x2c, 0x02, 0xfb, 0x3d, 0xb0, 0xe9, 0x93, 0xaa, 0x1a,
            0x63, 0xfb, 0x70, 0x27,
        ],
        sql: include_str!("../../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "candidate-cleanup-history",
        checksum_sha256: [
            0xab, 0x94, 0x9c, 0x4e, 0x6c, 0xbd, 0x09, 0x18, 0x50, 0x57, 0x99, 0x6f, 0x77, 0x4c,
            0x0e, 0x64, 0xed, 0xf4, 0xa4, 0x1f, 0x3c, 0x6b, 0x1f, 0xb5, 0x04, 0x23, 0x53, 0xc6,
            0x90, 0xea, 0xfe, 0x95,
        ],
        sql: include_str!("../../migrations/0002_candidate_cleanup_history.sql"),
    },
    Migration {
        version: 3,
        name: "candidate-plan-claims",
        checksum_sha256: [
            0xc2, 0x14, 0xbb, 0x41, 0xd9, 0xb7, 0x69, 0xd3, 0xa0, 0xab, 0x95, 0xa5, 0x1e, 0xed,
            0x31, 0x94, 0x0b, 0x18, 0xfd, 0x1e, 0x61, 0x27, 0xd8, 0xea, 0xc7, 0xe6, 0x4d, 0xe0,
            0xeb, 0x34, 0x8b, 0x92,
        ],
        sql: include_str!("../../migrations/0003_candidate_plan_claims.sql"),
    },
    Migration {
        version: 4,
        name: "candidate-evaluations",
        checksum_sha256: [
            0xc7, 0xba, 0x43, 0x9c, 0x4f, 0xb1, 0x7b, 0xff, 0x76, 0xbe, 0x85, 0xce, 0xc6, 0x4b,
            0x7f, 0xfb, 0xc5, 0xe8, 0x9e, 0x5c, 0x2c, 0xbf, 0x69, 0x60, 0x10, 0x37, 0x75, 0x67,
            0x4f, 0x4f, 0x98, 0x44,
        ],
        sql: include_str!("../../migrations/0004_candidate_evaluations.sql"),
    },
    Migration {
        version: 5,
        name: "snapshot-retention-tombstones",
        checksum_sha256: [
            0x53, 0x29, 0x31, 0xf0, 0xe8, 0x70, 0x8b, 0x89, 0xf0, 0xca, 0xb2, 0x61, 0xf5, 0x48,
            0x42, 0x1e, 0x53, 0x9a, 0x4d, 0xfb, 0x06, 0x83, 0x5a, 0x68, 0xa7, 0xbc, 0x59, 0xda,
            0x38, 0x28, 0x18, 0x0d,
        ],
        sql: include_str!("../../migrations/0005_snapshot_retention_tombstones.sql"),
    },
    Migration {
        version: 6,
        name: "snapshot-review-pins",
        checksum_sha256: [
            0xdb, 0x33, 0xd1, 0xbc, 0x0d, 0x57, 0x5a, 0x64, 0x5b, 0x30, 0x0e, 0x16, 0x08, 0xdd,
            0x9a, 0xeb, 0x7d, 0xc0, 0xc1, 0x1c, 0xca, 0xe9, 0xa9, 0xed, 0xe9, 0x01, 0xfc, 0xc1,
            0x79, 0xe9, 0x2c, 0x20,
        ],
        sql: include_str!("../../migrations/0006_snapshot_review_pins.sql"),
    },
    Migration {
        version: 7,
        name: "snapshot-retention-lookup",
        checksum_sha256: [
            0x60, 0x5c, 0xb0, 0xfc, 0x5c, 0x69, 0x7f, 0xc8, 0x95, 0x95, 0xda, 0xc1, 0x8e, 0xf9,
            0xdd, 0xb0, 0x52, 0x8c, 0x07, 0x02, 0x2b, 0xf7, 0xac, 0xf7, 0x45, 0xb5, 0xd3, 0x98,
            0xf7, 0x45, 0xac, 0xf2,
        ],
        sql: include_str!("../../migrations/0007_snapshot_retention_lookup.sql"),
    },
    Migration {
        version: 8,
        name: "snapshot-temp-leases",
        checksum_sha256: [
            0x0f, 0xbb, 0x03, 0xd6, 0xa4, 0x0a, 0xaf, 0x53, 0x45, 0xe7, 0xa3, 0xb8, 0x19, 0xee,
            0x07, 0x51, 0xb5, 0xf4, 0x61, 0xe3, 0xa0, 0x35, 0xe2, 0x8d, 0xf8, 0x32, 0x9d, 0xb3,
            0x51, 0x90, 0x72, 0x42,
        ],
        sql: include_str!("../../migrations/0008_snapshot_temp_leases.sql"),
    },
    Migration {
        version: 9,
        name: "scan-process-claims",
        checksum_sha256: [
            0x90, 0x89, 0x13, 0x4c, 0x36, 0x29, 0x4f, 0x1d, 0x23, 0x7f, 0x4d, 0x97, 0xd5, 0xcc,
            0x5d, 0xae, 0xd2, 0x5a, 0x2c, 0x3b, 0xa1, 0x21, 0xf3, 0x9b, 0xcc, 0x02, 0x57, 0xea,
            0x89, 0xa2, 0xc6, 0x72,
        ],
        sql: include_str!("../../migrations/0009_scan_process_claims.sql"),
    },
    Migration {
        version: 10,
        name: "disk-pressure-policy-revisions",
        checksum_sha256: [
            0x33, 0xee, 0x70, 0x0c, 0xd8, 0xbc, 0x07, 0x40, 0x05, 0x64, 0x1f, 0xc6, 0x0c, 0xe7,
            0x81, 0xc9, 0x46, 0xd2, 0xdf, 0x5b, 0xee, 0xcc, 0x7f, 0xee, 0xf4, 0xdf, 0xce, 0x69,
            0xf8, 0x6d, 0xd7, 0x01,
        ],
        sql: include_str!("../../migrations/0010_disk_pressure_policy_revisions.sql"),
    },
    Migration {
        version: 11,
        name: "disk-pressure-episodes",
        checksum_sha256: [
            0xd4, 0x00, 0xff, 0x3e, 0x9a, 0x3f, 0x6e, 0x47, 0xa0, 0x8e, 0x3e, 0xa1, 0x4e, 0xdd,
            0x9d, 0xd5, 0x94, 0x69, 0x35, 0x9f, 0x7d, 0xb5, 0xe7, 0xb9, 0xef, 0x61, 0xc4, 0xdc,
            0x78, 0x3c, 0xe8, 0xf4,
        ],
        sql: include_str!("../../migrations/0011_disk_pressure_episodes.sql"),
    },
    Migration {
        version: 12,
        name: "trusted-rust-target-plan-claims",
        checksum_sha256: [
            0xe0, 0xc2, 0xae, 0xab, 0xcb, 0xe3, 0xe9, 0x54, 0xa7, 0x47, 0x49, 0x4a, 0xea, 0xe9,
            0x6f, 0x0f, 0x53, 0x8e, 0xdf, 0x65, 0x25, 0x12, 0xdc, 0x91, 0x12, 0x51, 0xad, 0x57,
            0xbf, 0xcc, 0x6d, 0x4a,
        ],
        sql: include_str!("../../migrations/0012_trusted_rust_target_plan_claims.sql"),
    },
];

const V1_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 24] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidates_by_scan_status"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_volume_time"),
    ("index", "schedules_by_next_run"),
    ("table", "ai_insights"),
    ("table", "candidates"),
    ("table", "cleanup_items"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "volumes"),
];

const V2_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 33] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "schedules_by_next_run"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "volumes"),
];

const V3_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 34] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "schedules_by_next_run"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "volumes"),
];

const V4_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 36] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidate_evaluations_by_status"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "schedules_by_next_run"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evaluations"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "volumes"),
];

const V5_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 41] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidate_evaluations_by_status"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "scans_snapshot_identity"),
    ("index", "schedules_by_next_run"),
    ("index", "snapshot_retention_tombstones_by_commit"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evaluations"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "snapshot_retention_tombstones"),
    ("table", "volumes"),
    ("trigger", "snapshot_retention_tombstones_delete_guard"),
    ("trigger", "snapshot_retention_tombstones_update_guard"),
];

const V6_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 45] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidate_evaluations_by_status"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "scans_snapshot_identity"),
    ("index", "schedules_by_next_run"),
    ("index", "snapshot_retention_tombstones_by_commit"),
    ("index", "snapshot_review_pins_by_expiration"),
    ("index", "snapshot_review_pins_by_scan_expiration"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evaluations"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "snapshot_retention_tombstones"),
    ("table", "snapshot_review_pins"),
    ("table", "volumes"),
    ("trigger", "snapshot_retention_tombstones_delete_guard"),
    ("trigger", "snapshot_retention_tombstones_update_guard"),
    ("trigger", "snapshot_review_pins_update_guard"),
];

const V7_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 46] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidate_evaluations_by_status"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_snapshot_path"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "scans_snapshot_identity"),
    ("index", "schedules_by_next_run"),
    ("index", "snapshot_retention_tombstones_by_commit"),
    ("index", "snapshot_review_pins_by_expiration"),
    ("index", "snapshot_review_pins_by_scan_expiration"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evaluations"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "snapshot_retention_tombstones"),
    ("table", "snapshot_review_pins"),
    ("table", "volumes"),
    ("trigger", "snapshot_retention_tombstones_delete_guard"),
    ("trigger", "snapshot_retention_tombstones_update_guard"),
    ("trigger", "snapshot_review_pins_update_guard"),
];

const V8_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 50] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidate_evaluations_by_status"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_snapshot_path"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "scans_snapshot_identity"),
    ("index", "schedules_by_next_run"),
    ("index", "snapshot_retention_tombstones_by_commit"),
    ("index", "snapshot_review_pins_by_expiration"),
    ("index", "snapshot_review_pins_by_scan_expiration"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evaluations"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "snapshot_retention_tombstones"),
    ("table", "snapshot_review_pins"),
    ("table", "snapshot_temp_leases"),
    ("table", "volumes"),
    ("trigger", "scans_succeeded_without_temp_lease_guard"),
    ("trigger", "snapshot_retention_tombstones_delete_guard"),
    ("trigger", "snapshot_retention_tombstones_update_guard"),
    ("trigger", "snapshot_review_pins_update_guard"),
    ("trigger", "snapshot_temp_leases_insert_guard"),
    ("trigger", "snapshot_temp_leases_update_guard"),
];

const V9_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 56] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidate_evaluations_by_status"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scan_process_claims_by_owner"),
    ("index", "scan_process_claims_by_time"),
    ("index", "scans_by_snapshot_path"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "scans_snapshot_identity"),
    ("index", "schedules_by_next_run"),
    ("index", "snapshot_retention_tombstones_by_commit"),
    ("index", "snapshot_review_pins_by_expiration"),
    ("index", "snapshot_review_pins_by_scan_expiration"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evaluations"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scan_process_claims"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "snapshot_retention_tombstones"),
    ("table", "snapshot_review_pins"),
    ("table", "snapshot_temp_leases"),
    ("table", "volumes"),
    ("trigger", "scan_process_claims_insert_guard"),
    ("trigger", "scan_process_claims_update_guard"),
    ("trigger", "scans_succeeded_without_temp_lease_guard"),
    ("trigger", "scans_terminal_with_process_claim_guard"),
    ("trigger", "snapshot_retention_tombstones_delete_guard"),
    ("trigger", "snapshot_retention_tombstones_update_guard"),
    ("trigger", "snapshot_review_pins_update_guard"),
    ("trigger", "snapshot_temp_leases_insert_guard"),
    ("trigger", "snapshot_temp_leases_update_guard"),
];

// V10 changes one table definition but does not add or remove schema objects.
const V10_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 56] = V9_EXPECTED_SCHEMA_OBJECTS;

const V11_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 59] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidate_evaluations_by_status"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_pressure_episodes_by_volume_time"),
    ("index", "disk_pressure_episodes_open_by_volume"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scan_process_claims_by_owner"),
    ("index", "scan_process_claims_by_time"),
    ("index", "scans_by_snapshot_path"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "scans_snapshot_identity"),
    ("index", "schedules_by_next_run"),
    ("index", "snapshot_retention_tombstones_by_commit"),
    ("index", "snapshot_review_pins_by_expiration"),
    ("index", "snapshot_review_pins_by_scan_expiration"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evaluations"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_pressure_episodes"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scan_process_claims"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "snapshot_retention_tombstones"),
    ("table", "snapshot_review_pins"),
    ("table", "snapshot_temp_leases"),
    ("table", "volumes"),
    ("trigger", "scan_process_claims_insert_guard"),
    ("trigger", "scan_process_claims_update_guard"),
    ("trigger", "scans_succeeded_without_temp_lease_guard"),
    ("trigger", "scans_terminal_with_process_claim_guard"),
    ("trigger", "snapshot_retention_tombstones_delete_guard"),
    ("trigger", "snapshot_retention_tombstones_update_guard"),
    ("trigger", "snapshot_review_pins_update_guard"),
    ("trigger", "snapshot_temp_leases_insert_guard"),
    ("trigger", "snapshot_temp_leases_update_guard"),
];

const V12_EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 60] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidate_evaluations_by_status"),
    ("index", "candidates_by_scan_status"),
    ("index", "candidates_by_scan_time"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_recovery"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_pressure_episodes_by_volume_time"),
    ("index", "disk_pressure_episodes_open_by_volume"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scan_process_claims_by_owner"),
    ("index", "scan_process_claims_by_time"),
    ("index", "scans_by_snapshot_path"),
    ("index", "scans_by_started"),
    ("index", "scans_by_volume_time"),
    ("index", "scans_snapshot_identity"),
    ("index", "schedules_by_next_run"),
    ("index", "snapshot_retention_tombstones_by_commit"),
    ("index", "snapshot_review_pins_by_expiration"),
    ("index", "snapshot_review_pins_by_scan_expiration"),
    ("table", "ai_insights"),
    ("table", "candidate_blockers"),
    ("table", "candidate_evaluations"),
    ("table", "candidate_evidence"),
    ("table", "candidate_paths"),
    ("table", "candidate_plan_claims"),
    ("table", "candidates"),
    ("table", "cleanup_item_evidence"),
    ("table", "cleanup_item_paths"),
    ("table", "cleanup_items"),
    ("table", "cleanup_plan_warnings"),
    ("table", "cleanup_sessions"),
    ("table", "disk_pressure_episodes"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scan_process_claims"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "snapshot_retention_tombstones"),
    ("table", "snapshot_review_pins"),
    ("table", "snapshot_temp_leases"),
    ("table", "trusted_rust_target_plan_claims"),
    ("table", "volumes"),
    ("trigger", "scan_process_claims_insert_guard"),
    ("trigger", "scan_process_claims_update_guard"),
    ("trigger", "scans_succeeded_without_temp_lease_guard"),
    ("trigger", "scans_terminal_with_process_claim_guard"),
    ("trigger", "snapshot_retention_tombstones_delete_guard"),
    ("trigger", "snapshot_retention_tombstones_update_guard"),
    ("trigger", "snapshot_review_pins_update_guard"),
    ("trigger", "snapshot_temp_leases_insert_guard"),
    ("trigger", "snapshot_temp_leases_update_guard"),
];

// Canonical sqlite_schema representation produced by v1. A mismatch rejects
// supported databases rather than guessing about drift.
const V1_SCHEMA_FINGERPRINT: [u8; 32] = [
    0xd3, 0x24, 0xcb, 0x24, 0x32, 0xa3, 0xaa, 0x35, 0xc6, 0x82, 0x01, 0x7d, 0x4f, 0x87, 0xac, 0x5e,
    0xae, 0xc1, 0xb1, 0xef, 0x2b, 0xe4, 0x2f, 0x0a, 0x3a, 0xfd, 0x58, 0xf8, 0x94, 0x08, 0x6b, 0x12,
];

// Canonical sqlite_schema representation produced by the complete v2 chain.
const V2_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x80, 0xb2, 0x54, 0x60, 0x81, 0x1e, 0x96, 0x19, 0xd1, 0x38, 0x1d, 0x75, 0x89, 0x6c, 0x98, 0xae,
    0x18, 0x87, 0x36, 0x9a, 0xdb, 0x0d, 0xeb, 0x8c, 0xa7, 0x91, 0x6d, 0x07, 0xa3, 0xb8, 0x44, 0x83,
];

// Canonical sqlite_schema representation produced by the complete v3 chain.
const V3_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x44, 0x7c, 0xdb, 0xf5, 0x33, 0xbb, 0xdc, 0x51, 0x0b, 0xc9, 0x7f, 0x19, 0x77, 0x73, 0x37, 0x6e,
    0x66, 0xf3, 0x49, 0x36, 0x0e, 0xa9, 0x82, 0xfd, 0x97, 0x8c, 0xb8, 0xea, 0xdc, 0xc9, 0x06, 0x7c,
];

// Canonical sqlite_schema representation produced by the complete v4 chain.
const V4_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x40, 0x08, 0x26, 0x87, 0x4a, 0x1b, 0x11, 0x67, 0x31, 0x53, 0xae, 0xfb, 0xea, 0x87, 0x25, 0x60,
    0x8e, 0xdf, 0xd0, 0xf9, 0x38, 0xb0, 0x03, 0x07, 0x86, 0xa1, 0x24, 0xef, 0x5e, 0x9c, 0xee, 0x05,
];

// Canonical sqlite_schema representation produced by the complete v5 chain.
const V5_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x79, 0xb8, 0x22, 0x61, 0x3a, 0x33, 0x1d, 0xb0, 0xab, 0xb5, 0x07, 0xe6, 0x3a, 0x3c, 0xe4, 0x0e,
    0x69, 0x80, 0xb6, 0x5b, 0x3e, 0xde, 0x64, 0xf2, 0xbe, 0xe0, 0xd9, 0x26, 0x49, 0x7c, 0xf4, 0x43,
];

// Canonical sqlite_schema representation produced by the complete v6 chain.
const V6_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x17, 0x78, 0x52, 0x2e, 0x4f, 0xc1, 0xb8, 0xef, 0xa3, 0x8a, 0xee, 0x12, 0x02, 0x1a, 0xaf, 0x25,
    0x77, 0xa5, 0x75, 0x0c, 0x02, 0xfc, 0xb4, 0x5f, 0xb3, 0x87, 0x08, 0x23, 0x5b, 0x19, 0x98, 0x34,
];

// Canonical sqlite_schema representation produced by the complete v7 chain.
const V7_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x9d, 0x37, 0x9d, 0x27, 0x73, 0xdb, 0x12, 0x0f, 0x20, 0x99, 0x6a, 0x85, 0xd1, 0x7a, 0x45, 0x8d,
    0x8c, 0xf8, 0x02, 0x87, 0x79, 0x87, 0xd1, 0x09, 0x5e, 0x21, 0x47, 0xee, 0xc6, 0x18, 0x3d, 0x23,
];

// Canonical sqlite_schema representation produced by the complete v8 chain.
const V8_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x8a, 0x1d, 0xba, 0x6d, 0x1c, 0x34, 0xe1, 0x41, 0x44, 0x4e, 0x09, 0x0b, 0x4e, 0xc4, 0x51, 0x61,
    0xad, 0x12, 0x4a, 0xcd, 0xb3, 0xeb, 0x73, 0x6e, 0x30, 0xf6, 0x94, 0xd1, 0xc0, 0x20, 0x2e, 0x8b,
];

// Canonical sqlite_schema representation produced by the complete v9 chain.
const V9_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x90, 0x7a, 0x6b, 0x35, 0x60, 0x59, 0xf8, 0x27, 0x66, 0xf4, 0x79, 0x08, 0x76, 0xe9, 0x3e, 0xb2,
    0xe5, 0xf6, 0xf4, 0x47, 0x13, 0xb7, 0x3d, 0x2c, 0x34, 0x80, 0x26, 0xfb, 0xdc, 0xae, 0x8d, 0x0f,
];

// Canonical sqlite_schema representation produced by the complete v10 chain.
const V10_SCHEMA_FINGERPRINT: [u8; 32] = [
    0x94, 0x79, 0x70, 0x16, 0xdf, 0x91, 0x60, 0xa2, 0x81, 0xce, 0x27, 0x14, 0x84, 0x28, 0x20, 0xcc,
    0x30, 0x65, 0x1d, 0x98, 0x15, 0x60, 0x9d, 0x30, 0x5f, 0x42, 0x42, 0x09, 0x6b, 0x76, 0xdf, 0x0b,
];

// Canonical schema fingerprint for the complete v11 chain.
const V11_SCHEMA_FINGERPRINT: [u8; 32] = [
    0xba, 0x1a, 0xb6, 0xab, 0x95, 0x89, 0x22, 0x61, 0xe4, 0xc2, 0xac, 0xc3, 0xf0, 0x18, 0xb6, 0x03,
    0x26, 0xfa, 0x37, 0xf2, 0xab, 0xbd, 0x57, 0xec, 0xff, 0xbf, 0xa4, 0x6c, 0x5f, 0x6e, 0x92, 0x15,
];

// Canonical schema fingerprint for the complete v12 chain.
const V12_SCHEMA_FINGERPRINT: [u8; 32] = [
    0xfb, 0xba, 0xc0, 0x37, 0x7d, 0xab, 0x42, 0x6a, 0x84, 0x6f, 0x56, 0xd7, 0x7e, 0x2a, 0xff, 0xdf,
    0x08, 0xe9, 0xcc, 0xc8, 0x4f, 0x67, 0x3f, 0x6f, 0x61, 0x8a, 0xbc, 0x0d, 0x33, 0x2e, 0x1d, 0xc7,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SchemaState {
    Empty,
    Older { found: u32 },
    Current,
    Newer { found: u32 },
}

pub(crate) fn validate_compiled_migrations() -> Result<(), DatabaseOpenError> {
    if MIGRATIONS.is_empty()
        || MIGRATIONS.len() > MAX_LEDGER_ROWS as usize
        || MIGRATIONS.last().map(|migration| migration.version) != Some(DATABASE_SCHEMA_VERSION)
    {
        return Err(migration_error());
    }
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        let Some(compiled_checksum) = canonical_sha256(migration.sql) else {
            return Err(migration_error());
        };
        if migration.version != (index + 1) as u32
            || migration.name.is_empty()
            || migration.name.len() > 128
            || migration.name.chars().any(char::is_control)
            || MIGRATIONS[..index]
                .iter()
                .any(|previous| previous.name == migration.name)
            || migration.sql.as_bytes().contains(&0)
            || compiled_checksum != migration.checksum_sha256
        {
            return Err(migration_error());
        }
    }
    Ok(())
}

pub(crate) fn inspect_schema(connection: &Connection) -> Result<SchemaState, DatabaseOpenError> {
    inspect_schema_with_budget(
        connection,
        InspectionDepth::FullIntegrity,
        STARTUP_INSPECTION_BUDGET,
    )
}

/// Revalidates compatibility facts used for presentation without rescanning
/// every database page or every foreign-key row on each status render.
pub(crate) fn inspect_schema_for_status(
    connection: &Connection,
) -> Result<SchemaState, DatabaseOpenError> {
    inspect_schema_with_budget(
        connection,
        InspectionDepth::Compatibility,
        STATUS_INSPECTION_BUDGET,
    )
}

fn inspect_schema_with_budget(
    connection: &Connection,
    depth: InspectionDepth,
    budget: InspectionBudget,
) -> Result<SchemaState, DatabaseOpenError> {
    run_with_budget(connection, budget, |clock| {
        inspect_schema_inner(connection, depth, clock)
    })
}

fn inspect_schema_inner(
    connection: &Connection,
    depth: InspectionDepth,
    clock: InspectionClock,
) -> Result<SchemaState, DatabaseOpenError> {
    clock.checkpoint()?;
    if matches!(depth, InspectionDepth::FullIntegrity) {
        let quick_check: String = connection
            .pragma_query_value(None, "quick_check", |row| row.get(0))
            .map_err(map_inspection_error)?;
        if quick_check != "ok" {
            return Err(corrupt_error());
        }
    }

    let application_id = pragma_u32(connection, "application_id")?;
    let user_version = pragma_u32(connection, "user_version")?;
    let object_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
            [],
            |row| row.get(0),
        )
        .map_err(map_inspection_error)?;
    if !(0..=256).contains(&object_count) {
        return Err(corrupt_error());
    }

    if application_id == 0 && user_version == 0 && object_count == 0 {
        return Ok(SchemaState::Empty);
    }
    if application_id != DUX_APPLICATION_ID {
        return Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::UnrecognizedDatabase,
        ));
    }
    if user_version == 0 || object_count == 0 {
        return Err(corrupt_error());
    }

    let ledger_type: Option<String> = connection
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_inspection_error)?;
    if ledger_type.as_deref() != Some("table") {
        return Err(corrupt_error());
    }

    let ledger_count: i64 = connection
        .query_row("SELECT count(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .map_err(map_inspection_error)?;
    if ledger_count <= 0
        || ledger_count > MAX_LEDGER_ROWS
        || u32::try_from(ledger_count).ok() != Some(user_version)
    {
        return Err(corrupt_error());
    }

    let mut statement = connection
        .prepare(
            "SELECT version, typeof(name), length(CAST(name AS BLOB)), name, \
                    typeof(checksum_sha256), length(checksum_sha256), checksum_sha256, \
                    applied_at_unix_ms \
             FROM schema_migrations ORDER BY version",
        )
        .map_err(map_inspection_error)?;
    let mut rows = statement.query([]).map_err(map_inspection_error)?;
    let mut expected_version = 1_u32;
    while let Some(row) = rows.next().map_err(map_inspection_error)? {
        clock.checkpoint()?;
        let version_i64: i64 = row.get(0).map_err(map_inspection_error)?;
        let version = u32::try_from(version_i64).map_err(|_| corrupt_error())?;
        let name_type: String = row.get(1).map_err(map_inspection_error)?;
        let name_length: i64 = row.get(2).map_err(map_inspection_error)?;
        let checksum_type: String = row.get(4).map_err(map_inspection_error)?;
        let checksum_length: i64 = row.get(5).map_err(map_inspection_error)?;
        let applied_at: i64 = row.get(7).map_err(map_inspection_error)?;
        if version != expected_version
            || name_type != "text"
            || !(1..=128).contains(&name_length)
            || checksum_type != "blob"
            || checksum_length != 32
            || applied_at < 0
        {
            return Err(corrupt_error());
        }
        let name: String = row.get(3).map_err(map_inspection_error)?;
        let checksum: Vec<u8> = row.get(6).map_err(map_inspection_error)?;
        if name.len() != name_length as usize
            || name.chars().any(char::is_control)
            || checksum.len() != 32
        {
            return Err(corrupt_error());
        }
        if let Some(compiled) = MIGRATIONS.get((version - 1) as usize)
            && (name != compiled.name || checksum.as_slice() != compiled.checksum_sha256)
        {
            return Err(corrupt_error());
        }
        expected_version = expected_version.checked_add(1).ok_or_else(corrupt_error)?;
    }
    if expected_version.checked_sub(1) != Some(user_version) {
        return Err(corrupt_error());
    }

    if user_version > DATABASE_SCHEMA_VERSION {
        return Ok(SchemaState::Newer {
            found: user_version,
        });
    }
    if matches!(depth, InspectionDepth::FullIntegrity) {
        validate_foreign_keys(connection)?;
    }
    validate_supported_schema(connection, user_version, clock)?;
    if user_version < DATABASE_SCHEMA_VERSION {
        return Ok(SchemaState::Older {
            found: user_version,
        });
    }
    Ok(SchemaState::Current)
}

pub(crate) fn apply_pending_migrations(
    connection: &mut Connection,
    applied_at_unix_ms: i64,
) -> Result<(), DatabaseOpenError> {
    validate_compiled_migrations()?;
    run_with_budget(connection, MIGRATION_BUDGET, |clock| {
        // The caller owns `&mut Connection`, so the runtime nested-transaction
        // check here retains the same exclusivity while letting the progress
        // budget cover BEGIN, migration statements, validation, and COMMIT.
        let transaction =
            Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
                .map_err(map_migration_error)?;
        match inspect_schema_inner(&transaction, InspectionDepth::FullIntegrity, clock)? {
            SchemaState::Empty => apply_chain(&transaction, &MIGRATIONS, 0, applied_at_unix_ms)?,
            SchemaState::Older { found } => {
                apply_chain(&transaction, &MIGRATIONS, found, applied_at_unix_ms)?;
            }
            SchemaState::Current => {}
            SchemaState::Newer { .. } => return Err(corrupt_error()),
        }
        if inspect_schema_inner(&transaction, InspectionDepth::FullIntegrity, clock)?
            != SchemaState::Current
        {
            return Err(corrupt_error());
        }
        transaction.commit().map_err(map_migration_error)
    })
}

fn apply_chain(
    transaction: &Transaction<'_>,
    migrations: &[Migration],
    applied_version: u32,
    applied_at_unix_ms: i64,
) -> Result<(), DatabaseOpenError> {
    for migration in migrations
        .iter()
        .filter(|migration| migration.version > applied_version)
    {
        execute_migration_sql(transaction, migration.sql)?;
        transaction
            .execute(
                "INSERT INTO schema_migrations \
                 (version, name, checksum_sha256, applied_at_unix_ms) VALUES (?1, ?2, ?3, ?4)",
                params![
                    i64::from(migration.version),
                    migration.name,
                    migration.checksum_sha256.as_slice(),
                    applied_at_unix_ms,
                ],
            )
            .map_err(map_migration_error)?;
        transaction
            .pragma_update(None, "application_id", DUX_APPLICATION_ID)
            .map_err(map_migration_error)?;
        transaction
            .pragma_update(None, "user_version", migration.version)
            .map_err(map_migration_error)?;
    }
    Ok(())
}

fn execute_migration_sql(
    transaction: &Transaction<'_>,
    sql: &str,
) -> Result<(), DatabaseOpenError> {
    // The outer Rust transaction owns atomicity and ledger ordering. Embedded
    // transaction or savepoint control could commit schema changes before the
    // ledger insert, so migration batches never receive that authority.
    let mut authorizer = MigrationAuthorizerGuard::install(transaction)?;
    let result = transaction.execute_batch(sql).map_err(map_migration_error);
    authorizer.remove()?;
    result
}

fn validate_supported_schema(
    connection: &Connection,
    version: u32,
    clock: InspectionClock,
) -> Result<(), DatabaseOpenError> {
    match version {
        1 => validate_schema(
            connection,
            clock,
            &V1_EXPECTED_SCHEMA_OBJECTS,
            V1_SCHEMA_FINGERPRINT,
        ),
        2 => validate_schema(
            connection,
            clock,
            &V2_EXPECTED_SCHEMA_OBJECTS,
            V2_SCHEMA_FINGERPRINT,
        ),
        3 => validate_schema(
            connection,
            clock,
            &V3_EXPECTED_SCHEMA_OBJECTS,
            V3_SCHEMA_FINGERPRINT,
        ),
        4 => validate_schema(
            connection,
            clock,
            &V4_EXPECTED_SCHEMA_OBJECTS,
            V4_SCHEMA_FINGERPRINT,
        ),
        5 => validate_schema(
            connection,
            clock,
            &V5_EXPECTED_SCHEMA_OBJECTS,
            V5_SCHEMA_FINGERPRINT,
        ),
        6 => validate_schema(
            connection,
            clock,
            &V6_EXPECTED_SCHEMA_OBJECTS,
            V6_SCHEMA_FINGERPRINT,
        ),
        7 => validate_schema(
            connection,
            clock,
            &V7_EXPECTED_SCHEMA_OBJECTS,
            V7_SCHEMA_FINGERPRINT,
        ),
        8 => validate_schema(
            connection,
            clock,
            &V8_EXPECTED_SCHEMA_OBJECTS,
            V8_SCHEMA_FINGERPRINT,
        ),
        9 => validate_schema(
            connection,
            clock,
            &V9_EXPECTED_SCHEMA_OBJECTS,
            V9_SCHEMA_FINGERPRINT,
        ),
        10 => validate_schema(
            connection,
            clock,
            &V10_EXPECTED_SCHEMA_OBJECTS,
            V10_SCHEMA_FINGERPRINT,
        ),
        11 => validate_schema(
            connection,
            clock,
            &V11_EXPECTED_SCHEMA_OBJECTS,
            V11_SCHEMA_FINGERPRINT,
        ),
        12 => validate_schema(
            connection,
            clock,
            &V12_EXPECTED_SCHEMA_OBJECTS,
            V12_SCHEMA_FINGERPRINT,
        ),
        _ => Err(corrupt_error()),
    }
}

fn validate_schema(
    connection: &Connection,
    clock: InspectionClock,
    expected_objects: &[(&str, &str)],
    expected_fingerprint: [u8; 32],
) -> Result<(), DatabaseOpenError> {
    let mut statement = connection
        .prepare(
            "SELECT typeof(type), length(CAST(type AS BLOB)), type, \
                    typeof(name), length(CAST(name AS BLOB)), name \
             FROM sqlite_schema \
             WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name",
        )
        .map_err(map_inspection_error)?;
    let mut rows = statement.query([]).map_err(map_inspection_error)?;
    let mut actual = Vec::with_capacity(expected_objects.len());
    while let Some(row) = rows.next().map_err(map_inspection_error)? {
        clock.checkpoint()?;
        let type_storage: String = row.get(0).map_err(map_inspection_error)?;
        let type_length: i64 = row.get(1).map_err(map_inspection_error)?;
        let name_storage: String = row.get(3).map_err(map_inspection_error)?;
        let name_length: i64 = row.get(4).map_err(map_inspection_error)?;
        if type_storage != "text"
            || !(1..=MAX_SCHEMA_TYPE_BYTES).contains(&type_length)
            || name_storage != "text"
            || !(1..=MAX_SCHEMA_NAME_BYTES).contains(&name_length)
            || actual.len() >= expected_objects.len()
        {
            return Err(corrupt_error());
        }
        let object_type: String = row.get(2).map_err(map_inspection_error)?;
        let name: String = row.get(5).map_err(map_inspection_error)?;
        if object_type.len() != type_length as usize || name.len() != name_length as usize {
            return Err(corrupt_error());
        }
        actual.push((object_type, name));
    }
    if actual.len() != expected_objects.len()
        || actual.iter().zip(expected_objects).any(
            |((actual_type, actual_name), (expected_type, expected_name))| {
                actual_type != expected_type || actual_name != expected_name
            },
        )
    {
        return Err(corrupt_error());
    }

    let fingerprint = schema_fingerprint_with_clock(connection, clock)?;
    if fingerprint != expected_fingerprint {
        return Err(corrupt_error());
    }
    Ok(())
}

fn validate_foreign_keys(connection: &Connection) -> Result<(), DatabaseOpenError> {
    let violation: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_inspection_error)?;
    if violation.is_some() {
        return Err(corrupt_error());
    }
    Ok(())
}

fn schema_fingerprint_with_clock(
    connection: &Connection,
    clock: InspectionClock,
) -> Result<[u8; 32], DatabaseOpenError> {
    let mut statement = connection
        .prepare(
            "SELECT typeof(type), length(CAST(type AS BLOB)), type, \
                    typeof(name), length(CAST(name AS BLOB)), name, \
                    typeof(sql), length(CAST(sql AS BLOB)), sql \
             FROM sqlite_schema \
             WHERE name NOT GLOB 'sqlite_*' AND sql IS NOT NULL ORDER BY type, name",
        )
        .map_err(map_inspection_error)?;
    let mut rows = statement.query([]).map_err(map_inspection_error)?;
    let mut hasher = Sha256::new();
    let mut count = 0_u32;
    let mut total_bytes = 0_usize;
    while let Some(row) = rows.next().map_err(map_inspection_error)? {
        clock.checkpoint()?;
        let type_storage: String = row.get(0).map_err(map_inspection_error)?;
        let type_length: i64 = row.get(1).map_err(map_inspection_error)?;
        let name_storage: String = row.get(3).map_err(map_inspection_error)?;
        let name_length: i64 = row.get(4).map_err(map_inspection_error)?;
        let sql_storage: String = row.get(6).map_err(map_inspection_error)?;
        let sql_length: i64 = row.get(7).map_err(map_inspection_error)?;
        if type_storage != "text"
            || !(1..=MAX_SCHEMA_TYPE_BYTES).contains(&type_length)
            || name_storage != "text"
            || !(1..=MAX_SCHEMA_NAME_BYTES).contains(&name_length)
            || sql_storage != "text"
            || !(1..=MAX_SCHEMA_SQL_BYTES).contains(&sql_length)
            || count >= 64
        {
            return Err(corrupt_error());
        }
        let row_bytes =
            usize::try_from(type_length + name_length + sql_length).map_err(|_| corrupt_error())?;
        total_bytes = total_bytes
            .checked_add(row_bytes)
            .filter(|total| *total <= MAX_SCHEMA_FINGERPRINT_BYTES)
            .ok_or_else(corrupt_error)?;
        let object_type: String = row.get(2).map_err(map_inspection_error)?;
        let name: String = row.get(5).map_err(map_inspection_error)?;
        let sql: String = row.get(8).map_err(map_inspection_error)?;
        for value in [&object_type, &name, &sql] {
            clock.checkpoint()?;
            let canonical = canonical_lf(value).ok_or_else(corrupt_error)?;
            let length = u32::try_from(canonical.len()).map_err(|_| corrupt_error())?;
            hasher.update(length.to_le_bytes());
            hasher.update(canonical.as_bytes());
        }
        count = count.checked_add(1).ok_or_else(corrupt_error)?;
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
pub(crate) fn schema_fingerprint(connection: &Connection) -> Result<[u8; 32], DatabaseOpenError> {
    schema_fingerprint_with_clock(
        connection,
        InspectionClock {
            started_at: Instant::now(),
            max_elapsed: Duration::from_secs(60),
        },
    )
}

fn canonical_sha256(value: &str) -> Option<[u8; 32]> {
    let canonical = canonical_lf(value)?;
    Some(Sha256::digest(canonical.as_bytes()).into())
}

fn canonical_lf(value: &str) -> Option<Cow<'_, str>> {
    if !value.contains('\r') {
        return Some(Cow::Borrowed(value));
    }
    let normalized = value.replace("\r\n", "\n");
    (!normalized.contains('\r')).then_some(Cow::Owned(normalized))
}

fn pragma_u32(connection: &Connection, name: &str) -> Result<u32, DatabaseOpenError> {
    let value: i64 = connection
        .pragma_query_value(None, name, |row| row.get(0))
        .map_err(map_inspection_error)?;
    u32::try_from(value).map_err(|_| corrupt_error())
}

fn run_with_budget<T>(
    connection: &Connection,
    budget: InspectionBudget,
    operation: impl FnOnce(InspectionClock) -> Result<T, DatabaseOpenError>,
) -> Result<T, DatabaseOpenError> {
    debug_assert!(budget.max_callbacks > 0);
    let started_at = Instant::now();
    let clock = InspectionClock {
        started_at,
        max_elapsed: budget.max_elapsed,
    };
    let mut callbacks = 0_u64;
    let mut interrupted = false;
    // SQLite checks the deadline at this VM-instruction cadence. It bounds
    // compute-heavy statements but cannot preempt one blocking filesystem
    // operation between callbacks.
    connection
        .progress_handler(
            PROGRESS_OP_INTERVAL,
            Some(move || {
                if interrupted {
                    return true;
                }
                callbacks = callbacks.saturating_add(1);
                interrupted =
                    callbacks >= budget.max_callbacks || started_at.elapsed() >= budget.max_elapsed;
                interrupted
            }),
        )
        .map_err(map_budget_configuration_error)?;
    let mut guard = ProgressHandlerGuard {
        connection,
        installed: true,
    };

    let result = operation(clock);
    guard.remove().map_err(map_budget_configuration_error)?;
    let value = result?;
    clock.checkpoint()?;
    Ok(value)
}

fn map_inspection_error(error: rusqlite::Error) -> DatabaseOpenError {
    use rusqlite::ErrorCode;
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            DatabaseOpenError::new(DatabaseOpenErrorKind::Busy)
        }
        Some(ErrorCode::OperationInterrupted) => {
            DatabaseOpenError::new(DatabaseOpenErrorKind::InspectionLimitExceeded)
        }
        _ => corrupt_error(),
    }
}

fn map_migration_error(error: rusqlite::Error) -> DatabaseOpenError {
    use rusqlite::ErrorCode;
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            DatabaseOpenError::new(DatabaseOpenErrorKind::Busy)
        }
        Some(ErrorCode::OperationInterrupted) => {
            DatabaseOpenError::new(DatabaseOpenErrorKind::InspectionLimitExceeded)
        }
        _ => migration_error(),
    }
}

fn map_budget_configuration_error(_: rusqlite::Error) -> DatabaseOpenError {
    DatabaseOpenError::new(DatabaseOpenErrorKind::DatabaseUnavailable)
}

fn corrupt_error() -> DatabaseOpenError {
    DatabaseOpenError::new(DatabaseOpenErrorKind::CorruptDatabase)
}

fn migration_error() -> DatabaseOpenError {
    DatabaseOpenError::new(DatabaseOpenErrorKind::MigrationFailed)
}

use rusqlite::OptionalExtension;

#[cfg(test)]
pub(super) fn test_migrations() -> &'static [Migration] {
    &MIGRATIONS
}

#[cfg(test)]
pub(super) const fn test_v1_schema_fingerprint() -> [u8; 32] {
    V1_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v2_schema_fingerprint() -> [u8; 32] {
    V2_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v3_schema_fingerprint() -> [u8; 32] {
    V3_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v4_schema_fingerprint() -> [u8; 32] {
    V4_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v5_schema_fingerprint() -> [u8; 32] {
    V5_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v6_schema_fingerprint() -> [u8; 32] {
    V6_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v7_schema_fingerprint() -> [u8; 32] {
    V7_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v8_schema_fingerprint() -> [u8; 32] {
    V8_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v9_schema_fingerprint() -> [u8; 32] {
    V9_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v11_schema_fingerprint() -> [u8; 32] {
    V11_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) const fn test_v12_schema_fingerprint() -> [u8; 32] {
    V12_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) fn inspect_schema_with_test_budget(
    connection: &Connection,
    max_callbacks: u64,
    max_elapsed: Duration,
) -> Result<SchemaState, DatabaseOpenError> {
    inspect_schema_with_budget(
        connection,
        InspectionDepth::FullIntegrity,
        InspectionBudget {
            max_callbacks,
            max_elapsed,
        },
    )
}

#[cfg(test)]
pub(super) fn panic_with_test_budget(connection: &Connection) {
    let _: Result<(), DatabaseOpenError> = run_with_budget(
        connection,
        InspectionBudget {
            max_callbacks: 1,
            max_elapsed: Duration::ZERO,
        },
        |_| panic!("test panic while a progress handler is installed"),
    );
}

#[cfg(test)]
pub(super) fn apply_test_chain(
    transaction: &Transaction<'_>,
    migrations: &[Migration],
    applied_at_unix_ms: i64,
) -> Result<(), DatabaseOpenError> {
    apply_chain(transaction, migrations, 0, applied_at_unix_ms)
}

#[cfg(test)]
pub(super) fn apply_test_upgrade_chain(
    transaction: &Transaction<'_>,
    migrations: &[Migration],
    applied_version: u32,
    applied_at_unix_ms: i64,
) -> Result<(), DatabaseOpenError> {
    apply_chain(transaction, migrations, applied_version, applied_at_unix_ms)
}
