import Foundation

/// The only conversion boundary between generated history records and
/// app-owned Explorer models.
enum ExplorerSnapshotHistoryAdapter {
    private static let recordVersion: UInt32 = 1
    private static let maximumUnixMilliseconds: Int64 = 253_402_300_799_999

    static func map(
        _ page: RecentScanHistoryPage
    ) throws -> ExplorerSnapshotHistoryPage {
        guard page.recordVersion == recordVersion else {
            throw ExplorerSnapshotHistoryError.invalidResponse
        }
        let scans = try page.scans.map(mapItem)
        for pair in zip(scans, scans.dropFirst()) {
            let ordered = pair.0.startedAt > pair.1.startedAt
                || (pair.0.startedAt == pair.1.startedAt && pair.0.scanID < pair.1.scanID)
            guard ordered else {
                throw ExplorerSnapshotHistoryError.invalidResponse
            }
        }
        return ExplorerSnapshotHistoryPage(scans: scans, hasMore: page.hasMore)
    }

    private static func mapItem(
        _ raw: HistoricalScanSummary
    ) throws -> ExplorerHistoricalScan {
        guard
            raw.recordVersion == recordVersion,
            validScanID(raw.scanId),
            validUnixMilliseconds(raw.startedAtUnixMs),
            raw.completedAtUnixMs.map(validUnixMilliseconds) ?? true,
            raw.completedAtUnixMs.map({ $0 >= raw.startedAtUnixMs }) ?? true,
            validCoverage(raw.coverage)
        else {
            throw ExplorerSnapshotHistoryError.invalidResponse
        }

        let status: ExplorerHistoricalScanStatus
        switch raw.status {
        case .queued:
            status = .queued
        case .running:
            status = .running
        case .succeeded:
            status = .succeeded
        case .failed:
            status = .failed
        case .cancelled:
            status = .cancelled
        case .interrupted:
            status = .interrupted
        }
        let isSucceeded = status == .succeeded
        let isActive = status == .queued || status == .running
        guard
            (raw.counts != nil) == isSucceeded,
            !raw.snapshotRecorded || isSucceeded,
            isActive ? raw.completedAtUnixMs == nil : raw.completedAtUnixMs != nil
        else {
            throw ExplorerSnapshotHistoryError.invalidResponse
        }

        return ExplorerHistoricalScan(
            scanID: raw.scanId,
            startedAt: date(raw.startedAtUnixMs),
            completedAt: raw.completedAtUnixMs.map(date),
            status: status,
            counts: raw.counts.map {
                ExplorerHistoricalScanCounts(
                    directoryCount: $0.directoryCount,
                    fileCount: $0.fileCount,
                    logicalBytes: $0.logicalBytes,
                    allocatedBytes: $0.allocatedBytes
                )
            },
            coverage: mapCoverage(raw.coverage.status),
            coveragePermille: raw.coverage.measuredPermille,
            issueCount: raw.coverage.issueOccurrenceCount,
            snapshotRecorded: raw.snapshotRecorded
        )
    }

    static func validScanID(_ value: String) -> Bool {
        value.hasPrefix("scan:")
            && value.utf8.count > 5
            && value.utf8.count <= 128
            && value.unicodeScalars.allSatisfy { scalar in
                scalar.isASCII
                    && (CharacterSet.alphanumerics.contains(scalar)
                        || "._-:".unicodeScalars.contains(scalar))
            }
    }

    private static func validUnixMilliseconds(_ value: Int64) -> Bool {
        (0 ... maximumUnixMilliseconds).contains(value)
    }

    private static func date(_ unixMilliseconds: Int64) -> Date {
        Date(timeIntervalSince1970: Double(unixMilliseconds) / 1_000)
    }

    private static func validCoverage(_ coverage: ScanCoverageSummary) -> Bool {
        guard
            coverage.recordVersion == recordVersion,
            coverage.issueRecordCount <= 256,
            coverage.issueOccurrenceCount >= coverage.issueRecordCount,
            coverage.measuredPermille.map({ $0 <= 1_000 }) ?? true
        else {
            return false
        }
        switch coverage.status {
        case .unknown:
            return coverage.measuredPermille == nil
                && coverage.issueRecordCount == 0
                && coverage.issueOccurrenceCount == 0
        case .complete:
            return coverage.measuredPermille == 1_000
                && coverage.issueRecordCount == 0
                && coverage.issueOccurrenceCount == 0
        case .limitedAccess, .partial:
            return coverage.issueRecordCount > 0 && coverage.measuredPermille != 1_000
        }
    }

    private static func mapCoverage(_ coverage: ScanCoverageStatus) -> AppScanCoverage {
        switch coverage {
        case .unknown: .unknown
        case .complete: .complete
        case .limitedAccess: .limitedAccess
        case .partial: .partial
        }
    }
}
