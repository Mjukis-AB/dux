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
CORE_REGISTRY = REPO_ROOT / "dux-core/src/engine/registry.rs"
CORE_STORE = REPO_ROOT / "dux-core/src/persistence/automation_schedule.rs"
FFI = REPO_ROOT / "dux-ffi/src/lib.rs"
MIGRATION_V20 = REPO_ROOT / "dux-core/migrations/0020_automation_schedule_drafts.sql"
MIGRATION_V21 = REPO_ROOT / "dux-core/migrations/0021_automation_schedule_activation.sql"
MIGRATION_V22 = (
    REPO_ROOT / "dux-core/migrations/0022_automation_schedule_authoring_binding.sql"
)
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
SECURITY_DESIGN = REPO_ROOT / "SECURITY_DESIGN.md"
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


class AutomationScheduleBoundaryTests(unittest.TestCase):
    def test_ffi_v66_is_path_free_and_effect_dormant(self) -> None:
        ffi = read(FFI)
        self.assertIn("const FFI_CONTRACT_VERSION: u32 = 66;", ffi)
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
        security = read(SECURITY_DESIGN)
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
