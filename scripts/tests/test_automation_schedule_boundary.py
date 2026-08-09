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


class AutomationScheduleBoundaryTests(unittest.TestCase):
    def test_ffi_v63_is_path_free_and_disabled_only(self) -> None:
        ffi = read(FFI)
        self.assertIn("const FFI_CONTRACT_VERSION: u32 = 63;", ffi)
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
                "create_automation_schedule_draft",
                "replace_automation_schedule_draft",
                "delete_automation_schedule_draft",
            }.issubset(method_names)
        )
        self.assertFalse(
            any(
                "automation" in name
                and any(word in name for word in ("enable", "run", "execute", "trigger"))
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

    def test_no_cli_or_native_scheduler_consumes_automation_drafts(self) -> None:
        roots = [REPO_ROOT / "dux-cli/src"]
        files = [path for root in roots for path in root.rglob("*.rs")]
        files += [
            REPO_ROOT / "dux-macos/Dux/App/AppRuntime.swift",
            REPO_ROOT / "dux-macos/Dux/Services/MaintenanceScheduler.swift",
            REPO_ROOT / "dux-macos/Dux/Services/CapacitySamplingScheduler.swift",
        ]
        for path in files:
            source = read(path)
            self.assertNotIn("AutomationScheduleDraft", source, str(path))
            self.assertNotIn("automation_schedule_draft", source, str(path))


if __name__ == "__main__":
    unittest.main()
