import Foundation

/// The exact local disclosure presented before one remote explanation. These
/// values are inert observations; the separately held opaque session is the
/// only object which can consume the core-owned preview.
public struct ExplorerAIExplanationDisclosure: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let sourceScanID: String
    public let selectedRootNodeID: UInt64
    public let providerName: String
    public let model: String
    public let adapterID: String
    public let adapterRevision: Int
    public let maximumMetadataInputBytes: Int
    public let maximumEncodedRequestBytes: Int
    public let maximumResponseBytes: Int
    public let maximumOutputTokens: Int
    public let retentionReviewedOn: String
    public let providerPolicyURL: URL
    public let standardAPIDeletionWithinDays: Int
    public let flaggedInputOutputRetentionYears: Int
    public let safetyScoreRetentionYears: Int
    public let structuredOutputGrammarMayBeCachedHours: Int
    public let billingMayApply: Bool
    public let hasStatedRetentionExceptions: Bool
    public let mayRetainLongerForSafetyOrLegalReasons: Bool
    public let zeroDataRetentionIsInferred: Bool
    public let preview: ExplorerAIMetadataPreviewInfo

    public init(
        id: UUID,
        sourceScanID: String,
        selectedRootNodeID: UInt64,
        providerName: String,
        model: String,
        adapterID: String,
        adapterRevision: Int,
        maximumMetadataInputBytes: Int,
        maximumEncodedRequestBytes: Int,
        maximumResponseBytes: Int,
        maximumOutputTokens: Int,
        retentionReviewedOn: String,
        providerPolicyURL: URL,
        standardAPIDeletionWithinDays: Int,
        flaggedInputOutputRetentionYears: Int,
        safetyScoreRetentionYears: Int,
        structuredOutputGrammarMayBeCachedHours: Int,
        billingMayApply: Bool,
        hasStatedRetentionExceptions: Bool,
        mayRetainLongerForSafetyOrLegalReasons: Bool,
        zeroDataRetentionIsInferred: Bool,
        preview: ExplorerAIMetadataPreviewInfo
    ) {
        self.id = id
        self.sourceScanID = sourceScanID
        self.selectedRootNodeID = selectedRootNodeID
        self.providerName = providerName
        self.model = model
        self.adapterID = adapterID
        self.adapterRevision = adapterRevision
        self.maximumMetadataInputBytes = maximumMetadataInputBytes
        self.maximumEncodedRequestBytes = maximumEncodedRequestBytes
        self.maximumResponseBytes = maximumResponseBytes
        self.maximumOutputTokens = maximumOutputTokens
        self.retentionReviewedOn = retentionReviewedOn
        self.providerPolicyURL = providerPolicyURL
        self.standardAPIDeletionWithinDays = standardAPIDeletionWithinDays
        self.flaggedInputOutputRetentionYears = flaggedInputOutputRetentionYears
        self.safetyScoreRetentionYears = safetyScoreRetentionYears
        self.structuredOutputGrammarMayBeCachedHours = structuredOutputGrammarMayBeCachedHours
        self.billingMayApply = billingMayApply
        self.hasStatedRetentionExceptions = hasStatedRetentionExceptions
        self.mayRetainLongerForSafetyOrLegalReasons = mayRetainLongerForSafetyOrLegalReasons
        self.zeroDataRetentionIsInferred = zeroDataRetentionIsInferred
        self.preview = preview
    }
}

private struct ExplorerAIExplanationGroup: Equatable, Sendable {
    let ordinal: Int
    let title: String
    let reason: String
    let observedNodeIDs: Set<UInt64>
}

public enum ExplorerAIExplanationSource: Equatable, Sendable {
    case providerResponse
    case localCache(createdAt: Date, expiresAt: Date)

    public var isCached: Bool {
        if case .localCache = self { true } else { false }
    }
}

/// A display-only lookup result. It deliberately contains no item identifier,
/// path, candidate, safety judgment, plan, approval, scheduler, or effect.
public struct ExplorerAIExplanationDecoration: Equatable, Sendable {
    public let ordinal: Int
    public let title: String
    public let reason: String

    fileprivate init(ordinal: Int, title: String, reason: String) {
        self.ordinal = ordinal
        self.title = title
        self.reason = reason
    }

    public var accessibilitySummary: String {
        "AI group \(ordinal), \(title). This is not a safety or cleanup judgment."
    }
}

@_spi(DuxAITransport)
public struct ExplorerAIExplanationTransportGroup: Sendable {
    public let title: String
    public let reason: String
    public let observedNodeIDs: [UInt64]

    public init(title: String, reason: String, observedNodeIDs: [UInt64]) {
        self.title = title
        self.reason = reason
        self.observedNodeIDs = observedNodeIDs
    }
}

/// A Rust-validated, presentation-only result. Its public surface exposes only
/// inert prose and one-observed-item decoration lookups. Raw group membership
/// is private to this compiler module.
public struct ExplorerAIExplanationResult: Equatable, Sendable {
    public let source: ExplorerAIExplanationSource
    public let providerName: String
    public let model: String
    public let adapterRevision: Int
    public let sourceScanID: String
    public let selectedRootNodeID: UInt64
    public let inputDigestSHA256: String
    public let rootLabel: String
    public let summary: String
    public let labels: [String]
    private let groups: [ExplorerAIExplanationGroup]
    public let questions: [String]
    public let uncertainties: [String]
    public let researchSuggestions: [String]

