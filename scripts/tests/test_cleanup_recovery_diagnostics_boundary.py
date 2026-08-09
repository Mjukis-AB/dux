#!/usr/bin/env python3
"""Freeze the read-only, path-free active-cleanup diagnostic boundary."""

from pathlib import Path
import re
import unittest


REPO = Path(__file__).resolve().parents[2]
PERSISTENCE = (
    REPO / "dux-core/src/persistence/cleanup_recovery_diagnostic.rs"
).read_text(encoding="utf-8")
HISTORY = (REPO / "dux-core/src/persistence/history.rs").read_text(encoding="utf-8")
CORE_ENGINE = (
    REPO / "dux-core/src/engine/cleanup_recovery_diagnostic.rs"
).read_text(encoding="utf-8")
FFI = (REPO / "dux-ffi/src/lib.rs").read_text(encoding="utf-8")
ENGINE_SERVICE = (
    REPO / "dux-macos/Dux/Services/EngineService.swift"
).read_text(encoding="utf-8")
APP_MODEL = (REPO / "dux-macos/Dux/App/AppModel.swift").read_text(encoding="utf-8")
SETTINGS = (
    REPO / "dux-macos/Dux/Views/DuxSettingsView.swift"
).read_text(encoding="utf-8")
CLI = "\n".join(
    path.read_text(encoding="utf-8")
    for path in sorted((REPO / "dux-cli/src").rglob("*.rs"))
)
PROJECT = (REPO / "dux-macos/project.yml").read_text(encoding="utf-8")
ADR = (
    REPO / "docs/adr/0011-diagnostic-only-cleanup-crash-debt-v1.md"
).read_text(encoding="utf-8")
ADR_INDEX = (REPO / "docs/adr/README.md").read_text(encoding="utf-8")
RETENTION = (REPO / "docs/RETENTION.md").read_text(encoding="utf-8")
SECURITY = (REPO / "SECURITY_DESIGN.md").read_text(encoding="utf-8")
ROADMAP = (REPO / "ROADMAP.md").read_text(encoding="utf-8")


def between(source: str, start: str, end: str) -> str:
    start_at = source.index(start)
    return source[start_at : source.index(end, start_at)]


