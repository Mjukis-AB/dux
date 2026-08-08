import pathlib
import re
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPO_ROOT / ".github/workflows/release-macos-app.yml"
CLI_WORKFLOW_PATH = REPO_ROOT / ".github/workflows/release.yml"


class MacOSAppReleaseWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW_PATH.read_text(encoding="utf-8")

    def job(self, name: str) -> str:
        marker = f"  {name}:\n"
        start = self.workflow.index(marker)
        following = re.search(
            r"^  [a-z0-9][a-z0-9-]*:\n",
            self.workflow[start + len(marker):],
            flags=re.MULTILINE,
        )
        if following is None:
            return self.workflow[start:]
        end = start + len(marker) + following.start()
        return self.workflow[start:end]

    def step(self, name: str) -> str:
        marker = f"      - name: {name}\n"
        start = self.workflow.index(marker)
        following = re.search(
            r"^(?:      - name: |  [a-z0-9][a-z0-9-]*:\n)",
            self.workflow[start + len(marker):],
            flags=re.MULTILINE,
        )
        if following is None:
            return self.workflow[start:]
        end = start + len(marker) + following.start()
        return self.workflow[start:end]

    @staticmethod
    def step_names(job: str) -> list[str]:
        return re.findall(r"^      - name: (.+)$", job, flags=re.MULTILINE)

    def test_trigger_is_manual_only_and_permissions_are_read_only(self) -> None:
        trigger = self.workflow[
            self.workflow.index("on:\n"):self.workflow.index("permissions:\n")
        ]
        self.assertIn("workflow_dispatch:", trigger)
        for forbidden in [
            "push:",
            "pull_request:",
            "schedule:",
            "workflow_call:",
            "release:",
            "pull_request_target:",
            "repository_dispatch:",
        ]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, trigger)
        self.assertIn("permissions:\n  contents: read", self.workflow)
        self.assertNotRegex(self.workflow, r"(?m)^\s+[a-z-]+:\s*write\s*$")
        self.assertIn("group: macos-app-release", self.workflow)
        self.assertIn("cancel-in-progress: false", self.workflow)

    def test_source_gate_is_unprivileged_and_binds_exact_tag_commit(self) -> None:
        validate = self.job("validate")
        self.assertNotIn("environment:", validate)
        self.assertNotIn("secrets.", validate)
        self.assertIn("SELECTED_REF_TYPE: ${{ github.ref_type }}", validate)
        self.assertIn("SELECTED_REF_NAME: ${{ github.ref_name }}", validate)
        self.assertIn('[[ "$SELECTED_REF_TYPE" == tag ]]', validate)
        self.assertIn(
            '[[ "$SELECTED_REF_NAME" == "v$RELEASE_VERSION" ]]',
            validate,
        )
        self.assertIn("git rev-list -n 1", validate)
        self.assertIn('[[ "$tag_commit" == "$head_commit" ]]', validate)
        self.assertIn("github.event.repository.default_branch", validate)
        self.assertIn("git merge-base --is-ancestor", validate)
        self.assertIn('workspace["workspace"]["package"]["version"]', validate)
        self.assertIn(".designated_requirement ==", validate)
        self.assertIn("persist-credentials: false", validate)
        self.assertIn("fetch-depth: 0", validate)
        self.assertIn("clean: true", validate)
        self.assertIn("python3 scripts/check_destructive_calls.py", validate)

    def test_prepare_job_is_unprivileged_and_uses_pinned_build_tools(self) -> None:
        prepare = self.job("prepare")
        self.assertIn("needs: validate", prepare)
        self.assertNotIn("environment:", prepare)
        self.assertNotIn("secrets.", prepare)
        self.assertIn("runs-on: macos-15", prepare)
        self.assertIn("timeout-minutes: 180", prepare)
        self.assertIn(
            "DEVELOPER_DIR: /Applications/Xcode_16.4.app/Contents/Developer",
            prepare,
        )
        self.assertIn("ref: ${{ needs.validate.outputs.commit }}", prepare)
        self.assertIn("toolchain: 1.96.0", prepare)
        self.assertIn("targets: aarch64-apple-darwin,x86_64-apple-darwin", prepare)
        self.assertIn("Xcode 16.4", prepare)
        self.assertIn("Build version 16F6", prepare)
        self.assertIn("Version: 2.44.1", prepare)
        self.assertIn(
            "a2e905fb68446e9bb4008cdfe2e13e3f176d0cbcca828b71770f8e53fca91b73",
            prepare,
        )

    def test_signing_job_is_fresh_protected_and_build_tool_free(self) -> None:
        signing = self.job("sign-notarize")
        self.assertIn("needs: [validate, prepare]", signing)
        self.assertIn("environment: macos-release-signing", signing)
        self.assertIn("runs-on: macos-15", signing)
        self.assertIn("timeout-minutes: 180", signing)
        self.assertIn(
            "DEVELOPER_DIR: /Applications/Xcode_16.4.app/Contents/Developer",
            signing,
        )
        self.assertIn("ref: ${{ needs.validate.outputs.commit }}", signing)
        self.assertIn("Xcode 16.4", signing)
        self.assertIn("Build version 16F6", signing)
        for forbidden in ["dtolnay/rust-toolchain", "xcodegen", "cargo "]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, signing)

    def test_actions_are_the_exact_reviewed_node24_allowlist(self) -> None:
        actions = re.findall(r"^\s*uses:\s*([^\s#]+)", self.workflow, re.MULTILINE)
        checkout = "actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd"
        self.assertEqual(
            actions,
            [
                checkout,
                checkout,
                "dtolnay/rust-toolchain@fa04a1451ff1842e2626ccb99004d0195b455a88",
                "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
                checkout,
                "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
            ],
        )
        self.assertEqual(self.workflow.count("Node 24"), 5)

    def test_job_step_sequences_are_exact_and_signer_has_no_late_action(self) -> None:
        expected_total = 17
        self.assertEqual(
            len(re.findall(r"^      - ", self.workflow, flags=re.MULTILINE)),
            expected_total,
        )
        self.assertEqual(
            len(re.findall(r"^      - name: ", self.workflow, flags=re.MULTILINE)),
            expected_total,
        )
        self.assertEqual(
            self.step_names(self.job("validate")),
            [
                "Check out selected release source",
                "Validate tag, version, and public identity",
                "Verify release policy before requesting credentials",
            ],
        )
        self.assertEqual(
            self.step_names(self.job("prepare")),
            [
                "Check out exact validated commit",
                "Install pinned Rust toolchain",
                "Install checksum-pinned XcodeGen",
                "Verify pinned Apple build tools",
                "Build, test, and seal unsigned app without credentials",
                "Bind exact prepared envelope digest to job output",
                "Transfer only the unsigned sealed envelope",
            ],
        )
        signing = self.job("sign-notarize")
        self.assertEqual(
            self.step_names(signing),
            [
                "Check out exact validated commit for signing policy",
                "Download unsigned sealed envelope onto fresh runner",
                "Verify prepared envelope before requesting credentials",
                "Import protected Developer ID and notarization credentials",
                "Sign and notarize only the sealed prepared app",
                "Delete ephemeral release Keychain",
                "Verify exact ephemeral release output",
            ],
        )
        after_import = signing[signing.index("Import protected Developer ID"):]
        self.assertNotIn("uses:", after_import)
        self.assertEqual(after_import.count("python3"), 1)
        self.assertNotRegex(
            after_import,
            r"(?m)^\s*(?:from|import)\s+(?:http|requests|socket|subprocess|urllib)\b",
        )
        for forbidden in ["curl ", "wget ", "scp ", "rsync ", "ssh ", "nc ", "gh "]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, after_import)

    def test_raw_secrets_are_confined_to_one_import_step(self) -> None:
        credential_step = self.step(
            "Import protected Developer ID and notarization credentials"
        )
        secret_references = re.findall(r"secrets\.([A-Z0-9_]+)", self.workflow)
        self.assertEqual(
            set(secret_references),
            {
                "DUX_DEVELOPER_ID_P12_BASE64",
                "DUX_DEVELOPER_ID_P12_PASSWORD",
                "DUX_NOTARY_API_KEY_P8_BASE64",
                "DUX_NOTARY_KEY_ID",
                "DUX_NOTARY_ISSUER_ID",
            },
        )
        self.assertEqual(len(secret_references), 5)
        self.assertEqual(credential_step.count("secrets."), 5)
        self.assertNotIn("SPARKLE", credential_step)
        self.assertNotIn("generate_keys", credential_step)
        self.assertNotRegex(credential_step, r"(?m)^\s+-A(?:\s|$)")
        self.assertNotIn("-T /usr/bin/security", credential_step)
        self.assertIn("umask 077", credential_step)
        self.assertIn("DUX-DESTRUCTIVE: allow=macos-release-import-secret-remove", credential_step)
        self.assertIn('trap cleanup_import EXIT', credential_step)
        self.assertIn('credentials_ready=false', credential_step)
        self.assertIn('credentials_ready=true', credential_step)
        self.assertIn('security delete-keychain "$keychain_path" || true', credential_step)
        self.assertIn('security create-keychain', credential_step)
        self.assertIn('security set-key-partition-list', credential_step)
        self.assertIn('notarytool store-credentials dux-notary', credential_step)
        self.assertLess(
            credential_step.index("DUX_NOTARYTOOL_KEYCHAIN=%s"),
            credential_step.index("security create-keychain"),
        )
        workflow_without_credential_step = self.workflow.replace(credential_step, "")
        self.assertNotRegex(
            workflow_without_credential_step,
            r"(?i)(?:secrets\.|secrets\[|toJSON\(secrets\)|\$\{\{\s*secrets\s*\}\})",
        )

    def test_unsigned_envelope_crosses_an_explicit_public_artifact_boundary(self) -> None:
        prepare = self.job("prepare")
        signing = self.job("sign-notarize")
        upload = self.step("Transfer only the unsigned sealed envelope")
        download = self.step("Download unsigned sealed envelope onto fresh runner")
        verify = self.step("Verify prepared envelope before requesting credentials")
        bind = self.step("Bind exact prepared envelope digest to job output")

        self.assertEqual(self.workflow.count("actions/upload-artifact@"), 1)
        self.assertEqual(self.workflow.count("actions/download-artifact@"), 1)
        self.assertIn("retention-days: 1", upload)
        self.assertIn("compression-level: 0", upload)
        self.assertIn("path: ${{ runner.temp }}/dux-prepared-release", upload)
        self.assertNotIn("target/", upload)
        self.assertNotIn(".dmg", upload.lower())
        self.assertIn("path: ${{ runner.temp }}/dux-prepared-release", download)
        self.assertIn("digest-mismatch: error", download)
        self.assertIn(upload, prepare)
        self.assertNotIn(upload, signing)
        self.assertIn("prepared_envelope_sha256:", prepare)
        self.assertIn("artifact_digest:", prepare)
        self.assertIn("DUX prepared envelope v1", bind)
        self.assertIn(
            "DUX_PREPARED_ENVELOPE_SHA256: ${{ needs.prepare.outputs.prepared_envelope_sha256 }}",
            verify,
        )
        self.assertIn(
            "DUX_ACTIONS_ARTIFACT_SHA256: ${{ needs.prepare.outputs.artifact_digest }}",
            verify,
        )
        self.assertIn(
            '[[ "$actual_envelope_sha256" == "$DUX_PREPARED_ENVELOPE_SHA256" ]]',
            verify,
        )
        self.assertIn("--verify-prepared", verify)
        self.assertIn("DUX_RELEASE_COMMIT: ${{ needs.validate.outputs.commit }}", verify)
        self.assertNotIn("secrets.", prepare)
        self.assertNotIn("secrets.", upload)
        self.assertLess(signing.index("Download unsigned sealed envelope"), signing.index("Verify prepared envelope"))
        self.assertLess(signing.index("Verify prepared envelope"), signing.index("Import protected Developer ID"))

    def test_credentials_are_imported_only_after_sealed_preparation(self) -> None:
        prepare_step = self.step(
            "Build, test, and seal unsigned app without credentials"
        )
        verify_step = self.step(
            "Verify prepared envelope before requesting credentials"
        )
        release_step = self.step("Sign and notarize only the sealed prepared app")
        self.assertNotIn("secrets.", release_step)
        self.assertNotIn("secrets.", prepare_step)
        self.assertNotIn("secrets.", verify_step)
        self.assertIn("DUX_BUNDLE_IDENTIFIER: se.mjukis.dux", release_step)
        self.assertIn("DUX_TEAM_ID: SMQ3E8Y57T", release_step)
        self.assertIn("DUX_NOTARYTOOL_KEYCHAIN=%s", self.workflow)
        self.assertIn("--prepare", prepare_step)
        self.assertIn("--verify-prepared", verify_step)
        self.assertIn("--sign-prepared", release_step)
        self.assertIn("DUX_RELEASE_COMMIT: ${{ needs.validate.outputs.commit }}", release_step)
        self.assertNotIn("generate_appcast", release_step)
        self.assertNotIn("generate_keys", release_step)

        signing = self.job("sign-notarize")
        verify_index = signing.index(
            "Verify prepared envelope before requesting credentials"
        )
        credential_index = signing.index(
            "Import protected Developer ID and notarization credentials"
        )
        release_index = signing.index(
            "Sign and notarize only the sealed prepared app"
        )
        cleanup_index = signing.index("Delete ephemeral release Keychain")
        output_index = signing.index("Verify exact ephemeral release output")
        self.assertLess(verify_index, credential_index)
        self.assertLess(credential_index, release_index)
        self.assertLess(release_index, cleanup_index)
        self.assertLess(cleanup_index, output_index)

        credential_window = signing[credential_index:cleanup_index]
        self.assertNotIn("uses:", credential_window)
        for forbidden in ["cargo ", "python3", "xcodebuild", "xcodegen"]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, credential_window)

        cleanup = self.step("Delete ephemeral release Keychain")
        self.assertIn(
            'security lock-keychain "$DUX_NOTARYTOOL_KEYCHAIN" || true',
            cleanup,
        )
        self.assertIn(
            'security delete-keychain "$DUX_NOTARYTOOL_KEYCHAIN" || true',
            cleanup,
        )
        self.assertIn('test ! -e "$DUX_NOTARYTOOL_KEYCHAIN"', cleanup)

    def test_signed_output_is_exact_verified_and_never_uploaded_or_published(self) -> None:
        verify = self.step("Verify exact ephemeral release output")
        self.assertIn("release-manifest.txt", verify)
        self.assertIn('if {entry.name for entry in entries} != expected', verify)
        self.assertIn('entry.is_file() and not entry.is_symlink()', verify)
        self.assertIn("shasum -a 256 -c", verify)
        self.assertIn('wc -l <"$manifest"', verify)
        self.assertIn(".status == \"Accepted\"", verify)
        self.assertIn("xcode_version=16.4", verify)
        self.assertIn("sparkle_version=2.9.5", verify)
        for forbidden in [
            "cargo publish",
            "brew ",
            "gh release",
            "actions/create-release",
            "softprops/action-gh-release",
            "upload-release-asset",
            "crates.io",
            "HOMEBREW_TAP_TOKEN",
        ]:
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, self.workflow)
        upload = self.step("Transfer only the unsigned sealed envelope")
        self.assertNotIn("sign-notarize", upload)
        self.assertNotIn("release-manifest.txt", upload)
        self.assertNotIn("notarization", upload)
        self.assertNotIn("DUX-${{ inputs.version }}.dmg", self.workflow)

    def test_standalone_cli_release_remains_a_separate_lane(self) -> None:
        cli_workflow = CLI_WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertNotIn("release-macos-app", cli_workflow)
        self.assertNotIn("macos-release-signing", cli_workflow)
        self.assertNotIn("DUX_DEVELOPER_ID", cli_workflow)
        self.assertNotIn("DUX_NOTARY", cli_workflow)


if __name__ == "__main__":
    unittest.main()
