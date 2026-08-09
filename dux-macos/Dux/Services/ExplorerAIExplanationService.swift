import Foundation
@_spi(DuxAITransport) import DuxAIExplanationPresentation

protocol DuxAIMetadataPreviewServing: Sendable {
    func prepareAIMetadataPreview(
        scanID: String,
        nodeID: UInt64
    ) async throws -> any DuxAIMetadataPreviewLease
}

struct DuxAICachedExplanation: Sendable {
    let validated: NativeAIAnthropicMessagesV1CoreValidatedResult
    let createdAt: Date
    let expiresAt: Date
}

protocol ExplorerAIExplanationCancellableRun: AnyObject, Sendable {
    func cancel()
}

extension NativeAIAnthropicMessagesV1Run: ExplorerAIExplanationCancellableRun {}

struct NativeExplorerAIExplanationService: ExplorerAIExplanationServing {
    private let previews: any DuxAIMetadataPreviewServing

    init(previews: any DuxAIMetadataPreviewServing) {
        self.previews = previews
    }

    func prepare(
        scanID: String,
        selectedRootNodeID: UInt64
    ) async throws -> any ExplorerAIExplanationSession {
        let lease = try await previews.prepareAIMetadataPreview(
            scanID: scanID,
            nodeID: selectedRootNodeID
        )
        let provider = reviewedAnthropicMessagesV1Disclosure
        let disclosure = ExplorerAIExplanationDisclosure(
            id: UUID(),
            sourceScanID: scanID,
            selectedRootNodeID: selectedRootNodeID,
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
            preview: lease.preview
        )
        return NativeExplorerAIExplanationSession(
            lease: lease,
            disclosure: disclosure
        )
    }
}

final class NativeExplorerAIExplanationSession:
    ExplorerAIExplanationSession, @unchecked Sendable
{
    private enum State {
        case available
        case explaining
        case released
    }

    let disclosure: ExplorerAIExplanationDisclosure

    private let lock = NSLock()
    private let lease: any DuxAIMetadataPreviewLease
    private var state = State.available
    private var run: (any ExplorerAIExplanationCancellableRun)?
    private var cancellationRequested = false

    init(
        lease: any DuxAIMetadataPreviewLease,
        disclosure: ExplorerAIExplanationDisclosure
    ) {
        self.lease = lease
        self.disclosure = disclosure
    }

    func explain() async throws -> ExplorerAIExplanationResult {
        do {
            try beginExplanation()
            if let cached = try await lease.loadCachedAnthropicMessagesV1Explanation() {
                try Task.checkCancellation()
                return try Self.map(
                    cached.validated,
                    disclosure: disclosure,
                    source: .localCache(
                        createdAt: cached.createdAt,
                        expiresAt: cached.expiresAt
                    )
                )
            }
            try Task.checkCancellation()
            let orchestrator = NativeAIAnthropicMessagesV1Orchestrator(
                previewConsumer: lease
            )
            let result = try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { continuation in
                    let run = orchestrator.start { result in
                        continuation.resume(with: result)
                    }
                    install(run: run)
                }
            } onCancel: {
                cancelRun()
            }
            return try Self.map(
                result.validated,
                disclosure: disclosure,
                source: .providerResponse
            )
        } catch let failure as ExplorerAIExplanationFailure {
            throw failure
        } catch let failure as NativeAIAnthropicMessagesV1OrchestratorFailure {
            throw Self.presentationFailure(failure)
        } catch is CancellationError {
            throw ExplorerAIExplanationFailure.cancelled
        } catch {
            throw ExplorerAIExplanationFailure.unavailable
        }
    }

    func release() async {
        let run = lock.withLock { () -> (any ExplorerAIExplanationCancellableRun)? in
            guard state != .released else { return nil }
            state = .released
            cancellationRequested = true
            let run = self.run
            self.run = nil
            return run
        }
        run?.cancel()
        await lease.release()
    }

    func cancel() {
        cancelRun()
    }

    private func beginExplanation() throws {
        try lock.withLock {
            guard state == .available else {
                throw NativeAIAnthropicMessagesV1OrchestratorFailure.cancelled
            }
            state = .explaining
        }
    }

    func install(run: (any ExplorerAIExplanationCancellableRun)?) {
        let cancelImmediately = lock.withLock { () -> Bool in
            guard state != .released, !cancellationRequested else { return true }
            self.run = run
            return false
        }
        if cancelImmediately {
            run?.cancel()
        }
    }

    private func cancelRun() {
        let run = lock.withLock { () -> (any ExplorerAIExplanationCancellableRun)? in
            cancellationRequested = true
            return self.run
        }
        run?.cancel()
    }

    static func map(
        _ validated: NativeAIAnthropicMessagesV1CoreValidatedResult,
        disclosure: ExplorerAIExplanationDisclosure,
        source: ExplorerAIExplanationSource = .providerResponse
    ) throws -> ExplorerAIExplanationResult {
        guard
            validated.binding == .trusted,
            validated.sourceScanID == disclosure.sourceScanID,
            validated.selectedRootNodeID == disclosure.selectedRootNodeID,
            validated.inputDigestSHA256 == disclosure.preview.inputDigestSHA256,
            validated.inputSchemaVersion == disclosure.preview.inputSchemaVersion,
            validated.privacyPolicyRevision == disclosure.preview.privacyPolicyRevision,
            validated.binding.adapterID == disclosure.adapterID,
            validated.binding.adapterRevision == disclosure.adapterRevision,
            validated.binding.model == disclosure.model
        else {
            throw NativeAIAnthropicMessagesV1OrchestratorFailure.coreRejected
        }
        return ExplorerAIExplanationResult(
            source: source,
            providerName: disclosure.providerName,
            model: disclosure.model,
            adapterRevision: disclosure.adapterRevision,
            sourceScanID: validated.sourceScanID,
            selectedRootNodeID: validated.selectedRootNodeID,
            inputDigestSHA256: validated.inputDigestSHA256,
            rootLabel: disclosure.preview.rootLabel,
            summary: validated.summary,
            labels: validated.labels,
            transportGroups: validated.groups.map { group in
                ExplorerAIExplanationTransportGroup(
                    title: group.title,
                    reason: group.reason,
                    observedNodeIDs: group.snapshotNodeIDs
                )
            },
            questions: validated.questions,
            uncertainties: validated.uncertainties,
            researchSuggestions: validated.researchSuggestions
        )
    }

    private static func presentationFailure(
        _ failure: NativeAIAnthropicMessagesV1OrchestratorFailure
    ) -> ExplorerAIExplanationFailure {
        switch failure {
        case .missingCredential: .missingCredential
        case .credentialUnavailable: .credentialUnavailable
        case .deadlineExceeded: .timedOut
        case .cancelled: .cancelled
        case .authentication, .status: .providerRejected
        case .network: .networkUnavailable
        case .previewUnavailable: .previewExpired
        case .invalidCoreBinding, .requestRejected, .redirect,
             .invalidResponse, .responseTooLarge, .extractionRejected,
             .coreRejected:
            .invalidResponse
        }
    }
}
