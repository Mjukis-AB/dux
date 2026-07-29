import pathlib
import re
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
PRODUCT_ROOT = REPO_ROOT / "dux-macos" / "Dux"
SWIFT_SETTER = "setPermanentCleanupEnabled"
RUST_SETTER = "set_permanent_cleanup_enabled"


def normalized_swift(value: str) -> str:
    return re.sub(r"\s+", "", value)


class PermanentCleanupConsentBoundaryTests(unittest.TestCase):
    def product_swift_sources(self) -> list[pathlib.Path]:
        return sorted(
            path
            for path in PRODUCT_ROOT.rglob("*.swift")
            if "Generated" not in path.relative_to(PRODUCT_ROOT).parts
        )

    def test_every_product_swift_setter_use_matches_the_reviewed_call_graph(self) -> None:
        call_pattern = re.compile(
            rf"\b(?P<receiver>[A-Za-z_][A-Za-z0-9_]*)\s*\.\s*"
            rf"{SWIFT_SETTER}\s*\((?P<arguments>.*?)\)",
            re.DOTALL,
        )
        declaration_pattern = re.compile(rf"\bfunc\s+{SWIFT_SETTER}\s*\(")
        token_pattern = re.compile(rf"\b{SWIFT_SETTER}\b")
        observed_calls: list[tuple[pathlib.PurePosixPath, str, str]] = []
        observed_declarations: list[pathlib.PurePosixPath] = []

        for path in self.product_swift_sources():
            relative = pathlib.PurePosixPath(path.relative_to(REPO_ROOT).as_posix())
            source = path.read_text(encoding="utf-8")
            calls = list(call_pattern.finditer(source))
            declarations = list(declaration_pattern.finditer(source))
            covered_tokens = {
                match.start() + match.group(0).index(SWIFT_SETTER)
                for match in calls
            } | {
                match.start() + match.group(0).index(SWIFT_SETTER)
                for match in declarations
            }
            all_tokens = {match.start() for match in token_pattern.finditer(source)}
            self.assertEqual(
                all_tokens,
                covered_tokens,
                f"unclassified permanent-cleanup setter use in {relative}",
            )
            observed_calls.extend(
                (
                    relative,
                    match.group("receiver"),
                    normalized_swift(match.group("arguments")),
                )
                for match in calls
            )
            observed_declarations.extend(relative for _ in declarations)

        self.assertCountEqual(
            observed_calls,
            [
                (
                    pathlib.PurePosixPath("dux-macos/Dux/App/AppModel.swift"),
                    "service",
                    "enabled",
                ),
                (
                    pathlib.PurePosixPath("dux-macos/Dux/Services/EngineService.swift"),
                    "engine",
                    "enabled:enabled",
                ),
                (
                    pathlib.PurePosixPath("dux-macos/Dux/Views/DuxSettingsView.swift"),
                    "model",
                    "false",
                ),
                (
                    pathlib.PurePosixPath("dux-macos/Dux/Views/DuxSettingsView.swift"),
                    "model",
                    "true,confirmation:phrase",
                ),
            ],
        )
        self.assertCountEqual(
            observed_declarations,
            [
                pathlib.PurePosixPath("dux-macos/Dux/App/AppModel.swift"),
                pathlib.PurePosixPath("dux-macos/Dux/Services/EngineService.swift"),
                pathlib.PurePosixPath("dux-macos/Dux/Services/EngineService.swift"),
                pathlib.PurePosixPath("dux-macos/Dux/Services/EngineService.swift"),
            ],
        )

    def test_rust_production_setter_chain_has_no_literal_consent_minter(self) -> None:
        production_roots = [REPO_ROOT / "dux-core" / "src", REPO_ROOT / "dux-ffi" / "src"]
        call_pattern = re.compile(
            rf"\.\s*{RUST_SETTER}\s*\((?P<arguments>.*?)\)",
            re.DOTALL,
        )
        declaration_pattern = re.compile(rf"\bfn\s+{RUST_SETTER}\s*\(")
        token_pattern = re.compile(rf"\b{RUST_SETTER}\b")
        observed_calls: list[tuple[pathlib.PurePosixPath, str]] = []
        observed_declarations: list[pathlib.PurePosixPath] = []

        for root in production_roots:
            for path in sorted(root.rglob("*.rs")):
                if path.name == "tests.rs" or path.name.endswith("_tests.rs"):
                    continue
                relative = pathlib.PurePosixPath(path.relative_to(REPO_ROOT).as_posix())
                source = path.read_text(encoding="utf-8")
                inline_tests = re.search(r"(?m)^#\[cfg\(test\)\]\s*\nmod tests\s*\{", source)
                if inline_tests is not None:
                    source = source[: inline_tests.start()]
                calls = list(call_pattern.finditer(source))
                declarations = list(declaration_pattern.finditer(source))
                covered_tokens = {
                    match.start() + match.group(0).index(RUST_SETTER)
                    for match in calls
                } | {
                    match.start() + match.group(0).index(RUST_SETTER)
                    for match in declarations
                }
                all_tokens = {match.start() for match in token_pattern.finditer(source)}
                self.assertEqual(
                    all_tokens,
                    covered_tokens,
                    f"unclassified permanent-cleanup setter use in {relative}",
                )
                observed_calls.extend(
                    (relative, re.sub(r"\s+", "", match.group("arguments")))
                    for match in calls
                )
                observed_declarations.extend(
                    relative for _ in declarations
                )

        self.assertCountEqual(
            observed_calls,
            [
                (
                    pathlib.PurePosixPath("dux-core/src/engine/registry.rs"),
                    "enabled",
                ),
                (
                    pathlib.PurePosixPath("dux-ffi/src/lib.rs"),
                    "enabled",
                ),
            ],
        )
        self.assertCountEqual(
            observed_declarations,
            [
                pathlib.PurePosixPath("dux-core/src/engine/registry.rs"),
                pathlib.PurePosixPath(
                    "dux-core/src/persistence/permanent_cleanup.rs"
                ),
                pathlib.PurePosixPath("dux-ffi/src/lib.rs"),
            ],
        )


if __name__ == "__main__":
    unittest.main()
