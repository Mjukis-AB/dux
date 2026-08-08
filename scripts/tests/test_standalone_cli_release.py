import pathlib
import re
import tomllib
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKSPACE_MANIFEST = REPO_ROOT / "Cargo.toml"
CORE_MANIFEST = REPO_ROOT / "dux-core/Cargo.toml"
CLI_MANIFEST = REPO_ROOT / "dux-cli/Cargo.toml"
FFI_MANIFEST = REPO_ROOT / "dux-ffi/Cargo.toml"
RELEASE_WORKFLOW = REPO_ROOT / ".github/workflows/release.yml"
README = REPO_ROOT / "README.md"


class StandaloneCliReleaseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = RELEASE_WORKFLOW.read_text(encoding="utf-8")

    def job(self, name: str) -> str:
        marker = f"  {name}:\n"
        start = self.workflow.index(marker)
        following = re.search(
            r"^  [a-z0-9][a-z0-9-]*:\n",
            self.workflow[start + len(marker) :],
            flags=re.MULTILINE,
        )
        if following is None:
            return self.workflow[start:]
        end = start + len(marker) + following.start()
        return self.workflow[start:end]

    def test_only_core_and_cli_are_public_crates_with_one_exact_version(self) -> None:
        with WORKSPACE_MANIFEST.open("rb") as stream:
            workspace = tomllib.load(stream)
        with CORE_MANIFEST.open("rb") as stream:
            core = tomllib.load(stream)
        with CLI_MANIFEST.open("rb") as stream:
            cli = tomllib.load(stream)
        with FFI_MANIFEST.open("rb") as stream:
            ffi = tomllib.load(stream)

        version = workspace["workspace"]["package"]["version"]
        self.assertNotEqual(core["package"].get("publish"), False)
        self.assertNotEqual(cli["package"].get("publish"), False)
        self.assertIs(ffi["package"]["publish"], False)
        self.assertEqual(
            cli["dependencies"]["dux-core"],
            {"path": "../dux-core", "version": version},
        )
        self.assertEqual(cli["bin"], [{"name": "dux", "path": "src/main.rs"}])

    def test_cli_release_chain_stays_independent_from_app_credentials(self) -> None:
        verify = self.job("verify")
        build = self.job("build")
        publish = self.job("publish-crates")
        github_release = self.job("create-release")
        homebrew = self.job("homebrew")

        self.assertIn("needs: [verify, msrv, dependencies]", build)
        self.assertIn("needs: build", publish)
        self.assertIn("needs: publish-crates", github_release)
        self.assertIn("needs: create-release", homebrew)
        self.assertIn("cargo package --workspace --locked", verify)

        standalone_chain = "\n".join(
            [verify, self.job("msrv"), self.job("dependencies"), build,
             publish, github_release, homebrew]
        )
        for app_only_input in [
            "DUX_SIGNING_IDENTITY",
            "DUX_NOTARYTOOL_PROFILE",
            "SPARKLE",
            "xcodebuild",
            "notarytool",
            "release-notarized-dmg",
        ]:
            with self.subTest(app_only_input=app_only_input):
                self.assertNotIn(app_only_input, standalone_chain)

    def test_release_actions_are_immutable_full_sha_pins(self) -> None:
        actions = re.findall(r"^\s*uses:\s*([^\s#]+)", self.workflow, re.MULTILINE)
        self.assertGreater(len(actions), 0)
        for action in actions:
            with self.subTest(action=action):
                self.assertRegex(action, r"^[^@]+@[0-9a-f]{40}$")

    def test_release_builds_and_checksums_every_supported_cli_archive(self) -> None:
        build = self.job("build")
        matrix = build[build.index("      matrix:\n") : build.index("    steps:\n")]
        targets = re.findall(r"^\s*- target: (\S+)$", matrix, re.MULTILINE)
        self.assertEqual(
            targets,
            [
                "x86_64-unknown-linux-gnu",
                "x86_64-apple-darwin",
                "aarch64-apple-darwin",
                "x86_64-pc-windows-msvc",
            ],
        )
        self.assertIn("cargo build --release --locked --target", build)
        self.assertIn("-p dux-cli", build)
        self.assertIn("Smoke-test native binary", build)
        self.assertIn('shasum -a 256 "$archive"', build)
        self.assertIn("Get-FileHash -Algorithm SHA256", build)
        self.assertIn("if-no-files-found: error", build)

        github_release = self.job("create-release")
        self.assertIn("sha256sum --check SHA256SUMS", github_release)
        self.assertIn("release-assets/SHA256SUMS", github_release)

    def test_crates_and_homebrew_channels_use_verified_cli_outputs(self) -> None:
        publish = self.job("publish-crates")
        core_package = publish.index("cargo package -p dux-core --locked")
        core_publish = publish.index("cargo publish -p dux-core --locked")
        wait = publish.index("Wait for dux-core to become available")
        cli_package = publish.index("cargo package -p dux-cli --locked")
        cli_publish = publish.index("cargo publish -p dux-cli --locked")
        self.assertLess(core_package, core_publish)
        self.assertLess(core_publish, wait)
        self.assertLess(wait, cli_package)
        self.assertLess(cli_package, cli_publish)
        self.assertEqual(publish.count("cmp \"target/package/"), 2)
        self.assertIn("secrets.CARGO_REGISTRY_TOKEN", publish)

        homebrew = self.job("homebrew")
        self.assertIn("gh release download", homebrew)
        self.assertIn("--pattern SHA256SUMS", homebrew)
        self.assertIn("mjukis-ab/homebrew-tap.git", homebrew)
        self.assertIn("Formula/dux.rb", homebrew)
        for target in [
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-unknown-linux-gnu",
        ]:
            with self.subTest(target=target):
                self.assertIn(target, homebrew)
        self.assertIn('bin.install "dux"', homebrew)
        self.assertIn('system "#{bin}/dux", "--version"', homebrew)

    def test_user_documentation_keeps_both_standalone_install_routes(self) -> None:
        readme = README.read_text(encoding="utf-8")
        self.assertIn("### From crates.io", readme)
        self.assertIn("cargo install dux-cli", readme)
        self.assertIn("### From Homebrew (macOS/Linux)", readme)
        self.assertIn("brew tap mjukis-ab/tap", readme)
        self.assertIn("brew install dux", readme)


if __name__ == "__main__":
    unittest.main()
