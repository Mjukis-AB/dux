from __future__ import annotations

import pathlib
import stat
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
HARNESS = REPO_ROOT / "scripts/qualify-macos-apfs-boundaries.sh"
RUST_TESTS = REPO_ROOT / "dux-core/src/path_validation/protected/tests.rs"
DOCUMENTATION = REPO_ROOT / "docs/testing/macos-apfs-boundary-qualification.md"


class MacOSAPFSBoundaryQualificationPolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.harness = HARNESS.read_text(encoding="utf-8")
        self.rust_tests = RUST_TESTS.read_text(encoding="utf-8")
        self.documentation = DOCUMENTATION.read_text(encoding="utf-8")

    def test_harness_is_executable_nondestructive_and_exactly_scoped(self) -> None:
        self.assertTrue(HARNESS.stat().st_mode & stat.S_IXUSR)
        self.assertIn('[[ "$(uname -s)" != "Darwin" ]]', self.harness)
        self.assertIn("if (( $# != 0 )); then", self.harness)
        self.assertIn("pwd.getpwuid(os.geteuid()).pw_dir", self.harness)
        self.assertIn(
            'mktemp -d "$ACCOUNT_HOME/.dux-apfs-boundary-qualification.XXXXXX"',
            self.harness,
        )
        self.assertIn("hdiutil create", self.harness)
        self.assertIn("-fs APFS", self.harness)
        self.assertIn('hdiutil attach \\\n', self.harness)
        self.assertIn('-mountpoint "$MOUNT_POINT"', self.harness)
        self.assertIn('hdiutil detach "$MOUNT_POINT"', self.harness)
        self.assertNotIn("hdiutil detach -force", self.harness)
        self.assertNotIn("sudo", self.harness)
        self.assertNotIn("cargo run", self.harness)
        self.assertNotIn("DUX_INTERNAL_PERMANENT_SAFE_CLEANUP", self.harness)
        self.assertNotIn("CleanupQualification", self.harness)
        self.assertIn("trap cleanup EXIT", self.harness)
        self.assertIn("trap 'exit 130' INT", self.harness)
        self.assertIn("trap 'exit 143' TERM", self.harness)
        self.assertIn("trap 'exit 129' HUP", self.harness)
        self.assertIn("exec env \\\n", self.harness)
        self.assertIn(
            'cargo test --quiet --locked -p dux-core --lib "$TEST_NAME" -- --exact --ignored --nocapture',
            self.harness,
        )
        self.assertEqual(self.harness.count("rm -rf -- \"$FIXTURE_ROOT\""), 1)
        self.assertIn("allow=qualification-apfs-fixture-cleanup", self.harness)
        self.assertIn(
            'detach_required=0\nremove_fixture\n\necho "APFS boundary qualification passed."',
            self.harness,
        )

    def test_ignored_test_requires_real_fixture_and_proves_every_refusal(self) -> None:
        test_name = (
            "macos_apfs_boundary_qualification_fails_closed_after_nested_mount"
        )
        self.assertEqual(self.rust_tests.count(f"fn {test_name}()"), 1)
        self.assertIn(
            '#[ignore = "requires scripts/qualify-macos-apfs-boundaries.sh and a disposable APFS image"]',
            self.rust_tests,
        )
        for required in (
            "DUX_APFS_QUALIFICATION_MOUNT_POINT",
            "DUX_APFS_QUALIFICATION_CONTROL_DIRECTORY",
            "/System/Volumes/Data",
            "TrustedHomeMountError::Changed",
            "TrustedHomeMountError::DifferentMount",
            "CanonicalPathError::CrossVolume",
            "CanonicalPathError::SymlinkOrReparsePoint",
        ):
            self.assertIn(required, self.rust_tests)
        self.assertIn("from_secs(30)", self.rust_tests)
        self.assertNotIn("skipping APFS", self.rust_tests)

    def test_documentation_refuses_to_count_skips_or_effects_as_evidence(self) -> None:
        for required in (
            "does not invoke a cleanup planner, journal, executor, FFI, or Swift",
            "A skipped or unavailable fixture is not passing evidence",
            "APFS boundary qualification passed.",
            "No user-selected path",
            "macOS 14",
        ):
            self.assertIn(required, self.documentation)


if __name__ == "__main__":
    unittest.main()
