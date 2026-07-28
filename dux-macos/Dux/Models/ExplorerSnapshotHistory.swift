import Foundation

enum ExplorerHistoricalScanStatus: Equatable, Sendable {
    case queued
    case running
    case succeeded
    case failed
    case cancelled
    case interrupted
}

struct ExplorerHistoricalScanCounts: Equatable, Sendable {
    let directoryCount: UInt64
    let fileCount: UInt64
    let logicalBytes: UInt64
    let allocatedBytes: UInt64?
}

/// Read-only durable scan metadata. `snapshotRecorded` is a selection hint,
/// never proof that the snapshot is still available; review lease acquisition
/// performs that validation.
struct ExplorerHistoricalScan: Equatable, Identifiable, Sendable {
    var id: String { scanID }

    let scanID: String
    let startedAt: Date
    let completedAt: Date?
    let status: ExplorerHistoricalScanStatus
    let counts: ExplorerHistoricalScanCounts?
    let coverage: AppScanCoverage
    let coveragePermille: UInt16?
    let issueCount: UInt64
    let snapshotRecorded: Bool

    var canRequestReview: Bool {
        status == .succeeded && snapshotRecorded
    }
}

struct ExplorerSnapshotHistoryPage: Equatable, Sendable {
    let scans: [ExplorerHistoricalScan]
    let hasMore: Bool

    var newestReviewCandidate: ExplorerHistoricalScan? {
        scans.first(where: \.canRequestReview)
    }
}

enum ExplorerSnapshotHistoryError: Error, Equatable, Sendable {
    case invalidLimit
    case invalidResponse
}

/// Path-free, read-only cleanup outcome metadata. These values are display
/// observations only; they do not contain a plan, approval, path, or effect
/// authority.
enum CleanupHistoryRecordFormat: Equatable, Sendable {
    case legacyIncomplete
    case complete
}

enum CleanupHistoryMode: Equatable, Sendable {
    case dryRun
    case trash
    case permanentSafe
    case evictLocalCopy
}

enum CleanupHistoryTrigger: Equatable, Sendable {
    case manual
    case lowDisk
    case scheduled
    case cli
}

enum CleanupHistorySessionStatus: Equatable, Sendable {
    case planned
    case running
    case recovering
    case completed
    case partiallyCompleted
    case failed
    case cancelled
    case interrupted
    case rejected
    case dryRun
}

struct CleanupHistoryStatusCounts: Equatable, Sendable {
    let planned: UInt16
    let validating: UInt16
    let dryRun: UInt16
    let effectStarted: UInt16
    let trashed: UInt16
    let removed: UInt16
    let evicted: UInt16
    let skipped: UInt16
    let rejected: UInt16
    let failed: UInt16
    let changedSincePlan: UInt16
    let interrupted: UInt16
    let unavailable: UInt16
    let outcomeUnknown: UInt16
    let total: UInt16
}

struct CleanupHistoryCursorModel: Equatable, Sendable {
    let startedAt: Date
    let sessionID: String
}

struct CleanupHistorySessionSummaryModel: Equatable, Identifiable, Sendable {
    var id: String { sessionID }

    let sessionID: String
    let planID: String
    let format: CleanupHistoryRecordFormat
    let sourceScanID: String?
    let startedAt: Date
    let completedAt: Date?
    let planCreatedAt: Date?
    let planExpiresAt: Date?
    let mode: CleanupHistoryMode
    let trigger: CleanupHistoryTrigger
    let status: CleanupHistorySessionStatus
    let estimatedBytes: UInt64
    let verifiedCapacityDeltaBytes: Int64?
    let cancellationRequested: Bool?
    let itemTotal: UInt16
    let pathTotal: UInt16
    let evidenceTotal: UInt16
    let itemStatusCounts: CleanupHistoryStatusCounts
    let pathStatusCounts: CleanupHistoryStatusCounts
}

struct CleanupHistoryPageModel: Equatable, Sendable {
    let records: [CleanupHistorySessionSummaryModel]
    let nextCursor: CleanupHistoryCursorModel?
}

