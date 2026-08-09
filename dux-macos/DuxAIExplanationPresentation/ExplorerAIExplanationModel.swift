import Foundation
import Observation

public protocol ExplorerAIExplanationSession: AnyObject, Sendable {
    var disclosure: ExplorerAIExplanationDisclosure { get }
    func explain() async throws -> ExplorerAIExplanationResult
    func cancel()
    func release() async
}

/// The only provider-facing capability accepted by the presentation module.
/// It can prepare one opaque, single-use session and grants no other authority.
public protocol ExplorerAIExplanationServing: Sendable {
    func prepare(
        scanID: String,
        selectedRootNodeID: UInt64
    ) async throws -> any ExplorerAIExplanationSession
}

public struct UnavailableExplorerAIExplanationService: ExplorerAIExplanationServing {
    public init() {}

    public func prepare(
        scanID _: String,
        selectedRootNodeID _: UInt64
    ) async throws -> any ExplorerAIExplanationSession {
        throw ExplorerAIMetadataPreviewError.unavailable
    }
}

/// Immutable read-only facts from Explorer. Visible membership is deliberately
/// excluded from the authority identity so pagination alone cannot consume or
/// invalidate a consent session.
public struct ExplorerAIExplanationContext: Equatable, Sendable {
    public let revision: UInt64
    public let sourceScanID: String?
    public let selectedDirectoryNodeID: UInt64?
    public let currentDirectoryNodeID: UInt64?
    public let isEligible: Bool
    public let visibleObservedNodeIDs: Set<UInt64>

    public init(
        revision: UInt64,
        sourceScanID: String?,
        selectedDirectoryNodeID: UInt64?,
        currentDirectoryNodeID: UInt64?,
        isEligible: Bool,
        visibleObservedNodeIDs: Set<UInt64>
    ) {
        self.revision = revision
        self.sourceScanID = sourceScanID
        self.selectedDirectoryNodeID = selectedDirectoryNodeID
        self.currentDirectoryNodeID = currentDirectoryNodeID
        self.isEligible = isEligible
        self.visibleObservedNodeIDs = visibleObservedNodeIDs
    }

    fileprivate var authorityIdentity: AuthorityIdentity {
        AuthorityIdentity(
            revision: revision,
            sourceScanID: sourceScanID,
            selectedDirectoryNodeID: selectedDirectoryNodeID,
            currentDirectoryNodeID: currentDirectoryNodeID,
            isEligible: isEligible
        )
    }
}

@MainActor
public protocol ExplorerAIExplanationContextReading: AnyObject {
    var aiExplanationContext: ExplorerAIExplanationContext { get }
}

/// Explicit authority to retain only an already validated inert result while
/// Explorer descends into that result's exact root. Sessions are never retained.
public struct ExplorerAIExplanationPreservationScope: Equatable, Sendable {
    public let sourceScanID: String
    public let rootNodeID: UInt64
    public let nextContextRevision: UInt64

    public init(sourceScanID: String, rootNodeID: UInt64, nextContextRevision: UInt64) {
        self.sourceScanID = sourceScanID
        self.rootNodeID = rootNodeID
        self.nextContextRevision = nextContextRevision
    }
}

private struct AuthorityIdentity: Equatable, Sendable {
    let revision: UInt64
    let sourceScanID: String?
    let selectedDirectoryNodeID: UInt64?
    let currentDirectoryNodeID: UInt64?
    let isEligible: Bool
}

/// Serializes lifecycle ownership for one opaque provider session. Cancellation
/// is latched and every release caller joins the same single release invocation.
private final class ManagedExplanationSession: @unchecked Sendable {
    let disclosure: ExplorerAIExplanationDisclosure

    private let lock = NSLock()
    private let session: any ExplorerAIExplanationSession
    private var cancellationRequested = false
    private var releaseTask: Task<Void, Never>?

