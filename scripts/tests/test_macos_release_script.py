import base64
import hashlib
import json
import os
import pathlib
import plistlib
import re
import shlex
import subprocess
import sys
import tempfile
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
GIT_ATTRIBUTES = REPO_ROOT / ".gitattributes"
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
INFO_PLIST = REPO_ROOT / "dux-macos/Dux/Info.plist"
SPARKLE_UPDATE_CONTROLLER = (
    REPO_ROOT / "dux-macos/Dux/App/SparkleUpdateController.swift"
)
SPARKLE_PUBLIC_VERIFIER = (
    REPO_ROOT / "dux-macos/scripts/verify-sparkle-ed25519-signature.swift"
)
RELEASE_OPERATIONS = REPO_ROOT / "docs/MACOS_RELEASE_OPERATIONS.md"
PRODUCTION_IDENTITY = REPO_ROOT / "dux-macos/Config/ProductionIdentity.json"
PACKAGE_RESOLVED = (
    REPO_ROOT
    / "dux-macos/Dux.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved"
)

REVIEWED_RELEASE_SCRIPT_SHA256 = (
    "30d1f5466a6abd9ac3069e90d9e5e587ab6f5396e1a06711026bb5b29006c480"
)


class MacOSReleaseScriptTests(unittest.TestCase):
    def test_complete_release_script_matches_reviewed_digest(self) -> None:
        self.assertIn(
            "dux-macos/scripts/release-notarized-dmg.sh text eol=lf",
            GIT_ATTRIBUTES.read_text(encoding="utf-8").splitlines(),
            "the byte-sealed release script must retain LF on every checkout",
        )
        self.assertEqual(
            hashlib.sha256(SCRIPT.read_bytes()).hexdigest(),
            REVIEWED_RELEASE_SCRIPT_SHA256,
            "the security-sensitive release script changed; review the complete "
            "script and update its pinned digest explicitly",
        )

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
            "DUX_TEAM_ID": "SMQ3E8Y57T",
            "DUX_SIGNING_IDENTITY":
                "Developer ID Application: MJUKIS AB (SMQ3E8Y57T)",
            "DUX_NOTARYTOOL_PROFILE": "dux-notary",
            "DUX_RELEASE_COMMIT": "0123456789abcdef0123456789abcdef01234567",
        }

    def test_help_is_read_only_and_documents_immutable_output(self) -> None:
        result = self.run_script("--help")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("DUX_SIGNING_IDENTITY", result.stdout)
        self.assertIn("target/dux-macos-release/vX.Y.Z", result.stdout)
        self.assertIn("never accepts passwords", result.stdout)
        self.assertIn("DUX_NOTARYTOOL_KEYCHAIN", result.stdout)
        self.assertIn("--verify-prepared", result.stdout)

    def test_missing_configuration_fails_before_tooling(self) -> None:
        result = self.run_script("--prepare", "/private/tmp/dux-missing-config")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("DUX_VERSION", result.stderr)
        self.assertNotIn("release failed; diagnostics", result.stderr)

    def test_placeholder_bundle_identifier_is_rejected_before_tooling(self) -> None:
        environment = self.valid_environment()
        environment["DUX_BUNDLE_IDENTIFIER"] = "se.mjukis.dux.spike"

        result = self.run_script(
            "--prepare", "/private/tmp/dux-invalid-bundle", environment=environment
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("temporary spike bundle identifier", result.stderr)

    def test_non_developer_id_identity_is_rejected_before_tooling(self) -> None:
        environment = self.valid_environment()
        environment["DUX_SIGNING_IDENTITY"] = (
            "Apple Development: DUX Test (SMQ3E8Y57T)"
        )

        result = self.run_script(
            "--sign-prepared", "/private/tmp/dux-missing-envelope", environment=environment
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Developer ID Application identity", result.stderr)

    def test_option_shaped_notary_profile_is_rejected_before_tooling(self) -> None:
        environment = self.valid_environment()
        environment["DUX_NOTARYTOOL_PROFILE"] = "--apple-id"

        result = self.run_script(
            "--sign-prepared", "/private/tmp/dux-missing-envelope", environment=environment
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("safe Keychain profile name", result.stderr)

    def test_notary_keychain_must_be_an_existing_absolute_regular_file(self) -> None:
        for path in ["relative.keychain-db", "/definitely/missing/dux.keychain-db"]:
            environment = self.valid_environment()
            environment["DUX_NOTARYTOOL_KEYCHAIN"] = path
            with self.subTest(path=path):
                result = self.run_script(
                    "--sign-prepared",
                    "/private/tmp/dux-missing-envelope",
                    environment=environment,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("DUX_NOTARYTOOL_KEYCHAIN", result.stderr)

    def test_release_entitlements_are_reviewed_and_empty(self) -> None:
        with ENTITLEMENTS.open("rb") as stream:
            self.assertEqual(plistlib.load(stream), {})

    def test_production_identity_is_exact_public_and_cross_checked(self) -> None:
        identity = json.loads(PRODUCTION_IDENTITY.read_text(encoding="utf-8"))
        self.assertEqual(
            set(identity),
            {
                "record_version",
                "bundle_identifier",
                "team_id",
                "developer_id_application",
                "designated_requirement",
                "sparkle_keychain_account",
                "sparkle_public_ed_key",
            },
        )
        self.assertEqual(identity["record_version"], 1)
        self.assertEqual(identity["bundle_identifier"], "se.mjukis.dux")
        self.assertEqual(identity["team_id"], "SMQ3E8Y57T")
        self.assertEqual(
            identity["developer_id_application"],
            "Developer ID Application: MJUKIS AB (SMQ3E8Y57T)",
        )
        self.assertEqual(identity["sparkle_keychain_account"], "se.mjukis.dux")
        self.assertEqual(len(base64.b64decode(identity["sparkle_public_ed_key"])), 32)

        project = PROJECT_SPEC.read_text(encoding="utf-8")
        with INFO_PLIST.open("rb") as stream:
            info = plistlib.load(stream)
        release = SCRIPT.read_text(encoding="utf-8")
        controller = SPARKLE_UPDATE_CONTROLLER.read_text(encoding="utf-8")
        for value in [
            identity["bundle_identifier"],
            identity["team_id"],
        ]:
            self.assertIn(value, project)
        self.assertEqual(
            info["SUPublicEDKey"],
            identity["sparkle_public_ed_key"],
        )
        self.assertIs(info["SURequireSignedFeed"], True)
        self.assertIs(info["SUVerifyUpdateBeforeExtraction"], True)
        self.assertEqual(info["SUSignedFeedFailureExpirationInterval"], 0)
        self.assertNotIn("SUEnableAutomaticChecks", info)
        self.assertNotIn("SUAutomaticallyUpdate", info)
        package_resolution = json.loads(PACKAGE_RESOLVED.read_text(encoding="utf-8"))
        self.assertEqual(
            package_resolution["pins"],
            [
                {
                    "identity": "sparkle",
                    "kind": "remoteSourceControl",
                    "location": "https://github.com/sparkle-project/Sparkle",
                    "state": {
                        "revision": "79bc9e872948e47877e76f194cb0c8e0412b0b90",
                        "version": "2.9.5",
                    },
                }
            ],
        )
        for value in [
            identity["bundle_identifier"],
            identity["team_id"],
            identity["developer_id_application"],
            identity["designated_requirement"],
        ]:
            self.assertIn(value, release)
        for value in [
            identity["bundle_identifier"],
            identity["sparkle_public_ed_key"],
        ]:
            self.assertIn(value, controller)
        self.assertNotIn("SUFeedURL", project)
        self.assertNotIn("SUFeedURL", info)

    def test_release_preflight_rejects_every_frozen_identity_drift(self) -> None:
        changes = {
            "bundle": {"DUX_BUNDLE_IDENTIFIER": "se.mjukis.dux.other"},
            "team": {
                "DUX_TEAM_ID": "ABCDEFGHIJ",
                "DUX_SIGNING_IDENTITY":
                    "Developer ID Application: MJUKIS AB (ABCDEFGHIJ)",
            },
            "identity": {
                "DUX_SIGNING_IDENTITY":
                    "Developer ID Application: OTHER (SMQ3E8Y57T)",
            },
        }
        for key, environment_changes in changes.items():
            environment = self.valid_environment()
            environment.update(environment_changes)
            with self.subTest(key=key):
                result = self.run_script(
                    "--sign-prepared",
                    "/private/tmp/dux-missing-envelope",
                    environment=environment,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("frozen production identity", result.stderr)

    def test_script_uses_keychain_credentials_and_explicit_signing(self) -> None:
        source = SCRIPT.read_text(encoding="utf-8")

        self.assertNotIn("--apple-id", source)
        self.assertNotIn("--password", source)
        self.assertNotIn("codesign --force --deep", source)
        self.assertIn("--keychain-profile \"$notary_profile\"", source)
        self.assertIn('run_notarytool', source)
        self.assertIn('--keychain "$notary_keychain"', source)
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
        self.assertIn("actual_designated_requirement", source)
        self.assertIn(
            '[[ "$actual_designated_requirement" == '
            '"$PRODUCTION_DESIGNATED_REQUIREMENT" ]]',
            source,
        )
        self.assertIn('"signing_identity=$signing_identity"', source)
        self.assertIn(
            '"designated_requirement=$PRODUCTION_DESIGNATED_REQUIREMENT"',
            source,
        )
        self.assertIn(
            "--options runtime --preserve-metadata=entitlements",
            source,
        )
        self.assertNotIn("codesign --force --deep", source)
        self.assertIn("--prepare /absolute/path/to/new-envelope", source)
        self.assertIn("--sign-prepared /absolute/path/to/envelope", source)
        self.assertIn('PREPARED_ARCHIVE_NAME="DUX-unsigned.app.zip"', source)
        self.assertIn('jq -c --arg sha256 "$signed_sha256"', source)
        self.assertIn("-disableAutomaticPackageResolution", source)
        self.assertIn("-onlyUsePackageVersionsFromResolvedFile", source)
        self.assertIn('host_arch="$(uname -m)"', source)
        self.assertIn('"xcode_version=$xcode_version"', source)
        self.assertIn('"xcode_build=$xcode_build"', source)
        self.assertIn('"sparkle_version=2.9.5"', source)
        self.assertIn("verify_sparkle_update_policy", source)
        self.assertEqual(source.count("verify_clean_tagged_source"), 3)
        final_source_check = source.rindex("verify_clean_tagged_source")
        self.assertLess(
            source.index('compare_bundled_cli_payloads "$debug_app" "$unsigned_app"'),
            final_source_check,
        )
        self.assertLess(
            final_source_check,
            source.index('ditto -c -k --keepParent "$unsigned_app" "$archive"'),
        )
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

    def test_signing_phase_cannot_execute_build_test_or_bundled_app_code(self) -> None:
        source = SCRIPT.read_text(encoding="utf-8")
        signing = source[
            source.index("sign_prepared_release() {"):
            source.index("main() {")
        ]
        for forbidden in [
            "cargo ",
            "python3",
            "xcodebuild",
            "xcodegen",
            "generate-bindings.sh",
            "check_destructive_calls.py",
            "__bundle-metadata",
            'bash "$DEPLOYMENT_CHECK"',
        ]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, signing)
        self.assertIn('verify_prepared_envelope "$prepared_path" "$expected_commit"', signing)
        self.assertLess(
            signing.index('verify_prepared_envelope "$prepared_path" "$expected_commit"'),
            signing.index("security find-identity"),
        )
        self.assertIn('ditto -x -k "$prepared_path/$PREPARED_ARCHIVE_NAME"', signing)
        self.assertIn('assert_exact_code_inventory "$app" "$main_executable"', source)
        self.assertIn("app must contain exactly seven reviewed Mach-O executables", source)
        self.assertIn("app contains an unreviewed Mach-O executable", source)

    def test_public_prepared_verification_cannot_build_or_execute_project_code(self) -> None:
        source = SCRIPT.read_text(encoding="utf-8")
        verification = source[
            source.index("verify_prepared_release() {"):
            source.index("sign_prepared_release() {")
        ]
        for forbidden in [
            "cargo ",
            "python3",
            "xcodebuild",
            "xcodegen",
            "generate-bindings.sh",
            "check_destructive_calls.py",
            "__bundle-metadata",
            'bash "$DEPLOYMENT_CHECK"',
        ]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, verification)
        self.assertIn(
            'verify_prepared_envelope "$prepared_path" "$expected_commit"',
            verification,
        )
        self.assertIn('verify_prepared_app_shape "$inspection_path/DUX.app"', verification)

    def test_prepared_verification_and_signing_transitive_helpers_execute_no_project_code(self) -> None:
        source = SCRIPT.read_text(encoding="utf-8")
        definitions = list(
            re.finditer(r"(?m)^([a-z][a-z0-9_]*)\(\) \{\n", source)
        )
        bodies: dict[str, str] = {}
        for index, definition in enumerate(definitions):
            end = definitions[index + 1].start() if index + 1 < len(definitions) else len(source)
            bodies[definition.group(1)] = source[definition.end():end]

        graph: dict[str, set[str]] = {}
        for name, body in bodies.items():
            lexer = shlex.shlex(body, posix=True, punctuation_chars=";&|()")
            lexer.whitespace_split = True
            lexer.commenters = "#"
            tokens = set(lexer)
            graph[name] = {
                candidate
                for candidate in bodies
                if candidate != name
                and candidate in tokens
            }

        def closure(root: str) -> set[str]:
            found: set[str] = set()
            pending = [root]
            while pending:
                name = pending.pop()
                if name in found:
                    continue
                found.add(name)
                pending.extend(graph[name] - found)
            return found

        reviewed = closure("verify_prepared_release") | closure("sign_prepared_release")
        forbidden_functions = {
            "compare_app_layouts",
            "compare_bundled_cli_payloads",
            "prepare_release",
            "verify_bundled_cli_payload",
            "verify_clean_tagged_source",
            "verify_permanent_cleanup_feature_gate",
            "verify_unsigned_app_shape",
        }
        self.assertTrue(reviewed.isdisjoint(forbidden_functions), reviewed)
        reviewed_source = "\n".join(bodies[name] for name in sorted(reviewed))
        for forbidden in [
            "cargo ",
            "python3",
            "xcodebuild",
            "xcodegen",
            "generate-bindings.sh",
            "check_destructive_calls.py",
            "__bundle-metadata",
            'bash "$DEPLOYMENT_CHECK"',
        ]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, reviewed_source)
        self.assertNotRegex(
            reviewed_source,
            r'(?m)^\s*"\$(?:cli|executable|main_executable)"(?:\s|$)',
        )

    def test_prepared_envelope_and_shipped_sparkle_policy_are_fail_closed(self) -> None:
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("prepared envelope must contain exactly three files", source)
        self.assertIn("prepared manifest must contain exactly thirteen lines", source)
        self.assertIn('shasum -a 256 -c "$PREPARED_CHECKSUM_NAME"', source)
        self.assertIn('plutil -extract SUPublicEDKey raw', source)
        self.assertIn('plutil -extract SURequireSignedFeed raw', source)
        self.assertIn('plutil -extract SUVerifyUpdateBeforeExtraction raw', source)
        self.assertIn('plutil -extract SUSignedFeedFailureExpirationInterval raw', source)
        self.assertIn('plutil -extract SUFeedURL raw', source)
        self.assertGreaterEqual(source.count('verify_sparkle_update_policy "$app"'), 3)

    def test_custody_tools_are_pinned_and_canary_verifier_is_public_only(self) -> None:
        operations = RELEASE_OPERATIONS.read_text(encoding="utf-8")
        verifier = SPARKLE_PUBLIC_VERIFIER.read_text(encoding="utf-8")
        self.assertIn(
            "34b9b2071f3de0012eca3faa3a9290bb94e62131e9a74f6dc91514a000097a6c",
            operations,
        )
        self.assertIn("umask 077", operations)
        self.assertIn('stat -f %Lp "$destination"', operations)
        self.assertIn('stat -f %z "$destination"', operations)
        self.assertIn('stat -f %z "$seed_path"', operations)
        self.assertIn("len(decoded) != 32", operations)
        self.assertIn("os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC", operations)
        self.assertIn("status.st_size != 44", operations)
        self.assertGreaterEqual(operations.count("set -euo pipefail"), 3)
        self.assertIn("expected_volume_uuid='PASTE-RECORDED-VOLUME-UUID'", operations)
        self.assertIn("diskutil info -plist", operations)
        self.assertIn("diskutil mount readOnly", operations)
        self.assertIn('test "$(plist_field Encryption)" = true', operations)
        self.assertIn('test "$(plist_field WritableVolume)" = false', operations)
        self.assertIn(
            '/verified/Sparkle/bin/generate_keys --account se.mjukis.dux -f "$seed_path"',
            operations,
        )
        self.assertIn("/verified/Sparkle/bin/sign_update", operations)
        self.assertIn("PUBLIC_SIGNATURE=$(", operations)
        self.assertIn("verify-sparkle-ed25519-signature.swift", operations)
        self.assertIn("import CryptoKit", verifier)
        self.assertIn("Curve25519.Signing.PublicKey", verifier)
        self.assertIn("isValidSignature", verifier)
        self.assertIn("O_RDONLY | O_NOFOLLOW | O_CLOEXEC", verifier)
        self.assertIn("fstat(descriptor, &status)", verifier)
        self.assertIn("status.st_nlink == 1", verifier)
        self.assertIn("expectedLength: 32", verifier)
        self.assertIn("expectedLength: 64", verifier)
        for forbidden in ["generate_keys", "Keychain", "PrivateKey"]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, verifier)

    @unittest.skipUnless(sys.platform == "darwin", "CryptoKit verifier requires macOS")
    def test_public_canary_verifier_accepts_rfc8032_and_rejects_unsafe_inputs(self) -> None:
        public_key = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo="
        signature = (
            "5VZDAMNgrHKQhuLMgG6CioSHfx645dl02HPgZSJJAVVfuIIVkKM7rMYeOXAc+"
            "bRr0lv18FlbviRlUUFDjnoQCw=="
        )
        with tempfile.TemporaryDirectory(prefix="dux-sparkle-verifier-") as raw:
            root = pathlib.Path(raw)
            binary = root / "verifier"
            # DUX-DESTRUCTIVE: allow=test-public-sparkle-verifier-compile-spawn -- compile only the fixed public-only verifier in an isolated temporary test directory
            compilation = subprocess.run(
                [
                    "xcrun",
                    "swiftc",
                    "-module-cache-path",
                    str(root / "module-cache"),
                    str(SPARKLE_PUBLIC_VERIFIER),
                    "-o",
                    str(binary),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(compilation.returncode, 0, compilation.stderr)

            payload = root / "empty-canary"
            payload.write_bytes(b"")

            def verify(
                file: pathlib.Path,
                candidate_signature: str = signature,
                candidate_key: str = public_key,
            ) -> subprocess.CompletedProcess[str]:
                # DUX-DESTRUCTIVE: allow=test-public-sparkle-verifier-run-spawn -- execute only the freshly compiled public verifier against controlled temporary test fixtures
                return subprocess.run(
                    [
                        str(binary),
                        "--public-key-base64",
                        candidate_key,
                        "--signature-base64",
                        candidate_signature,
                        "--file",
                        str(file),
                    ],
                    check=False,
                    capture_output=True,
                    text=True,
                )

            accepted = verify(payload)
            self.assertEqual(accepted.returncode, 0, accepted.stderr)

            modified_signature = bytearray(base64.b64decode(signature))
            modified_signature[0] ^= 1
            wrong_key = base64.b64encode(bytes(32)).decode("ascii")
            changed_payload = root / "changed-canary"
            changed_payload.write_bytes(b"changed")
            linked_payload = root / "linked-canary"
            os.symlink(payload, linked_payload)
            hardlink_source = root / "hardlink-source"
            hardlink_source.write_bytes(b"")
            hardlinked_payload = root / "hardlinked-canary"
            os.link(hardlink_source, hardlinked_payload)
            oversized_payload = root / "oversized-canary"
            with oversized_payload.open("wb") as stream:
                stream.truncate(16 * 1_024 * 1_024 + 1)

            rejected = {
                "modified signature": verify(
                    payload,
                    base64.b64encode(modified_signature).decode("ascii"),
                ),
                "modified payload": verify(changed_payload),
                "wrong key": verify(payload, candidate_key=wrong_key),
                "noncanonical base64": verify(payload, signature + "="),
                "short public key": verify(
                    payload,
                    candidate_key=base64.b64encode(bytes(31)).decode("ascii"),
                ),
                "short signature": verify(
                    payload,
                    base64.b64encode(bytes(63)).decode("ascii"),
                ),
                "symlink": verify(linked_payload),
                "hard link": verify(hardlinked_payload),
                "oversized payload": verify(oversized_payload),
            }
            for name, result in rejected.items():
                with self.subTest(name=name):
                    self.assertNotEqual(result.returncode, 0, result.stdout)

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

    def test_permanent_cleanup_ui_is_excluded_from_public_release(self) -> None:
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
        debug = project_spec.split("        Debug:\n", 1)[1].split(
            "        Release:\n", 1
        )[0]
        release_config = project_spec.split("        Release:\n", 1)[1].split(
            "        CleanupQualification:\n", 1
        )[0]
        qualification = project_spec.split("        CleanupQualification:\n", 1)[1]
        self.assertIn("DUX_INTERNAL_PERMANENT_SAFE_CLEANUP", debug)
        self.assertNotIn("DUX_INTERNAL_PERMANENT_SAFE_CLEANUP", release_config)
        self.assertNotIn("DUX_CLEANUP_QUALIFICATION", release_config)
        self.assertIn("DUX_INTERNAL_PERMANENT_SAFE_CLEANUP", qualification)
        self.assertIn("DUX_CLEANUP_QUALIFICATION", qualification)
        self.assertIn(
            "verify_public_release_not_cleanup_qualification",
            source,
        )


if __name__ == "__main__":
    unittest.main()