enum CleanupHistoryItemStatus: Equatable, Sendable {
    case planned
    case validating
    case dryRun
    case effectStarted
    case trashed
    case removed
    case evicted
    case skipped
    case rejected
    case failed
    case changedSincePlan
    case interrupted
    case unavailable
    case outcomeUnknown
}

enum CleanupHistoryWarning: Equatable, Sendable {
    case estimatedBytesUnverified
    case dryRunDoesNotMutate
    case trashDoesNotFreeSpaceImmediately
    case permanentRemovalCannotBeUndone
    case cloudEvictionRequiresNetworkToRedownload
}

/// One ordered, path-free cleanup-session observation. It cannot be converted
/// into a candidate, plan, approval, or effect.
struct CleanupHistoryItemModel: Equatable, Identifiable, Sendable {
    var id: UInt16 { ordinal }

    let ordinal: UInt16
    let ruleID: String
    let ruleRevision: UInt32
    let category: ExplorerCandidateCategory?
    let safety: ExplorerCandidateSafety?
    let action: ExplorerCandidateAction?
    let ruleScheduleEligible: Bool?
    let newestModificationAt: Date?
    let estimatedBytes: UInt64
    let status: CleanupHistoryItemStatus
    let errorRecorded: Bool
    let errorCategory: String?
    let pathCount: UInt16
    let evidenceCount: UInt16
}

/// Exact-session history remains immutable presentation data. In particular,
/// it deliberately omits paths, evidence payloads, candidate IDs, execution
/// fences, and every mutation capability.
struct CleanupHistorySessionDetailModel: Equatable, Sendable {
    let summary: CleanupHistorySessionSummaryModel
    let items: [CleanupHistoryItemModel]
    let warnings: [CleanupHistoryWarning]
}

enum CleanupHistoryLoadState: Equatable, Sendable {
    case idle
    case loading
    case loaded
    case failed(CleanupHistoryServiceError)
}

enum CleanupHistoryDetailLoadState: Equatable, Sendable {
    case idle
    case loading
    case loaded(CleanupHistorySessionDetailModel)
    case failed(CleanupHistoryServiceError)
}

enum CleanupHistoryServiceError: Error, Equatable, Sendable {
    case closed
    case invalidSessionID
    case invalidLimit
    case invalidCursor
    case sessionNotFound
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case budgetExceeded
    case corruptData
    case unavailable
    case internalState
    case invalidResponse
}

enum ExplorerSnapshotReviewAcquisitionError: Error, Equatable, Sendable {
    case closed
    case scanNotFound
    case snapshotUnavailable
    case retryable
    case budgetExceeded
    case unavailable
    case invalidResponse
}

enum ExplorerSnapshotNodeSort: CaseIterable, Equatable, Hashable, Sendable {
    case nameAscending
    case logicalBytesDescending
    case allocatedBytesDescending
    case modifiedNewest
}

enum ExplorerSnapshotNodeKind: Equatable, Sendable {
    case directory
    case file
    case symlink
    case other
    case error
}

/// Historical, display-only storage classification derived by Rust from the
/// exact scan's immutable candidate evaluation. It grants no cleanup authority.
enum ExplorerStorageCategory: CaseIterable, Equatable, Hashable, Sendable {
    case unclassified
    case developerArtifact
    case applicationCache
    case browserCache
    case logAndDiagnostic
    case installerAndDownload
    case deviceAndSimulatorData
    case cloudFile
    case largeReviewItem
    case protectedSystemData
    case unknownStorage
}

enum ExplorerSnapshotLivePathPurpose: Equatable, Sendable {
    case reveal
    case copyPath
    case quickLook
}

enum ExplorerSnapshotLivePathError: Error, Equatable, Sendable {
    case reviewExpired
    case reviewNotAcquired
    case nodeNotFound
    case unsupportedItem
    case identityUnavailable
    case unavailable
    case changedSinceScan
    case accessDenied
    case invalidResponse
}

