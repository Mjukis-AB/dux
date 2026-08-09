#!/usr/bin/env python3

from pathlib import Path
import re
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
ADR_PATH = REPO_ROOT / "docs/adr/0013-metadata-only-remote-ai-transport.md"
CREDENTIAL_STORE_PATH = (
    REPO_ROOT / "dux-macos/Dux/Services/AIProviderCredentialStore.swift"
)
LIFECYCLE_PATH = REPO_ROOT / "dux-macos/Dux/Services/NativeAIRemoteLifecycle.swift"
ANTHROPIC_ADAPTER_PATH = (
    REPO_ROOT / "dux-macos/Dux/Services/AnthropicMessagesV1Adapter.swift"
)
ORCHESTRATOR_PATH = (
    REPO_ROOT
    / "dux-macos/Dux/Services/NativeAIAnthropicMessagesV1Orchestrator.swift"
)
ENGINE_SERVICE_PATH = REPO_ROOT / "dux-macos/Dux/Services/EngineService.swift"
AI_CORE_BRIDGE_PATH = REPO_ROOT / "dux-core/src/engine/ai_metadata_preview.rs"
AI_COORDINATOR_PATH = (
    REPO_ROOT / "dux-macos/Dux/Services/ExplorerAIExplanationService.swift"
)
AI_SETTINGS_PATH = REPO_ROOT / "dux-macos/Dux/App/AIProviderSettingsModel.swift"
AI_PRESENTATION_ROOT = REPO_ROOT / "dux-macos/DuxAIExplanationPresentation"
AI_MODEL_PATH = AI_PRESENTATION_ROOT / "ExplorerAIExplanationModel.swift"
AI_TYPES_PATH = AI_PRESENTATION_ROOT / "ExplorerAIExplanationTypes.swift"
AI_PREVIEW_PATH = AI_PRESENTATION_ROOT / "ExplorerAIMetadataPreview.swift"
AI_VIEW_PATH = AI_PRESENTATION_ROOT / "ExplorerAIExplanationViews.swift"
AI_BROWSER_PATH = (
    REPO_ROOT / "dux-macos/Dux/Services/ExplorerSnapshotBrowser.swift"
)
AI_HOST_ADAPTER_PATH = (
    REPO_ROOT
    / "dux-macos/Dux/Services/ExplorerAIExplanationPresentationAdapter.swift"
)
SUPPLEMENTAL_PRESENTATION_PATH = (
    REPO_ROOT / "dux-macos/Dux/Models/ExplorerSupplementalPresentation.swift"
)
SNAPSHOT_BROWSER_VIEW_PATH = (
    REPO_ROOT / "dux-macos/Dux/Views/ExplorerSnapshotBrowserView.swift"
)
SNAPSHOT_TREEMAP_VIEW_PATH = (
    REPO_ROOT / "dux-macos/Dux/Views/ExplorerSnapshotTreemapView.swift"
)
PROJECT_YML_PATH = REPO_ROOT / "dux-macos/project.yml"
SNAPSHOT_REVIEW_CONTROLLER_PATH = (
    REPO_ROOT / "dux-macos/Dux/Services/SnapshotReviewController.swift"
)
GENERATED_SWIFT_PATH = REPO_ROOT / "dux-macos/Dux/Generated/DuxFFI.swift"
ANTHROPIC_REVIEW_PATH = (
    REPO_ROOT / "docs/provider-reviews/anthropic-messages-v1.md"
)
AI_AUTHORITY_REVIEW_PATH = (
    REPO_ROOT / "docs/security-reviews/m7-ai-authority-isolation.md"
)


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


def without_swift_debug_blocks(source: str) -> str:
    """Return production Swift while ignoring DEBUG-only fixture declarations."""
    kept = []
    frames = []
    excluded = False
    for line in source.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("#if "):
            parent_excluded = excluded
            debug_branch = bool(re.match(r"#if\s+DEBUG(?:\s|$)", stripped))
            frames.append((parent_excluded, debug_branch))
            excluded = parent_excluded or debug_branch
            continue
        if stripped.startswith("#elseif ") and frames:
            parent_excluded, debug_branch = frames[-1]
            excluded = parent_excluded or (
                debug_branch and bool(re.search(r"\bDEBUG\b", stripped))
            )
            continue
        if stripped == "#else" and frames:
            parent_excluded, debug_branch = frames[-1]
            excluded = parent_excluded
            continue
        if stripped == "#endif" and frames:
            excluded, _ = frames.pop()
            continue
        if not excluded:
            kept.append(line)
    return "".join(kept)


def without_inline_rust_tests(source: str) -> str:
    """Drop the conventional trailing cfg(test) module from production policy."""
    marker = re.search(r"(?m)^#\[cfg\(test\)\]\s*\nmod tests\s*\{", source)
    return source[: marker.start()] if marker else source


def without_swift_comments_and_literals(source: str) -> str:
    """Replace comments and string literals while preserving declarations."""
    pattern = re.compile(
        r'(?s)/\*.*?\*/|//[^\n]*|""".*?"""|"(?:\\.|[^"\\])*"'
    )
    return pattern.sub("", source)


