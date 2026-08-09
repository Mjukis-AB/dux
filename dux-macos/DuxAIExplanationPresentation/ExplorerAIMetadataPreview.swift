import Foundation

public enum ExplorerAIMetadataPreviewNodeKind: String, Equatable, Sendable {
    case directory
    case file
    case symlink
    case other
    case unavailable

    public var genericLabelPrefix: String {
        switch self {
        case .directory: "Directory"
        case .file: "File"
        case .symlink: "Symbolic link"
        case .other: "Other item"
        case .unavailable: "Unavailable item"
        }
    }
}

public struct ExplorerAIMetadataPreviewAgeSummary: Equatable, Sendable {
    public let within7DaysLogicalBytes: UInt64
    public let days8To30LogicalBytes: UInt64
    public let days31To90LogicalBytes: UInt64
    public let olderThan90DaysLogicalBytes: UInt64
    public let unknownAgeLogicalBytes: UInt64

    public init(
        within7DaysLogicalBytes: UInt64,
        days8To30LogicalBytes: UInt64,
        days31To90LogicalBytes: UInt64,
        olderThan90DaysLogicalBytes: UInt64,
        unknownAgeLogicalBytes: UInt64
    ) {
        self.within7DaysLogicalBytes = within7DaysLogicalBytes
        self.days8To30LogicalBytes = days8To30LogicalBytes
        self.days31To90LogicalBytes = days31To90LogicalBytes
        self.olderThan90DaysLogicalBytes = olderThan90DaysLogicalBytes
        self.unknownAgeLogicalBytes = unknownAgeLogicalBytes
    }

    public var values: [UInt64] {
        [
            within7DaysLogicalBytes,
            days8To30LogicalBytes,
            days31To90LogicalBytes,
            olderThan90DaysLogicalBytes,
            unknownAgeLogicalBytes,
        ]
    }

    public func checkedTotal() -> UInt64? {
        var total = UInt64.zero
        for value in values {
            let addition = total.addingReportingOverflow(value)
            guard !addition.overflow else { return nil }
            total = addition.partialValue
        }
        return total
    }
}

public struct ExplorerAIMetadataPreviewChild: Equatable, Identifiable, Sendable {
    public var id: String { inputNodeID }

    public let inputNodeID: String
    public let label: String
    public let kind: ExplorerAIMetadataPreviewNodeKind
    public let logicalBytes: UInt64
    public let ageSummary: ExplorerAIMetadataPreviewAgeSummary

    public init(
        inputNodeID: String,
        label: String,
        kind: ExplorerAIMetadataPreviewNodeKind,
        logicalBytes: UInt64,
        ageSummary: ExplorerAIMetadataPreviewAgeSummary
    ) {
        self.inputNodeID = inputNodeID
        self.label = label
        self.kind = kind
        self.logicalBytes = logicalBytes
        self.ageSummary = ageSummary
    }
}

/// One short-lived, read-only disclosure of the exact path-free metadata that
/// Rust shaped. It grants no provider, candidate, approval, or cleanup authority.
public struct ExplorerAIMetadataPreviewInfo: Equatable, Sendable {
    public let inputSchemaVersion: UInt64
    public let privacyPolicyRevision: UInt64
    public let preparedAtUnixMilliseconds: Int64
    public let expiresAtUnixMilliseconds: Int64
    public let inputDigestSHA256: String
    public let encodedInputJSONUTF8: Data
    public let inspectedNodeCount: UInt64
    public let includedDirectChildCount: UInt64
    public let excludedSensitiveDirectChildCount: UInt64
    public let omittedEligibleDirectChildCount: UInt64
    public let rootLabel: String
    public let totalLogicalBytes: UInt64
    public let ageSummary: ExplorerAIMetadataPreviewAgeSummary
    public let childrenComplete: Bool
    public let omittedChildCount: UInt64
    public let omittedLogicalBytes: UInt64
    public let omittedAgeSummary: ExplorerAIMetadataPreviewAgeSummary
    public let children: [ExplorerAIMetadataPreviewChild]
    public let contentIncluded: Bool
    public let sourceNamesIncluded: Bool
    public let sourcePathsIncluded: Bool

    public init(
        inputSchemaVersion: UInt64,
        privacyPolicyRevision: UInt64,
        preparedAtUnixMilliseconds: Int64,
        expiresAtUnixMilliseconds: Int64,
        inputDigestSHA256: String,
        encodedInputJSONUTF8: Data,
        inspectedNodeCount: UInt64,
        includedDirectChildCount: UInt64,
        excludedSensitiveDirectChildCount: UInt64,
        omittedEligibleDirectChildCount: UInt64,
        rootLabel: String,
        totalLogicalBytes: UInt64,
        ageSummary: ExplorerAIMetadataPreviewAgeSummary,
        childrenComplete: Bool,
        omittedChildCount: UInt64,
        omittedLogicalBytes: UInt64,
        omittedAgeSummary: ExplorerAIMetadataPreviewAgeSummary,
        children: [ExplorerAIMetadataPreviewChild],
        contentIncluded: Bool,
        sourceNamesIncluded: Bool,
        sourcePathsIncluded: Bool
    ) {
        self.inputSchemaVersion = inputSchemaVersion
        self.privacyPolicyRevision = privacyPolicyRevision
        self.preparedAtUnixMilliseconds = preparedAtUnixMilliseconds
        self.expiresAtUnixMilliseconds = expiresAtUnixMilliseconds
        self.inputDigestSHA256 = inputDigestSHA256
        self.encodedInputJSONUTF8 = encodedInputJSONUTF8
        self.inspectedNodeCount = inspectedNodeCount
        self.includedDirectChildCount = includedDirectChildCount
        self.excludedSensitiveDirectChildCount = excludedSensitiveDirectChildCount
        self.omittedEligibleDirectChildCount = omittedEligibleDirectChildCount
        self.rootLabel = rootLabel
        self.totalLogicalBytes = totalLogicalBytes
        self.ageSummary = ageSummary
        self.childrenComplete = childrenComplete
        self.omittedChildCount = omittedChildCount
        self.omittedLogicalBytes = omittedLogicalBytes
        self.omittedAgeSummary = omittedAgeSummary
        self.children = children
        self.contentIncluded = contentIncluded
        self.sourceNamesIncluded = sourceNamesIncluded
        self.sourcePathsIncluded = sourcePathsIncluded
    }

    public var encodedInputJSON: String {
        String(decoding: encodedInputJSONUTF8, as: UTF8.self)
    }
}

public enum ExplorerAIMetadataPreviewError: Error, Equatable, Sendable {
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