    init(_ session: any ExplorerAIExplanationSession) {
        self.session = session
        disclosure = session.disclosure
    }

    func explain() async throws -> ExplorerAIExplanationResult {
        try await session.explain()
    }

    func cancel() {
        let shouldCancel = lock.withLock { () -> Bool in
            guard !cancellationRequested else { return false }
            cancellationRequested = true
            return true
        }
        if shouldCancel { session.cancel() }
    }

    func release() async {
        let task = lock.withLock { () -> Task<Void, Never> in
            if let releaseTask { return releaseTask }
            let task = Task { [session] in await session.release() }
            releaseTask = task
            return task
        }
        await task.value
    }
}

@MainActor
@Observable
public final class ExplorerAIExplanationModel {
    public private(set) var phase = ExplorerAIExplanationPhase.idle

    @ObservationIgnored
    private let explanations: any ExplorerAIExplanationServing
    @ObservationIgnored
    private let contextReader: any ExplorerAIExplanationContextReading
    @ObservationIgnored
    private var generation: UInt64 = 0
    @ObservationIgnored
    private var session: ManagedExplanationSession?
    @ObservationIgnored
    private var sessionAuthority: AuthorityIdentity?
    @ObservationIgnored
    private var terminalFenceStarted = false
    @ObservationIgnored
    private var terminalFenceTask: Task<Void, Never>?
    @ObservationIgnored
    private var acceptedOperationCount = 0
    @ObservationIgnored
    private var acceptedOperationWaiters: [CheckedContinuation<Void, Never>] = []

    public init(
        explanations: any ExplorerAIExplanationServing,
        contextReader: any ExplorerAIExplanationContextReading
    ) {
        self.explanations = explanations
        self.contextReader = contextReader
    }

    public var context: ExplorerAIExplanationContext {
        contextReader.aiExplanationContext
    }

    public var canPreview: Bool {
        let context = contextReader.aiExplanationContext
        return !terminalFenceStarted
            && context.isEligible
            && context.sourceScanID != nil
            && context.selectedDirectoryNodeID != nil
            && !phase.isBusy
            && session == nil
    }

    public var activeDisclosure: ExplorerAIExplanationDisclosure? {
        switch phase {
        case let .awaitingConsent(disclosure), let .explaining(disclosure):
            disclosure
        case .idle, .preparing, .ready, .failed:
            nil
        }
    }

    public var isExplaining: Bool {
        if case .explaining = phase { true } else { false }
    }

    public var resultForCurrentDirectory: ExplorerAIExplanationResult? {
        guard
            case let .ready(result) = phase,
            result.sourceScanID == context.sourceScanID,
            result.selectedRootNodeID == context.currentDirectoryNodeID
        else { return nil }
        return result
    }

    public func decoration(
        forObservedNodeID nodeID: UInt64
    ) -> ExplorerAIExplanationDecoration? {
        resultForCurrentDirectory?.decoration(forObservedNodeID: nodeID)
    }

    public func accessibilitySuffix(forObservedNodeID nodeID: UInt64) -> String {
        guard let decoration = decoration(forObservedNodeID: nodeID) else { return "" }
        return ". \(decoration.accessibilitySummary)"
    }

