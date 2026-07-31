#!/usr/bin/env python3
"""Keep terminal admission out of operations joined by AppRuntime."""

from pathlib import Path
import re
import unittest


REPO = Path(__file__).resolve().parents[2]
SOURCE_ROOT = REPO / "dux-macos" / "Dux"
ALLOWED = {
    Path("App/AppRuntime.swift"),
    Path("App/DuxApp.swift"),
    Path("App/DuxAppDelegate.swift"),
}
FORBIDDEN = re.compile(
    r"\b(?:AppRuntime|DuxAppRuntimeServing|NativeRuntimeTerminalIntent|"
    r"NativeRuntimeTerminalCompletion|NativeRuntimeTerminalRequestResult)\b|"
    r"\b(?:quiesceForAppDataReset|requestTerminal)\s*\("
)


def without_comments(source: str) -> str:
    """Remove Swift comments; matches are identifiers, not string contents."""

    source = re.sub(r"/\*.*?\*/", "", source, flags=re.DOTALL)
    return re.sub(r"//[^\n]*", "", source)


class NativeRuntimeLayeringTests(unittest.TestCase):
    def test_only_top_level_ingress_references_terminal_runtime(self) -> None:
        violations: list[str] = []
        for path in sorted(SOURCE_ROOT.rglob("*.swift")):
            relative = path.relative_to(SOURCE_ROOT)
            if relative in ALLOWED:
                continue
            source = without_comments(path.read_text(encoding="utf-8"))
            for match in FORBIDDEN.finditer(source):
                line = source.count("\n", 0, match.start()) + 1
                violations.append(f"{relative}:{line}: {match.group(0)}")

        self.assertEqual(
            violations,
            [],
            "terminal runtime references must stay in top-level ingress; "
            "pass observations/events downward instead:\n" + "\n".join(violations),
        )


if __name__ == "__main__":
    unittest.main()
