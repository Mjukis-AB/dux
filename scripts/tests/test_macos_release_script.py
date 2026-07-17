import os
import pathlib
import plistlib
import subprocess
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "dux-macos/scripts/release-notarized-dmg.sh"
ENTITLEMENTS = REPO_ROOT / "dux-macos/Config/Release.entitlements"


class MacOSReleaseScriptTests(unittest.TestCase):
    def run_script(self, *arguments: str, environment: dict[str, str] | None = None):
        clean_environment = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("DUX_")
        }
        if environment:
            clean_environment.update(environment)
        # DUX-DESTRUCTIVE: allow=test-release-script-spawn -- execute only the fixed release script, whose tested preflight exits before build or signing tools
        return subprocess.run(
            [str(SCRIPT), *arguments],
            check=False,
            capture_output=True,
            text=True,
            env=clean_environment,
        )

    def valid_environment(self) -> dict[str, str]:
        return {
            "DUX_VERSION": "1.2.3",
            "DUX_BUILD_NUMBER": "7",
            "DUX_BUNDLE_IDENTIFIER": "se.mjukis.dux",
            "DUX_TEAM_ID": "ABCDEFGHIJ",
            "DUX_SIGNING_IDENTITY":
                "Developer ID Application: DUX Test (ABCDEFGHIJ)",
            "DUX_NOTARYTOOL_PROFILE": "dux-notary",
        }

    def test_help_is_read_only_and_documents_immutable_output(self) -> None:
        result = self.run_script("--help")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("DUX_SIGNING_IDENTITY", result.stdout)
        self.assertIn("target/dux-macos-release/vX.Y.Z", result.stdout)
        self.assertIn("never accepts passwords", result.stdout)

    def test_missing_configuration_fails_before_tooling(self) -> None:
        result = self.run_script()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("DUX_VERSION", result.stderr)
        self.assertNotIn("release failed; diagnostics", result.stderr)

    def test_placeholder_bundle_identifier_is_rejected_before_tooling(self) -> None:
        environment = self.valid_environment()
        environment["DUX_BUNDLE_IDENTIFIER"] = "se.mjukis.dux.spike"

        result = self.run_script(environment=environment)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("temporary spike bundle identifier", result.stderr)

    def test_non_developer_id_identity_is_rejected_before_tooling(self) -> None:
        environment = self.valid_environment()
        environment["DUX_SIGNING_IDENTITY"] = "Apple Development: DUX Test (ABCDEFGHIJ)"

        result = self.run_script(environment=environment)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Developer ID Application identity", result.stderr)

    def test_option_shaped_notary_profile_is_rejected_before_tooling(self) -> None:
        environment = self.valid_environment()
        environment["DUX_NOTARYTOOL_PROFILE"] = "--apple-id"

        result = self.run_script(environment=environment)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("safe Keychain profile name", result.stderr)

    def test_release_entitlements_are_reviewed_and_empty(self) -> None:
        with ENTITLEMENTS.open("rb") as stream:
            self.assertEqual(plistlib.load(stream), {})

    def test_script_uses_keychain_credentials_and_explicit_signing(self) -> None:
        source = SCRIPT.read_text(encoding="utf-8")

        self.assertNotIn("--apple-id", source)
        self.assertNotIn("--password", source)
        self.assertNotIn("codesign --force --deep", source)
        self.assertIn("--keychain-profile \"$notary_profile\"", source)
        self.assertIn("grep -Eq 'flags=.*\\([^)]*runtime'", source)
        self.assertIn('grep -Fxq "TeamIdentifier=$team_id"', source)
        self.assertEqual(source.count("submit_and_require_accepted \"$"), 2)
        self.assertIn("xcrun stapler staple \"$staged_app\"", source)
        self.assertIn("xcrun stapler staple \"$staged_dmg\"", source)


if __name__ == "__main__":
    unittest.main()