enum ExplorerSnapshotNameEncoding: Equatable, Sendable {
    case unixBytes
    case windowsUTF16LittleEndian
}

/// Lossless historical observation. These bytes support display and snapshot
/// navigation only; they are never a current filesystem or cleanup path.
struct ExplorerSnapshotNodeName: Equatable, Sendable {
    let encoding: ExplorerSnapshotNameEncoding
    let encodedBytes: Data
    let display: String
}

struct ExplorerSnapshotTimestamp: Equatable, Sendable {
    let secondsSinceUnixEpoch: UInt64
    let nanoseconds: UInt32
}

struct ExplorerSnapshotScanFlags: Equatable, Sendable {
    let inaccessible: Bool
    let timedOut: Bool
    let hardLinkDuplicate: Bool
    let mountBoundary: Bool
}

struct ExplorerSnapshotNode: Equatable, Identifiable, Sendable {
    let id: UInt64
    let parentID: UInt64?
    let depth: UInt32
    let kind: ExplorerSnapshotNodeKind
    let category: ExplorerStorageCategory
    let name: ExplorerSnapshotNodeName
    let logicalBytes: UInt64
    let allocatedBytes: UInt64?
    let fileCount: UInt64
    let childCount: UInt64
    let modifiedAt: ExplorerSnapshotTimestamp?
    let accessedAt: ExplorerSnapshotTimestamp?
    let scanFlags: ExplorerSnapshotScanFlags
}

struct ExplorerSnapshotNodePage: Equatable, Sendable {
    let parentID: UInt64
    let offset: UInt64
    let totalChildren: UInt64
    let hasMore: Bool
    let nodes: [ExplorerSnapshotNode]
}

struct ExplorerSnapshotTreemapCell: Equatable, Identifiable, Sendable {
    var id: UInt64 { node.id }

    let node: ExplorerSnapshotNode
    let logicalRank: UInt64
}

/// One bounded, path-free visualization projection for a directory. `Other`
/// is aggregate accounting only and never becomes a node or cleanup target.
struct ExplorerSnapshotTreemap: Equatable, Sendable {
    let parentID: UInt64
    let totalChildren: UInt64
    let totalChildLogicalBytes: UInt64
    let otherChildCount: UInt64
    let otherLogicalBytes: UInt64
    let zeroLogicalChildCount: UInt64
    let cells: [ExplorerSnapshotTreemapCell]

    func cell(nodeID: UInt64) -> ExplorerSnapshotTreemapCell? {
        cells.first { $0.node.id == nodeID }
    }

    var hasOther: Bool {
        otherChildCount > 0
    }
}

/// One historical file observation from a bounded whole-snapshot projection.
/// Parent components are display context only and never form a live path or
/// cleanup target.
struct ExplorerSnapshotLargeFile: Equatable, Identifiable, Sendable {
    var id: UInt64 { node.id }

    let node: ExplorerSnapshotNode
    let parentContext: [ExplorerSnapshotNodeName]
    let contextTruncated: Bool

    var parentDisplay: String {
        let joined = parentContext.map(\.display).joined(separator: " / ")
        return contextTruncated ? "… / \(joined)" : joined
    }
}

struct ExplorerSnapshotLargeFilesPage: Equatable, Sendable {
    let minimumLogicalBytes: UInt64
    let modifiedBefore: ExplorerSnapshotTimestamp?
    let totalMatchingFiles: UInt64
    let totalMatchingLogicalBytes: UInt64
    let hasMore: Bool
    let files: [ExplorerSnapshotLargeFile]
}

enum ExplorerSnapshotContentMode: String, CaseIterable, Equatable, Sendable {
    case browse
    case candidates
    case largeFiles
    case coverage
}

