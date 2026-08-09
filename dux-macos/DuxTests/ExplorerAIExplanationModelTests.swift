@testable import DUX
import DuxAIExplanationPresentation
import XCTest

@MainActor
final class ExplorerAIExplanationModelTests: XCTestCase {
    func testPaginationVisibilityDoesNotInvalidateConsentAuthority() async throws {
        let context = AIExplanationContextReaderStub(
            context: aiContext(revision: 1, selected: 42, current: 1, visible: [7])
        )
        let session = try AIExplanationModelSession(
            disclosure: aiDisclosureFixture(),
            outcome: .success(aiModelResultFixture())
        )
        let model = ExplorerAIExplanationModel(
            explanations: AIExplanationImmediateService(session: session),
            contextReader: context
        )

        await model.previewSelection()
        context.context = aiContext(revision: 1, selected: 42, current: 1, visible: [8])
        await model.synchronizeContext()

        guard case let .awaitingConsent(disclosure) = model.phase else {
            return XCTFail("pagination-only visibility changed consent state")
        }
        XCTAssertEqual(disclosure.selectedRootNodeID, 42)
        XCTAssertEqual(session.releaseCount, 0)
        XCTAssertEqual(session.cancellationCount, 0)
    }

    func testResultSurvivesOnlyExplicitDescentIntoItsExactRoot() async throws {
        let context = AIExplanationContextReaderStub(
            context: aiContext(revision: 7, selected: 42, current: 1, visible: [7])
        )
        let session = try AIExplanationModelSession(
            disclosure: aiDisclosureFixture(),
            outcome: .success(aiModelResultFixture())
        )
        let model = ExplorerAIExplanationModel(
            explanations: AIExplanationImmediateService(session: session),
            contextReader: context
        )
        await model.previewSelection()
        await model.explain(disclosureID: session.disclosure.id)
        await model.explain(disclosureID: session.disclosure.id)
        XCTAssertEqual(session.explainCount, 1)
        XCTAssertNil(model.resultForCurrentDirectory)

        await model.invalidateBeforeContextChange(
            preserving: ExplorerAIExplanationPreservationScope(
                sourceScanID: "scan:ai",
                rootNodeID: 42,
                nextContextRevision: 8
            )
        )
        context.context = aiContext(
            revision: 8,
            selected: nil,
            current: 42,
            visible: [8]
        )
        await model.synchronizeContext()

        XCTAssertNotNil(model.resultForCurrentDirectory)
        XCTAssertEqual(model.decoration(forObservedNodeID: 8)?.ordinal, 1)
        XCTAssertEqual(
            model.decoration(forObservedNodeID: 8)?.title,
            "Generated artifacts"
        )
        XCTAssertNil(model.decoration(forObservedNodeID: 9))
    }

    func testWrongPreservationScopeClearsValidatedResult() async throws {
        let context = AIExplanationContextReaderStub(
            context: aiContext(revision: 3, selected: 42, current: 1, visible: [7])
        )
        let session = try AIExplanationModelSession(
            disclosure: aiDisclosureFixture(),
            outcome: .success(aiModelResultFixture())
        )
        let model = ExplorerAIExplanationModel(
            explanations: AIExplanationImmediateService(session: session),
            contextReader: context
        )
        await model.previewSelection()
        await model.explain(disclosureID: session.disclosure.id)

        await model.invalidateBeforeContextChange(
            preserving: ExplorerAIExplanationPreservationScope(
                sourceScanID: "scan:other",
                rootNodeID: 42,
                nextContextRevision: 4
            )
        )

        XCTAssertEqual(model.phase, .idle)
    }