    @_spi(DuxAITransport)
    public init(
        source: ExplorerAIExplanationSource = .providerResponse,
        providerName: String,
        model: String,
        adapterRevision: Int,
        sourceScanID: String,
        selectedRootNodeID: UInt64,
        inputDigestSHA256: String,
        rootLabel: String,
        summary: String,
        labels: [String],
        transportGroups: [ExplorerAIExplanationTransportGroup],
        questions: [String],
        uncertainties: [String],
        researchSuggestions: [String]
    ) {
        self.source = source
        self.providerName = providerName
        self.model = model
        self.adapterRevision = adapterRevision
        self.sourceScanID = sourceScanID
        self.selectedRootNodeID = selectedRootNodeID
        self.inputDigestSHA256 = inputDigestSHA256
        self.rootLabel = rootLabel
        self.summary = summary
        self.labels = labels
        groups = transportGroups.enumerated().map { index, group in
            ExplorerAIExplanationGroup(
                ordinal: index + 1,
                title: group.title,
                reason: group.reason,
                observedNodeIDs: Set(group.observedNodeIDs)
            )
        }
        self.questions = questions
        self.uncertainties = uncertainties
        self.researchSuggestions = researchSuggestions
    }

    public var groupCount: Int { groups.count }

    public func decoration(
        forObservedNodeID nodeID: UInt64
    ) -> ExplorerAIExplanationDecoration? {
        guard let group = groups.first(where: { $0.observedNodeIDs.contains(nodeID) }) else {
            return nil
        }
        return ExplorerAIExplanationDecoration(
            ordinal: group.ordinal,
            title: group.title,
            reason: group.reason
        )
    }

    func visibleCount(
        groupOrdinal: Int,
        observedNodeIDs: Set<UInt64>
    ) -> Int {
        guard let group = groups.first(where: { $0.ordinal == groupOrdinal }) else {
            return 0
        }
        return group.observedNodeIDs.intersection(observedNodeIDs).count
    }

    func groupPresentations() -> [ExplorerAIExplanationDecoration] {
        groups.map {
            ExplorerAIExplanationDecoration(
                ordinal: $0.ordinal,
                title: $0.title,
                reason: $0.reason
            )
        }
    }

    func itemCount(groupOrdinal: Int) -> Int {
        groups.first(where: { $0.ordinal == groupOrdinal })?.observedNodeIDs.count ?? 0
    }
}

public enum ExplorerAIExplanationFailure: Error, Equatable, Sendable {
    case selectionRequired
    case selectionNotDirectory
    case incompleteCoverage
    case sensitiveSelection
    case previewExpired
    case missingCredential
    case credentialUnavailable
    case timedOut
    case cancelled
    case providerRejected
    case networkUnavailable
    case invalidResponse
    case unavailable

    public var title: String {
        switch self {
        case .selectionRequired: "Select a folder first"
        case .selectionNotDirectory: "Only folders can be explained"
        case .incompleteCoverage: "Complete scan coverage is required"
        case .sensitiveSelection: "This folder is too sensitive to send"
        case .previewExpired: "The metadata preview expired"
        case .missingCredential: "Add an Anthropic API key"
        case .credentialUnavailable: "The API key is unavailable"
        case .timedOut: "Anthropic did not finish in time"
        case .cancelled: "Explanation cancelled"
        case .providerRejected: "Anthropic rejected the request"
        case .networkUnavailable: "Anthropic could not be reached"
        case .invalidResponse: "The AI response was rejected"
        case .unavailable: "AI explanation is unavailable"
        }
    }

    public var detail: String {
        switch self {
        case .selectionRequired:
            "Choose one historical folder in the table, then preview its path-free metadata."
        case .selectionNotDirectory:
            "DUX explains one selected directory and its direct storage groups. Files are not sent individually."
        case .incompleteCoverage:
            "Run a complete scan without recorded coverage issues before asking AI to explain this selection."
        case .sensitiveSelection:
            "DUX's privacy policy rejected the selected folder before any provider request was created."
        case .previewExpired:
            "Nothing was sent. Preview the selection again to create a fresh, single-use disclosure."
        case .missingCredential:
            "Save a DUX-specific Anthropic API key in Settings, then preview the selection again."
        case .credentialUnavailable:
            "DUX could not read its Keychain item. Unlock the Mac or review the provider setting."
        case .timedOut:
            "The one-shot request was stopped. The snapshot and deterministic findings were not changed."
        case .cancelled:
            "No AI result was applied. The snapshot and deterministic findings were not changed."
        case .providerRejected:
            "Check the API key and Anthropic account. DUX did not retry or change deterministic findings."
        case .networkUnavailable:
            "DUX made no retry. The snapshot and deterministic findings were not changed."
        case .invalidResponse:
            "DUX discarded the complete answer because it failed the fixed adapter or Rust validation contract."
        case .unavailable:
            "The retained review or explanation service is unavailable. Nothing was sent or changed."
        }
    }
}

public enum ExplorerAIExplanationPhase: Equatable, Sendable {
    case idle
    case preparing
    case awaitingConsent(ExplorerAIExplanationDisclosure)
    case explaining(ExplorerAIExplanationDisclosure)
    case ready(ExplorerAIExplanationResult)
    case failed(ExplorerAIExplanationFailure)

    public var isBusy: Bool {
        switch self {
        case .preparing, .explaining: true
        case .idle, .awaitingConsent, .ready, .failed: false
        }
    }
}
