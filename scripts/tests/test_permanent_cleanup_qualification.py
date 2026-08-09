import pathlib
import re
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"
HARNESS = REPO_ROOT / "scripts" / "test_ffi_rust_target_cleanup.sh"
FFI_SOURCE = REPO_ROOT / "dux-ffi" / "src" / "lib.rs"

SUCCESS_TEST = (
    "tests::rust_target_cleanup_is_engine_bound_consume_once_path_free_"
    "and_history_correlated"
)
REFUSAL_TEST = (
    "tests::rust_target_cleanup_refusal_is_one_shot_and_close_drains_"
    "start_operation"
)


class PermanentCleanupQualificationTests(unittest.TestCase):
    def test_macos_ci_runs_the_exact_harness_as_a_required_bounded_step(self) -> None:
        source = WORKFLOW.read_text(encoding="utf-8")
        step = re.search(
            r"(?ms)^      - name: Qualify permanent-safe cleanup through the FFI "
            r"boundary\n(?P<body>(?:        .+\n)+)",
            source,
        )
        self.assertIsNotNone(step)
        body = step.group("body")
        self.assertIn("timeout-minutes: 15", body)
        self.assertIn("run: ./scripts/test_ffi_rust_target_cleanup.sh", body)
        self.assertNotIn("continue-on-error", body)

        job_start = source.index("  macos-ffi-app:")
        job_end = source.index("\n  lint:", job_start)
        job = source[job_start:job_end]
        self.assertEqual(
            job.count("run: ./scripts/test_ffi_rust_target_cleanup.sh"),
            1,
        )

    def test_harness_runs_only_the_two_reviewed_ignored_tests_once_each(self) -> None:
        harness = HARNESS.read_text(encoding="utf-8")
        self.assertIn("set -euo pipefail", harness)
        self.assertIn(
            "cargo test --color never -p dux-ffi --no-run --locked",
            harness,
        )
        self.assertEqual(harness.count("--ignored"), 2)
        self.assertEqual(harness.count("--exact"), 2)
        self.assertEqual(harness.count(f"    {SUCCESS_TEST} 2>&1"), 1)
        self.assertEqual(harness.count(f"    {REFUSAL_TEST} 2>&1"), 1)
        self.assertIn("expected exactly one Cargo-reported dux-ffi test binary", harness)
        self.assertIn("the exact FFI cleanup success regression did not run and pass once", harness)
        self.assertIn("the exact FFI cleanup refusal regression did not run and pass once", harness)

    def test_destructive_fixtures_stay_out_of_the_ordinary_parallel_lane(self) -> None:
        source = FFI_SOURCE.read_text(encoding="utf-8")
        for test_name in [SUCCESS_TEST.removeprefix("tests::"), REFUSAL_TEST.removeprefix("tests::")]:
            pattern = re.compile(
                r'#\[ignore = "requires no active cargo/rustc process; run '
                r'scripts/test_ffi_rust_target_cleanup\.sh"\]\s*'
                rf"fn {re.escape(test_name)}\(\)",
                re.MULTILINE,
            )
            self.assertRegex(source, pattern)

    def test_workflow_color_environment_cannot_change_discovery_output(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")
        harness = HARNESS.read_text(encoding="utf-8")
        self.assertIn("CARGO_TERM_COLOR: always", workflow)
        self.assertIn("cargo test --color never", harness)


if __name__ == "__main__":
    unittest.main()