def swift_member_block(source: str, function_name: str) -> str:
    matches = list(re.finditer(rf"\bfunc\s+{re.escape(function_name)}\b", source))
    if not matches:
        raise AssertionError(f"missing Swift member {function_name}")
    match = matches[-1]
    opening = source.find("{", match.end())
    if opening < 0:
        raise AssertionError(f"missing body for Swift member {function_name}")
    depth = 0
    for index in range(opening, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[match.start() : index + 1]
    raise AssertionError(f"unterminated Swift member {function_name}")


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
        self.assertIn(
            "one-shot retained-proof handoff checkpoint",
            squash(roadmap),
        )
        self.assertRegex(
            roadmap,
            r"(?m)^- \[x\] Implement the approved remote request deadline,",
        )
        self.assertRegex(
            roadmap,
            r"(?m)^  - \[x\] 2026-08-09 one-shot retained-proof handoff checkpoint:",
        )
        self.assertNotIn(
            "Leave both this checkpoint and its parent task open",
            squash(roadmap),
        )
        self.assertIn("fixed Anthropic Messages v1 adapter review", squash(roadmap))
        self.assertIn(
            "- [x] Validate tools-disabled behavior for each approved remote adapter",
            squash(roadmap),
        )
        self.assertIn(
            "- [x] Implement “Explain selection” and group overlays",
            squash(roadmap),
        )
        self.assertIn("- [x] Add “View metadata sent”", squash(roadmap))
        self.assertIn("ADR 0013 accepts a fixed metadata-only", squash(security))
        self.assertIn("explicit Explorer consent flow", squash(security))
        self.assertIn(
            "provider-labeled inert presentation, with optional sealed non-authoritative caching",
            squash(security),
        )
        self.assertIn("FFI v61 may consume that exact available preview", squash(security))
        self.assertIn("### 11.4 Sealed cache and explicit clearing", security)
        self.assertIn("exact local schema-v19 cache lookup", squash(security))
        self.assertIn(
            "full-population, two-minute, engine-bound consume-once clear",
            squash(security),
        )
        self.assertIn("Rust maps validated request-local group IDs", squash(security))
        self.assertIn("## Approved remote boundary and implementation", contract)
        self.assertIn(
            "The permitted opaque Rust attempt retains the moved sealed proof",
            squash(contract),
        )
        self.assertIn("The only current runtime provider is the fixed Anthropic", squash(contract))
        self.assertIn("separate one-shot **Explain selection** button", squash(contract))
        self.assertIn("## Sealed AI insight cache v1", contract)
        self.assertIn("does not read Keychain, create a request, or start networking", squash(contract))
        self.assertIn("runs no `VACUUM` or compaction", squash(contract))
        self.assertTrue(ANTHROPIC_REVIEW_PATH.is_file())
        provider_review = ANTHROPIC_REVIEW_PATH.read_text(encoding="utf-8")
        for required in (
            "Status: Runtime-enabled only through explicit Explorer preview and one-shot consent",
            "Adapter ID: `anthropic-messages-v1`",
            "Adapter revision: 1",
            "Model: `claude-sonnet-4-6`",
            "## Data handling disclosure",
            "## Suspension conditions",
            "That exact orchestrator now has one production caller",
            "memory-only",
        ):
            self.assertIn(required, provider_review)
        self.assertIn(
            "does not supersede this ADR's prohibition", squash(adr9)
        )
        self.assertIn("0013-metadata-only-remote-ai-transport.md", index)
        self.assertNotIn("record_ai_insight(input_digest, insight)", roadmap)
        self.assertIn("AiExplanationAttemptSession", roadmap)
        self.assertNotIn("OpaqueAiExplanationPreview", roadmap)
        self.assertNotIn("no file content by default", roadmap.lower())
        self.assertIn("No file content can be sent in v1", roadmap)
        self.assertIn(
            "- [x] Prove through type/module boundaries that AI cannot create plans.",
            roadmap,
        )
        self.assertIn("compiler-isolated presentation checkpoint", roadmap)
        self.assertIn(
            "dependency-free `DuxAIExplanationPresentation`",
            squash(contract),
        )
        self.assertIn(
            "complete the structural AI-to-plan isolation proof",
            squash(security),
        )
        self.assertIn("(**implemented 2026-08-09**", read(
            "docs/adr/0013-metadata-only-remote-ai-transport.md"
        ))
        self.assertNotIn("stronger structural proof", contract)
        self.assertNotIn("stronger structural no-AI-to-plan proof", security)
        self.assertTrue(AI_AUTHORITY_REVIEW_PATH.is_file())
        authority_review = AI_AUTHORITY_REVIEW_PATH.read_text(encoding="utf-8")
        for required in (
            "Result: approved for the uncached M7 explanation flow",
            "## Compiled dependency and capability graph",
            "## Public type and SPI audit",
            "## Indirect-bridge threat cases",
            "835/835 passed",
            "135 passed, 2 intentionally ignored",
            "No live API key was read, stored, or verified",
        ):
            self.assertIn(required, authority_review)

    def test_swift_ai_presentation_target_is_dependency_free_and_one_way(self) -> None:
        project = PROJECT_YML_PATH.read_text(encoding="utf-8")
        target_match = re.search(
            r"(?ms)^  DuxAIExplanationPresentation:\n"
            r"(?P<body>.*?)(?=^  Dux:\n)",
            project,
        )
        self.assertIsNotNone(target_match)
        target = target_match.group("body")
        self.assertIn("type: library.static", target)
        self.assertIn("path: DuxAIExplanationPresentation", target)
        self.assertNotIn("dependencies:", target)
        self.assertNotIn("Generated/DuxFFI", target)
        self.assertNotIn("Sparkle", target)

        app_match = re.search(
            r"(?ms)^  Dux:\n(?P<body>.*?)(?=^  DuxTests:\n)",
            project,
        )
        self.assertIsNotNone(app_match)
        app = app_match.group("body")
        self.assertEqual(app.count("target: DuxAIExplanationPresentation"), 1)
        tests = project[project.index("  DuxTests:") :]
        self.assertIn(
            "- target: DuxAIExplanationPresentation\n        link: false",
            tests,
        )

        generated_project = read("dux-macos/Dux.xcodeproj/project.pbxproj")
        self.assertIn("DuxAIExplanationPresentation", generated_project)
        self.assertIn("libDuxAIExplanationPresentation.a", generated_project)

    def test_native_ai_host_projection_has_no_reverse_authority_edge(self) -> None:
        host = without_swift_comments_and_literals(
            AI_HOST_ADAPTER_PATH.read_text(encoding="utf-8")
        )
        supplemental = without_swift_comments_and_literals(
            SUPPLEMENTAL_PRESENTATION_PATH.read_text(encoding="utf-8")
        )
        coordinator = without_swift_comments_and_literals(
            AI_COORDINATOR_PATH.read_text(encoding="utf-8")
        )

        context_getter = host
        for forbidden in (
            "reviewCandidate",
            "prepareSelectedRustTargetPlanReview",
            "makeRustTargetCleanupConfirmation",
            "startConfirmedRustTargetCleanup",
            "startRustTargetDryRun",
            "executeConfirmedTrash",
            "selectTableNode",
            "selectTreemapCell",
        ):
            self.assertNotIn(forbidden, context_getter)
            self.assertNotIn(forbidden, coordinator)
        self.assertIn("browser?.supplementalPresentationContext", host)
        self.assertIn("private weak var browser", host)
        self.assertIn("private let previews: any DuxAIMetadataPreviewServing", coordinator)
        self.assertIn("private let lease: any DuxAIMetadataPreviewLease", coordinator)
        self.assertNotIn("DuxSnapshotReviewBrowsing", coordinator)
        self.assertNotIn("ExplorerSnapshotBrowserModel", coordinator)
        self.assertNotIn("ExplorerAIExplanation", supplemental)
        self.assertNotIn("DuxAIExplanationPresentation", supplemental)

        browser = AI_BROWSER_PATH.read_text(encoding="utf-8")
        self.assertIn("fileprivate init(", browser)
        self.assertEqual(
            sum(
                "ExplorerTrashConfirmation(" in path.read_text(encoding="utf-8")
                for path in (REPO_ROOT / "dux-macos/Dux").rglob("*.swift")
                if not is_test_source(path)
            ),
            1,
        )

    def test_checkpoint_allows_only_the_exact_one_shot_graph(self) -> None:
        roots = (
            REPO_ROOT / "dux-core/src",
            REPO_ROOT / "dux-ffi/src",
            REPO_ROOT / "dux-cli/src",
            REPO_ROOT / "dux-macos/Dux",
            AI_PRESENTATION_ROOT,
        )
        production_files = [
            path
            for root in roots
            for path in root.rglob("*")
            if path.is_file()
            and path.suffix in {".rs", ".swift"}
            and not is_test_source(path)
        ]
        sources = {}
        for path in production_files:
            source = path.read_text(encoding="utf-8", errors="replace")
            if path.suffix == ".swift":
                source = without_swift_debug_blocks(source)
            elif path.suffix == ".rs":
                source = without_inline_rust_tests(source)
            sources[path] = source
        combined = "\n".join(sources.values())

        self.assertTrue(CREDENTIAL_STORE_PATH.is_file())
        self.assertTrue(LIFECYCLE_PATH.is_file())
        self.assertTrue(ANTHROPIC_ADAPTER_PATH.is_file())
        self.assertTrue(ORCHESTRATOR_PATH.is_file())
        self.assertTrue(ENGINE_SERVICE_PATH.is_file())
        self.assertTrue(GENERATED_SWIFT_PATH.is_file())
        credential = sources[CREDENTIAL_STORE_PATH]
        lifecycle = sources[LIFECYCLE_PATH]
        anthropic = sources[ANTHROPIC_ADAPTER_PATH]
        orchestrator = sources[ORCHESTRATOR_PATH]
        engine_service = sources[ENGINE_SERVICE_PATH]

        for path, source in sources.items():
            if path != LIFECYCLE_PATH:
                for forbidden in ("URLSession", "NSURLConnection", "NWConnection"):
                    self.assertNotIn(forbidden, source, path)
            if path not in {LIFECYCLE_PATH, ANTHROPIC_ADAPTER_PATH}:
                self.assertNotIn("URLRequest", source, path)
            if path != CREDENTIAL_STORE_PATH:
                for forbidden in (
                    "SecItemAdd",
                    "SecItemCopyMatching",
                    "SecItemUpdate",
                    "SecItemDelete",
                ):
                    self.assertNotIn(forbidden, source, path)
            if path not in {
                CREDENTIAL_STORE_PATH,
                ANTHROPIC_ADAPTER_PATH,
                AI_CORE_BRIDGE_PATH,
            }:
                self.assertNotIn("anthropic-messages-v1", source, path)
            if path != CREDENTIAL_STORE_PATH:
                self.assertNotIn("openai-responses-v1", source, path)
            if path != ANTHROPIC_ADAPTER_PATH:
                for forbidden in (
                    "api.anthropic.com",
                    '"x-api-key"',
                    '"anthropic-version"',
                ):
                    self.assertNotIn(forbidden, source, path)
            if path not in {
                ANTHROPIC_ADAPTER_PATH,
                AI_CORE_BRIDGE_PATH,
            }:
                self.assertNotIn('"claude-sonnet-4-6"', source, path)
            if path.suffix == ".swift" and path != ANTHROPIC_ADAPTER_PATH:
                self.assertNotIn('"POST"', source, path)

        for forbidden in (
            "api.openai.com",
            "URLSession.shared",
            "URLSessionConfiguration.default",
            "URLSessionConfiguration.background",
            ".uploadTask(",
            ".downloadTask(",
            ".webSocketTask(",
            ".streamTask(",
            "URLCredential(trust:",
            "SecTrustEvaluate",
            '"Authorization"',
            "httpAdditionalHeaders",
            "RemoteAiTransport",
            "AIProviderAdapter",
        ):
            self.assertNotIn(forbidden, combined)

        self.assertEqual(lifecycle.count("URLSessionConfiguration.ephemeral"), 1)
        self.assertEqual(lifecycle.count("session = URLSession("), 1)
        self.assertEqual(lifecycle.count("session.dataTask(with:"), 1)
        self.assertEqual(lifecycle.count("completionHandler(nil)"), 1)
        self.assertIn("configuration.httpCookieStorage = nil", lifecycle)
        self.assertIn("configuration.urlCache = nil", lifecycle)
        self.assertIn("configuration.urlCredentialStorage = nil", lifecycle)
        self.assertIn("configuration.httpShouldSetCookies = false", lifecycle)
        self.assertIn("configuration.waitsForConnectivity = false", lifecycle)
        self.assertIn("configuration.isDiscretionary = false", lifecycle)
        self.assertIn(".reloadIgnoringLocalCacheData", lifecycle)
        self.assertIn("384 * 1_024", lifecycle)
        self.assertIn("64 * 1_024", lifecycle)
        self.assertIn("60 * 1_000_000_000", lifecycle)
        self.assertLess(
            lifecycle.index("let now = clock.nowNanoseconds()"),
            lifecycle.index("let preparation = preparer.prepare("),
        )
        self.assertIn("deadlineNanoseconds: deadline", lifecycle)
        self.assertIn(".performDefaultHandling", lifecycle)
        self.assertIn(".cancelAuthenticationChallenge", lifecycle)
        self.assertNotIn("NativeAIFoundationNetworkFactory()", lifecycle)

        self.assertEqual(anthropic.count("https://api.anthropic.com/v1/messages"), 1)
        self.assertEqual(anthropic.count('"x-api-key"'), 1)
        self.assertEqual(anthropic.count('"anthropic-version"'), 1)
        self.assertEqual(anthropic.count('"claude-sonnet-4-6"'), 1)
        self.assertEqual(anthropic.count('static let method = "POST"'), 1)
        self.assertEqual(
            anthropic.count("request.httpMethod = AnthropicMessagesV1Constants.method"),
            1,
        )
        for required in (
            "struct AnthropicMessagesV1Adapter",
            "struct AnthropicMessagesV1JSONParser",
            "maximumInputBytes = 256 * 1_024",
            "maximumRequestBytes = 384 * 1_024",
            "maximumResponseBytes = 64 * 1_024",
            "maximumJSONDepth = 32",
            '"output_config"',
            '"json_schema"',
        ):
            self.assertIn(required, anthropic)
        raw_anthropic = ANTHROPIC_ADAPTER_PATH.read_text(encoding="utf-8")
        self.assertIn("#if DEBUG", raw_anthropic)
        self.assertIn("AnthropicMessagesV1TestHarness", raw_anthropic)
        self.assertNotIn("AnthropicMessagesV1TestHarness", anthropic)
        for forbidden in (
            "URLSession",
            "AIProviderCredentialStore",
            "AIProviderCredentialRequestReading",
            "NativeAIRemoteLifecycleKernel",
            "NativeAIFoundationNetworkFactory",
            "UserDefaults",
            "FileManager",
            "NSPasteboard",
            "print(",
            "Logger(",
            "os_log",
        ):
            self.assertNotIn(forbidden, anthropic)

        for call in (
            "SecItemAdd",
            "SecItemCopyMatching",
            "SecItemUpdate",
            "SecItemDelete",
        ):
            self.assertEqual(credential.count(call), 1)
        for required in (
            "kSecClassGenericPassword",
            "kSecUseDataProtectionKeychain: true",
            'service = "se.mjukis.dux.ai-provider-key.v1"',
            "kSecAttrSynchronizable: false",
            "kSecAttrAccessibleWhenUnlockedThisDeviceOnly",
            "kSecUseAuthenticationContext",
            "interactionNotAllowed = true",
            "AIProviderCredentialSettingsStoring",
            "AIProviderCredentialRequestReading",
            "maximumUTF8ByteCount = 512",
        ):
            self.assertIn(required, credential)
        settings_capability = re.search(
            r"protocol AIProviderCredentialSettingsStoring: Sendable\s*"
            r"\{(?P<body>.*?)\n\}",
            credential,
            re.DOTALL,
        )
        request_capability = re.search(
            r"protocol AIProviderCredentialRequestReading: Sendable\s*"
            r"\{(?P<body>.*?)\n\}",
            credential,
            re.DOTALL,
        )
        self.assertIsNotNone(settings_capability)
        self.assertIsNotNone(request_capability)
        self.assertNotIn("readForSingleRequest", settings_capability.group("body"))
        for mutation in ("presence", "replace", "delete"):
            self.assertNotIn(mutation, request_capability.group("body"))
        for forbidden in (
            "UserDefaults",
            "FileManager",
            "NSPasteboard",
            "print(",
            "Logger(",
            "os_log",
            "URLSession",
            "URLRequest",
        ):
            self.assertNotIn(forbidden, credential)

        def assert_only_in(symbol: str, allowed: set[Path]) -> None:
            offenders = [
                path.relative_to(REPO_ROOT).as_posix()
                for path, source in sources.items()
                if path not in allowed and symbol in source
            ]
            self.assertEqual(offenders, [], f"{symbol} escaped into {offenders}")

        # Declarations stay in their owning boundary; the orchestrator is the
        # only production consumer of all four narrow capability families.
        assert_only_in(
            "AIProviderCredentialRequestReading",
            {CREDENTIAL_STORE_PATH, ORCHESTRATOR_PATH},
        )
        assert_only_in(
            "AnthropicMessagesV1Adapter",
            {ANTHROPIC_ADAPTER_PATH, ORCHESTRATOR_PATH},
        )
        for lifecycle_symbol in (
            "NativeAIRemoteLifecycleKernel",
            "NativeAIRemotePreparer",
            "NativeAISystemClock",
            "NativeAIFoundationNetworkFactory",
        ):
            assert_only_in(lifecycle_symbol, {LIFECYCLE_PATH, ORCHESTRATOR_PATH})
        assert_only_in(
            "NativeAIRemoteSealedRequest",
            {LIFECYCLE_PATH, ANTHROPIC_ADAPTER_PATH, ORCHESTRATOR_PATH},
        )
        for core_bridge_symbol in (
            "NativeAIAnthropicMessagesV1PreviewConsuming",
            "NativeAIAnthropicMessagesV1Attempt",
            "NativeAIAnthropicMessagesV1Binding",
        ):
            assert_only_in(core_bridge_symbol, {ORCHESTRATOR_PATH, ENGINE_SERVICE_PATH})
        assert_only_in(
            "NativeAIAnthropicMessagesV1CoreValidatedResult",
            {
                ORCHESTRATOR_PATH,
                LIFECYCLE_PATH,
                ENGINE_SERVICE_PATH,
                AI_COORDINATOR_PATH,
            },
        )

        native_ai_graph = {
            CREDENTIAL_STORE_PATH,
            ANTHROPIC_ADAPTER_PATH,
            LIFECYCLE_PATH,
            ORCHESTRATOR_PATH,
            ENGINE_SERVICE_PATH,
            GENERATED_SWIFT_PATH,
            AI_COORDINATOR_PATH,
            AI_SETTINGS_PATH,
            AI_HOST_ADAPTER_PATH,
            *AI_PRESENTATION_ROOT.glob("*.swift"),
            REPO_ROOT / "dux-macos/Dux/App/AppRuntime.swift",
        }
        for path, source in sources.items():
            if path.suffix != ".swift" or path in native_ai_graph:
                continue
            for forbidden in (
                "AnthropicMessagesV1",
                "AiExplanation",
                "NativeAI",
            ):
                self.assertNotIn(forbidden, source, path)

        # The exact consent coordinator is the sole product caller. Settings
        # may read only its immutable reviewed disclosure; it cannot start it.
        assert_only_in(
            "NativeAIAnthropicMessagesV1Orchestrator",
            {ORCHESTRATOR_PATH, AI_COORDINATOR_PATH},
        )
        self.assertEqual(
            [
                path.relative_to(REPO_ROOT).as_posix()
                for path, source in sources.items()
                if re.search(
                    r"\bNativeAIAnthropicMessagesV1Orchestrator\s*\(",
                    source,
                )
            ],
            ["dux-macos/Dux/Services/ExplorerAIExplanationService.swift"],
        )
        coordinator = sources[AI_COORDINATOR_PATH]
        self.assertIn(
            "NativeAIAnthropicMessagesV1Orchestrator( previewConsumer: lease )",
            squash(coordinator),
        )
        for forbidden in (
            "SwiftUI",
            "AppModel",
            "UserDefaults",
            "SQLite",
            "Candidate",
            "Planner",
            "Approval",
            "Scheduler",
            "Executor",
            "executeTrash",
            "startRustTargetCleanup",
            "startRustTargetDryRun",
            "URLSession",
            "FileManager",
            "NSPasteboard",
            "print(",
            "Logger(",
            "os_log",
        ):
            self.assertNotIn(forbidden, coordinator)

        consent_view = sources[AI_VIEW_PATH]
        self.assertEqual(consent_view.count("Link("), 1)
        self.assertIn("destination: disclosure.providerPolicyURL", consent_view)
        consent_view_code = without_swift_comments_and_literals(consent_view)
        for forbidden in (
            "AttributedString",
            "Markdown",
            "openURL",
            "NSDataDetector",
            "executeTrash",
            "CleanupConfirmation",
            "PlanReview",
        ):
            self.assertNotIn(forbidden, consent_view_code)
        self.assertIn("Text(verbatim: result.summary)", consent_view)
        self.assertIn("Text(verbatim: group.reason)", consent_view)

        # Compiler and capability proof: the AI module has no app dependency,
        # Browser/action owners know no AI type, and raw group membership stays
        # private except at the single reviewed transport SPI import.
        ai_presentation_allowlist = {
            AI_MODEL_PATH,
            AI_TYPES_PATH,
            AI_PREVIEW_PATH,
            AI_COORDINATOR_PATH,
            AI_VIEW_PATH,
            AI_HOST_ADAPTER_PATH,
            REPO_ROOT / "dux-macos/Dux/App/AppRuntime.swift",
        }
        for path, source in sources.items():
            if "ExplorerAIExplanation" in source and path not in ai_presentation_allowlist:
                self.fail(
                    "Explorer AI presentation escaped into "
                    + path.relative_to(REPO_ROOT).as_posix()
                )
        browser = without_swift_comments_and_literals(sources[AI_BROWSER_PATH])
        browser_view = without_swift_comments_and_literals(
            sources[SNAPSHOT_BROWSER_VIEW_PATH]
        )
        treemap_view = without_swift_comments_and_literals(
            sources[SNAPSHOT_TREEMAP_VIEW_PATH]
        )
        for source, label in (
            (browser, "Browser model"),
            (browser_view, "Browser view"),
            (treemap_view, "Treemap view"),
        ):
            for forbidden in (
                "ExplorerAIExplanation",
                "aiExplanation",
                "snapshotNodeIDs",
            ):
                self.assertNotIn(forbidden, source, label)

        for action_member in (
            "reviewCandidate",
            "prepareSelectedRustTargetPlanReview",
            "makeRustTargetCleanupConfirmation",
            "startConfirmedRustTargetCleanup",
            "startRustTargetDryRun",
            "executeConfirmedTrash",
        ):
            action_source = swift_member_block(browser, action_member)
            for forbidden in (
                "aiExplanation",
                "ExplorerAIExplanation",
                "snapshotNodeIDs",
                "supplementalPresentationContext",
            ):
                self.assertNotIn(forbidden, action_source, action_member)
        trash_execution = swift_member_block(browser, "executeConfirmedTrash")
        self.assertIn("ExplorerTrashConfirmation", trash_execution)
        signature = trash_execution[: trash_execution.index("{")]
        self.assertNotIn("UInt64", signature)
        self.assertNotIn("String", signature)

        module_sources = {
            path: sources[path] for path in AI_PRESENTATION_ROOT.glob("*.swift")
        }
        module_code = "\n".join(
            without_swift_comments_and_literals(source)
            for source in module_sources.values()
        )
        imports = set(re.findall(r"(?m)^import\s+([A-Za-z0-9_]+)", module_code))
        self.assertLessEqual(imports, {"Foundation", "Observation", "SwiftUI"})
        for forbidden in (
            "DUX",
            "DuxFFI",
            "SnapshotReview",
            "Candidate",
            "PlanReview",
            "CleanupConfirmation",
            "Executor",
            "Scheduler",
            "Persistence",
            "executeConfirmedTrash",
            "startConfirmedRustTargetCleanup",
            "startRustTargetDryRun",
            "FileManager",
            "URLSession",
        ):
            self.assertNotIn(forbidden, module_code)
        self.assertIn("private let groups", sources[AI_TYPES_PATH])
        public_types = re.sub(
            r"(?s)@_spi\(DuxAITransport\)\s*"
            r"public struct ExplorerAIExplanationTransportGroup.*?"
            r"(?=/// A Rust-validated)",
            "",
            sources[AI_TYPES_PATH],
        )
        self.assertNotRegex(
            public_types,
            r"public\s+let\s+(?:snapshot|observed)NodeIDs",
        )
        self.assertEqual(
            [
                path.relative_to(REPO_ROOT).as_posix()
                for path, source in sources.items()
                if "@_spi(DuxAITransport) import" in source
            ],
            ["dux-macos/Dux/Services/ExplorerAIExplanationService.swift"],
        )

        for required in (
            "NativeAIAnthropicMessagesV1Orchestrator",
            "NativeAIAnthropicMessagesV1Run",
            "AIProviderCredentialRequestReading",
            "AnthropicMessagesV1Adapter",
            "NativeAIRemoteLifecycleKernel",
            "NativeAIFoundationNetworkFactory",
            "prepareRequest(",
            "extractResponse(",
        ):
            self.assertIn(required, orchestrator)
        fixed_binding = re.search(
            r"struct NativeAIAnthropicMessagesV1Binding\b(?P<body>.*?)\n\}",
            orchestrator,
            re.DOTALL,
        )
        self.assertIsNotNone(fixed_binding)
        self.assertIn("private init(", fixed_binding.group("body"))
        self.assertIn("static let trusted", fixed_binding.group("body"))
        self.assertIn(
            "readForSingleRequest( for: .anthropicMessagesV1 )",
            squash(orchestrator),
        )
        for exact_once in (
            "previewConsumer.consumeAnthropicMessagesV1PreviewOnce(",
            "readForSingleRequest(",
            "prepareRequest(",
            "extractResponse(",
            "attempt.validateOnce(",
            "kernel.start()",
        ):
            self.assertEqual(orchestrator.count(exact_once), 1, exact_once)
        consume_position = orchestrator.index(
            "previewConsumer.consumeAnthropicMessagesV1PreviewOnce("
        )
        credential_position = orchestrator.index("readForSingleRequest(")
        request_position = orchestrator.index("prepareRequest(")
        self.assertLess(consume_position, credential_position)
        self.assertLess(credential_position, request_position)
        for forbidden in (
            "AppModel",
            "SwiftUI",
            "AppKit",
            "UserDefaults",
            "Candidate",
            "Cleanup",
            "Approval",
            "Scheduler",
            "Planner",
            "Executor",
            "Persistence",
        ):
            self.assertNotIn(forbidden, orchestrator)

        # Generated UniFFI attempt types are isolated behind EngineService.
        generated_attempt_symbols = (
            "AiExplanationAttemptSession",
            "AiExplanationAttemptInfo",
            "AiExplanationAttemptError",
            "AiExplanationAttemptReleaseOutcome",
            "AiExplanationResult",
            "beginAnthropicMessagesV1Explanation",
        )
        generated = GENERATED_SWIFT_PATH.read_text(encoding="utf-8")
        for symbol in generated_attempt_symbols:
            self.assertIn(symbol, generated)
            offenders = [
                path.relative_to(REPO_ROOT).as_posix()
                for path, source in sources.items()
                if path.suffix == ".swift"
                and path not in {GENERATED_SWIFT_PATH, ENGINE_SERVICE_PATH}
                and symbol in source
            ]
            self.assertEqual(offenders, [], f"generated {symbol}: {offenders}")
        for required in (
            "NativeAIAnthropicMessagesV1PreviewConsuming",
            "beginAnthropicMessagesV1Explanation",
        ):
            self.assertIn(required, engine_service)
        self.assertRegex(
            engine_service,
            r"validateOnce\s*\(\s*outputJsonUtf8:",
        )

        cli = "\n".join(
            path.read_text(encoding="utf-8")
            for path in (REPO_ROOT / "dux-cli/src").rglob("*.rs")
        )
        self.assertNotIn("AiExplanation", cli)

        for source in (credential, lifecycle, anthropic, orchestrator):
            for forbidden in ("Candidate", "Cleanup", "Approval", "SwiftUI", "AppKit"):
                self.assertNotIn(forbidden, source)

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
        self.assertIn("const FFI_CONTRACT_VERSION: u32 = 62;", read("dux-ffi/src/lib.rs"))
        self.assertEqual(read("dux-macos/Config/Release.entitlements").count("<key>"), 0)

    def test_ffi_v61_attempt_is_fixed_single_use_and_drained_first(self) -> None:
        ffi = read("dux-ffi/src/lib.rs")
        self.assertIn("const FFI_CONTRACT_VERSION: u32 = 62;", ffi)
        self.assertNotIn("AiExplanationAttemptRequest", ffi)

        provider = re.search(
            r"pub enum AiExplanationProvider\s*\{(?P<body>.*?)\n\}", ffi, re.DOTALL
        )
        transport = re.search(
            r"pub enum AiExplanationTransport\s*\{(?P<body>.*?)\n\}", ffi, re.DOTALL
        )
        self.assertIsNotNone(provider)
        self.assertIsNotNone(transport)
        self.assertEqual(
            re.findall(
                r"^\s*([A-Z][A-Za-z0-9_]*)\s*,",
                provider.group("body"),
                re.MULTILINE,
            ),
            ["Anthropic"],
        )
        self.assertEqual(
            re.findall(
                r"^\s*([A-Z][A-Za-z0-9_]*)\s*,",
                transport.group("body"),
                re.MULTILINE,
            ),
            ["MessagesV1"],
        )

        begin = re.search(
            r"pub fn begin_anthropic_messages_v1_explanation\s*"
            r"\((?P<body>.*?)\)\s*->\s*"
            r"Result<Arc<AiExplanationAttemptSession>,\s*AiExplanationAttemptError>",
            ffi,
            re.DOTALL,
        )
        self.assertIsNotNone(begin)
        self.assertEqual(
            squash(begin.group("body")).strip(),
            "&self, preview: Arc<AiMetadataPreviewSession>,",
        )

        exported_start = ffi.index("#[uniffi::export]\nimpl AiExplanationAttemptSession")
        impl_start = ffi.index("impl AiExplanationAttemptSession", exported_start)
        private_start = ffi.index(
            "\nimpl AiExplanationAttemptSession",
            impl_start + len("impl AiExplanationAttemptSession"),
        )
        exported_attempt = ffi[exported_start:private_start]
        self.assertEqual(
            re.findall(r"pub fn\s+([a-z][a-z0-9_]*)\s*\(", exported_attempt),
            ["info", "validate_once", "release"],
        )
        validate = re.search(
            r"pub fn validate_once\s*\((?P<body>.*?)\)\s*->\s*"
            r"Result<AiExplanationResult,\s*AiExplanationAttemptError>",
            exported_attempt,
            re.DOTALL,
        )
        self.assertIsNotNone(validate)
        self.assertEqual(
            squash(validate.group("body")).strip(),
            "&self, output_json_utf8: Vec<u8>,",
        )

        for struct_name in (
            "AiExplanationAttemptInfo",
            "AiExplanationGroup",
            "AiExplanationResult",
        ):
            public_record = re.search(
                rf"pub struct {struct_name}\s*\{{(?P<body>.*?)\n\}}",
                ffi,
                re.DOTALL,
            )
            self.assertIsNotNone(public_record)
            public_fields = re.findall(
                r"pub\s+([a-z][a-z0-9_]*)\s*:", public_record.group("body")
            )
            for forbidden in (
                "url",
                "endpoint",
                "header",
                "credential",
                "secret",
                "request",
                "body",
                "callback",
                "validator",
                "path",
                "action",
                "plan",
                "approval",
                "schedule",
                "executor",
                "cache",
                "persistence",
            ):
                offenders = [
                    field for field in public_fields if forbidden in field.split("_")
                ]
                self.assertEqual(offenders, [], f"{struct_name}: {forbidden}")

        normalized_begin = squash(begin.group("body")).strip().lower()
        for forbidden in (
            "provider",
            "model",
            "url",
            "header",
            "credential",
            "request",
            "body",
            "callback",
            "validator",
            "digest",
            "json",
            "node",
        ):
            self.assertNotIn(forbidden, normalized_begin)

        self.assertEqual(
            len(
                re.findall(
                    r"release_registered_ai_explanation_attempts\(\);\s*"
                    r"self\.release_registered_ai_metadata_previews\(\);",
                    ffi,
                )
            ),
            2,
        )
        self.assertEqual(
            len(
                re.findall(
                    r"release_ai_explanation_attempt_registry\(&ai_explanation_attempts\);\s*"
                    r"release_ai_metadata_preview_registry\(&ai_metadata_previews\);",
                    ffi,
                )
            ),
            1,
        )
        reset_start = ffi.index("fn drain_registered_ffi_children_for_reset(")
        reset_end = ffi.index("\nfn take_live_registry", reset_start)
        reset_drain = ffi[reset_start:reset_end]
        self.assertLess(
            reset_drain.index("ai_explanation_attempts:"),
            reset_drain.index("ai_metadata_previews:"),
        )
        self.assertLess(
            reset_drain.index("for attempt in take_live_registry(ai_explanation_attempts)"),
            reset_drain.index("for preview in take_live_registry(ai_metadata_previews)"),
        )

        bridge = read("dux-core/src/engine/ai_metadata_preview.rs")
        for required in (
            "validate_cacheable_explanation_output_v1(output_json_utf8)",
            "validate_explanation_output_v1(canonical_output_json_utf8)",
            "snapshot_node_ids: source",
        ):
            self.assertIn(required, bridge)
        for forbidden in (
            "std::net",
            "reqwest",
            "URLSession",
            "crate::planner",
            "crate::executor",
            "crate::persistence",
        ):
            self.assertNotIn(forbidden, bridge)

        ai_module = read("dux-core/src/ai/mod.rs")
        for required in (
            "let snapshot_node_ids = shaped.included_snapshot_node_ids();",
            "for request_local_id in group.input_node_ids()",
            "mapped_ids.push(",
        ):
            self.assertIn(required, ai_module)

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

    def test_preview_boundary_accepts_no_privacy_or_transport_facts(self) -> None:
        bridge_path = REPO_ROOT / "dux-core/src/engine/ai_metadata_preview.rs"
        self.assertTrue(bridge_path.is_file())
        bridge = bridge_path.read_text(encoding="utf-8")
        self.assertIn("use crate::ai::{", bridge)
        self.assertIn("shape_ai_metadata_preview_v1", bridge)
        self.assertIn("AI_METADATA_PREVIEW_LIFETIME", bridge)
        for forbidden in (
            "crate::cleanup",
            "crate::planner",
            "crate::executor",
            "crate::domain::candidate",
            "crate::domain::rule",
            "std::fs",
            "std::io",
            "std::net",
            "std::path",
            "std::process",
        ):
            self.assertNotIn(forbidden, bridge)

        consumers = []
        for path in (REPO_ROOT / "dux-core/src").rglob("*.rs"):
            if (REPO_ROOT / "dux-core/src/ai") in path.parents or is_test_source(path):
                continue
            if "crate::ai" in path.read_text(encoding="utf-8"):
                consumers.append(path.relative_to(REPO_ROOT).as_posix())
        self.assertEqual(
            sorted(consumers),
            [
                "dux-core/src/engine/ai_insight_cache.rs",
                "dux-core/src/engine/ai_metadata_preview.rs",
            ],
        )

        ffi = read("dux-ffi/src/lib.rs")
        request = re.search(
            r"pub struct AiMetadataPreviewRequest\s*\{(?P<body>.*?)\n\}",
            ffi,
            re.DOTALL,
        )
        self.assertIsNotNone(request)
        fields = re.findall(r"pub\s+([a-z][a-z0-9_]*)\s*:", request.group("body"))
        self.assertEqual(fields, ["record_version", "selected_node_id"])
        signature = re.search(
            r"pub fn prepare_ai_metadata_preview\s*\((?P<body>.*?)\)\s*"
            r"->\s*Result<Arc<AiMetadataPreviewSession>,\s*AiMetadataPreviewError>",
            ffi,
            re.DOTALL,
        )
        self.assertIsNotNone(signature)
        normalized_signature = squash(signature.group("body"))
        self.assertIn("parent: Arc<SnapshotReviewSession>", normalized_signature)
        self.assertIn("request: AiMetadataPreviewRequest", normalized_signature)
        for forbidden in (
            "json",
            "digest",
            "coverage",
            "privacy",
            "provider",
            "model",
            "url",
            "header",
            "credential",
            "callback",
            "driver",
            "plan",
            "path",
        ):
            self.assertNotIn(forbidden, normalized_signature.lower())

        session_api = re.search(
            r"impl AiMetadataPreviewSession\s*\{(?P<body>.*?)\n\}",
            ffi,
            re.DOTALL,
        )
        self.assertIsNotNone(session_api)
        self.assertIn("pub fn info", session_api.group("body"))
        self.assertIn("pub fn release", session_api.group("body"))
        self.assertIn(
            "pub fn load_cached_anthropic_messages_v1_explanation",
            session_api.group("body"),
        )
        for forbidden in (
            "consume",
            "send",
            "execute",
            "callback",
            "credential",
            "urlrequest",
            "urlsession",
        ):
            self.assertNotIn(forbidden, session_api.group("body").lower())

        for required in (
            "pub content_included: bool",
            "pub source_names_included: bool",
            "pub source_paths_included: bool",
        ):
            self.assertIn(required, ffi)

        native_consumers = []
        for path in (REPO_ROOT / "dux-macos").rglob("*.swift"):
            if is_test_source(path):
                continue
            source = path.read_text(encoding="utf-8")
            if "AIMetadataPreview" in source or "AiMetadataPreview" in source:
                native_consumers.append(path.relative_to(REPO_ROOT).as_posix())
        self.assertEqual(
            sorted(native_consumers),
            sorted(
                [
                    "dux-macos/Dux/Generated/DuxFFI.swift",
                    "dux-macos/Dux/Services/EngineService.swift",
                    "dux-macos/Dux/Services/ExplorerAIExplanationService.swift",
                    "dux-macos/Dux/Services/SnapshotReviewController.swift",
                    "dux-macos/DuxAIExplanationPresentation/ExplorerAIMetadataPreview.swift",
                    "dux-macos/DuxAIExplanationPresentation/ExplorerAIExplanationModel.swift",
                    "dux-macos/DuxAIExplanationPresentation/ExplorerAIExplanationTypes.swift",
                ]
            ),
        )
        cli = "\n".join(
            path.read_text(encoding="utf-8")
            for path in (REPO_ROOT / "dux-cli/src").rglob("*.rs")
        )
        self.assertNotIn("AiMetadataPreview", cli)


if __name__ == "__main__":
    unittest.main()