    func testLatePreparedSessionIsCancelledAndReleasedAfterAuthorityDrift() async throws {
        let context = AIExplanationContextReaderStub(
            context: aiContext(revision: 1, selected: 42, current: 1, visible: [7])
        )
        let service = AIExplanationSuspendedPrepareService()
        let model = ExplorerAIExplanationModel(explanations: service, contextReader: context)
        let preview = Task { await model.previewSelection() }
        try await eventuallyAIModel { service.hasSuspendedPrepare }

        context.context = aiContext(revision: 2, selected: 43, current: 1, visible: [7])
        let staleSession = try AIExplanationModelSession(
            disclosure: aiDisclosureFixture(rootNodeID: 42),
            outcome: .success(aiModelResultFixture())
        )
        service.resume(with: staleSession)
        await preview.value

        XCTAssertEqual(model.phase, .idle)
        XCTAssertEqual(staleSession.cancellationCount, 1)
        XCTAssertEqual(staleSession.releaseCount, 1)
    }

    func testTerminalFenceJoinsLatePrepareAndRejectsPublication() async throws {
        let context = AIExplanationContextReaderStub(
            context: aiContext(revision: 1, selected: 42, current: 1, visible: [7])
        )
        let service = AIExplanationSuspendedPrepareService()
        let model = ExplorerAIExplanationModel(explanations: service, contextReader: context)
        let preview = Task { await model.previewSelection() }
        try await eventuallyAIModel { service.hasSuspendedPrepare }

        let fence = model.beginTerminalFence()
        let staleSession = try AIExplanationModelSession(
            disclosure: aiDisclosureFixture(),
            outcome: .success(aiModelResultFixture())
        )
        service.resume(with: staleSession)
        await fence.value
        await preview.value

        XCTAssertEqual(model.phase, .idle)
        XCTAssertFalse(model.canPreview)
        XCTAssertEqual(staleSession.cancellationCount, 1)
        XCTAssertEqual(staleSession.releaseCount, 1)
    }

    func testCancelLatchesAcrossLatePrepareInstallation() async throws {
        let context = AIExplanationContextReaderStub(
            context: aiContext(revision: 1, selected: 42, current: 1, visible: [7])
        )
        let service = AIExplanationSuspendedPrepareService()
        let model = ExplorerAIExplanationModel(explanations: service, contextReader: context)
        let preview = Task { await model.previewSelection() }
        try await eventuallyAIModel { service.hasSuspendedPrepare }

        await model.cancel()
        let staleSession = try AIExplanationModelSession(
            disclosure: aiDisclosureFixture(),
            outcome: .success(aiModelResultFixture())
        )
        service.resume(with: staleSession)
        await preview.value

        XCTAssertEqual(model.phase, .idle)
        XCTAssertEqual(staleSession.cancellationCount, 1)
        XCTAssertEqual(staleSession.releaseCount, 1)
    }

    func testTerminalFenceCancelsAndJoinsActiveExplanation() async throws {
        let context = AIExplanationContextReaderStub(
            context: aiContext(revision: 1, selected: 42, current: 1, visible: [7])
        )
        let session = AIExplanationModelSession(
            disclosure: aiDisclosureFixture(),
            outcome: .suspendedUntilCancelled
        )
        let model = ExplorerAIExplanationModel(
            explanations: AIExplanationImmediateService(session: session),
            contextReader: context
        )
        await model.previewSelection()
        let explanation = Task { await model.explain(disclosureID: session.disclosure.id) }
        try await eventuallyAIModel { session.hasSuspendedExplanation }

        let fence = model.beginTerminalFence()
        await fence.value
        await explanation.value

        XCTAssertEqual(model.phase, .idle)
        XCTAssertEqual(session.cancellationCount, 1)
        XCTAssertEqual(session.releaseCount, 1)
    }
}

@MainActor
private final class AIExplanationContextReaderStub: ExplorerAIExplanationContextReading {
    var context: ExplorerAIExplanationContext

    init(context: ExplorerAIExplanationContext) {
        self.context = context
    }

    var aiExplanationContext: ExplorerAIExplanationContext { context }
}

private struct AIExplanationImmediateService: ExplorerAIExplanationServing {
    let session: AIExplanationModelSession