enum ExplorerScanCoverageIssueKind: CaseIterable, Equatable, Hashable, Sendable {
    case permissionDenied
    case timedOut
    case differentFilesystem
    case networkOrVirtualFilesystem
    case symlinkSkipped
    case fileChangedDuringScan
    case metadataError
    case cancelled
    case policyExcluded
    case depthLimited
    case probePoolExhausted
    case filesystemBoundaryUnknown
    case issueLimitReached
}

enum ExplorerScanCoverageLocationScope: Equatable, Sendable {
    case global
    case scanRoot
    case descendant
}

struct ExplorerScanCoverageIssue: Equatable, Identifiable, Sendable {
    var id: UInt16 { ordinal }

    let ordinal: UInt16
    let kind: ExplorerScanCoverageIssueKind
    let occurrenceCount: UInt32
    let locationScope: ExplorerScanCoverageLocationScope
    let locationComponents: [String]
    let locationTruncated: Bool

    var locationDisplay: String {
        switch locationScope {
        case .global: "Whole scan"
        case .scanRoot: "Scan root"
        case .descendant:
            locationComponents.joined(separator: " / ")
                + (locationTruncated ? " · shortened" : "")
        }
    }
}

struct ExplorerScanCoverageDetails: Equatable, Sendable {
    let scanID: String
    let coverage: AppScanCoverage
    let measuredPermille: UInt16?
    let totalIssueRecords: UInt16
    let totalIssueOccurrences: UInt64
    let issues: [ExplorerScanCoverageIssue]
}

struct ExplorerScanCoverageDetailsPage: Equatable, Sendable {
    let scanID: String
    let coverage: AppScanCoverage
    let measuredPermille: UInt16?
    let offset: UInt16
    let totalIssueRecords: UInt16
    let totalIssueOccurrences: UInt64
    let hasMore: Bool
    let issues: [ExplorerScanCoverageIssue]
}

enum ExplorerScanCoverageError: Error, Equatable, Sendable {
    case invalidRequest
    case scanNotFound
    case retryable
    case unavailable
    case invalidResponse
}

enum ExplorerSnapshotLargeFileThreshold: UInt64, CaseIterable, Equatable, Sendable {
    case mebibytes100 = 104_857_600
    case mebibytes500 = 524_288_000
    case gibibyte1 = 1_073_741_824
    case gibibytes5 = 5_368_709_120
    case gibibytes10 = 10_737_418_240
}

enum ExplorerSnapshotLargeFileAge: CaseIterable, Equatable, Sendable {
    case any
    case days30
    case days90
    case year1

    func modifiedBefore(now: Date) -> ExplorerSnapshotTimestamp? {
        guard self != .any else {
            return nil
        }
        let seconds: TimeInterval = switch self {
        case .any: 0
        case .days30: 30 * 24 * 60 * 60
        case .days90: 90 * 24 * 60 * 60
        case .year1: 365 * 24 * 60 * 60
        }
        let cutoff = now.addingTimeInterval(-seconds).timeIntervalSince1970
        guard cutoff.isFinite, cutoff >= 0, cutoff <= Double(UInt64.max) else {
            return nil
        }
        let wholeSeconds = cutoff.rounded(.down)
        let fractional = cutoff - wholeSeconds
        return ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: UInt64(wholeSeconds),
            nanoseconds: UInt32((fractional * 1_000_000_000).rounded(.down))
        )
    }
}

enum ExplorerSnapshotNodeError: Error, Equatable, Sendable {
    case reviewNotAcquired
    case reviewExpired
    case nodeNotFound
    case nodeNotDirectory
    case invalidPage
    case invalidLimit
    case budgetExceeded
    case invalidResponse
}

enum ExplorerSnapshotTreemapError: Error, Equatable, Sendable {
    case reviewNotAcquired
    case reviewExpired
    case nodeNotFound
    case nodeNotDirectory
    case invalidBudget
    case budgetExceeded
    case invalidResponse
}

enum ExplorerSnapshotLargeFilesError: Error, Equatable, Sendable {
    case reviewNotAcquired
    case reviewExpired
    case invalidRequest
    case budgetExceeded
    case invalidResponse
}
