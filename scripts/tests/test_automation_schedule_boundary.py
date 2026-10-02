#!/usr/bin/env python3

from pathlib import Path
import json
import re
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
CORE_DOMAIN = REPO_ROOT / "dux-core/src/domain/automation_schedule.rs"
CORE_AUTHORING_CATALOG = (
    REPO_ROOT / "dux-core/src/domain/automation_authoring_catalog.rs"
)
CORE_ELIGIBILITY = REPO_ROOT / "dux-core/src/domain/automation_eligibility.rs"
CORE_ENGINE = REPO_ROOT / "dux-core/src/engine/automation.rs"
CORE_RUNTIME_ENGINE = REPO_ROOT / "dux-core/src/engine/automation_runtime.rs"
CORE_RUNTIME_PERSISTENCE = REPO_ROOT / "dux-core/src/persistence/automation_runtime.rs"
CORE_REGISTRY = REPO_ROOT / "dux-core/src/engine/registry.rs"
CORE_LIB = REPO_ROOT / "dux-core/src/lib.rs"
CORE_PERSISTENCE_STATUS = REPO_ROOT / "dux-core/src/persistence/status.rs"
CORE_PERSISTENCE_STORAGE = REPO_ROOT / "dux-core/src/persistence/storage.rs"
CORE_CLEANUP_LEASE = (
    REPO_ROOT / "dux-core/src/persistence/cleanup_journal/lease.rs"
)
CORE_CLEANUP_EXECUTOR = REPO_ROOT / "dux-core/src/cleanup/executor.rs"
CORE_EXACT_PATH_REVIEW = REPO_ROOT / "dux-core/src/planner/exact_path_review.rs"
CORE_PERSISTENCE_STORE = REPO_ROOT / "dux-core/src/persistence/store.rs"
CORE_STORE = REPO_ROOT / "dux-core/src/persistence/automation_schedule.rs"
FFI = REPO_ROOT / "dux-ffi/src/lib.rs"
MIGRATION_V20 = REPO_ROOT / "dux-core/migrations/0020_automation_schedule_drafts.sql"
MIGRATION_V21 = REPO_ROOT / "dux-core/migrations/0021_automation_schedule_activation.sql"
MIGRATION_V22 = (
    REPO_ROOT / "dux-core/migrations/0022_automation_schedule_authoring_binding.sql"
)
MIGRATION_V23 = REPO_ROOT / "dux-core/migrations/0023_cleanup_admission_protocol.sql"
CATALOG = REPO_ROOT / "dux-core/catalogs/candidate-rules-v1.json"
MAINTENANCE_SCHEDULER = (
    REPO_ROOT / "dux-macos/Dux/Services/MaintenanceScheduler.swift"
)
ADR = REPO_ROOT / "docs/adr/0014-automation-clock-wake-and-missed-run-semantics.md"
ACTIVATION_ADR = (
    REPO_ROOT / "docs/adr/0015-automation-activation-and-utc-recurrence.md"
)
AUTHORING_CATALOG_ADR = (
    REPO_ROOT / "docs/adr/0016-automation-category-scope-membership-consent.md"
)
SECURITY_REVIEW = (
    REPO_ROOT / "docs/security-reviews/m8-automation-scheduler-wake.md"
)
ACTIVATION_SECURITY_REVIEW = (
    REPO_ROOT / "docs/security-reviews/m8-automation-activation-controls.md"
)
AUTHORING_SECURITY_REVIEW = (
    REPO_ROOT
    / "docs/security-reviews/m8-automation-schedule-authoring-controls.md"
)
AUTHORING_CATALOG_SECURITY_REVIEW = (
    REPO_ROOT
    / "docs/security-reviews/m8-automation-selectable-scope-authoring.md"
)
CORE_RUNTIME_SECURITY_REVIEW = (
    REPO_ROOT
    / "docs/security-reviews/m8-automation-core-runtime-evidence.md"
)
SCAN_ADMISSION_SECURITY_REVIEW = (
    REPO_ROOT
    / "docs/security-reviews/m8-automation-scan-admission-witness.md"
)
CLEANUP_ADMISSION_SECURITY_REVIEW = (
    REPO_ROOT
    / "docs/security-reviews/m8-automation-cleanup-admission-protocol.md"
)
SECURITY_DESIGN = REPO_ROOT / "SECURITY_DESIGN.md"
ROADMAP = REPO_ROOT / "ROADMAP.md"
CHANGELOG = REPO_ROOT / "CHANGELOG.md"
APP_RUNTIME = REPO_ROOT / "dux-macos/Dux/App/AppRuntime.swift"
NATIVE_AUTOMATION_MODEL = (
    REPO_ROOT / "dux-macos/Dux/Models/AutomationSchedule.swift"
)
NATIVE_AUTOMATION_SETTINGS = (
    REPO_ROOT / "dux-macos/Dux/App/AutomationScheduleSettingsModel.swift"
)
NATIVE_ENGINE_SERVICE = REPO_ROOT / "dux-macos/Dux/Services/EngineService.swift"
NATIVE_GENERATED_FFI = REPO_ROOT / "dux-macos/Dux/Generated/DuxFFI.swift"
NATIVE_AUTOMATION_VIEW = (
    REPO_ROOT / "dux-macos/Dux/Views/AutomationScheduleSettingsView.swift"
)

AUTOMATION_TIMING_NAME = re.compile(
    r"automation.*(?:scheduler|scheduling|timing|clock|wake|deadline)"
    r"|(?:scheduler|scheduling|timing|clock|wake|deadline).*automation",
    re.IGNORECASE,
)
AUTOMATION_RUNTIME_EVIDENCE_NAME = re.compile(
    r"automation.*(?:runtime|evidence|blocker)"
    r"|(?:runtime|evidence|blocker).*automation",
    re.IGNORECASE,
)


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def rust_struct_fields(source: str, name: str) -> list[str]:
    match = re.search(
        rf"pub struct {re.escape(name)}\s*\{{(?P<body>.*?)\n\}}",
        source,
        re.DOTALL,
    )
    if match is None:
        raise AssertionError(f"missing Rust struct {name}")
    return re.findall(r"^\s*pub\s+([a-z][a-z0-9_]*)\s*:", match.group("body"), re.MULTILINE)


def without_source_comments(source: str) -> str:
    source = re.sub(r"/\*.*?\*/", "", source, flags=re.DOTALL)
    return re.sub(r"//[^\n]*", "", source)


def automation_timing_sources() -> list[Path]:
    roots_and_suffixes = [
        (REPO_ROOT / "dux-core/src", "*.rs"),
        (REPO_ROOT / "dux-macos/Dux", "*.swift"),
    ]
    discovered: set[Path] = set()
    declaration = re.compile(
        r"\b(?:Dux)?Automation(?:Schedule)?"
        r"(?:Scheduler|Scheduling|Timing|Clock|Wake|Deadline)\b"
    )
    for root, suffix in roots_and_suffixes:
        for path in root.rglob(suffix):
            if AUTOMATION_TIMING_NAME.search(path.stem):
                discovered.add(path)
                continue
            if declaration.search(without_source_comments(read(path))):
                discovered.add(path)
    return sorted(discovered)