    /// Creates a local, path-free disclosure. No provider call is possible on
    /// this path; the opaque returned session remains private until consent.
    public func previewSelection() async {
        guard beginAcceptedOperation() else { return }
        defer { finishAcceptedOperation() }

        let initial = contextReader.aiExplanationContext
        guard
            initial.isEligible,
            let scanID = initial.sourceScanID,
            let selectedRootNodeID = initial.selectedDirectoryNodeID
        else {
            phase = .failed(.selectionRequired)
            return
        }

        await clearSessionAndPresentation(cancel: true)
        generation &+= 1
        let operation = generation
        let authority = contextReader.aiExplanationContext.authorityIdentity
        phase = .preparing

        do {
            let prepared = try ManagedExplanationSession(
                await explanations.prepare(
                    scanID: scanID,
                    selectedRootNodeID: selectedRootNodeID
                )
            )
            guard
                operation == generation,
                !terminalFenceStarted,
                !Task.isCancelled,
                authority == contextReader.aiExplanationContext.authorityIdentity,
                prepared.disclosure.sourceScanID == scanID,
                prepared.disclosure.selectedRootNodeID == selectedRootNodeID
            else {
                prepared.cancel()
                await prepared.release()
                if operation == generation, !terminalFenceStarted {
                    phase = .idle
                }
                return
            }
            session = prepared
            sessionAuthority = authority
            phase = .awaitingConsent(prepared.disclosure)
        } catch {
            guard
                operation == generation,
                !terminalFenceStarted,
                !Task.isCancelled,
                authority == contextReader.aiExplanationContext.authorityIdentity
            else {
                if operation == generation, !terminalFenceStarted {
                    phase = .idle
                }
                return
            }
            phase = .failed(Self.presentationFailure(error))
        }
    }

    /// The sole one-shot send edge. It consumes only the exact disclosure that
    /// is already visible and only while its non-pagination authority matches.
    public func explain(disclosureID: UUID) async {
        guard beginAcceptedOperation() else { return }
        defer { finishAcceptedOperation() }
        guard
            case let .awaitingConsent(disclosure) = phase,
            disclosure.id == disclosureID,
            let session,
            session.disclosure == disclosure,
            let authority = sessionAuthority,
            authority == contextReader.aiExplanationContext.authorityIdentity
        else { return }

        generation &+= 1
        let operation = generation
        phase = .explaining(disclosure)
        do {
            let result = try await withTaskCancellationHandler {
                try await session.explain()
            } onCancel: {
                session.cancel()
            }
            await session.release()
            clearSessionIfIdentical(session)
            guard
                operation == generation,
                !terminalFenceStarted,
                !Task.isCancelled,
                authority == contextReader.aiExplanationContext.authorityIdentity,
                result.sourceScanID == disclosure.sourceScanID,
                result.selectedRootNodeID == disclosure.selectedRootNodeID,
                result.inputDigestSHA256 == disclosure.preview.inputDigestSHA256,
                result.providerName == disclosure.providerName,
                result.model == disclosure.model,
                result.adapterRevision == disclosure.adapterRevision
            else {
                if operation == generation, !terminalFenceStarted {
                    phase = .idle
                }
                return
            }
            phase = .ready(result)
        } catch {
            await session.release()
            clearSessionIfIdentical(session)
            guard
                operation == generation,
                !terminalFenceStarted,
                !Task.isCancelled,
                authority == contextReader.aiExplanationContext.authorityIdentity
            else {
                if operation == generation, !terminalFenceStarted {
                    phase = .idle
                }
                return
            }
            phase = .failed(Self.presentationFailure(error))
        }
    }

    public func cancel() async {
        guard beginAcceptedOperation() else { return }
        defer { finishAcceptedOperation() }
        await clearSessionAndPresentation(cancel: true)
    }

    public func dismiss() async {
        guard beginAcceptedOperation() else { return }
        defer { finishAcceptedOperation() }
        await clearSessionAndPresentation(cancel: true)
    }

    /// Must be called before an Explorer authority change. Only an exact,
    /// already validated result can survive, and no provider session survives.
    public func invalidateBeforeContextChange(
        preserving scope: ExplorerAIExplanationPreservationScope?
    ) async {
        guard beginAcceptedOperation() else { return }
        defer { finishAcceptedOperation() }
        generation &+= 1
        let retainedResult: ExplorerAIExplanationResult? = if
            let scope,
            scope.nextContextRevision > contextReader.aiExplanationContext.revision,
            case let .ready(result) = phase,
            result.sourceScanID == scope.sourceScanID,
            result.selectedRootNodeID == scope.rootNodeID
        {
            result
        } else {
            nil
        }
        let heldSession = session
        session = nil
        sessionAuthority = nil
        heldSession?.cancel()
        phase = retainedResult.map(ExplorerAIExplanationPhase.ready) ?? .idle
        await heldSession?.release()
    }

