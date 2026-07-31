import os
import pathlib
import plistlib
import subprocess
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "dux-macos/scripts/release-notarized-dmg.sh"
BUNDLED_CLI_BUILDER = REPO_ROOT / "dux-macos/scripts/build-bundled-cli.sh"
BUNDLED_CLI_EMBEDDER = (
    REPO_ROOT / "dux-macos/scripts/embed-verified-bundled-cli.sh"
)
CLI_METADATA_FINALIZER = (
    REPO_ROOT / "dux-macos/scripts/finalize-bundled-cli-metadata.py"
)
ENTITLEMENTS = REPO_ROOT / "dux-macos/Config/Release.entitlements"
PROJECT_SPEC = REPO_ROOT / "dux-macos/project.yml"


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
        self.assertIn('--identifier "$cli_signing_identifier" "$cli"', source)
        self.assertIn("verify_signed_bundled_cli", source)
        self.assertIn("verify_development_bundled_cli", source)
        self.assertIn("verify_sparkle_shape", source)
        self.assertIn("sign_sparkle", source)
        self.assertIn(
            "--options runtime --preserve-metadata=entitlements",
            source,
        )
        self.assertNotIn("codesign --force --deep", source)
        self.assertIn("--mode rebind", source)
        self.assertLess(
            source.index('sign_bundled_cli "$staged_app"'),
            source.index('sign_nested_code "$staged_app" "$main_executable"'),
        )
        self.assertLess(
            source.index('sign_nested_code "$staged_app" "$main_executable"'),
            source.index(
                'codesign --force --sign "$signing_identity" --timestamp \\\n'
                '        --options runtime --generate-entitlement-der \\\n'
                '        --entitlements "$ENTITLEMENTS_PATH" "$staged_app"'
            ),
        )

    def test_universal_bundled_cli_builder_is_fixed_and_fail_closed(self) -> None:
        source = BUNDLED_CLI_BUILDER.read_text(encoding="utf-8")

        self.assertIn(
            'readonly OUTPUT_BINARY="$GENERATED_ROOT/dux-cli-bundled"',
            source,
        )
        self.assertIn(
            'readonly OUTPUT_METADATA="$GENERATED_ROOT/dux-cli-bundled-metadata.json"',
            source,
        )
        self.assertIn('readonly ARM64_TARGET="aarch64-apple-darwin"', source)
        self.assertIn('readonly X86_64_TARGET="x86_64-apple-darwin"', source)
        self.assertIn('export MACOSX_DEPLOYMENT_TARGET="14.0"', source)
        self.assertIn(
            'readonly DEVELOPMENT_SIGNING_IDENTIFIER="se.mjukis.dux.spike.cli.debug"',
            source,
        )
        self.assertEqual(source.count("--package dux-cli"), 1)
        self.assertIn('--target "$target"', source)
        self.assertIn("--release", source)
        self.assertIn(
            '"$staged_binary" __bundle-metadata >"$raw_metadata"',
            source,
        )
        self.assertIn('lipo -create "$arm64_binary" "$x86_64_binary"', source)
        self.assertIn('bash "$DEPLOYMENT_CHECK" "$staged_binary"', source)
        self.assertIn('codesign --force --sign -', source)
        self.assertIn(
            'codesign --verify --all-architectures --strict --verbose=2 "$binary"',
            source,
        )
        self.assertLess(
            source.index('sign_and_verify_development_binary "$staged_binary"'),
            source.index('binary_sha256="$(shasum -a 256 "$staged_binary"'),
        )
        self.assertIn('"$#" -eq 0', source)

    def test_cli_manifest_contract_and_app_resources_are_exact(self) -> None:
        finalizer = CLI_METADATA_FINALIZER.read_text(encoding="utf-8")
        embedder = BUNDLED_CLI_EMBEDDER.read_text(encoding="utf-8")
        project_spec = PROJECT_SPEC.read_text(encoding="utf-8")
        release = SCRIPT.read_text(encoding="utf-8")

        for key in [
            "record_version",
            "product",
            "version",
            "database_schema_version",
            "snapshot_format_version",
            "sha256",
            "architectures",
        ]:
            self.assertIn(f'"{key}"', finalizer)
        self.assertIn('"dux-cli"', finalizer)
        self.assertIn("postCompileScripts:", project_spec)
        self.assertIn(
            '"$SRCROOT/scripts/embed-verified-bundled-cli.sh"',
            project_spec,
        )
        self.assertIn("$(SRCROOT)/Generated/dux-cli-bundled", project_spec)
        self.assertIn(
            "$(SRCROOT)/Generated/dux-cli-bundled-metadata.json",
            project_spec,
        )
        self.assertIn("basedOnDependencyAnalysis: false", project_spec)
        self.assertIn('cp -p "$SOURCE_BINARY" "$staged_binary"', embedder)
        self.assertIn(
            'verify_pair "$staged_binary" "$staged_metadata"',
            embedder,
        )
        self.assertIn(
            'verify_pair "$destination_binary" "$destination_metadata"',
            embedder,
        )
        self.assertIn("verify_bundled_cli_payload \"$app\"", release)
        self.assertIn("compare_bundled_cli_payloads", release)
        self.assertIn("cli_database_schema_version=", release)
        self.assertIn("cli_snapshot_format_version=", release)
        self.assertIn("cli_sha256=", release)

    def test_permanent_cleanup_ui_is_internal_debug_only(self) -> None:
        source = SCRIPT.read_text(encoding="utf-8")
        project_spec = PROJECT_SPEC.read_text(encoding="utf-8")

        self.assertIn(
            'verify_permanent_cleanup_feature_gate',
            source,
        )
        self.assertIn(
            'permanent cleanup UI must not be compiled into public Release builds',
            source,
        )
        self.assertIn(
            'Debug:\n'
            '          SWIFT_ACTIVE_COMPILATION_CONDITIONS: '
            '"$(inherited) DUX_INTERNAL_PERMANENT_SAFE_CLEANUP"\n'
            '        Release:',
            project_spec,
        )


if __name__ == "__main__":
    unittest.main()
