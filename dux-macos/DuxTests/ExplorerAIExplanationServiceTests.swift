@testable import DUX
import XCTest

final class ExplorerAIExplanationServiceTests: XCTestCase {
    func testReviewedDisclosureMatchesFixedAdapterAndPrivacyFacts() {
        let disclosure = reviewedAnthropicMessagesV1Disclosure

        XCTAssertEqual(disclosure.providerName, "Anthropic")
        XCTAssertEqual(disclosure.adapterID, AnthropicMessagesV1Constants.adapterID)
        XCTAssertEqual(
            disclosure.adapterRevision,
            AnthropicMessagesV1Constants.adapterRevision
        )
        XCTAssertEqual(disclosure.model, AnthropicMessagesV1Constants.model)
        XCTAssertEqual(
            disclosure.maximumMetadataInputBytes,
            AnthropicMessagesV1Constants.maximumInputBytes
        )
        XCTAssertEqual(
            disclosure.maximumEncodedRequestBytes,
            AnthropicMessagesV1Constants.maximumRequestBytes
        )
        XCTAssertEqual(
            disclosure.maximumResponseBytes,
            AnthropicMessagesV1Constants.maximumResponseBytes
        )
        XCTAssertEqual(disclosure.maximumOutputTokens, AnthropicMessagesV1Constants.maxTokens)
        XCTAssertEqual(disclosure.standardAPIDeletionWithinDays, 30)
        XCTAssertEqual(disclosure.flaggedInputOutputRetentionYears, 2)
        XCTAssertEqual(disclosure.safetyScoreRetentionYears, 7)
        XCTAssertEqual(disclosure.structuredOutputGrammarMayBeCachedHours, 24)
        XCTAssertTrue(disclosure.billingMayApply)
        XCTAssertTrue(disclosure.hasStatedRetentionExceptions)
        XCTAssertTrue(disclosure.mayRetainLongerForSafetyOrLegalReasons)
        XCTAssertFalse(disclosure.zeroDataRetentionIsInferred)
        XCTAssertEqual(disclosure.providerPolicyURL.scheme, "https")
    }

    func testValidatedResultMapsOnlyWhenEveryDisclosureBindingMatches() throws {
        let disclosure = aiDisclosureFixture()
        let validated = aiValidatedResultFixture()

        let result = try NativeExplorerAIExplanationSession.map(
            validated,
            disclosure: disclosure
        )

        XCTAssertEqual(result.providerName, "Anthropic")
        XCTAssertEqual(result.sourceScanID, "scan:ai")
        XCTAssertEqual(result.selectedRootNodeID, 42)
        XCTAssertEqual(result.inputDigestSHA256, String(repeating: "a", count: 64))
        XCTAssertEqual(result.groups.count, 1)
        XCTAssertEqual(result.groups[0].snapshotNodeIDs, [7, 8])
        XCTAssertEqual(result.group(containing: 8)?.id, 1)
        XCTAssertNil(result.group(containing: 9))
    }

    func testMismatchedDigestRejectsWholePresentation() {
        let disclosure = aiDisclosureFixture()
        let source = aiValidatedResultFixture()
        let mismatched = NativeAIAnthropicMessagesV1CoreValidatedResult(
            recordVersion: source.recordVersion,
            inputSchemaVersion: source.inputSchemaVersion,
            outputSchemaVersion: source.outputSchemaVersion,
            privacyPolicyRevision: source.privacyPolicyRevision,
            providerBindingRevision: source.providerBindingRevision,
            binding: source.binding,
            inputDigestSHA256: String(repeating: "b", count: 64),
            sourceScanID: source.sourceScanID,
            selectedRootNodeID: source.selectedRootNodeID,
            summary: source.summary,
            labels: source.labels,
            groups: source.groups,
            questions: source.questions,
            uncertainties: source.uncertainties,
            researchSuggestions: source.researchSuggestions
        )

        XCTAssertThrowsError(
            try NativeExplorerAIExplanationSession.map(
                mismatched,
                disclosure: disclosure
            )
        ) { error in
            XCTAssertEqual(
                error as? NativeAIAnthropicMessagesV1OrchestratorFailure,
                .coreRejected
            )
        }
    }

    func testCancellationBeforeRunInstallationCancelsLateRun() async {
        let lease = AIExplanationPreviewLeaseStub()
        let session = NativeExplorerAIExplanationSession(
            lease: lease,
            disclosure: aiDisclosureFixture()
        )
        let run = AIExplanationCancellableRunSpy()

        session.cancel()
        session.install(run: run)
        await session.release()

        XCTAssertEqual(run.cancellationCount, 1)
        let leaseReleases = await lease.releaseCount()
        XCTAssertEqual(leaseReleases, 1)
    }
}

private final class AIExplanationCancellableRunSpy:
    ExplorerAIExplanationCancellableRun, @unchecked Sendable
{
    private let lock = NSLock()
    private var cancellations = 0

    var cancellationCount: Int { lock.withLock { cancellations } }

    func cancel() {
        lock.withLock { cancellations += 1 }
    }
}

