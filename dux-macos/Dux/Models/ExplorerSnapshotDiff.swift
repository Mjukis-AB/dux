import Foundation

enum ExplorerSnapshotDiffDirection: Equatable, Sendable {
    case growth
    case shrinkage
    case unchanged
}

struct ExplorerSnapshotDiffValue: Equatable, Sendable {
    let direction: ExplorerSnapshotDiffDirection
    let magnitudeBytes: UInt64

    var display: String {
        switch direction {
        case .growth:
            "+\(StorageByteFormatter.string(from: magnitudeBytes))"
        case .shrinkage:
            "−\(StorageByteFormatter.string(from: magnitudeBytes))"
        case .unchanged:
            "0 B"
        }
    }
}

enum ExplorerSnapshotDiffChange: Equatable, Sendable {
    case added
    case removed
    case grew
    case shrank
    case unchanged
    case replaced

    var title: String {
        switch self {
        case .added: "First observed"
        case .removed: "No longer observed"
        case .grew: "Grew"
        case .shrank: "Shrunk"
        case .unchanged: "Unchanged"
        case .replaced: "Replaced"
        }
    }

    var symbol: String {
        switch self {
        case .added: "+"
        case .removed: "−"
        case .grew: "↑"
        case .shrank: "↓"
        case .unchanged: "="
        case .replaced: "⇄"
        }
    }

    var systemImage: String {
        switch self {
        case .added: "plus.circle.fill"
        case .removed: "minus.circle.fill"
        case .grew: "arrow.up.circle.fill"
        case .shrank: "arrow.down.circle.fill"
        case .unchanged: "equal.circle"
        case .replaced: "arrow.trianglehead.2.clockwise.rotate.90.circle.fill"
        }
    }
}

enum ExplorerSnapshotDiffSort: String, CaseIterable, Equatable, Sendable {
    case magnitudeDescending
    case nameAscending
    case currentBytesDescending

    var title: String {
        switch self {
        case .magnitudeDescending: "Largest change"
        case .nameAscending: "Name"
        case .currentBytesDescending: "Current size"
        }
    }
}

struct ExplorerSnapshotDiffCoverage: Equatable, Sendable {
    let status: AppScanCoverage
    let measuredPermille: UInt16?
    let issueRecordCount: UInt64
    let issueOccurrenceCount: UInt64

    var title: String {
        switch status {
        case .complete: "Complete coverage"
        case .limitedAccess: "Limited access"
        case .partial: "Partial coverage"
        case .unknown: "Unknown coverage"
        }
    }

    var detail: String {
        var parts: [String] = []
        if let measuredPermille {
            parts.append(
                (Double(measuredPermille) / 10)
                    .formatted(.number.precision(.fractionLength(measuredPermille % 10 == 0 ? 0 : 1)))
                    + "% measured"
            )
        } else {
            parts.append("measurement unavailable")
        }
        if issueOccurrenceCount > 0 {
            parts.append(
                "\(issueOccurrenceCount) \(issueOccurrenceCount == 1 ? "issue" : "issues")"
            )
        }
        return parts.joined(separator: " · ")
    }
}

struct ExplorerSnapshotDiffInfo: Equatable, Sendable {
    let currentScanID: String
    let baselineScanID: String
    let currentStartedAt: Date
    let currentCompletedAt: Date
    let baselineStartedAt: Date
    let baselineCompletedAt: Date
    let currentCoverage: ExplorerSnapshotDiffCoverage
    let baselineCoverage: ExplorerSnapshotDiffCoverage
}

struct ExplorerSnapshotDiffNode: Equatable, Identifiable, Sendable {
    let id: UInt64
    let parentID: UInt64?
    let depth: UInt32
    let name: ExplorerSnapshotNodeName
    let kind: ExplorerSnapshotNodeKind
    let currentKind: ExplorerSnapshotNodeKind?
    let baselineKind: ExplorerSnapshotNodeKind?
    let category: ExplorerStorageCategory
    let change: ExplorerSnapshotDiffChange
    let logicalChange: ExplorerSnapshotDiffValue
    let currentLogicalBytes: UInt64?
    let baselineLogicalBytes: UInt64?
    let currentAllocatedBytes: UInt64?
    let baselineAllocatedBytes: UInt64?
    let allocatedChange: ExplorerSnapshotDiffValue?
    let currentFileCount: UInt64?
    let baselineFileCount: UInt64?
    let currentChildCount: UInt64?
    let baselineChildCount: UInt64?
    let currentScanFlags: ExplorerSnapshotScanFlags?
    let baselineScanFlags: ExplorerSnapshotScanFlags?
    let canDescend: Bool

