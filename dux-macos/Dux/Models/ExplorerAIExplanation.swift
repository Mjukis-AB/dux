import Foundation

/// The exact local disclosure presented before one remote explanation. These
/// values are inert observations; the separately held opaque session is the
/// only object which can consume the core-owned preview.
struct ExplorerAIExplanationDisclosure: Identifiable, Equatable, Sendable {
    let id: UUID
    let sourceScanID: String
    let selectedRootNodeID: UInt64
    let providerName: String
    let model: String
    let adapterID: String
    let adapterRevision: Int
    let maximumMetadataInputBytes: Int
    let maximumEncodedRequestBytes: Int
    let maximumResponseBytes: Int
    let maximumOutputTokens: Int
    let retentionReviewedOn: String
    let providerPolicyURL: URL
    let standardAPIDeletionWithinDays: Int
    let flaggedInputOutputRetentionYears: Int
    let safetyScoreRetentionYears: Int
    let structuredOutputGrammarMayBeCachedHours: Int
    let billingMayApply: Bool
    let hasStatedRetentionExceptions: Bool
    let mayRetainLongerForSafetyOrLegalReasons: Bool
    let zeroDataRetentionIsInferred: Bool
    let preview: ExplorerAIMetadataPreviewInfo
}

struct ExplorerAIExplanationGroup: Identifiable, Equatable, Sendable {
    let id: Int
    let title: String
    let reason: String
    let snapshotNodeIDs: [UInt64]

    func contains(nodeID: UInt64) -> Bool {
        snapshotNodeIDs.contains(nodeID)
    }
}

/// A Rust-validated, presentation-only result. It deliberately has no path,
/// candidate, rule, action, approval, scheduling, planning, or effect type.
struct ExplorerAIExplanationResult: Equatable, Sendable {
    let providerName: String
    let model: String
    let adapterRevision: Int
    let sourceScanID: String
    let selectedRootNodeID: UInt64
    let inputDigestSHA256: String
    let rootLabel: String
    let summary: String
    let labels: [String]
    let groups: [ExplorerAIExplanationGroup]
    let questions: [String]
    let uncertainties: [String]
    let researchSuggestions: [String]

    func group(containing nodeID: UInt64) -> ExplorerAIExplanationGroup? {
        groups.first { $0.contains(nodeID: nodeID) }
    }
}

enum ExplorerAIExplanationFailure: Error, Equatable, Sendable {
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

    var title: String {
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

    var detail: String {
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

enum ExplorerAIExplanationPhase: Equatable, Sendable {
    case idle
    case preparing
    case awaitingConsent(ExplorerAIExplanationDisclosure)
    case explaining(ExplorerAIExplanationDisclosure)
    case ready(ExplorerAIExplanationResult)
    case failed(ExplorerAIExplanationFailure)

    var isBusy: Bool {
        switch self {
        case .preparing, .explaining: true
        case .idle, .awaitingConsent, .ready, .failed: false
        }
    }
}
