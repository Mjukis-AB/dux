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