    var currentSizeText: String {
        currentLogicalBytes.map(StorageByteFormatter.string(from:)) ?? "Not observed"
    }

    var baselineSizeText: String {
        baselineLogicalBytes.map(StorageByteFormatter.string(from:)) ?? "Not observed"
    }

    var accessibilitySummary: String {
        [
            name.display,
            "\(change.symbol) \(change.title)",
            "change \(logicalChange.display)",
            "current \(currentSizeText)",
            "previous \(baselineSizeText)",
            "category \(category.presentation.accessibilityPhrase)",
        ].joined(separator: ", ")
    }

    var hasObservationWarning: Bool {
        currentScanFlags?.hasAny == true || baselineScanFlags?.hasAny == true
    }
}

struct ExplorerSnapshotDiffNodePage: Equatable, Sendable {
    let parentID: UInt64
    let offset: UInt64
    let totalChildren: UInt64
    let hasMore: Bool
    let totalGrowthBytes: UInt64
    let totalShrinkageBytes: UInt64
    let unchangedChildCount: UInt64
    let replacedChildCount: UInt64
    let nodes: [ExplorerSnapshotDiffNode]
}

struct ExplorerSnapshotDiffTreemapCell: Equatable, Identifiable, Sendable {
    var id: UInt64 { node.id }

    let node: ExplorerSnapshotDiffNode
    let magnitudeRank: UInt64
}

struct ExplorerSnapshotDiffTreemap: Equatable, Sendable {
    let parentID: UInt64
    let totalChildren: UInt64
    let changedChildCount: UInt64
    let totalGrowthBytes: UInt64
    let totalShrinkageBytes: UInt64
    let otherGrowthChildCount: UInt64
    let otherGrowthBytes: UInt64
    let otherShrinkageChildCount: UInt64
    let otherShrinkageBytes: UInt64
    let unchangedChildCount: UInt64
    let replacedChildCount: UInt64
    let cells: [ExplorerSnapshotDiffTreemapCell]

    func cell(nodeID: UInt64) -> ExplorerSnapshotDiffTreemapCell? {
        cells.first { $0.id == nodeID }
    }
}

struct ExplorerSnapshotDiffReviewHandle: Equatable, Sendable {
    let id: UUID
    let info: ExplorerSnapshotDiffInfo
}

enum ExplorerSnapshotDiffSelection: Equatable, Sendable {
    case node(UInt64)
    case otherGrowth
    case otherShrinkage
}

enum ExplorerSnapshotDiffFailure: Error, Equatable, Sendable {
    case notAvailable
    case expired
    case invalidRequest
    case budgetExceeded
    case invalidResponse
    case closed
    case unavailable

    var title: String {
        switch self {
        case .notAvailable: "No comparable snapshot"
        case .expired: "Comparison expired"
        case .invalidRequest: "Folder comparison unavailable"
        case .budgetExceeded: "Comparison is too large"
        case .invalidResponse: "Comparison could not be validated"
        case .closed: "DUX is closing"
        case .unavailable: "Changes unavailable"
        }
    }

    var detail: String {
        switch self {
        case .notAvailable:
            "DUX needs an older retained scan of this exact storage root before it can show changes."
        case .expired:
            "The baseline lease expired. Browse remains available; prepare the comparison again."
        case .invalidRequest:
            "This historical folder is no longer available in both snapshots."
        case .budgetExceeded:
            "The bounded comparison could not fit within DUX’s review budget."
        case .invalidResponse:
            "DUX rejected an inconsistent comparison response. Browse was not changed."
        case .closed:
            "The comparison was released without changing the ordinary snapshot."
        case .unavailable:
            "DUX could not prepare this read-only comparison. Browse remains available."
        }
    }
}

enum ExplorerSnapshotDiffPhase: Equatable, Sendable {
    case idle
    case loading
    case ready
    case failed(ExplorerSnapshotDiffFailure)
}

private extension ExplorerSnapshotScanFlags {
    var hasAny: Bool {
        inaccessible || timedOut || hardLinkDuplicate || mountBoundary
    }
}