private actor AIExplanationPreviewLeaseStub: DuxAIMetadataPreviewLease {
    nonisolated let preview = aiPreviewFixture()
    private var releases = 0

    func readInfo() -> ExplorerAIMetadataPreviewInfo { preview }

    func consumeAnthropicMessagesV1PreviewOnce(
        deadlineNanoseconds _: UInt64,
        deadlineObservation _: NativeAIAnthropicMessagesV1DeadlineObservation
    ) async throws -> any NativeAIAnthropicMessagesV1Attempt {
        throw NativeAIAnthropicMessagesV1OrchestratorFailure.previewUnavailable
    }

    func release() {
        releases += 1
    }

    func releaseCount() -> Int { releases }
}

func aiPreviewFixture() -> ExplorerAIMetadataPreviewInfo {
    let ages = ExplorerAIMetadataPreviewAgeSummary(
        within7DaysLogicalBytes: 1024,
        days8To30LogicalBytes: 0,
        days31To90LogicalBytes: 0,
        olderThan90DaysLogicalBytes: 0,
        unknownAgeLogicalBytes: 0
    )
    return ExplorerAIMetadataPreviewInfo(
        inputSchemaVersion: 1,
        privacyPolicyRevision: 1,
        preparedAtUnixMilliseconds: 1000,
        expiresAtUnixMilliseconds: 121_000,
        inputDigestSHA256: String(repeating: "a", count: 64),
        encodedInputJSONUTF8: Data("{\"schema_version\":1}".utf8),
        inspectedNodeCount: 3,
        includedDirectChildCount: 2,
        excludedSensitiveDirectChildCount: 0,
        omittedEligibleDirectChildCount: 0,
        rootLabel: "Selected folder",
        totalLogicalBytes: 1024,
        ageSummary: ages,
        childrenComplete: true,
        omittedChildCount: 0,
        omittedLogicalBytes: 0,
        omittedAgeSummary: ExplorerAIMetadataPreviewAgeSummary(
            within7DaysLogicalBytes: 0,
            days8To30LogicalBytes: 0,
            days31To90LogicalBytes: 0,
            olderThan90DaysLogicalBytes: 0,
            unknownAgeLogicalBytes: 0
        ),
        children: [],
        contentIncluded: false,
        sourceNamesIncluded: false,
        sourcePathsIncluded: false
    )
}

func aiDisclosureFixture(
    id: UUID = UUID(),
    scanID: String = "scan:ai",
    rootNodeID: UInt64 = 42
) -> ExplorerAIExplanationDisclosure {
    let provider = reviewedAnthropicMessagesV1Disclosure
    return ExplorerAIExplanationDisclosure(
        id: id,
        sourceScanID: scanID,
        selectedRootNodeID: rootNodeID,
        providerName: provider.providerName,
        model: provider.model,
        adapterID: provider.adapterID,
        adapterRevision: provider.adapterRevision,
        maximumMetadataInputBytes: provider.maximumMetadataInputBytes,
        maximumEncodedRequestBytes: provider.maximumEncodedRequestBytes,
        maximumResponseBytes: provider.maximumResponseBytes,
        maximumOutputTokens: provider.maximumOutputTokens,
        retentionReviewedOn: provider.retentionReviewedOn,
        providerPolicyURL: provider.providerPolicyURL,
        standardAPIDeletionWithinDays: provider.standardAPIDeletionWithinDays,
        flaggedInputOutputRetentionYears: provider.flaggedInputOutputRetentionYears,
        safetyScoreRetentionYears: provider.safetyScoreRetentionYears,
        structuredOutputGrammarMayBeCachedHours:
        provider.structuredOutputGrammarMayBeCachedHours,
        billingMayApply: provider.billingMayApply,
        hasStatedRetentionExceptions: provider.hasStatedRetentionExceptions,
        mayRetainLongerForSafetyOrLegalReasons:
        provider.mayRetainLongerForSafetyOrLegalReasons,
        zeroDataRetentionIsInferred: provider.zeroDataRetentionIsInferred,
        preview: aiPreviewFixture()
    )
}

func aiValidatedResultFixture(
    scanID: String = "scan:ai",
    rootNodeID: UInt64 = 42,
    groupNodeIDs: [UInt64] = [7, 8]
) -> NativeAIAnthropicMessagesV1CoreValidatedResult {
    NativeAIAnthropicMessagesV1CoreValidatedResult(
        recordVersion: 1,
        inputSchemaVersion: 1,
        outputSchemaVersion: 1,
        privacyPolicyRevision: 1,
        providerBindingRevision: 1,
        binding: .trusted,
        inputDigestSHA256: String(repeating: "a", count: 64),
        sourceScanID: scanID,
        selectedRootNodeID: rootNodeID,
        summary: "A validated storage explanation.",
        labels: ["Build data"],
        groups: [
            NativeAIAnthropicMessagesV1ValidatedGroup(
                recordVersion: 1,
                title: "Generated artifacts",
                snapshotNodeIDs: groupNodeIDs,
                reason: "These items share an observed storage pattern."
            ),
        ],
        questions: ["Is this pattern expected?"],
        uncertainties: ["Age alone does not explain ownership."],
        researchSuggestions: ["Consider a deterministic rule after review."]
    )
}
