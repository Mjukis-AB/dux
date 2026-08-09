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
        cli_sources = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted((REPO / "dux-cli/src").rglob("*.rs"))
        )
        self.assertNotIn("cleanup_recovery_diagnostic", cli_sources)
        self.assertNotIn("CleanupRecoveryDiagnostic", cli_sources)


if __name__ == "__main__":
    unittest.main()
