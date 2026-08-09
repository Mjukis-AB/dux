#!/usr/bin/env python3

from pathlib import Path
import re
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
ADR_PATH = REPO_ROOT / "docs/adr/0013-metadata-only-remote-ai-transport.md"


def read(path: str) -> str:
    return (REPO_ROOT / path).read_text(encoding="utf-8")


def squash(text: str) -> str:
    return re.sub(r"\s+", " ", text)


def is_test_source(path: Path) -> bool:
    stem = path.stem.lower()
    return (
        stem in {"test", "tests"}
        or stem.endswith("_test")
        or stem.endswith("_tests")
        or path.name.endswith("Tests.swift")
        or any(part.lower() in {"test", "tests"} for part in path.parts)
    )


class AiRemoteTransportArchitectureTests(unittest.TestCase):
    def test_adr_is_accepted_and_complete(self) -> None:
        adr = ADR_PATH.read_text(encoding="utf-8")
        self.assertIn("# ADR 0013: Metadata-only remote AI transport", adr)
        self.assertIn("- Status: Accepted", adr)
        self.assertIn("- Decision date: 2026-08-09", adr)
        for heading in (
            "## Context",
            "## Decision",
            "## Disclosure and consent boundary",
            "## Credential boundary",
            "## Network and lifecycle boundary",
            "## Provider capability boundary",
            "## Validation and authority boundary",
            "## Provider privacy and retention disclosure",
            "## Consequences",
            "## Alternatives considered",
            "## Validation criteria",
            "## Reconsider when",
            "## References",
        ):
            self.assertIn(heading, adr)

    def test_endpoints_and_adapter_ids_are_closed(self) -> None:
        adr = ADR_PATH.read_text(encoding="utf-8")
        normalized = squash(adr)
        expected = {
            "anthropic-messages-v1": "https://api.anthropic.com/v1/messages",
            "openai-responses-v1": "https://api.openai.com/v1/responses",
        }
        for adapter_id, endpoint in expected.items():
            self.assertIn(f"`{adapter_id}`", adr)
            self.assertEqual(adr.count(endpoint), 1)
        for required in (
            "MUST NOT offer an arbitrary URL",
            "custom header map",
            "generic `Data -> Data` transport",
            "Adding or changing an origin, path, method, authentication scheme",
        ):
            self.assertIn(required, normalized)

    def test_consent_credentials_and_lifecycle_are_frozen(self) -> None:
        adr = ADR_PATH.read_text(encoding="utf-8")
        normalized = squash(adr)
        for required in (
            "AI remains optional and off by default",
            "explicit user invocation",
            "PrivacyShapedAiInputV1",
            "kSecClassGenericPassword",
            "kSecUseDataProtectionKeychain=true",
            "se.mjukis.dux.ai-provider-key.v1",
            "fixed adapter ID as `kSecAttrAccount`",
            "kSecAttrSynchronizable=false",
            "kSecAttrAccessibleWhenUnlockedThisDeviceOnly",
            "Credential verification performs no network request",
            "URLSessionConfiguration.ephemeral",
            "no redirect",
            "384 KiB",
            "60-second monotonic wall deadline",
            "64-KiB decompressed response-body limit",
            "no automatic retry",
            "There is no process tree to terminate",
        ):
            self.assertIn(required, normalized)

    def test_tools_persistence_and_authority_are_denied(self) -> None:
        adr = ADR_PATH.read_text(encoding="utf-8")
        normalized = squash(adr)
        for required in (
            "sends no client tool, function, MCP",
            "does not request optional prompt caching",
            "store: false` does not itself disable provider prompt caching",
            "up to 30 days of abuse-monitoring retention",
            "tool-shaped output is rejected",
            "existing Rust v1 output validator MUST accept",
            "Provider failure, cancellation, timeout",
            "No request or response persistence is approved",
            "is not an admissible cache schema",
            "canonical validated output at 64 KiB",
            "candidate",
            "plan",
            "approval",
            "schedule",
            "executor input",
        ):
            self.assertIn(required, normalized)

    def test_normative_documents_agree_on_checkpoint_state(self) -> None:
        roadmap = read("ROADMAP.md")
        security = read("SECURITY_DESIGN.md")
        contract = read("docs/AI_CONTRACT.md")
        adr9 = read("docs/adr/0009-reject-direct-local-ai-subprocesses.md")
        index = read("docs/adr/README.md")

        self.assertIn(
            "- [x] Select and approve a metadata-only remote transport",
            squash(roadmap),
        )
        self.assertIn("Process-tree cleanup is inapplicable", roadmap)
        self.assertIn("ADR 0013 accepts a fixed metadata-only", squash(security))
        self.assertIn(
            "disabled/no-provider remains the sole runtime state", squash(security)
        )
        self.assertIn("## Approved future remote boundary", contract)
        self.assertIn(
            "no engine, FFI, Swift, provider, or network consumer",
            squash(contract),
        )
        self.assertIn(
            "does not supersede this ADR's prohibition", squash(adr9)
        )
        self.assertIn("0013-metadata-only-remote-ai-transport.md", index)
        self.assertNotIn("record_ai_insight(input_digest, insight)", roadmap)
        self.assertIn("OpaqueAiExplanationPreview", roadmap)
        self.assertNotIn("no file content by default", roadmap.lower())
        self.assertIn("No file content can be sent in v1", roadmap)

    def test_checkpoint_adds_no_production_remote_ai_consumer(self) -> None:
        roots = (
            REPO_ROOT / "dux-core/src",
            REPO_ROOT / "dux-ffi/src",
            REPO_ROOT / "dux-cli/src",
            REPO_ROOT / "dux-macos/Dux",
        )
        production_files = [
            path
            for root in roots
            for path in root.rglob("*")
            if path.is_file()
            and path.suffix in {".rs", ".swift"}
            and not is_test_source(path)
        ]
        combined = "\n".join(
            path.read_text(encoding="utf-8", errors="replace")
            for path in production_files
        )
        for forbidden in (
            "api.anthropic.com",
            "api.openai.com",
            "anthropic-messages-v1",
            "openai-responses-v1",
            "URLSession.shared",
            "URLSession(",
            "URLSessionConfiguration.ephemeral",
            "URLRequest(",
            "NSURLConnection",
            "NWConnection",
            "SecItemAdd",
            "SecItemCopyMatching",
            "RemoteAiTransport",
            "AIProviderAdapter",
        ):
            self.assertNotIn(forbidden, combined)

        manifests = "\n".join(
            path.read_text(encoding="utf-8", errors="replace")
            for path in (
                REPO_ROOT / "Cargo.toml",
                REPO_ROOT / "dux-core/Cargo.toml",
                REPO_ROOT / "dux-ffi/Cargo.toml",
                REPO_ROOT / "dux-cli/Cargo.toml",
                REPO_ROOT / "fuzz/Cargo.toml",
                REPO_ROOT / "dux-macos/project.yml",
                REPO_ROOT / "dux-macos/Dux.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved",
            )
            if path.exists()
        )
        self.assertIsNone(
            re.search(
                r"(?i)api\.anthropic\.com|api\.openai\.com|"
                r"anthropic-messages|openai-responses|"
                r"\b(reqwest|ureq|hyper|isahc|surf)\b",
                manifests,
            )
        )

    def test_private_ai_source_has_no_transport_or_authority_import(self) -> None:
        ai_root = REPO_ROOT / "dux-core/src/ai"
        production = "\n".join(
            path.read_text(encoding="utf-8")
            for path in ai_root.rglob("*.rs")
            if not is_test_source(path)
        ).lower()
        for forbidden in (
            "std::process",
            "reqwest",
            "urlsession",
            "crate::ffi",
            "crate::engine",
            "crate::planner",
            "crate::executor",
            "crate::domain::candidate",
        ):
            self.assertNotIn(forbidden, production)

        crate_root = read("dux-core/src/lib.rs")
        self.assertIn("mod ai;", crate_root)
        self.assertNotRegex(crate_root, r"pub(?:\([^)]*\))?\s+mod\s+ai\s*;")


if __name__ == "__main__":
    unittest.main()