class CleanupRecoveryDiagnosticsBoundaryTests(unittest.TestCase):
    def test_persistence_reader_has_no_probe_recovery_or_effect_edge(self) -> None:
        production = PERSISTENCE.split("#[cfg(test)]", 1)[0]
        for forbidden in (
            "probe_process_instance",
            "try_recover",
            "CleanupJournalClaim",
            "CleanupJournalLease",
            "cleanup.lock",
            "legacy_target_path",
            "root_path",
            "remove_file",
            "remove_dir",
            "trash_item",
            "evict",
        ):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, production)
        self.assertIn(
            "run_bounded_cleanup_recovery_diagnostic_query", production
        )
        diagnostic_budget = between(
            HISTORY,
            "pub(super) fn run_bounded_cleanup_recovery_diagnostic_query<T>",
            "\n}\n",
        )
        self.assertIn("2_000_000", diagnostic_budget)
        self.assertIn("Duration::from_secs(10)", diagnostic_budget)
        self.assertIn("LIMIT 65", production)
        self.assertIn("INDEXED BY cleanup_sessions_by_recovery", production)

    def test_public_records_are_aggregate_only(self) -> None:
        ffi_record = between(
            FFI,
            "pub struct CleanupRecoveryDiagnosticCensus {",
            "\n}\n",
        )
        fields = re.findall(r"pub ([a-z0-9_]+):", ffi_record)
        self.assertEqual(
            fields,
            [
                "record_version",
                "inspected_active_count",
                "running_count",
                "recovering_count",
                "same_host_current_boot_count",
                "same_host_prior_boot_count",
                "foreign_host_count",
                "stored_unproven_count",
                "current_context_unavailable_count",
                "has_more",
            ],
        )
        getters = re.findall(r"pub const fn ([a-z0-9_]+)\(", CORE_ENGINE)
        self.assertEqual(
            getters,
            [
                "inspected_active_count",
                "running_count",
                "recovering_count",
                "same_host_current_boot_count",
                "same_host_prior_boot_count",
                "foreign_host_count",
                "stored_unproven_count",
                "current_context_unavailable_count",
                "has_more",
            ],
        )

    def test_native_service_and_model_expose_observation_only(self) -> None:
        protocol = between(
            ENGINE_SERVICE,
            "protocol DuxCleanupRecoveryDiagnosticsServing",
            "protocol DuxDirectCargoEnrollmentServing",
        )
        declarations = re.findall(r"\bfunc ([A-Za-z0-9_]+)\(", protocol)
        self.assertEqual(
            set(declarations),
            {"loadCleanupRecoveryDiagnostics"},
        )

        model = between(
            APP_MODEL,
            "func loadCleanupRecoveryDiagnostics() async",
            "func invalidateCleanupHistoryOperations()",
        )
        for forbidden in (
            "tryRecover",
            "preparePermanentCleanup",
            "startConfirmedCleanup",
            "clearCleanupHistory",
            "retryCleanupEffect",
            "removeCleanupTarget",
        ):
            self.assertNotIn(forbidden, model)

    def test_settings_card_has_refresh_but_no_mutation_action(self) -> None:
        card = between(
            SETTINGS,
            "private func cleanupRecoveryDiagnosticsSettings",
            "private func cleanupDiagnosticsBar",
        )
        self.assertEqual(
            re.findall(r'Button\("([^"]+)"\)', card),
            ["Refresh cleanup diagnostics"],
        )
        self.assertIn("not a user-file inspection", card)
        self.assertIn("does not prove a process is", card)
        self.assertIn('unproven records remain "', card)
        self.assertIn('+ "non-executable.', card)
        calls = set(re.findall(r"model\.([A-Za-z0-9_]+)\(", card))
        self.assertEqual(calls, {"refreshCleanupRecoveryDiagnostics"})

    def test_no_cli_consumer_exists(self) -> None:
        self.assertNotIn("cleanup_recovery_diagnostic", CLI)
        self.assertNotIn("CleanupRecoveryDiagnostic", CLI)

    def test_v1_policy_is_accepted_and_consistent(self) -> None:
        self.assertIn("**Status:** Accepted", ADR)
        self.assertIn(
            "durable non-executability plus the ADR 0010 read-only diagnostic",
            ADR,
        )
        self.assertIn("Retained crash debt does block", ADR)
        self.assertIn("whole-app-data-reset", ADR)
        self.assertIn(
            "0011-diagnostic-only-cleanup-crash-debt-v1.md", ADR_INDEX
        )
        self.assertIn(
            "ADR 0011 accepts durable non-executability", RETENTION
        )
        self.assertIn(
            "ADR 0011 accepts that diagnostic plus durable non-executability",
            SECURITY,
        )
        self.assertIn(
            "diagnostic-only durable non-executability for v1", ROADMAP
        )

    def test_no_public_cleanup_debt_reconciliation_identifier_exists(self) -> None:
        public_sources = "\n".join((FFI, ENGINE_SERVICE, APP_MODEL, SETTINGS, CLI))
        for forbidden in (
            "reconcile_cleanup_recovery_session",
            "reconcileCleanupRecoverySession",
            "prepare_cleanup_recovery_reconciliation",
            "prepareCleanupRecoveryReconciliation",
            "resume_cleanup_recovery_session",
            "resumeCleanupRecoverySession",
            "retry_cleanup_recovery_session",
            "retryCleanupRecoverySession",
            "clear_cleanup_recovery_session",
            "clearCleanupRecoverySession",
            "dismiss_cleanup_recovery_session",
            "dismissCleanupRecoverySession",
        ):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, public_sources)

    def test_release_keeps_permanent_cleanup_compiled_out(self) -> None:
        app_configs = between(PROJECT, "      configs:\n", "\n\n  DuxTests:")
        debug = between(app_configs, "        Debug:\n", "        Release:\n")
        release = between(
            app_configs,
            "        Release:\n",
            "        CleanupQualification:\n",
        )
        qualification = app_configs.split("        CleanupQualification:\n", 1)[1]
        condition = "DUX_INTERNAL_PERMANENT_SAFE_CLEANUP"
        self.assertIn(condition, debug)
        self.assertNotIn(condition, release)
        self.assertIn(condition, qualification)
        self.assertIn("DUX_CLEANUP_QUALIFICATION", qualification)


if __name__ == "__main__":
    unittest.main()