    func prepare(
        scanID _: String,
        selectedRootNodeID _: UInt64
    ) async throws -> any ExplorerAIExplanationSession {
        session
    }
}

private final class AIExplanationSuspendedPrepareService: ExplorerAIExplanationServing,
    @unchecked Sendable
{
    private let lock = NSLock()
    private var continuation:
        CheckedContinuation<any ExplorerAIExplanationSession, any Error>?

    var hasSuspendedPrepare: Bool { lock.withLock { continuation != nil } }

    func prepare(
        scanID _: String,
        selectedRootNodeID _: UInt64
    ) async throws -> any ExplorerAIExplanationSession {
        try await withCheckedThrowingContinuation { continuation in
            lock.withLock { self.continuation = continuation }
        }
    }

    func resume(with session: any ExplorerAIExplanationSession) {
        let continuation = lock.withLock { () -> CheckedContinuation<
            any ExplorerAIExplanationSession,
            any Error
        >? in
            let continuation = self.continuation
            self.continuation = nil
            return continuation
        }
        continuation?.resume(returning: session)
    }
}

private final class AIExplanationModelSession: ExplorerAIExplanationSession,
    @unchecked Sendable
{
    enum Outcome {
        case success(ExplorerAIExplanationResult)
        case suspendedUntilCancelled
    }

    let disclosure: ExplorerAIExplanationDisclosure
    private let lock = NSLock()
    private let outcome: Outcome
    private var explains = 0
    private var cancellations = 0
    private var releaseInvocations = 0
    private var explanationContinuation:
        CheckedContinuation<ExplorerAIExplanationResult, any Error>?

    init(disclosure: ExplorerAIExplanationDisclosure, outcome: Outcome) {
        self.disclosure = disclosure
        self.outcome = outcome
    }

    var cancellationCount: Int { lock.withLock { cancellations } }
    var explainCount: Int { lock.withLock { explains } }
    var releaseCount: Int { lock.withLock { releaseInvocations } }
    var hasSuspendedExplanation: Bool {
        lock.withLock { explanationContinuation != nil }
    }

    func explain() async throws -> ExplorerAIExplanationResult {
        lock.withLock { explains += 1 }
        switch outcome {
        case let .success(result):
            return result
        case .suspendedUntilCancelled:
            return try await withCheckedThrowingContinuation { continuation in
                let alreadyCancelled = lock.withLock { () -> Bool in
                    guard cancellations == 0 else { return true }
                    explanationContinuation = continuation
                    return false
                }
                if alreadyCancelled {
                    continuation.resume(throwing: ExplorerAIExplanationFailure.cancelled)
                }
            }
        }
    }

    func cancel() {
        let continuation = lock.withLock { () -> CheckedContinuation<
            ExplorerAIExplanationResult,
            any Error
        >? in
            cancellations += 1
            let continuation = explanationContinuation
            explanationContinuation = nil
            return continuation
        }
        continuation?.resume(throwing: ExplorerAIExplanationFailure.cancelled)
    }

    func release() async {
        lock.withLock { releaseInvocations += 1 }
    }
}

@MainActor
private func aiModelResultFixture() throws -> ExplorerAIExplanationResult {
    try NativeExplorerAIExplanationSession.map(
        aiValidatedResultFixture(),
        disclosure: aiDisclosureFixture()
    )
}

private func aiContext(
    revision: UInt64,
    selected: UInt64?,
    current: UInt64?,
    visible: Set<UInt64>
) -> ExplorerAIExplanationContext {
    ExplorerAIExplanationContext(
        revision: revision,
        sourceScanID: "scan:ai",
        selectedDirectoryNodeID: selected,
        currentDirectoryNodeID: current,
        isEligible: true,
        visibleObservedNodeIDs: visible
    )
}

private func eventuallyAIModel(
    _ predicate: @escaping @Sendable () async -> Bool
) async throws {
    for _ in 0 ..< 2000 {
        if await predicate() { return }
        await Task.yield()
    }
    XCTFail("condition did not become true")
}
