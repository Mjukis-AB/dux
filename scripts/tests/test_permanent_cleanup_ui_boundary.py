#!/usr/bin/env python3
"""Freeze the path-free, fail-closed native cleanup presentation boundary."""

from pathlib import Path
import unittest


REPO = Path(__file__).resolve().parents[2]
VIEWS = REPO / "dux-macos" / "Dux" / "Views"
EXPLORER = (VIEWS / "ExplorerView.swift").read_text(encoding="utf-8")
SNAPSHOT = (VIEWS / "ExplorerSnapshotBrowserView.swift").read_text(encoding="utf-8")
STATUS = (VIEWS / "ExplorerRustTargetCleanupStatusView.swift").read_text(
    encoding="utf-8"
)


class PermanentCleanupUIBoundaryTests(unittest.TestCase):
    def test_cleanup_status_is_global_and_path_free(self) -> None:
        self.assertEqual(EXPLORER.count("ExplorerRustTargetCleanupStatusView("), 1)
        self.assertLess(
            EXPLORER.index("ExplorerRustTargetCleanupStatusView("),
            EXPLORER.index("NavigationSplitView"),
        )
        self.assertNotIn("ExplorerRustTargetCleanupStatusView", SNAPSHOT)
        self.assertNotIn("rustTargetCleanupBanner", SNAPSHOT)
        for forbidden in (
            "ExplorerSnapshotBrowserModel",
            "@Bindable",
            "target.display",
            "target.encodedBytes",
            "startConfirmedRustTargetCleanup",
            "makeRustTargetCleanupConfirmation",
        ):
            self.assertNotIn(forbidden, STATUS)
        for required in (
            "let cleanupState: ExplorerRustTargetCleanupState",
            "let historyFinalizationInProgress: Bool",
            "let cancelCleanup: () async -> Void",
            "let dismissResult: () async -> Void",
        ):
            self.assertIn(required, STATUS)

    def test_cleanup_action_rechecks_loaded_stored_consent(self) -> None:
        self.assertIn(
            "switch permanentCleanupAvailability",
            SNAPSHOT,
        )
        self.assertIn(
            "guard permanentCleanupAvailability == .enabled else",
            SNAPSHOT,
        )
        self.assertIn(
            "await browser.startConfirmedRustTargetCleanup(confirmation)",
            SNAPSHOT,
        )
        self.assertLess(
            SNAPSHOT.index("guard permanentCleanupAvailability == .enabled else"),
            SNAPSHOT.index("await browser.startConfirmedRustTargetCleanup(confirmation)"),
        )

    def test_terminal_status_offers_history_without_retry(self) -> None:
        self.assertIn("View this session in Cleanup History", STATUS)
        self.assertIn("Open Cleanup History", STATUS)
        self.assertIn("case .cancelled:", STATUS)
        self.assertIn("if let result = poll.result", STATUS)
        self.assertNotIn('Button("Retry', STATUS)
        self.assertNotIn("retryRustTargetCleanup", STATUS)


if __name__ == "__main__":
    unittest.main()
