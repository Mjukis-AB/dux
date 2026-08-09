import hashlib
import pathlib
import re
import unittest


REPO = pathlib.Path(__file__).resolve().parents[2]
PROJECT = (REPO / "dux-macos" / "project.yml").read_text(encoding="utf-8")
INFO = (REPO / "dux-macos" / "Dux" / "Info.plist").read_text(encoding="utf-8")
MENU = (
    REPO / "dux-macos" / "Dux" / "Views" / "MenuBarContentView.swift"
).read_text(encoding="utf-8")
EXPLORER = (
    REPO / "dux-macos" / "Dux" / "Views" / "ExplorerView.swift"
).read_text(encoding="utf-8")
NOTICE = (
    REPO / "dux-macos" / "Dux" / "Views" / "CleanupQualificationNotice.swift"
).read_text(encoding="utf-8")
BUILDER_PATH = REPO / "dux-macos" / "scripts" / "build-cleanup-qualification-app.sh"
VERIFIER_PATH = (
    REPO / "dux-macos" / "scripts" / "verify-signed-cleanup-qualification.sh"
)
BUILDER = BUILDER_PATH.read_text(encoding="utf-8")
VERIFIER = VERIFIER_PATH.read_text(encoding="utf-8")
RELEASE = (
    REPO / "dux-macos" / "scripts" / "release-notarized-dmg.sh"
).read_text(encoding="utf-8")
PROTOCOL = (
    REPO / "docs" / "testing" / "signed-app-destructive-qualification.md"
).read_text(encoding="utf-8")
ADR = (
    REPO / "docs" / "adr" / "0012-non-shipping-signed-cleanup-qualification.md"
).read_text(encoding="utf-8")
GIT_ATTRIBUTES = (REPO / ".gitattributes").read_text(encoding="utf-8")
REVIEWED_BUILDER_SHA256 = (
    "bec43e40ec3d03fdf95d0cd5ac4f13151c0012735d0374141dbd0c94d80b3c09"
)
REVIEWED_VERIFIER_SHA256 = (
    "feda04d64d475ab0d0e787c1d1590bd9d8b14ba305c546c03b4913e7296e832c"
)