    /// Rejects a retained session or result after unannounced authority drift.
    /// Visibility-only paging does not affect the compared identity.
    public func synchronizeContext() async {
        guard beginAcceptedOperation() else { return }
        defer { finishAcceptedOperation() }
        let authority = contextReader.aiExplanationContext.authorityIdentity
        if let sessionAuthority, sessionAuthority != authority {
            await clearSessionAndPresentation(cancel: true)
            return
        }
        if case let .ready(result) = phase {
            let context = contextReader.aiExplanationContext
            let belongsToSelection = result.sourceScanID == context.sourceScanID
                && result.selectedRootNodeID == context.selectedDirectoryNodeID
            let belongsToCurrentDirectory = result.sourceScanID == context.sourceScanID
                && result.selectedRootNodeID == context.currentDirectoryNodeID
            if !belongsToSelection, !belongsToCurrentDirectory {
                generation &+= 1
                phase = .idle
            }
        }
    }

    /// Installs a synchronous terminal fence, cancels/releases any held session,
    /// then joins every operation accepted before the fence was installed.
    public func beginTerminalFence() -> Task<Void, Never> {
        if let terminalFenceTask { return terminalFenceTask }
        terminalFenceStarted = true
        generation &+= 1
        let heldSession = session
        session = nil
        sessionAuthority = nil
        phase = .idle
        heldSession?.cancel()

        let task = Task { @MainActor [weak self] in
            await heldSession?.release()
            await self?.waitForAcceptedOperations()
        }
        terminalFenceTask = task
        return task
    }

    public func quiesceForTerminalRuntime() async {
        await beginTerminalFence().value
    }

    private func clearSessionAndPresentation(cancel: Bool) async {
        generation &+= 1
        let heldSession = session
        session = nil
        sessionAuthority = nil
        if cancel { heldSession?.cancel() }
        phase = .idle
        await heldSession?.release()
    }

    private func clearSessionIfIdentical(_ candidate: ManagedExplanationSession) {
        guard let session, session === candidate else {
            return
        }
        self.session = nil
        sessionAuthority = nil
    }

    private func beginAcceptedOperation() -> Bool {
        guard !terminalFenceStarted else { return false }
        acceptedOperationCount += 1
        return true
    }

    private func finishAcceptedOperation() {
        precondition(acceptedOperationCount > 0)
        acceptedOperationCount -= 1
        guard acceptedOperationCount == 0 else { return }
        let waiters = acceptedOperationWaiters
        acceptedOperationWaiters.removeAll()
        waiters.forEach { $0.resume() }
    }

    private func waitForAcceptedOperations() async {
        guard acceptedOperationCount > 0 else { return }
        await withCheckedContinuation { continuation in
            acceptedOperationWaiters.append(continuation)
        }
    }

    private static func presentationFailure(_ error: Error) -> ExplorerAIExplanationFailure {
        if error is CancellationError { return .cancelled }
        if let failure = error as? ExplorerAIExplanationFailure { return failure }
        if let failure = error as? ExplorerAIMetadataPreviewError {
            return switch failure {
            case .incompleteCoverage: .incompleteCoverage
            case .selectionNotDirectory: .selectionNotDirectory
            case .sensitiveSelection: .sensitiveSelection
            case .previewUnavailable: .previewExpired
            case .wrongReview, .reviewUnavailable, .selectionUnavailable,
                 .unsupportedObservation, .budgetExceeded, .invalidClock,
                 .unsafeStorage, .corruptData, .busy, .closed, .unavailable,
                 .invalidResponse:
                .unavailable
            }
        }
        return .unavailable
    }
}
