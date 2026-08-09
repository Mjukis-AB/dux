import Foundation

enum ExplorerAIMetadataPreviewNodeKind: String, Equatable, Sendable {
    case directory
    case file
    case symlink
    case other
    case unavailable

    var genericLabelPrefix: String {
        switch self {
        case .directory: "Directory"
        case .file: "File"
        case .symlink: "Symbolic link"
        case .other: "Other item"
        case .unavailable: "Unavailable item"
        }
    }
}

struct ExplorerAIMetadataPreviewAgeSummary: Equatable, Sendable {
    let within7DaysLogicalBytes: UInt64
    let days8To30LogicalBytes: UInt64
    let days31To90LogicalBytes: UInt64
    let olderThan90DaysLogicalBytes: UInt64
    let unknownAgeLogicalBytes: UInt64

    var values: [UInt64] {
        [
            within7DaysLogicalBytes,
            days8To30LogicalBytes,
            days31To90LogicalBytes,
            olderThan90DaysLogicalBytes,
            unknownAgeLogicalBytes,
        ]
    }

    func checkedTotal() -> UInt64? {
        var total = UInt64.zero
        for value in values {
            let addition = total.addingReportingOverflow(value)
            guard !addition.overflow else { return nil }
            total = addition.partialValue
        }
        return total
    }
}

struct ExplorerAIMetadataPreviewChild: Equatable, Identifiable, Sendable {
    var id: String { inputNodeID }

    let inputNodeID: String
    let label: String
    let kind: ExplorerAIMetadataPreviewNodeKind
    let logicalBytes: UInt64
    let ageSummary: ExplorerAIMetadataPreviewAgeSummary
}

/// One short-lived, read-only disclosure of the exact path-free metadata that
/// Rust shaped. It grants no provider, candidate, approval, or cleanup authority.
struct ExplorerAIMetadataPreviewInfo: Equatable, Sendable {
    let inputSchemaVersion: UInt64
    let privacyPolicyRevision: UInt64
    let preparedAtUnixMilliseconds: Int64
    let expiresAtUnixMilliseconds: Int64
    let inputDigestSHA256: String
    let encodedInputJSONUTF8: Data
    let inspectedNodeCount: UInt64
    let includedDirectChildCount: UInt64
    let excludedSensitiveDirectChildCount: UInt64
    let omittedEligibleDirectChildCount: UInt64
    let rootLabel: String
    let totalLogicalBytes: UInt64
    let ageSummary: ExplorerAIMetadataPreviewAgeSummary
    let childrenComplete: Bool
    let omittedChildCount: UInt64
    let omittedLogicalBytes: UInt64
    let omittedAgeSummary: ExplorerAIMetadataPreviewAgeSummary
    let children: [ExplorerAIMetadataPreviewChild]

    var encodedInputJSON: String {
        String(decoding: encodedInputJSONUTF8, as: UTF8.self)
    }
}

enum ExplorerAIMetadataPreviewError: Error, Equatable, Sendable {
    case wrongReview
    case reviewUnavailable
    case incompleteCoverage
    case selectionUnavailable
    case selectionNotDirectory
    case sensitiveSelection
    case unsupportedObservation
    case budgetExceeded
    case invalidClock
    case unsafeStorage
    case corruptData
    case busy
    case previewUnavailable
    case closed
    case unavailable
    case invalidResponse
}