def automation_runtime_evidence_sources() -> list[Path]:
    root = REPO_ROOT / "dux-core/src"
    discovered: set[Path] = set()
    declaration = re.compile(
        r"\b(?:AutomationCoreRuntime\w*|StoredAutomationRuntimeObservation)\b"
    )
    for path in root.rglob("*.rs"):
        source = without_source_comments(read(path))
        if AUTOMATION_RUNTIME_EVIDENCE_NAME.search(path.stem) or declaration.search(
            source
        ):
            discovered.add(path)
    return sorted(discovered)


class AutomationScheduleBoundaryTests(unittest.TestCase):
    def test_ffi_v66_is_path_free_and_effect_dormant(self) -> None:
        ffi = read(FFI)
        self.assertIn("const FFI_CONTRACT_VERSION: u32 = 67;", ffi)
        self.assertIn("const AUTOMATION_OVERVIEW_RECORD_VERSION: u32 = 3;", ffi)
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleDraftInput"),
            [
                "record_version",
                "scope",
                "cadence",
                "minimum_age_seconds",
                "minimum_reclaimable_bytes",
                "maximum_bytes_per_run",
                "excluded_rules",
                "notify_before_run",
                "confirmation_mode",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleOverview"),
            [
                "record_version",
                "global_control",
                "execution_available",
                "eligible_rule_count",
                "schedules",
                "schedule_eligibility",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleEligibilityAssessment"),
            [
                "record_version",
                "policy_revision",
                "schedule_id",
                "schedule_revision",
                "status",
                "included_statically_eligible_rule_count",
                "reasons",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationGlobalControlStatus"),
            [
                "record_version",
                "enabled",
                "source",
                "revision",
                "updated_at_unix_ms",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationSchedulePeriodicRecurrence"),
            [
                "record_version",
                "cursor_revision",
                "recurrence_policy_revision",
                "anchor_at_unix_ms",
                "next_occurrence_ordinal",
                "next_run_at_unix_ms",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleSuggestionFeed"),
            [
                "record_version",
                "derivation_revision",
                "source_session_count",
                "has_older_source_sessions",
                "qualifying_rule_count",
                "suggestions",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleSuggestion"),
            [
                "record_version",
                "rank",
                "rule_id",
                "rule_revision",
                "successful_manual_run_count",
                "manual_regrowth_cycle_count",
                "latest_manual_attempt_at_unix_ms",
                "latest_regrowth_at_unix_ms",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleAuthoringCatalog"),
            [
                "record_version",
                "authoring_policy_revision",
                "maximum_selected_exclusions",
                "statically_selectable_rule_count",
                "categories",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleAuthoringCategory"),
            [
                "record_version",
                "category",
                "scope_membership_digest_sha256",
                "rules",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleAuthoringRule"),
            [
                "record_version",
                "rule_id",
                "rule_revision",
                "title_key",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleCategoryDraftInput"),
            [
                "record_version",
                "authoring_policy_revision",
                "scope_membership_digest_sha256",
                "category",
                "cadence",
                "minimum_age_seconds",
                "minimum_reclaimable_bytes",
                "maximum_bytes_per_run",
                "excluded_rules",
                "notify_before_run",
                "confirmation_mode",
            ],
        )
        catalog_fields = (
            rust_struct_fields(ffi, "AutomationScheduleAuthoringCatalog")
            + rust_struct_fields(ffi, "AutomationScheduleAuthoringCategory")
            + rust_struct_fields(ffi, "AutomationScheduleAuthoringRule")
        )
        for forbidden in (
            "path",
            "root",
            "scan_id",
            "candidate_id",
            "schedule_id",
            "eligible",
            "runnable",
            "plan_id",
            "approval",
            "journal",
            "task",
            "callback",
            "trigger",
            "notification",
            "effect",
        ):
            self.assertNotIn(forbidden, catalog_fields)
        suggestion_fields = rust_struct_fields(ffi, "AutomationScheduleSuggestion")
        for forbidden in (
            "path",
            "scope",
            "schedule_id",
            "draft_id",
            "candidate_id",
            "scan_id",
            "plan_id",
            "approval",
            "task",
            "callback",
            "enabled",
            "eligible",
            "runnable",
        ):
            self.assertNotIn(forbidden, suggestion_fields)
        schedule_fields = rust_struct_fields(ffi, "AutomationScheduleStatus")
        for forbidden in (
            "path",
            "node_id",
            "candidate_id",
            "plan_id",
            "approval",
            "task",
            "last_run",
            "trigger",
        ):
            self.assertNotIn(forbidden, schedule_fields)
        self.assertIn("execution_available: false", ffi)

        method_names = set(
            re.findall(
                r"pub fn ([a-z][a-z0-9_]*)\s*\(",
                ffi,
            )
        )
        self.assertTrue(
            {
                "get_automation_schedule_overview",
                "get_automation_schedule_suggestions",
                "get_automation_schedule_authoring_catalog",
                "create_automation_schedule_draft",
                "create_automation_category_schedule_draft",
                "rebind_automation_category_schedule_draft",
                "replace_automation_schedule_draft",
                "delete_automation_schedule_draft",
                "set_automation_global_enabled",
                "reset_automation_global_control",
                "enable_automation_schedule",
                "pause_automation_schedule",
                "resume_automation_schedule",
                "disable_automation_schedule",
            }.issubset(method_names)
        )
        self.assertEqual(
            {
                name
                for name in method_names
                if "automation_schedule_authoring_catalog" in name
                or "automation_category_schedule_draft" in name
            },
            {
                "get_automation_schedule_authoring_catalog",
                "create_automation_category_schedule_draft",
                "rebind_automation_category_schedule_draft",
            },
        )
        self.assertFalse(
            any(
                "automation" in name
                and any(
                    word in name
                    for word in (
                        "run",
                        "execute",
                        "trigger",
                        "start",
                        "claim",
                        "admit",
                        "wake",
                        "due",
                        "poll",
                    )
                )
                for name in method_names
            )
        )

    def test_core_contract_has_no_target_or_effect_capability(self) -> None:
        domain = read(CORE_DOMAIN)
        authoring_catalog = read(CORE_AUTHORING_CATALOG)
        eligibility = read(CORE_ELIGIBILITY)
        engine = read(CORE_ENGINE)
        store = read(CORE_STORE)
        production_store = store.split("#[cfg(test)]\nmod tests", 1)[0]
        combined = "\n".join((domain, authoring_catalog, eligibility, engine))
        for forbidden in (
            "std::path",
            "PathBuf",
            "CleanupPlan",
            "CandidateId",
            "CleanupPlanId",
            "Journal",
            "EffectRequest",
            "ScanId",
        ):
            self.assertNotIn(forbidden, combined)
        self.assertNotRegex(combined, r"\b(last_run|trigger_source)\s*:")
        self.assertIn("state = 'enabled'", production_store)
        self.assertIn("AutomationScheduleState::Paused", production_store)
        self.assertNotIn("CleanupPlan", production_store)
        self.assertNotIn("EffectRequest", production_store)

        registry = read(CORE_REGISTRY)
        automation_methods = [
            name
            for name in re.findall(
                r"pub fn ([a-z][a-z0-9_]*)\s*\(", registry
            )
            if "automation" in name
        ]
        self.assertEqual(
            set(automation_methods),
            {
                "automation_overview",
                "automation_schedule_suggestions",
                "automation_schedule_authoring_catalog",
                "create_automation_schedule_draft",
                "create_automation_category_schedule_draft",
                "rebind_automation_category_schedule_draft",
                "replace_automation_schedule_draft",
                "delete_automation_schedule_draft",
                "set_automation_global_enabled",
                "reset_automation_global_control",
                "enable_automation_schedule",
                "pause_automation_schedule",
                "resume_automation_schedule",
                "disable_automation_schedule",
            },
        )
        public_rebind = registry.split(
            "pub fn rebind_automation_category_schedule_draft(", 1
        )[1].split("\n    pub fn ", 1)[0]
        rebind_helper = registry.split(
            "fn replace_automation_schedule_draft_with_catalog_loader(", 1
        )[1].split("\n    fn ", 1)[0]
        rebind_boundary = public_rebind + rebind_helper
        self.assertIn("expected_revision", rebind_boundary)
        self.assertIn("authoring_binding", rebind_boundary)
        self.assertIn("AutomationScheduleState::Disabled", rebind_boundary)
        self.assertGreaterEqual(
            rebind_boundary.count("AutomationScheduleScope::Category"), 2
        )
        self.assertIn("replacement_category != stored_category", rebind_helper)
        self.assertIn("replacement_rule != stored_rule", rebind_helper)
        self.assertIn("InvalidAuthoringSelection", rebind_boundary)

    def test_core_eligibility_is_complete_fresh_and_observation_only(self) -> None:
        eligibility = read(CORE_ELIGIBILITY)
        self.assertIn("AUTOMATION_ELIGIBILITY_POLICY_REVISION: u32 = 2", eligibility)
        self.assertIn("AUTOMATION_REQUIRED_MANUAL_SUCCESSES: u16 = 2", eligibility)
        self.assertIn("AUTOMATION_REQUIRED_RECENT_RUNS: usize = 2", eligibility)
        self.assertIn(
            "AUTOMATION_CURRENT_EVIDENCE_MAX_AGE: Duration = CLEANUP_PLAN_VALIDITY",
            eligibility,
        )
        for gate in (
            "ShippedPolicy",
            "ManualHistory",
            "RecentRunSafety",
            "CurrentCandidateAge",
            "CurrentCandidateSize",
            "ActivityGuard",
            "EvidenceFreshness",
            "ExecutionPrivilege",
        ):
            self.assertIn(gate, eligibility)
        self.assertIn("No cleanup API accepts this type", eligibility)
        self.assertIn("rule.guards().inactive_processes().is_empty()", eligibility)
        for forbidden in (
            "std::path",
            "PathBuf",
            "CleanupPlan",
            "CleanupPlanId",
            "Journal",
            "EffectRequest",
            "execute_cleanup",
        ):
            self.assertNotIn(forbidden, eligibility)

    def test_core_runtime_blocker_observation_is_private_bounded_and_read_only(
        self,
    ) -> None:
        self.assertTrue(CORE_RUNTIME_ENGINE.is_file())
        self.assertTrue(CORE_RUNTIME_PERSISTENCE.is_file())
        engine = without_source_comments(read(CORE_RUNTIME_ENGINE))
        persistence = without_source_comments(read(CORE_RUNTIME_PERSISTENCE))
        production_persistence = persistence.split("#[cfg(test)]", 1)[0]
        registry = without_source_comments(read(CORE_REGISTRY))
        combined = "\n".join((engine, persistence))

        self.assertRegex(
            engine,
            r"pub\(crate\)\s+const\s+AUTOMATION_CORE_RUNTIME_POLICY_REVISION:\s*u32\s*=\s*1\s*;",
        )
        self.assertIn("AutomationCoreRuntimeAssessment", engine)
        self.assertRegex(
            engine,
            r"pub\(crate\)\s+struct\s+AutomationCoreRuntimeAssessment\b",
        )
        self.assertRegex(
            registry,
            r"pub\(crate\)\s+fn\s+observe_automation_core_runtime\s*\(",
        )
        for gate in (
            "EngineLifecycle",
            "RuntimeIdentity",
            "ScanWork",
            "CleanupWork",
        ):
            self.assertIn(gate, engine)
        for status in ("Passed", "Blocked", "Unproven"):
            self.assertIn(status, engine)
        self.assertRegex(
            engine,
            r"\[\s*AutomationCoreRuntimeGateAssessment\s*;\s*4\s*\]",
        )
        self.assertIn("SystemTime::now()", registry)
        entrypoint_start = registry.index(
            "pub(crate) fn observe_automation_core_runtime"
        )
        entrypoint_end = registry.index(
            "pub fn start_format_size_batch", entrypoint_start
        )
        entrypoint = registry[entrypoint_start:entrypoint_end]
        for forbidden_authority in (
            "AutomationSchedule",
            "AutomationEligibility",
            "AutomationSchedulerRuntimeEvidence",
            "AutomationSchedulerDecision",
            "Candidate",
            "CleanupPlan",
            "Approval",
            "Journal",
            "Planner",
            "Executor",
            "Notification",
            "start_scan",
            "start_cleanup",
            "execute_cleanup",
            "claim_cleanup",
        ):
            self.assertNotIn(forbidden_authority, entrypoint)
        store_source = without_source_comments(read(REPO_ROOT / "dux-core/src/persistence/store.rs"))
        store_entrypoint_start = store_source.index(
            "pub(crate) fn observe_automation_runtime_store"
        )
        store_entrypoint_end = store_source.index(
            "pub(crate) fn acquire_scan_scope_lease", store_entrypoint_start
        )
        store_entrypoint = store_source[store_entrypoint_start:store_entrypoint_end]
        self.assertIn("self.connection.try_lock()", store_entrypoint)
        for mutating_or_repairing_helper in (
            "lock_current_history_connection",
            "repair_sqlite_sidecars",
            "validate_history_storage_after_write",
            "execute_batch",
            "transaction",
        ):
            self.assertNotIn(mutating_or_repairing_helper, store_entrypoint)

        for forbidden in (
            "std::path",
            "PathBuf",
            "AutomationScheduleId",
            "AutomationScheduleAuthoringBinding",
            "AutomationSchedulerRuntimeEvidence",
            "AutomationSchedulerDecision",
            "AutomationCurrentCandidateEvidence",
            "AutomationEligibilityAssessment",
            "AutomationEligibilityInput",
            "AutomationManualHistoryEvidence",
            "AutomationActivityEvidence",
            "ActivityGuard",
            "CandidateId",
            "ScanId",
            "CleanupPlan",
            "CleanupPlanId",
            "Approval",
            "JournalClaim",
            "EffectRequest",
            "Planner",
            "Executor",
            "DuxMaintenanceKind",
        ):
            self.assertNotIn(forbidden, combined)
        for forbidden_call in (
            "execute_cleanup",
            "start_confirmed_cleanup",
            "prepare_cleanup",
            "start_scan",
            "start_cleanup",
            "claim_cleanup",
            "recover_cleanup",
        ):
            self.assertNotRegex(combined, rf"\b{forbidden_call}\s*\(")
        self.assertNotRegex(
            production_persistence,
            r"(?i)\b(?:INSERT|UPDATE|DELETE|REPLACE)\b",
        )

        public_core = without_source_comments(read(CORE_LIB))
        ffi = without_source_comments(read(FFI))
        for private_name in (
            "AUTOMATION_CORE_RUNTIME_POLICY_REVISION",
            "AutomationCoreRuntimeAssessment",
            "AutomationCoreRuntimeGate",
            "AutomationCoreRuntimeGateAssessment",
            "AutomationCoreRuntimeGateStatus",
            "AutomationCoreRuntimeReason",
            "observe_automation_core_runtime",
        ):
            self.assertNotIn(private_name, public_core)
            self.assertNotIn(private_name, ffi)
        native_sources = list((REPO_ROOT / "dux-macos/Dux").rglob("*.swift"))
        native = "\n".join(
            without_source_comments(read(path)) for path in native_sources
        )
        self.assertNotIn("AutomationCoreRuntimeAssessment", native)
        self.assertNotIn("observeAutomationCoreRuntime", native)
        self.assertIn("const FFI_CONTRACT_VERSION: u32 = 67", ffi)
        self.assertIn("const AUTOMATION_OVERVIEW_RECORD_VERSION: u32 = 3", ffi)
        self.assertIn(
            "pub const DATABASE_SCHEMA_VERSION: u32 = 23",
            read(CORE_PERSISTENCE_STATUS),
        )
        self.assertFalse(
            any(
                "automation_runtime" in path.name
                for path in (REPO_ROOT / "dux-core/migrations").glob("*.sql")
            )
        )

    def test_scan_admission_witness_is_retained_zero_wait_and_scan_only(
        self,
    ) -> None:
        engine = without_source_comments(read(CORE_RUNTIME_ENGINE))
        persistence = without_source_comments(read(CORE_RUNTIME_PERSISTENCE))
        registry = without_source_comments(read(CORE_REGISTRY))
        store = without_source_comments(
            read(REPO_ROOT / "dux-core/src/persistence/store.rs")
        )
        storage = without_source_comments(read(CORE_PERSISTENCE_STORAGE))

        # A scalar store read cannot clear both admission paths. Only the
        # retained writer interval may clear scan uncertainty; cleanup remains
        # a separately represented fail-closed fact.
        self.assertIn("scan_admission_unresolved: bool", persistence)
        self.assertIn("cleanup_admission_unresolved: bool", persistence)
        self.assertIn("with_retained_scan_admission", persistence)
        retained_scan = persistence.split(
            "pub(super) const fn with_retained_scan_admission", 1
        )[1].split("pub(super) const fn cleanup_contention", 1)[0]
        self.assertIn("self.scan_admission_unresolved = false", retained_scan)
        self.assertNotIn("cleanup_admission_unresolved = false", retained_scan)

        entrypoint_start = registry.index(
            "pub(crate) fn observe_automation_core_runtime"
        )
        entrypoint_end = registry.index(
            "pub fn start_format_size_batch", entrypoint_start
        )
        entrypoint = registry[entrypoint_start:entrypoint_end]
        self.assertIn("self.inner.scan_admission.try_lock()", entrypoint)
        self.assertIn("retained_store_observation", entrypoint)
        self.assertIn("retains_scan_admission()", entrypoint)
        self.assertLess(
            entrypoint.index("drop(retained_store_observation)"),
            entrypoint.index("drop(scan_admission)"),
        )
        for waiting_or_recovering in (
            "lock_registry_recover",
            "thread::sleep",
            "Task::sleep",
            "recover_cleanup",
            "claim_cleanup",
        ):
            self.assertNotIn(waiting_or_recovering, entrypoint)

        self.assertIn("enum AutomationRuntimeStoreObservation<'a>", store)
        self.assertRegex(
            store,
            r"struct\s+AutomationRuntimeStoreObservationGuard<'a>\s*\{\s*"
            r"_writer:\s*WriterLockGuard,\s*"
            r"_connection:\s*MutexGuard<'a,\s*Connection>,\s*"
            r"_cleanup:\s*CleanupLockGuard,",
        )
        store_entrypoint_start = store.index(
            "pub(crate) fn observe_automation_runtime_store"
        )
        store_entrypoint_end = store.index(
            "pub(crate) fn acquire_scan_scope_lease", store_entrypoint_start
        )
        store_entrypoint = store[store_entrypoint_start:store_entrypoint_end]
        for required in (
            "try_acquire_cleanup_lock_for_observation()",
            "self.status.try_lock()",
            "self.connection.try_lock()",
            "try_acquire_writer_lock_for_observation()",
            "inspect_schema_for_status(&connection)",
            "try_validate_cleanup_lock_guard_for_observation",
            "try_validate_writer_lock_guard_for_observation",
            "with_retained_scan_admission()",
        ):
            self.assertIn(required, store_entrypoint)
        for mutating_or_repairing in (
            "lock_current_history_connection",
            "repair_sqlite_sidecars",
            "validate_history_storage_after_write",
            "execute_batch",
            "transaction",
            "acquire_scan_scope_lease(",
            "recover_cleanup(",
        ):
            self.assertNotIn(mutating_or_repairing, store_entrypoint)

        writer_probe_start = storage.index(
            "pub(crate) fn try_acquire_writer_lock_for_observation"
        )
        writer_probe_end = storage.index(
            "pub(crate) fn acquire_cleanup_lock(", writer_probe_start
        )
        writer_probe = storage[writer_probe_start:writer_probe_end]
        for required in (
            "try_validate_control_objects()",
            "compare_exchange(false, true",
            "Instant::now(), true",
            "try_validate_writer_lock_guard_for_observation",
            "validate_retained_file",
        ):
            self.assertIn(required, writer_probe)
        for forbidden in (
            "repair",
            "provision",
            "create_dir",
            "transaction",
            "thread::sleep",
            "acquire_writer_lock(",
        ):
            self.assertNotIn(forbidden, writer_probe)

        # The witness remains an observation input, not current-candidate,
        # scheduling, FFI, or action authority.
        combined = "\n".join((engine, persistence, entrypoint, store_entrypoint))
        for forbidden in (
            "AutomationSchedulerRuntimeEvidence",
            "AutomationSchedulerDecision",
            "AutomationCurrentCandidateEvidence",
            "AutomationEligibilityAssessment",
            "CleanupPlan",
            "EffectRequest",
            "execute_cleanup",
            "start_cleanup",
        ):
            self.assertNotIn(forbidden, combined)
        ffi = without_source_comments(read(FFI))
        self.assertNotIn("AutomationRuntimeStoreObservation", ffi)
        self.assertNotIn("observe_automation_core_runtime", ffi)
        self.assertIn("const FFI_CONTRACT_VERSION: u32 = 67", ffi)

    def test_cleanup_admission_protocol_v23_is_prepublication_move_only_and_not_yet_runtime_clear(
        self,
    ) -> None:
        registry = without_source_comments(read(CORE_REGISTRY))
        lease = without_source_comments(read(CORE_CLEANUP_LEASE))
        executor = without_source_comments(read(CORE_CLEANUP_EXECUTOR))
        planner = without_source_comments(read(CORE_EXACT_PATH_REVIEW))
        runtime = without_source_comments(read(CORE_RUNTIME_PERSISTENCE))
        migration = read(MIGRATION_V23)

        self.assertIn("cleanup-admission protocol v1", migration)
        self.assertNotRegex(migration, r"(?i)\b(?:CREATE|ALTER|DROP|INSERT|UPDATE|DELETE)\b")
        self.assertIn("struct CleanupAdmissionLease", lease)
        self.assertIn("acquire_cleanup_journal_lease_inner(Duration::ZERO)", lease)
        self.assertIn("fn revalidate_for_publication", lease)
        for required in (
            "belongs_to_store(store)",
            "validate_cleanup_lock_for_journal",
            "store.status()",
            "DATABASE_SCHEMA_VERSION",
            "DatabaseAccess::ReadWriteCurrent",
        ):
            self.assertIn(required, lease)
        self.assertRegex(
            read(CORE_CLEANUP_LEASE),
            r"#\[cfg\(test\)\]\s*pub\(crate\) fn acquire_cleanup_journal_lease",
        )
        self.assertRegex(
            read(CORE_CLEANUP_LEASE),
            r"#\[cfg\(test\)\]\s*pub\(crate\) fn acquire_cleanup_admission_lease",
        )
        self.assertNotRegex(
            lease,
            r"(?:derive\([^)]*Clone[^)]*\)|impl\s+Clone\s+for)\s*CleanupAdmissionLease",
        )

        # Exhaustively freeze the production admission graph. A newly added
        # acquisition, conversion, planned-row write, or late raw store write
        # must update this reviewed allowlist instead of silently reopening the
        # queue-before-lock race.
        self.assertEqual(registry.count(".try_acquire_cleanup_admission_lease()"), 1)
        self.assertEqual(registry.count(".into_revalidated_journal_lease("), 1)
        self.assertEqual(executor.count(".into_revalidated_journal_lease("), 1)
        self.assertEqual(planner.count(".into_revalidated_journal_lease("), 1)
        self.assertEqual(executor.count(".record_cleanup_session_planned(store,"), 1)
        self.assertEqual(planner.count(".record_cleanup_session_planned(store,"), 1)
        self.assertEqual(lease.count("fn record_cleanup_session_planned("), 1)
        self.assertRegex(
            read(CORE_PERSISTENCE_STORE),
            r"#\[cfg\(test\)\]\s*pub\(crate\) fn record_cleanup_session_planned",
        )
        self.assertEqual(
            read(CORE_PERSISTENCE_STORE).count(
                "pub(crate) fn record_cleanup_session_planned"
            ),
            1,
        )

        admission = registry.split("fn try_acquire_cleanup_admission", 1)[1].split(
            "fn reserve_trash_cleanup", 1
        )[0]
        self.assertIn("try_acquire_cleanup_admission_lease()", admission)
        self.assertIn("revalidate_for_publication", admission)
        self.assertNotIn("registry.lock()", admission)

        reservation = registry.split("fn reserve_trash_cleanup", 1)[1].split(
            "fn quarantine_cleanup", 1
        )[0]
        self.assertLess(
            reservation.index("try_acquire_cleanup_admission()"),
            reservation.index("registry.try_lock()"),
        )
        self.assertIn("TryLockError::WouldBlock", reservation)
        self.assertIn("admission: Some(admission)", reservation)

        dry_start = registry.split("fn start_rust_target_dry_run_with_hook", 1)[1].split(
            "pub fn start_permanent_safe_cleanup", 1
        )[0]
        permanent_start = registry.split(
            "fn start_permanent_safe_cleanup_with_hook", 1
        )[1].split("fn execute_approved_permanent_safe_session_with_bound_capacity", 1)[0]
        for entrypoint in (dry_start, permanent_start):
            self.assertLess(
                entrypoint.index("try_acquire_cleanup_admission()"),
                entrypoint.index("registry.try_lock()"),
            )
            self.assertIn("TryLockError::WouldBlock", entrypoint)
            self.assertIn("admission", entrypoint)
            self.assertIn("work: Work", entrypoint)

        dry_worker = registry.split("fn run_rust_target_dry_run_task", 1)[1].split(
            "fn rust_target_dry_run_validation_outcome", 1
        )[0]
        permanent_worker = registry.split("fn run_permanent_safe_cleanup_task", 1)[1]
        self.assertIn("admission.into_revalidated_journal_lease", dry_worker)
        self.assertNotIn("acquire_cleanup_journal_lease", dry_worker)
        self.assertIn("begin_cleanup_session_with_lease", permanent_worker)
        self.assertNotIn("acquire_cleanup_journal_lease", permanent_worker)

        trash_entrypoint = executor.split(
            "pub(crate) fn execute_reviewed_trash_selection", 1
        )[1].split("pub(crate) enum TrashSelectionExecutionError", 1)[0]
        self.assertIn("admission: CleanupAdmissionLease", trash_entrypoint)
        self.assertRegex(
            trash_entrypoint,
            r"admission\s*\.record_cleanup_session_planned\(store,",
        )
        self.assertIn("begin_with_admission", trash_entrypoint)
        self.assertIn("into_revalidated_journal_lease", executor)
        self.assertIn("begin_cleanup_session_with_lease", planner)
        self.assertIn("into_revalidated_journal_lease", planner)
        self.assertRegex(
            read(CORE_EXACT_PATH_REVIEW),
            r"#\[cfg\(test\)\]\s*pub\(crate\) fn begin_cleanup_session",
        )

        # This slice establishes the admission protocol only. The runtime
        # observer must remain fail-closed until it retains and validates its
        # own cleanup-admission witness in the following slice.
        runtime_query = runtime.split("fn inspect_automation_runtime_work", 1)[1]
        self.assertIn("cleanup_admission_unresolved: true", runtime_query)
        self.assertNotIn("cleanup_admission_unresolved: false", runtime_query)
        self.assertNotIn("with_retained_cleanup_admission", runtime)

    def test_core_runtime_evidence_discovery_covers_both_private_layers(self) -> None:
        relative = {
            path.relative_to(REPO_ROOT).as_posix()
            for path in automation_runtime_evidence_sources()
            if not path.name.endswith("_tests.rs")
        }
        self.assertEqual(
            relative,
            {
                "dux-core/src/engine/automation_runtime.rs",
                "dux-core/src/engine/registry.rs",
                "dux-core/src/persistence/automation_runtime.rs",
                "dux-core/src/persistence/mod.rs",
                "dux-core/src/persistence/store.rs",
            },
        )

    def test_authoring_catalog_is_core_owned_exact_and_authority_free(self) -> None:
        catalog = read(CORE_AUTHORING_CATALOG)
        registry = read(CORE_REGISTRY)
        ffi = read(FFI)
        self.assertIn(
            "AUTOMATION_SCHEDULE_AUTHORING_CATALOG_POLICY_REVISION: u32 = 1",
            catalog,
        )
        for required in (
            "rule.schedule_eligible()",
            "SafetyTier::SafeRegenerable",
            "CandidateAction::RemoveKnownRegenerableContents",
            "RuleScope::UserCacheDirectory",
            "protected_descendants().is_empty()",
            "dux.automation.schedule-authoring.membership.v1",
            "Sha256",
        ):
            self.assertIn(required, catalog)
        self.assertIn("bundled_automation_schedule_authoring_catalog", registry)
        projection = ffi.split(
            "fn automation_schedule_authoring_catalog(", 1
        )[1].split("const fn automation_authoring_category_index", 1)[0]
        self.assertIn(
            "automation_authoring_membership_digest(",
            projection,
        )
        self.assertIn('b"dux.automation.schedule-authoring.membership.v1\\0"', ffi)
        self.assertIn("Sha256::new()", ffi)
        self.assertIn(".to_be_bytes()", ffi)
        for forbidden in (
            "std::path",
            "PathBuf",
            "CandidateId",
            "ScanId",
            "CleanupPlan",
            "Approval",
            "Journal",
            "EffectRequest",
            "execute_cleanup",
        ):
            self.assertNotIn(forbidden, without_source_comments(catalog))

    def test_schema_v22_preserves_activation_and_does_not_fabricate_category_consent(
        self,
    ) -> None:
        migration_v20 = read(MIGRATION_V20)
        migration_v21 = read(MIGRATION_V21)
        migration_v22 = read(MIGRATION_V22)
        self.assertIn("DUX-DESTRUCTIVE:", migration_v20)
        self.assertIn("CHECK (state = 'disabled_draft')", migration_v20)
        self.assertIn("DUX-DESTRUCTIVE:", migration_v21)
        self.assertIn("state IN ('disabled', 'enabled', 'paused')", migration_v21)
        self.assertIn("cadence IN ('weekly', 'monthly')", migration_v21)
        self.assertIn("SELECT\n    schedule_id, 'disabled', NULL", migration_v21)
        self.assertIn("recurrence_policy_revision = 1", migration_v21)
        self.assertIn("next_run_unix_ms > recurrence_anchor_unix_ms", migration_v21)
        self.assertIn("schedules_enabled_by_next_run", migration_v21)
        self.assertNotIn("last_run_unix_ms", migration_v21)
        self.assertNotIn("automation_global_control", migration_v21)
        self.assertIn("ON DELETE CASCADE", migration_v21)
        self.assertIn("DUX-DESTRUCTIVE:", migration_v22)
        self.assertIn("authoring_policy_revision", migration_v22)
        self.assertIn("authoring_membership_sha256", migration_v22)
        self.assertIn("length(authoring_membership_sha256) = 32", migration_v22)
        self.assertRegex(
            migration_v22,
            r"SELECT(?s:.*?)NULL(?s:.*?)NULL(?s:.*?)FROM schedules",
        )
        self.assertIn("ON DELETE CASCADE", migration_v22)

    def test_no_shipped_rule_is_presently_schedule_eligible(self) -> None:
        document = json.loads(read(CATALOG))
        rules = document["rules"]
        self.assertGreater(len(rules), 0)
        self.assertTrue(all(rule["schedule_eligible"] is False for rule in rules))

    def test_no_authority_layer_consumes_automation_drafts_or_suggestions(self) -> None:
        roots = [
            REPO_ROOT / "dux-cli/src",
            REPO_ROOT / "dux-core/src/ai",
            REPO_ROOT / "dux-core/src/cleanup",
            REPO_ROOT / "dux-core/src/planner",
        ]
        files = [path for root in roots for path in root.rglob("*.rs")]
        files += [
            APP_RUNTIME,
            MAINTENANCE_SCHEDULER,
            REPO_ROOT / "dux-macos/Dux/Services/CapacitySamplingScheduler.swift",
        ]
        files += automation_timing_sources()
        for path in sorted(set(files)):
            source = read(path)
            self.assertNotIn("AutomationScheduleDraft", source, str(path))
            self.assertNotIn("automation_schedule_draft", source, str(path))
            self.assertNotIn("AutomationScheduleStatus", source, str(path))
            self.assertNotIn("automation_schedule_status", source, str(path))
            self.assertNotIn("AutomationScheduleSuggestion", source, str(path))
            self.assertNotIn("automation_schedule_suggestion", source, str(path))
            self.assertNotIn("AutomationScheduleAuthoringCatalog", source, str(path))
            self.assertNotIn("automation_schedule_authoring_catalog", source, str(path))
            self.assertNotIn("AutomationScheduleAuthoringBinding", source, str(path))
            self.assertNotIn("automation_schedule_authoring_binding", source, str(path))
            self.assertNotIn("rebind_automation_category_schedule_draft", source, str(path))
            self.assertNotIn("rebindAutomationCategorySchedule", source, str(path))

    def test_native_authoring_is_explicit_path_free_and_not_ai_driven(self) -> None:
        model = read(NATIVE_AUTOMATION_MODEL)
        settings = read(NATIVE_AUTOMATION_SETTINGS)
        view = read(NATIVE_AUTOMATION_VIEW)

        self.assertIn("ExactPolicyDecimal.parseGiB", model)
        self.assertIn("multipliedReportingOverflow", model)
        for exact_bound in ("3_153_600_000", "UInt64(Int64.max)"):
            self.assertIn(exact_bound, model)
        self.assertIn("beginCreatingSchedule", settings)
        self.assertIn("loadAuthoringCatalog", settings)
        self.assertIn("authoringCatalog", settings)
        self.assertIn("beginCreatingCategorySchedule", settings)
        self.assertIn("beginReviewingCategorySchedule", settings)
        self.assertIn("rebindAutomationCategorySchedule", settings)
        self.assertIn("historySuggestionFeed?.suggestions.contains", settings)
        self.assertIn("schedule.state == .disabled", settings)
        self.assertIn("expectedRevision", settings)
        self.assertIn("createAutomationSchedule", settings)
        self.assertIn("replaceAutomationSchedule", settings)
        self.assertIn("preservesReviewedImmutableFields", settings)
        self.assertIn("immutableFieldsChanged", settings)
        self.assertIn('Button("Review disabled schedule…")', view)
        self.assertIn('"Save disabled schedule"', view)
        self.assertIn('LabeledContent("Pre-run notices")', view)
        self.assertIn('LabeledContent("Run approval")', view)
        self.assertNotRegex(view, r'Toggle\s*\(\s*"Notify before')
        self.assertNotRegex(view, r'Picker\s*\(\s*"Run approval"')
        self.assertIn(
            "throw AutomationScheduleServiceError.outcomeUnknown",
            read(NATIVE_ENGINE_SERVICE),
        )
        self.assertIn("NoEnabledSchedulesDuxAutomationDecisionSource", read(APP_RUNTIME))
        combined_native = "\n".join((model, settings, view))
        self.assertNotIn("ExplorerCandidateCategory.allCases", combined_native)
        self.assertNotRegex(view, r"ForEach\s*\(\s*ExplorerCandidateCategory")
        self.assertNotRegex(view, r"\[\s*ExplorerCandidateCategory\.")
        self.assertNotRegex(
            view,
            r'TextField\s*\(\s*"(?:Category|Rule ID|Rule identifier)',
        )
        self.assertIn("catalog.categories", view)
        self.assertIn("beginReviewingCategorySchedule", view)
        self.assertIn("case rebindCategory", model)
        native_service = read(NATIVE_ENGINE_SERVICE)
        self.assertIn("rebindAutomationCategorySchedule", native_service)
        self.assertIn("rebindAutomationCategoryScheduleDraft", native_service)
        self.assertIn(
            "func rebindAutomationCategoryScheduleDraft(",
            read(NATIVE_GENERATED_FFI),
        )
        self.assertIn("import CryptoKit", model)
        self.assertIn("dux.automation.schedule-authoring.membership.v1", model)
        self.assertRegex(model, r"SHA256(?:\.hash|\(\))")
        rebind_entry = settings.split(
            "func beginReviewingCategorySchedule(", 1
        )[1].split("\n    func ", 1)[0]
        self.assertIn("authoringCatalogIsFresh", rebind_entry)
        self.assertIn("catalog.categories.first", rebind_entry)
        self.assertIn("schedule.state == .disabled", rebind_entry)
        native_rebind = native_service.rsplit(
            "func rebindAutomationCategorySchedule(", 1
        )[1].split("\n    func ", 1)[0]
        self.assertIn("expectedRevision", native_rebind)
        self.assertIn("rebindAutomationCategoryScheduleDraft", native_rebind)

        for path in (NATIVE_AUTOMATION_MODEL, NATIVE_AUTOMATION_SETTINGS, NATIVE_AUTOMATION_VIEW):
            source = without_source_comments(read(path))
            for forbidden in ("PathBuf", "CandidateId", "CleanupPlan", "JournalClaim"):
                self.assertNotIn(forbidden, source, str(path))

        ai_sources = [
            path
            for path in (REPO_ROOT / "dux-macos/Dux").rglob("*.swift")
            if "ai" in path.stem.lower()
        ]
        self.assertTrue(ai_sources)
        for path in ai_sources:
            source = without_source_comments(read(path))
            for forbidden in (
                "beginCreatingSchedule",
                "createAutomationSchedule",
                "replaceAutomationSchedule",
                "AutomationScheduleEditorSession",
                "beginCreatingCategorySchedule",
                "createAutomationCategorySchedule",
                "createAutomationCategoryScheduleDraft",
                "beginReviewingCategorySchedule",
                "rebindAutomationCategorySchedule",
                "rebindAutomationCategoryScheduleDraft",
                "AutomationScheduleAuthoringCatalogModel",
            ):
                self.assertNotIn(forbidden, source, str(path))

    def test_discovered_automation_timing_sources_are_effect_dormant(self) -> None:
        forbidden_identifiers = (
            "AutomationScheduleDraft",
            "automation_schedule_draft",
            "AutomationScheduleSuggestion",
            "automation_schedule_suggestion",
            "AutomationScheduleAuthoringCatalog",
            "automation_schedule_authoring_catalog",
            "AutomationScheduleAuthoringCategory",
            "AutomationScheduleAuthoringRule",
            "AutomationScheduleAuthoringBinding",
            "rebind_automation_category_schedule_draft",
            "rebindAutomationCategorySchedule",
            "rebindAutomationCategoryScheduleDraft",
            "AutomationEligibilityAssessment",
            "AutomationScheduleDraftEligibilityAssessment",
            "AutomationScheduleStatus",
            "AutomationGlobalControlStatus",
            "PathBuf",
            "URL",
            "CandidateId",
            "candidate_id",
            "ScanId",
            "scan_id",
            "CleanupPlan",
            "CleanupPlanId",
            "cleanup_plan",
            "Approval",
            "approval",
            "CleanupJournal",
            "JournalClaim",
            "journal_claim",
            "EffectRequest",
            "effect_request",
            "Effect",
            "effect",
            "Planner",
            "planner",
            "Executor",
            "executor",
            "CleanupTask",
            "DuxMaintenanceTask",
            "DuxMaintenanceKind",
            "EngineService",
            "DuxEngine",
            "DuxFFI",
        )
        forbidden_calls = (
            "execute_cleanup",
            "start_confirmed_cleanup",
            "prepare_cleanup",
            "startMaintenance",
            "startScan",
            "startCleanup",
            "runCleanup",
            "preparePermanentCleanup",
            "startConfirmedCleanup",
        )
        for path in automation_timing_sources():
            source = without_source_comments(read(path))
            for forbidden in forbidden_identifiers:
                with self.subTest(path=path, forbidden=forbidden):
                    self.assertNotRegex(
                        source,
                        rf"\b{re.escape(forbidden)}\b",
                    )
            for call in forbidden_calls:
                with self.subTest(path=path, call=call):
                    self.assertNotRegex(
                        source,
                        rf"\b{re.escape(call)}\s*\(",
                    )

    def test_automation_timing_source_discovery_covers_core_and_native(self) -> None:
        relative = {
            path.relative_to(REPO_ROOT).as_posix()
            for path in automation_timing_sources()
        }
        self.assertTrue(
            {
                "dux-core/src/domain/automation_scheduler.rs",
                "dux-macos/Dux/Services/AutomationDecisionScheduler.swift",
            }.issubset(relative)
        )

    def test_automation_timing_sources_have_no_persistence_edge(self) -> None:
        forbidden = (
            "rusqlite",
            "Connection",
            "Transaction",
            "Storage",
            "std::fs",
            "FileManager",
            "UserDefaults",
            "INSERT INTO",
            "UPDATE automation",
            "DELETE FROM",
        )
        for path in automation_timing_sources():
            source = without_source_comments(read(path))
            for token in forbidden:
                with self.subTest(path=path, token=token):
                    self.assertNotIn(token, source)

    def test_native_scheduler_derives_no_calendar_policy_or_persistence(self) -> None:
        native_sources = [
            path for path in automation_timing_sources() if path.suffix == ".swift"
        ]
        self.assertTrue(native_sources, "native automation scheduler source is missing")
        combined = "\n".join(without_source_comments(read(path)) for path in native_sources)
        for forbidden in (
            "AutomationScheduleCadence",
            "Calendar",
            "DateComponents",
            "dateByAdding",
            "next_run",
            "last_run",
            "UserDefaults",
            "SQLite",
            "INSERT INTO",
            "UPDATE automation",
            "DELETE FROM",
        ):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, combined)
        for recurrence in ("weekly", "monthly", "timeZone", "daylightSaving"):
            with self.subTest(recurrence=recurrence):
                self.assertNotRegex(combined, rf"\b{recurrence}\b")
        self.assertIn("Task.sleep(for:", combined)

    def test_production_automation_decision_source_is_statically_empty(self) -> None:
        native_sources = [
            path for path in automation_timing_sources() if path.suffix == ".swift"
        ]
        combined = "\n".join(read(path) for path in native_sources)
        start = combined.index("struct NoEnabledSchedulesDuxAutomationDecisionSource")
        end = combined.index("struct DuxAutomationDecisionSchedulerTiming", start)
        empty_source = combined[start:end]
        self.assertIn("due: nil", empty_source)
        self.assertIn("nextCheckAt: nil", empty_source)
        for forbidden in ("EngineService", "DuxEngine", "DuxFFI"):
            self.assertNotIn(forbidden, empty_source)

        runtime = without_source_comments(read(APP_RUNTIME))
        constructions = list(
            re.finditer(r"DuxAutomationDecisionScheduler\s*\(", runtime)
        )
        self.assertGreater(len(constructions), 0)
        for construction in constructions:
            snippet = runtime[construction.start() : construction.start() + 220]
            self.assertRegex(
                snippet,
                r"source:\s*NoEnabledSchedulesDuxAutomationDecisionSource\s*\(\s*\)",
            )

    def test_ordinary_maintenance_scheduler_has_no_automation_kind(self) -> None:
        source = without_source_comments(read(MAINTENANCE_SCHEDULER))
        self.assertNotRegex(source, r"\bAutomation(?:Schedule|Scheduler|Timing)\b")
        self.assertNotRegex(source, r"\bcase\s+automation(?:Schedule|Cleanup)?\b")
        self.assertNotRegex(source, r"\.automation(?:Schedule|Cleanup)?\b")

    def test_effect_dormant_clock_wake_prerequisite_is_documented(self) -> None:
        adr = read(ADR)
        activation_adr = read(ACTIVATION_ADR)
        authoring_catalog_adr = read(AUTHORING_CATALOG_ADR)
        review = read(SECURITY_REVIEW)
        activation_review = read(ACTIVATION_SECURITY_REVIEW)
        authoring_review = read(AUTHORING_SECURITY_REVIEW)
        authoring_catalog_review = read(AUTHORING_CATALOG_SECURITY_REVIEW)
        core_runtime_review = read(CORE_RUNTIME_SECURITY_REVIEW)
        scan_admission_review = read(SCAN_ADMISSION_SECURITY_REVIEW)
        cleanup_admission_review = read(CLEANUP_ADMISSION_SECURITY_REVIEW)
        security = read(SECURITY_DESIGN)
        roadmap = read(ROADMAP)
        changelog = read(CHANGELOG)
        self.assertIn("**Status:** Accepted", adr)
        self.assertIn("two deliberately separate clock domains", adr)
        self.assertIn("globally at most one path-free request", adr)
        self.assertIn("production source of enabled/due schedules is empty", adr)
        self.assertIn("Explicitly deferred decisions", adr)
        self.assertIn("The Milestone 8 scheduler task remains open.", review)
        self.assertIn("**Status:** Accepted", activation_adr)
        self.assertIn("Original-anchor monthly calculation", activation_adr)
        self.assertIn("Low-disk-only cadence", activation_adr)
        self.assertIn("execution remains unavailable", activation_review)
        self.assertIn("effect-dormant schedule creation", authoring_review)
        self.assertIn("There is no generic category picker", authoring_review)
        self.assertIn("never retries an unknown create", authoring_review)
        self.assertIn("execution stays unavailable", authoring_review)
        self.assertIn("**Status:** Accepted", authoring_catalog_adr)
        self.assertIn("### Exact membership consent", authoring_catalog_adr)
        self.assertIn("Schema v22", authoring_catalog_adr)
        self.assertIn("Automation eligibility policy revision 2", authoring_catalog_adr)
        self.assertIn("independently recomputes", authoring_catalog_adr)
        self.assertIn("rebind_automation_category_schedule_draft", authoring_catalog_adr)
        self.assertIn("same category scope", authoring_catalog_adr)
        self.assertIn("Status: accepted only for effect-dormant", authoring_catalog_review)
        self.assertIn("production catalog is empty", authoring_catalog_review)
        self.assertIn("cannot silently", authoring_catalog_review)
        self.assertIn("CryptoKit", authoring_catalog_review)
        self.assertIn("Disabled category re-review boundary", authoring_catalog_review)
        self.assertIn(
            "accepted only for a sealed, core-owned, observation-only runtime-",
            core_runtime_review,
        )
        self.assertIn("exactly four ordered gate assessments", core_runtime_review)
        self.assertIn("cannot be converted into it", core_runtime_review)
        self.assertIn("The Milestone 8 scheduler task remains open.", core_runtime_review)
        self.assertIn(
            "accepted only for a retained, inspect-only scan-admission observation",
            scan_admission_review,
        )
        self.assertIn(
            "already commits an exact durable scope lease",
            scan_admission_review,
        )
        self.assertIn("Cleanup remains deliberately unproven", scan_admission_review)
        self.assertIn("acquisition order", scan_admission_review)
        self.assertIn("The Milestone 8 scheduler task remains open.", scan_admission_review)
        self.assertIn(
            "accepted only for the schema-v23 cleanup-admission protocol",
            cleanup_admission_review,
        )
        self.assertIn("No covered entry point may set", cleanup_admission_review)
        self.assertIn("Schema-v23 protocol epoch", cleanup_admission_review)
        self.assertIn("Runtime-observer boundary", cleanup_admission_review)
        self.assertIn("The Milestone 8 parent remains open.", cleanup_admission_review)
        self.assertIn("sealed core runtime-blocker observation prerequisite", roadmap)
        self.assertIn("retained scan-admission observation witness", roadmap)
        self.assertIn("schema-v23 cleanup-admission protocol prerequisite", roadmap)
        self.assertIn("This is not the deferred runtime/current-evidence adapter", security)
        self.assertIn("m8-automation-core-runtime-evidence.md", security)
        self.assertIn("m8-automation-scan-admission-witness.md", security)
        self.assertIn("m8-automation-cleanup-admission-protocol.md", security)
        self.assertIn("M8 sealed core runtime-blocker observation", changelog)
        self.assertIn("retained, scan-only admission witness", changelog)
        self.assertIn("schema-v23 M8 cleanup-admission protocol", changelog)
        self.assertIn(
            "docs/security-reviews/m8-automation-scheduler-wake.md",
            security,
        )
        self.assertIn(
            "docs/security-reviews/m8-automation-activation-controls.md",
            security,
        )
        self.assertIn(
            "docs/security-reviews/m8-automation-schedule-authoring-controls.md",
            security,
        )
        self.assertIn(
            "docs/security-reviews/m8-automation-selectable-scope-authoring.md",
            security,
        )


if __name__ == "__main__":
    unittest.main()