class SignedAppDestructiveQualificationTests(unittest.TestCase):
    def test_configuration_is_release_optimized_but_non_shipping(self) -> None:
        self.assertIn("defaultConfig: Debug", PROJECT)
        self.assertIn(
            "configs:\n  Debug: debug\n  Release: release\n  CleanupQualification: release",
            PROJECT,
        )
        qualification = PROJECT.split("        CleanupQualification:\n", 1)[1]
        self.assertIn("PRODUCT_BUNDLE_IDENTIFIER: se.mjukis.dux", qualification)
        self.assertNotIn("PRODUCT_NAME:", qualification)
        self.assertIn("PRODUCT_NAME: DUX", PROJECT)
        self.assertIn('DUX_CLEANUP_QUALIFICATION_PROTOCOL_VERSION: "1"', qualification)
        self.assertIn("DUX_INTERNAL_PERMANENT_SAFE_CLEANUP", qualification)
        self.assertIn("DUX_CLEANUP_QUALIFICATION", qualification)

        release = PROJECT.split("        Release:\n", 1)[1].split(
            "        CleanupQualification:\n", 1
        )[0]
        self.assertNotIn("DUX_INTERNAL_PERMANENT_SAFE_CLEANUP", release)
        self.assertNotIn("DUX_CLEANUP_QUALIFICATION", release)

    def test_signed_bundle_metadata_binds_protocol_and_source(self) -> None:
        self.assertIn("$(DUX_DISPLAY_NAME)", INFO)
        self.assertIn("DUXCleanupQualificationProtocolVersion", INFO)
        self.assertIn("$(DUX_CLEANUP_QUALIFICATION_PROTOCOL_VERSION)", INFO)
        self.assertIn("DUXCleanupQualificationSourceCommit", INFO)
        self.assertIn("$(DUX_CLEANUP_QUALIFICATION_SOURCE_COMMIT)", INFO)
        self.assertIn("DUX_CLEANUP_QUALIFICATION_SOURCE_COMMIT=\"$commit\"", BUILDER)

    def test_qualification_is_visibly_distinct_on_both_primary_surfaces(self) -> None:
        self.assertIn("#if DUX_CLEANUP_QUALIFICATION", NOTICE)
        self.assertIn("Signed cleanup qualification", NOTICE)
        self.assertIn("Disposable test data only", NOTICE)
        self.assertIn("This is not a public DUX build", NOTICE)
        self.assertIn("cleanup-qualification.notice", NOTICE)
        self.assertRegex(
            MENU,
            r"#if DUX_CLEANUP_QUALIFICATION\s+"
            r"CleanupQualificationNotice\(compact: true\)",
        )
        self.assertRegex(
            EXPLORER,
            r"#if DUX_CLEANUP_QUALIFICATION\s+"
            r"CleanupQualificationNotice\(compact: false\)",
        )

    def test_public_release_packaging_rejects_every_qualification_marker(self) -> None:
        self.assertIn("verify_public_release_not_cleanup_qualification", RELEASE)
        self.assertIn(
            "public Release app carries the cleanup-qualification protocol marker",
            RELEASE,
        )
        self.assertIn(
            "public Release app carries cleanup-qualification source identity",
            RELEASE,
        )
        self.assertIn(
            "permanent cleanup UI must not be compiled into public Release builds",
            RELEASE,
        )
        self.assertIn(
            "-configuration CleanupQualification",
            RELEASE,
        )
        self.assertIn(
            "signed cleanup qualification must carry its non-shipping marker",
            RELEASE,
        )

    def test_builder_requires_clean_exact_source_and_does_not_sign_or_install(self) -> None:
        for required in (
            "qualification build requires a clean worktree, including untracked files",
            "DUX_QUALIFICATION_COMMIT does not match HEAD",
            "DerivedData path must not already exist",
            "-configuration CleanupQualification",
            "CODE_SIGNING_ALLOWED=NO",
            'ARCHS="arm64 x86_64"',
            "generation changed tracked source",
            "--untracked-files=all",
            "qualification app must contain exactly seven Mach-O files",
            "embedded Sparkle version is not the reviewed",
            "bundled CLI metadata does not match its bytes, version, or schemas",
        ):
            with self.subTest(required=required):
                self.assertIn(required, BUILDER)
        for forbidden in (
            "codesign --force",
            "notarytool submit",
            "stapler staple",
            "/Applications/DUX Cleanup Qualification.app",
            "--untracked-files=no",
        ):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, BUILDER)

    def test_installed_verifier_is_read_only_and_fail_closed(self) -> None:
        for required in (
            'EXPECTED_APP="/Applications/DUX Cleanup Qualification.app"',
            "qualification app must contain exactly seven Mach-O files",
            "codesign --verify --deep --all-architectures --strict",
            "Developer ID Application: MJUKIS AB (SMQ3E8Y57T)",
            "designated requirement does not match production",
            "xcrun stapler validate",
            "source=Notarized Developer ID",
            "qualification_protocol=1",
            "executable_sha256=",
            "qualification app contains an unreviewed nested bundle",
            "embedded Sparkle version is not the reviewed",
            "org.sparkle-project.Sparkle.Updater",
            "org.sparkle-project.DownloaderService",
            "org.sparkle-project.InstallerLauncher",
            "bundled CLI metadata does not match its signed bytes, version, or schemas",
            "DUX_QUALIFICATION_ARCHIVE_SHA256 must be a lowercase SHA-256",
            "archive_sha256=",
            "verify_empty_entitlements \"$sparkle/XPCServices/Downloader.xpc\"",
        ):
            with self.subTest(required=required):
                self.assertIn(required, VERIFIER)
        for forbidden in (
            "codesign --force",
            "notarytool submit",
            "stapler staple",
            "trashItem",
        ):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, VERIFIER)
        self.assertNotRegex(VERIFIER, r"(?m)^\s*(?:open|rm|mv)\s")

    def test_security_sensitive_scripts_match_reviewed_bytes(self) -> None:
        self.assertEqual(
            hashlib.sha256(BUILDER_PATH.read_bytes()).hexdigest(),
            REVIEWED_BUILDER_SHA256,
            "the qualification builder changed; review the complete script and update its digest",
        )
        self.assertEqual(
            hashlib.sha256(VERIFIER_PATH.read_bytes()).hexdigest(),
            REVIEWED_VERIFIER_SHA256,
            "the qualification verifier changed; review the complete script and update its digest",
        )

    def test_security_scripts_have_checkout_stable_line_endings(self) -> None:
        self.assertIn(
            "dux-macos/scripts/build-cleanup-qualification-app.sh text eol=lf",
            GIT_ATTRIBUTES,
        )
        self.assertIn(
            "dux-macos/scripts/verify-signed-cleanup-qualification.sh text eol=lf",
            GIT_ATTRIBUTES,
        )

    def test_protocol_requires_real_trash_and_permanent_safe_evidence(self) -> None:
        protocol = " ".join(PROTOCOL.split())
        for required in (
            "Status: repository lane implemented; credentialed device evidence not yet run.",
            "Intel Mac",
            "Apple Silicon Mac",
            "fresh disposable local account",
            "Full Disk Access ungranted",
            "Real Explorer Trash",
            "Finder **Put Back**",
            "Add project folder…",
            "exact direct Cargo 1.96.0 executable",
            "Run once and enroll",
            "Run dry check",
            "ENABLE PERMANENT CLEANUP",
            "target/CACHEDIR.TAG",
            "history_correlation=exact",
            "retry_surface=absent",
            "archive_sha256=<64 lowercase hex>",
            "final private archive SHA-256",
        ):
            with self.subTest(required=required):
                self.assertIn(required, protocol)
        self.assertNotIn("Check without removing", protocol)
        self.assertIn("**Status:** Accepted", ADR)
        self.assertIn("There is no qualification-only engine", ADR)
        self.assertIn("never public release candidates", ADR)
        self.assertIn(
            "../../SECURITY_DESIGN.md#17-verification-and-enforcement",
            ADR,
        )
        self.assertIn(
            "../../ROADMAP.md#milestone-5-deterministic-recommendations-and-reviewed-cleanup",
            ADR,
        )


if __name__ == "__main__":
    unittest.main()
