#!/usr/bin/env python3

from pathlib import Path
import json
import re
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
CORE_DOMAIN = REPO_ROOT / "dux-core/src/domain/automation_schedule.rs"
CORE_ELIGIBILITY = REPO_ROOT / "dux-core/src/domain/automation_eligibility.rs"
CORE_ENGINE = REPO_ROOT / "dux-core/src/engine/automation.rs"
CORE_REGISTRY = REPO_ROOT / "dux-core/src/engine/registry.rs"
CORE_STORE = REPO_ROOT / "dux-core/src/persistence/automation_schedule.rs"
FFI = REPO_ROOT / "dux-ffi/src/lib.rs"
MIGRATION = REPO_ROOT / "dux-core/migrations/0020_automation_schedule_drafts.sql"
CATALOG = REPO_ROOT / "dux-core/catalogs/candidate-rules-v1.json"
MAINTENANCE_SCHEDULER = (
    REPO_ROOT / "dux-macos/Dux/Services/MaintenanceScheduler.swift"
)
ADR = REPO_ROOT / "docs/adr/0014-automation-clock-wake-and-missed-run-semantics.md"
SECURITY_REVIEW = (
    REPO_ROOT / "docs/security-reviews/m8-automation-scheduler-wake.md"
)
SECURITY_DESIGN = REPO_ROOT / "SECURITY_DESIGN.md"
APP_RUNTIME = REPO_ROOT / "dux-macos/Dux/App/AppRuntime.swift"

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
    def test_ffi_v64_is_path_free_and_disabled_only(self) -> None:
        ffi = read(FFI)
        self.assertIn("const FFI_CONTRACT_VERSION: u32 = 64;", ffi)
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
                "global_enabled",
                "execution_available",
                "eligible_rule_count",
                "disabled_drafts",
                "draft_eligibility",
            ],
        )
        self.assertEqual(
            rust_struct_fields(ffi, "AutomationScheduleDraftEligibilityAssessment"),
            [
                "record_version",
                "policy_revision",
                "schedule_id",
                "draft_revision",
                "status",
                "included_statically_eligible_rule_count",
                "reasons",
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
        draft_fields = rust_struct_fields(ffi, "AutomationScheduleDraft")
        for forbidden in (
            "path",
            "node_id",
            "candidate_id",
            "plan_id",
            "approval",
            "task",
            "next_run",
            "last_run",
            "trigger",
        ):
            self.assertNotIn(forbidden, draft_fields)
        self.assertIn("enabled: false", ffi)
        self.assertIn("global_enabled: false", ffi)
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
                "create_automation_schedule_draft",
                "replace_automation_schedule_draft",
                "delete_automation_schedule_draft",
            }.issubset(method_names)
        )
        self.assertFalse(
            any(
                "automation" in name
                and any(
                    word in name
                    for word in (
                        "enable",
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
        eligibility = read(CORE_ELIGIBILITY)
        engine = read(CORE_ENGINE)
        store = read(CORE_STORE)
        production_store = store.split("#[cfg(test)]\nmod tests", 1)[0]
        combined = "\n".join((domain, eligibility, engine))
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
        self.assertNotRegex(
            combined,
            r"\b(enabled|next_run|last_run|trigger_source)\s*:",
        )
        self.assertIn("state = 'disabled_draft'", production_store)
        self.assertNotIn("state = 'enabled'", production_store)

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
                "create_automation_schedule_draft",
                "replace_automation_schedule_draft",
                "delete_automation_schedule_draft",
            },
        )

    def test_core_eligibility_is_complete_fresh_and_observation_only(self) -> None:
        eligibility = read(CORE_ELIGIBILITY)
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

    def test_schema_discards_unadmitted_rows_and_cannot_store_enabled_state(self) -> None:
        migration = read(MIGRATION)
        self.assertIn("DUX-DESTRUCTIVE:", migration)
        self.assertIn("DROP TABLE schedules;", migration)
        self.assertIn("CHECK (state = 'disabled_draft')", migration)
        self.assertNotIn("next_run_unix_ms", migration)
        self.assertNotIn("last_run_unix_ms", migration)
        self.assertIn("ON DELETE CASCADE", migration)

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
            self.assertNotIn("AutomationScheduleSuggestion", source, str(path))
            self.assertNotIn("automation_schedule_suggestion", source, str(path))

    def test_discovered_automation_timing_sources_are_effect_dormant(self) -> None:
        forbidden_identifiers = (
            "AutomationScheduleDraft",
            "automation_schedule_draft",
            "AutomationScheduleSuggestion",
            "automation_schedule_suggestion",
            "AutomationEligibilityAssessment",
            "AutomationScheduleDraftEligibilityAssessment",
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
        review = read(SECURITY_REVIEW)
        security = read(SECURITY_DESIGN)
        self.assertIn("**Status:** Accepted", adr)
        self.assertIn("two deliberately separate clock domains", adr)
        self.assertIn("globally at most one path-free request", adr)
        self.assertIn("production source of enabled/due schedules is empty", adr)
        self.assertIn("Explicitly deferred decisions", adr)
        self.assertIn("The Milestone 8 scheduler task remains open.", review)
        self.assertIn(
            "docs/security-reviews/m8-automation-scheduler-wake.md",
            security,
        )


if __name__ == "__main__":
    unittest.main()
