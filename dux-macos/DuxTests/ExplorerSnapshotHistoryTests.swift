import XCTest
@testable import DUX

final class ExplorerSnapshotHistoryTests: XCTestCase {
    func testMapsNewestFirstHistoryAndIdentifiesNewestReviewCandidate() throws {
        let running = historicalScan(
            id: "scan:newest-running",
            started: 3_000,
            completed: nil,
            status: .running,
            counts: nil,
            coverage: unknownCoverage(),
            snapshotRecorded: false
        )
        let succeeded = historicalScan(
            id: "scan:reviewable",
            started: 2_000,
            completed: 2_500,
            status: .succeeded,
            counts: HistoricalScanCounts(
                directoryCount: 4,
                fileCount: 8,
                logicalBytes: 12,
                allocatedBytes: 16
            ),
            coverage: completeCoverage(),
            snapshotRecorded: true
        )

        let mapped = try ExplorerSnapshotHistoryAdapter.map(
            RecentScanHistoryPage(
                recordVersion: 1,
                scans: [running, succeeded],
                hasMore: true
            )
        )

        XCTAssertTrue(mapped.hasMore)
        XCTAssertEqual(mapped.scans.map(\.scanID), ["scan:newest-running", "scan:reviewable"])
        XCTAssertFalse(mapped.scans[0].canRequestReview)
        XCTAssertEqual(mapped.newestReviewCandidate?.scanID, "scan:reviewable")
        XCTAssertEqual(
            mapped.newestReviewCandidate?.counts,
            ExplorerHistoricalScanCounts(
                directoryCount: 4,
                fileCount: 8,
                logicalBytes: 12,
                allocatedBytes: 16
            )
        )
        XCTAssertEqual(mapped.newestReviewCandidate?.coverage, .complete)
    }

    func testRejectsUnorderedAndAuthorityShapedHistory() {
        let older = validSucceeded(id: "scan:older", started: 1_000)
        let newer = validSucceeded(id: "scan:newer", started: 2_000)
        XCTAssertThrowsError(
            try ExplorerSnapshotHistoryAdapter.map(
                RecentScanHistoryPage(
                    recordVersion: 1,
                    scans: [older, newer],
                    hasMore: false
                )
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerSnapshotHistoryError, .invalidResponse)
        }

        let runningWithSnapshot = historicalScan(
            id: "scan:running-snapshot",
            started: 3_000,
            completed: nil,
            status: .running,
            counts: nil,
            coverage: unknownCoverage(),
            snapshotRecorded: true
        )
        XCTAssertThrowsError(
            try ExplorerSnapshotHistoryAdapter.map(
                RecentScanHistoryPage(
                    recordVersion: 1,
                    scans: [runningWithSnapshot],
                    hasMore: false
                )
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerSnapshotHistoryError, .invalidResponse)
        }
    }

    func testRejectsMalformedRecordAndCoverageShapes() {
        let wrongVersion = HistoricalScanSummary(
            recordVersion: 2,
            scanId: "scan:wrong-version",
            startedAtUnixMs: 1_000,
            completedAtUnixMs: 2_000,
            status: .succeeded,
            counts: HistoricalScanCounts(
                directoryCount: 1,
                fileCount: 1,
                logicalBytes: 1,
                allocatedBytes: nil
            ),
            coverage: completeCoverage(),
            snapshotRecorded: true
        )
        let malformedCoverage = historicalScan(
            id: "scan:bad-coverage",
            started: 1_000,
            completed: 2_000,
            status: .succeeded,
            counts: HistoricalScanCounts(
                directoryCount: 1,
                fileCount: 1,
                logicalBytes: 1,
                allocatedBytes: nil
            ),
            coverage: ScanCoverageSummary(
                recordVersion: 1,
                status: .complete,
                measuredPermille: 999,
                issueRecordCount: 0,
                issueOccurrenceCount: 0
            ),
            snapshotRecorded: true
        )

        for raw in [wrongVersion, malformedCoverage] {
            XCTAssertThrowsError(
                try ExplorerSnapshotHistoryAdapter.map(
                    RecentScanHistoryPage(recordVersion: 1, scans: [raw], hasMore: false)
                )
            ) { error in
                XCTAssertEqual(error as? ExplorerSnapshotHistoryError, .invalidResponse)
            }
        }
    }

    func testServiceRejectsUnboundedHistoryLimitBeforeOpeningEngine() async {
        let service = EngineService()
        for limit: UInt16 in [0, 201] {
            do {
                _ = try await service.loadRecentSnapshotHistory(limit: limit)
                XCTFail("expected invalid limit")
            } catch {
                XCTAssertEqual(error as? ExplorerSnapshotHistoryError, .invalidLimit)
            }
        }
    }

    private func validSucceeded(id: String, started: Int64) -> HistoricalScanSummary {
        historicalScan(
            id: id,
            started: started,
            completed: started + 100,
            status: .succeeded,
            counts: HistoricalScanCounts(
                directoryCount: 1,
                fileCount: 2,
                logicalBytes: 3,
                allocatedBytes: 4
            ),
            coverage: completeCoverage(),
            snapshotRecorded: true
        )
    }

    private func historicalScan(
        id: String,
        started: Int64,
        completed: Int64?,
        status: HistoricalScanStatus,
        counts: HistoricalScanCounts?,
        coverage: ScanCoverageSummary,
        snapshotRecorded: Bool
    ) -> HistoricalScanSummary {
        HistoricalScanSummary(
            recordVersion: 1,
            scanId: id,
            startedAtUnixMs: started,
            completedAtUnixMs: completed,
            status: status,
            counts: counts,
            coverage: coverage,
            snapshotRecorded: snapshotRecorded
        )
    }

    private func completeCoverage() -> ScanCoverageSummary {
        ScanCoverageSummary(
            recordVersion: 1,
            status: .complete,
            measuredPermille: 1_000,
            issueRecordCount: 0,
            issueOccurrenceCount: 0
        )
    }

    private func unknownCoverage() -> ScanCoverageSummary {
        ScanCoverageSummary(
            recordVersion: 1,
            status: .unknown,
            measuredPermille: nil,
            issueRecordCount: 0,
            issueOccurrenceCount: 0
        )
    }
}
