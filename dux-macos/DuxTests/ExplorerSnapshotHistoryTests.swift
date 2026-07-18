import Darwin
import XCTest
@testable import DUX

private func liveTarget(
    bytes: Data,
    exactText: String?,
    nodeID: UInt64 = 7,
    purpose: SnapshotLiveTargetPurpose = .quickLook,
    encoding: SnapshotNameEncoding = .unixBytes
) -> SnapshotLiveTarget {
    SnapshotLiveTarget(
        recordVersion: 1,
        nodeId: nodeID,
        purpose: purpose,
        kind: .file,
        pathEncoding: encoding,
        absolutePathBytes: bytes,
        displayPath: String(decoding: bytes, as: UTF8.self),
        exactTextPath: exactText
    )
}

private func fileSystemBytes(_ url: URL) -> Data {
    url.withUnsafeFileSystemRepresentation { pointer in
        guard let pointer else {
            return Data()
        }
        return Data(bytes: pointer, count: strlen(pointer))
    }
}

final class ExplorerSnapshotHistoryTests: XCTestCase {
    func testLivePathAdapterPreservesUnicodeAndRejectsLossyNonUnicodeURL() throws {
        let unicodePaths = [
            "/tmp/Données/report.mov",
            "/tmp/e\u{301}/#report? 1%.mov",
            "/tmp/é/#report? 1%.mov",
        ]
        for path in unicodePaths {
            let bytes = Data(path.utf8)
            let item = try ExplorerSnapshotLivePathAdapter.map(
                liveTarget(bytes: bytes, exactText: path),
                requestedNodeID: 7,
                requestedPurpose: .quickLook
            )
            XCTAssertEqual(item.exactTextPath, path)
            XCTAssertEqual(fileSystemBytes(item.url), bytes)
        }

        let nonUnicodeBytes = Data([0x2f, 0x74, 0x6d, 0x70, 0x2f, 0xff])
        XCTAssertThrowsError(
            try ExplorerSnapshotLivePathAdapter.map(
                liveTarget(bytes: nonUnicodeBytes, exactText: nil),
                requestedNodeID: 7,
                requestedPurpose: .quickLook
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerSnapshotLivePathError, .unsupportedItem)
        }
    }

    func testLivePathAdapterRejectsContradictoryOrUnsafeRecords() {
        let validBytes = Data("/tmp/report.mov".utf8)
        let invalid: [SnapshotLiveTarget] = [
            liveTarget(bytes: validBytes, exactText: "/tmp/report.mov", nodeID: 8),
            liveTarget(bytes: validBytes, exactText: "/tmp/report.mov", purpose: .reveal),
            liveTarget(
                bytes: validBytes,
                exactText: "/tmp/report.mov",
                encoding: .windowsUtf16LittleEndian
            ),
            liveTarget(bytes: Data("relative".utf8), exactText: "relative"),
            liveTarget(bytes: Data("/tmp/../secret".utf8), exactText: "/tmp/../secret"),
            liveTarget(bytes: Data([0x2f, 0x74, 0x00, 0x78]), exactText: nil),
            liveTarget(bytes: Data("/tmp/next\u{0085}line".utf8), exactText: "/tmp/next\u{0085}line"),
            liveTarget(bytes: Data("/tmp/control\u{009b}".utf8), exactText: "/tmp/control\u{009b}"),
            liveTarget(bytes: validBytes, exactText: "/tmp/different.mov"),
            SnapshotLiveTarget(
                recordVersion: 1,
                nodeId: 7,
                purpose: .quickLook,
                kind: .file,
                pathEncoding: .unixBytes,
                absolutePathBytes: validBytes,
                displayPath: "different",
                exactTextPath: "/tmp/report.mov"
            ),
        ]

        for record in invalid {
            XCTAssertThrowsError(
                try ExplorerSnapshotLivePathAdapter.map(
                    record,
                    requestedNodeID: 7,
                    requestedPurpose: .quickLook
                )
            )
        }
    }

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

private final class RecordingTrashFileManager: TrashFileManaging {
    enum Failure: Error {
        case denied
    }

    var calls = 0
    var requestedURL: URL?
    var shouldFail = false

    func moveToTrash(at url: URL) throws {
        calls += 1
        requestedURL = url
        if shouldFail {
            throw Failure.denied
        }
    }
}

final class MacOSTrashPlatformAdapterTests: XCTestCase {
    func testRecordingAdapterReceivesTheExactReviewedURLOnceWithoutMutation() {
        let fileManager = RecordingTrashFileManager()
        let adapter = MacOSTrashPlatformAdapter(fileManager: fileManager)
        let reviewedURL = URL(fileURLWithPath: "/private/tmp/dux-reviewed-item")

        guard case .success = adapter.trash(reviewedURL) else {
            return XCTFail("the recording adapter should report success")
        }
        XCTAssertEqual(fileManager.calls, 1)
        XCTAssertEqual(fileManager.requestedURL, reviewedURL)
    }

    func testFoundationFailureMapsToUnknownAndIsNotRetried() {
        let fileManager = RecordingTrashFileManager()
        fileManager.shouldFail = true
        let adapter = MacOSTrashPlatformAdapter(fileManager: fileManager)

        guard case .failure(.outcomeUnknown) = adapter.trash(
            URL(fileURLWithPath: "/private/tmp/dux-reviewed-item")
        ) else {
            return XCTFail("Foundation failures must map to an unknown outcome")
        }
        XCTAssertEqual(fileManager.calls, 1)
    }
}

final class ExplorerScanCoverageDetailsAdapterTests: XCTestCase {
    func testMapsAndAssemblesExactRootRelativeCoveragePages() throws {
        let coverage = ScanCoverageSummary(
            recordVersion: 1,
            status: .partial,
            measuredPermille: 500,
            issueRecordCount: 2,
            issueOccurrenceCount: 5
        )
        let raw = ScanCoverageDetailsPage(
            recordVersion: 1,
            scanId: "scan:coverage",
            coverage: coverage,
            offset: 0,
            totalIssueRecords: 2,
            totalIssueOccurrences: 5,
            hasMore: false,
            issues: [
                issue(
                    ordinal: 0,
                    kind: .permissionDenied,
                    count: 2,
                    scope: .scanRoot
                ),
                issue(
                    ordinal: 1,
                    kind: .metadataError,
                    count: 3,
                    scope: .descendant,
                    components: ["Library", "Caches"]
                ),
            ]
        )
        let page = try ExplorerScanCoverageDetailsAdapter.map(
            raw,
            requestedScanID: "scan:coverage",
            requestedOffset: 0,
            requestedLimit: 64
        )
        let details = try ExplorerScanCoverageDetailsAdapter.assemble(
            [page],
            requestedScanID: "scan:coverage"
        )
        XCTAssertEqual(details.coverage, .partial)
        XCTAssertEqual(details.measuredPermille, 500)
        XCTAssertEqual(details.totalIssueOccurrences, 5)
        XCTAssertEqual(details.issues.map(\.ordinal), [0, 1])
        XCTAssertEqual(details.issues[0].locationDisplay, "Scan root")
        XCTAssertEqual(details.issues[1].locationDisplay, "Library / Caches")
    }

    func testAssemblesTwoPagesAndRejectsChangedCrossPageTotals() throws {
        let coverage = ScanCoverageSummary(
            recordVersion: 1,
            status: .partial,
            measuredPermille: nil,
            issueRecordCount: 2,
            issueOccurrenceCount: 2
        )
        let first = try ExplorerScanCoverageDetailsAdapter.map(
            ScanCoverageDetailsPage(
                recordVersion: 1,
                scanId: "scan:coverage-pages",
                coverage: coverage,
                offset: 0,
                totalIssueRecords: 2,
                totalIssueOccurrences: 2,
                hasMore: true,
                issues: [issue(
                    ordinal: 0,
                    kind: .permissionDenied,
                    count: 1,
                    scope: .scanRoot
                )]
            ),
            requestedScanID: "scan:coverage-pages",
            requestedOffset: 0,
            requestedLimit: 1
        )
        let second = try ExplorerScanCoverageDetailsAdapter.map(
            ScanCoverageDetailsPage(
                recordVersion: 1,
                scanId: "scan:coverage-pages",
                coverage: coverage,
                offset: 1,
                totalIssueRecords: 2,
                totalIssueOccurrences: 2,
                hasMore: false,
                issues: [issue(
                    ordinal: 1,
                    kind: .metadataError,
                    count: 1,
                    scope: .descendant,
                    components: ["Library"]
                )]
            ),
            requestedScanID: "scan:coverage-pages",
            requestedOffset: 1,
            requestedLimit: 1
        )
        let details = try ExplorerScanCoverageDetailsAdapter.assemble(
            [first, second],
            requestedScanID: "scan:coverage-pages"
        )
        XCTAssertEqual(details.issues.map(\.ordinal), [0, 1])

        let changedSecond = ExplorerScanCoverageDetailsPage(
            scanID: second.scanID,
            coverage: second.coverage,
            measuredPermille: second.measuredPermille,
            offset: second.offset,
            totalIssueRecords: second.totalIssueRecords,
            totalIssueOccurrences: 3,
            hasMore: second.hasMore,
            issues: second.issues
        )
        XCTAssertThrowsError(
            try ExplorerScanCoverageDetailsAdapter.assemble(
                [first, changedSecond],
                requestedScanID: "scan:coverage-pages"
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerScanCoverageError, .invalidResponse)
        }
    }

    func testRejectsHostileLocationAndContradictoryCoverage() throws {
        let malformedLocation = ScanCoverageDetailsPage(
            recordVersion: 1,
            scanId: "scan:coverage",
            coverage: ScanCoverageSummary(
                recordVersion: 1,
                status: .limitedAccess,
                measuredPermille: nil,
                issueRecordCount: 1,
                issueOccurrenceCount: 1
            ),
            offset: 0,
            totalIssueRecords: 1,
            totalIssueOccurrences: 1,
            hasMore: false,
            issues: [issue(
                ordinal: 0,
                kind: .permissionDenied,
                count: 1,
                scope: .global,
                components: ["absolute-root-must-not-appear"]
            )]
        )
        XCTAssertThrowsError(
            try ExplorerScanCoverageDetailsAdapter.map(
                malformedLocation,
                requestedScanID: "scan:coverage",
                requestedOffset: 0,
                requestedLimit: 64
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerScanCoverageError, .invalidResponse)
        }

        let nonPermission = ScanCoverageDetailsPage(
            recordVersion: 1,
            scanId: "scan:coverage",
            coverage: ScanCoverageSummary(
                recordVersion: 1,
                status: .limitedAccess,
                measuredPermille: nil,
                issueRecordCount: 1,
                issueOccurrenceCount: 1
            ),
            offset: 0,
            totalIssueRecords: 1,
            totalIssueOccurrences: 1,
            hasMore: false,
            issues: [issue(
                ordinal: 0,
                kind: .timedOut,
                count: 1,
                scope: .scanRoot
            )]
        )
        let mapped = try ExplorerScanCoverageDetailsAdapter.map(
            nonPermission,
            requestedScanID: "scan:coverage",
            requestedOffset: 0,
            requestedLimit: 64
        )
        XCTAssertThrowsError(
            try ExplorerScanCoverageDetailsAdapter.assemble(
                [mapped],
                requestedScanID: "scan:coverage"
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerScanCoverageError, .invalidResponse)
        }
    }

    func testRejectsNoncanonicalIssueKindOrder() throws {
        let raw = ScanCoverageDetailsPage(
            recordVersion: 1,
            scanId: "scan:coverage-order",
            coverage: ScanCoverageSummary(
                recordVersion: 1,
                status: .partial,
                measuredPermille: nil,
                issueRecordCount: 2,
                issueOccurrenceCount: 2
            ),
            offset: 0,
            totalIssueRecords: 2,
            totalIssueOccurrences: 2,
            hasMore: false,
            issues: [
                issue(ordinal: 0, kind: .metadataError, count: 1, scope: .scanRoot),
                issue(ordinal: 1, kind: .timedOut, count: 1, scope: .scanRoot),
            ]
        )
        let page = try ExplorerScanCoverageDetailsAdapter.map(
            raw,
            requestedScanID: "scan:coverage-order",
            requestedOffset: 0,
            requestedLimit: 64
        )
        XCTAssertThrowsError(
            try ExplorerScanCoverageDetailsAdapter.assemble(
                [page],
                requestedScanID: "scan:coverage-order"
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerScanCoverageError, .invalidResponse)
        }
    }

    func testAcceptsExplicitlyShortenedSingleLongComponent() throws {
        let component = String(repeating: "x", count: 128)
        let raw = ScanCoverageDetailsPage(
            recordVersion: 1,
            scanId: "scan:coverage-long-component",
            coverage: ScanCoverageSummary(
                recordVersion: 1,
                status: .partial,
                measuredPermille: nil,
                issueRecordCount: 1,
                issueOccurrenceCount: 1
            ),
            offset: 0,
            totalIssueRecords: 1,
            totalIssueOccurrences: 1,
            hasMore: false,
            issues: [HistoricalScanIssue(
                recordVersion: 1,
                ordinal: 0,
                kind: .metadataError,
                occurrenceCount: 1,
                locationScope: .descendant,
                locationComponents: [component],
                locationTruncated: true
            )]
        )
        let page = try ExplorerScanCoverageDetailsAdapter.map(
            raw,
            requestedScanID: "scan:coverage-long-component",
            requestedOffset: 0,
            requestedLimit: 64
        )
        let details = try ExplorerScanCoverageDetailsAdapter.assemble(
            [page],
            requestedScanID: "scan:coverage-long-component"
        )
        XCTAssertEqual(details.issues[0].locationDisplay, "\(component) · shortened")
    }

    func testRejectsAuthorityShapedDescendantComponents() {
        let oversizedScalarCluster = "a" + String(repeating: "\u{301}", count: 128)
        for component in [".", "..", "/Users", "a/b", "bad\0name", oversizedScalarCluster] {
            let raw = ScanCoverageDetailsPage(
                recordVersion: 1,
                scanId: "scan:coverage-component",
                coverage: ScanCoverageSummary(
                    recordVersion: 1,
                    status: .partial,
                    measuredPermille: nil,
                    issueRecordCount: 1,
                    issueOccurrenceCount: 1
                ),
                offset: 0,
                totalIssueRecords: 1,
                totalIssueOccurrences: 1,
                hasMore: false,
                issues: [HistoricalScanIssue(
                    recordVersion: 1,
                    ordinal: 0,
                    kind: .metadataError,
                    occurrenceCount: 1,
                    locationScope: .descendant,
                    locationComponents: [component],
                    locationTruncated: false
                )]
            )
            XCTAssertThrowsError(
                try ExplorerScanCoverageDetailsAdapter.map(
                    raw,
                    requestedScanID: "scan:coverage-component",
                    requestedOffset: 0,
                    requestedLimit: 64
                )
            ) { error in
                XCTAssertEqual(error as? ExplorerScanCoverageError, .invalidResponse)
            }
        }
    }

    private func issue(
        ordinal: UInt16,
        kind: HistoricalScanIssueKind,
        count: UInt32,
        scope: HistoricalScanIssueLocationScope,
        components: [String] = []
    ) -> HistoricalScanIssue {
        HistoricalScanIssue(
            recordVersion: 1,
            ordinal: ordinal,
            kind: kind,
            occurrenceCount: count,
            locationScope: scope,
            locationComponents: components,
            locationTruncated: false
        )
    }
}

final class ExplorerSnapshotNodeAdapterTests: XCTestCase {
    func testMapsLosslessRootAndBoundedChildPage() throws {
        let root = try ExplorerSnapshotNodeAdapter.mapRoot(
            node(
                id: 0,
                parentID: nil,
                depth: 0,
                kind: .directory,
                name: "/Users/example",
                childCount: 2
            )
        )
        XCTAssertEqual(root.id, 0)
        XCTAssertEqual(root.name.display, "/Users/example")
        XCTAssertEqual(root.name.encodedBytes, Data("/Users/example".utf8))

        let raw = SnapshotNodePage(
            recordVersion: 1,
            parentId: 0,
            offset: 0,
            totalChildren: 2,
            hasMore: true,
            nodes: [node(id: 1, parentID: 0, depth: 1, kind: .file, name: "large")]
        )
        let page = try ExplorerSnapshotNodeAdapter.mapPage(
            raw,
            expectedParentID: 0,
            expectedOffset: 0,
            requestedLimit: 1
        )
        XCTAssertEqual(page.nodes.map(\.name.display), ["large"])
        XCTAssertTrue(page.hasMore)
    }

    func testRejectsMismatchedDisplayAndImpossiblePagination() {
        let mismatched = SnapshotNode(
            recordVersion: 2,
            id: 0,
            parentId: nil,
            depth: 0,
            kind: .directory,
            category: .unclassified,
            name: SnapshotNodeName(
                encoding: .unixBytes,
                encodedBytes: Data("/actual".utf8),
                display: "/different"
            ),
            logicalBytes: 0,
            allocatedBytes: nil,
            fileCount: 0,
            childCount: 0,
            modifiedAt: nil,
            accessedAt: nil,
            scanFlags: flags()
        )
        XCTAssertThrowsError(try ExplorerSnapshotNodeAdapter.mapRoot(mismatched)) { error in
            XCTAssertEqual(error as? ExplorerSnapshotNodeError, .invalidResponse)
        }

        let impossible = SnapshotNodePage(
            recordVersion: 1,
            parentId: 0,
            offset: 1,
            totalChildren: 1,
            hasMore: true,
            nodes: []
        )
        XCTAssertThrowsError(
            try ExplorerSnapshotNodeAdapter.mapPage(
                impossible,
                expectedParentID: 0,
                expectedOffset: 1,
                requestedLimit: 1
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerSnapshotNodeError, .invalidResponse)
        }

        let shortPage = SnapshotNodePage(
            recordVersion: 1,
            parentId: 0,
            offset: 0,
            totalChildren: 2,
            hasMore: true,
            nodes: []
        )
        XCTAssertThrowsError(
            try ExplorerSnapshotNodeAdapter.mapPage(
                shortPage,
                expectedParentID: 0,
                expectedOffset: 0,
                requestedLimit: 1
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerSnapshotNodeError, .invalidResponse)
        }
    }

    func testMapsExplicitLossyUnixAndUTF16DisplaysWithoutLosingBytes() throws {
        let unixBytes = Data([0x66, 0x80])
        let unix = try ExplorerSnapshotNodeAdapter.mapPage(
            SnapshotNodePage(
                recordVersion: 1,
                parentId: 0,
                offset: 0,
                totalChildren: 1,
                hasMore: false,
                nodes: [node(
                    id: 1,
                    parentID: 0,
                    depth: 1,
                    kind: .file,
                    nameBytes: unixBytes,
                    display: "f\u{fffd}"
                )]
            ),
            expectedParentID: 0,
            expectedOffset: 0,
            requestedLimit: 1
        )
        XCTAssertEqual(unix.nodes[0].name.encodedBytes, unixBytes)
        XCTAssertEqual(unix.nodes[0].name.display, "f\u{fffd}")

        let utf16Bytes = Data([0x00, 0xd8])
        let windows = try ExplorerSnapshotNodeAdapter.mapPage(
            SnapshotNodePage(
                recordVersion: 1,
                parentId: 0,
                offset: 0,
                totalChildren: 1,
                hasMore: false,
                nodes: [node(
                    id: 1,
                    parentID: 0,
                    depth: 1,
                    kind: .file,
                    nameBytes: utf16Bytes,
                    display: "\u{fffd}",
                    encoding: .windowsUtf16LittleEndian
                )]
            ),
            expectedParentID: 0,
            expectedOffset: 0,
            requestedLimit: 1
        )
        XCTAssertEqual(windows.nodes[0].name.encodedBytes, utf16Bytes)
        XCTAssertEqual(windows.nodes[0].name.display, "\u{fffd}")
    }

    func testMapsEveryStorageCategoryWithoutInferringFromDisplayName() throws {
        let categories: [(SnapshotStorageCategory, ExplorerStorageCategory)] = [
            (.unclassified, .unclassified),
            (.developerArtifact, .developerArtifact),
            (.applicationCache, .applicationCache),
            (.browserCache, .browserCache),
            (.logAndDiagnostic, .logAndDiagnostic),
            (.installerAndDownload, .installerAndDownload),
            (.deviceAndSimulatorData, .deviceAndSimulatorData),
            (.cloudFile, .cloudFile),
            (.largeReviewItem, .largeReviewItem),
            (.protectedSystemData, .protectedSystemData),
            (.unknownStorage, .unknownStorage),
        ]
        let raw = SnapshotNodePage(
            recordVersion: 1,
            parentId: 0,
            offset: 0,
            totalChildren: UInt64(categories.count),
            hasMore: false,
            nodes: categories.enumerated().map { index, category in
                node(
                    id: UInt64(index + 1),
                    parentID: 0,
                    depth: 1,
                    kind: .file,
                    name: "same-display-name",
                    category: category.0
                )
            }
        )

        let mapped = try ExplorerSnapshotNodeAdapter.mapPage(
            raw,
            expectedParentID: 0,
            expectedOffset: 0,
            requestedLimit: UInt16(categories.count)
        )
        XCTAssertEqual(mapped.nodes.map(\.category), categories.map(\.1))
    }

    func testMapsBoundedTreemapWithExactOtherAccounting() throws {
        let raw = SnapshotTreemap(
            recordVersion: 1,
            parentId: 0,
            totalChildren: 3,
            totalChildLogicalBytes: 35,
            otherChildCount: 1,
            otherLogicalBytes: 5,
            zeroLogicalChildCount: 0,
            cells: [
                SnapshotTreemapCell(
                    recordVersion: 1,
                    node: node(
                        id: 1,
                        parentID: 0,
                        depth: 1,
                        kind: .directory,
                        name: "large",
                        logicalBytes: 20
                    ),
                    logicalRank: 0
                ),
                SnapshotTreemapCell(
                    recordVersion: 1,
                    node: node(
                        id: 2,
                        parentID: 0,
                        depth: 1,
                        kind: .file,
                        name: "medium",
                        logicalBytes: 10
                    ),
                    logicalRank: 1
                ),
            ]
        )

        let mapped = try ExplorerSnapshotNodeAdapter.mapTreemap(
            raw,
            expectedParentID: 0,
            requestedMaxCells: 2
        )

        XCTAssertEqual(mapped.cells.map(\.id), [1, 2])
        XCTAssertEqual(mapped.cells.map(\.logicalRank), [0, 1])
        XCTAssertEqual(mapped.otherChildCount, 1)
        XCTAssertEqual(mapped.otherLogicalBytes, 5)
        XCTAssertTrue(mapped.hasOther)
    }

    func testRejectsHostileTreemapRanksCountsAndTotals() {
        let cell = SnapshotTreemapCell(
            recordVersion: 1,
            node: node(
                id: 1,
                parentID: 0,
                depth: 1,
                kind: .file,
                name: "item",
                logicalBytes: 10
            ),
            logicalRank: 0
        )
        let malformed = [
            SnapshotTreemap(
                recordVersion: 2,
                parentId: 0,
                totalChildren: 1,
                totalChildLogicalBytes: 10,
                otherChildCount: 0,
                otherLogicalBytes: 0,
                zeroLogicalChildCount: 0,
                cells: [cell]
            ),
            SnapshotTreemap(
                recordVersion: 1,
                parentId: 0,
                totalChildren: 2,
                totalChildLogicalBytes: 10,
                otherChildCount: 0,
                otherLogicalBytes: 0,
                zeroLogicalChildCount: 0,
                cells: [cell]
            ),
            SnapshotTreemap(
                recordVersion: 1,
                parentId: 0,
                totalChildren: 1,
                totalChildLogicalBytes: 11,
                otherChildCount: 0,
                otherLogicalBytes: 0,
                zeroLogicalChildCount: 0,
                cells: [cell]
            ),
            SnapshotTreemap(
                recordVersion: 1,
                parentId: 0,
                totalChildren: 1,
                totalChildLogicalBytes: 10,
                otherChildCount: 0,
                otherLogicalBytes: 0,
                zeroLogicalChildCount: 0,
                cells: [SnapshotTreemapCell(
                    recordVersion: 1,
                    node: cell.node,
                    logicalRank: 1
                )]
            ),
        ]

        for raw in malformed {
            XCTAssertThrowsError(
                try ExplorerSnapshotNodeAdapter.mapTreemap(
                    raw,
                    expectedParentID: 0,
                    requestedMaxCells: 2
                )
            )
        }
    }

    func testMapsBoundedLargeFilesWithExactAggregateAndContext() throws {
        let cutoff = ExplorerSnapshotTimestamp(secondsSinceUnixEpoch: 10, nanoseconds: 0)
        let raw = SnapshotLargeFilePage(
            recordVersion: 1,
            totalMatchingFiles: 2,
            totalMatchingLogicalBytes: 30,
            hasMore: false,
            files: [
                largeFile(id: 10, name: "alpha", logicalBytes: 20),
                largeFile(id: 11, name: "beta", logicalBytes: 10),
            ]
        )

        let page = try ExplorerSnapshotLargeFilesAdapter.map(
            raw,
            minimumLogicalBytes: 10,
            modifiedBefore: cutoff,
            requestedMaxResults: 100
        )

        XCTAssertEqual(page.files.map(\.id), [10, 11])
        XCTAssertEqual(page.files.map(\.parentDisplay), ["Downloads", "Downloads"])
        XCTAssertEqual(page.totalMatchingFiles, 2)
        XCTAssertEqual(page.totalMatchingLogicalBytes, 30)
        XCTAssertFalse(page.hasMore)
    }

    func testRejectsHostileLargeFileOrderContextAndAggregate() {
        let first = largeFile(id: 10, name: "alpha", logicalBytes: 20)
        let second = largeFile(id: 11, name: "beta", logicalBytes: 10)
        let malformed = [
            SnapshotLargeFilePage(
                recordVersion: 1,
                totalMatchingFiles: 2,
                totalMatchingLogicalBytes: 30,
                hasMore: false,
                files: [second, first]
            ),
            SnapshotLargeFilePage(
                recordVersion: 1,
                totalMatchingFiles: 2,
                totalMatchingLogicalBytes: 29,
                hasMore: false,
                files: [first, second]
            ),
            SnapshotLargeFilePage(
                recordVersion: 1,
                totalMatchingFiles: 3,
                totalMatchingLogicalBytes: 30,
                hasMore: false,
                files: [first, second]
            ),
            SnapshotLargeFilePage(
                recordVersion: 1,
                totalMatchingFiles: 1,
                totalMatchingLogicalBytes: 20,
                hasMore: false,
                files: [SnapshotLargeFile(
                    recordVersion: 1,
                    node: first.node,
                    parentContext: [],
                    contextTruncated: false
                )]
            ),
            SnapshotLargeFilePage(
                recordVersion: 1,
                totalMatchingFiles: 1,
                totalMatchingLogicalBytes: 20,
                hasMore: true,
                files: []
            ),
        ]

        for raw in malformed {
            XCTAssertThrowsError(
                try ExplorerSnapshotLargeFilesAdapter.map(
                    raw,
                    minimumLogicalBytes: 10,
                    modifiedBefore: nil,
                    requestedMaxResults: 100
                )
            ) { error in
                XCTAssertEqual(error as? ExplorerSnapshotLargeFilesError, .invalidResponse)
            }
        }
    }

    func testLargeFileRequestRejectsZeroThresholdAndInvalidTimestamp() {
        XCTAssertThrowsError(
            try ExplorerSnapshotLargeFilesAdapter.request(
                minimumLogicalBytes: 0,
                modifiedBefore: nil,
                maxResults: 100
            )
        )
        XCTAssertThrowsError(
            try ExplorerSnapshotLargeFilesAdapter.request(
                minimumLogicalBytes: 1,
                modifiedBefore: ExplorerSnapshotTimestamp(
                    secondsSinceUnixEpoch: 1,
                    nanoseconds: 1_000_000_000
                ),
                maxResults: 100
            )
        )
    }

    private func largeFile(
        id: UInt64,
        name: String,
        logicalBytes: UInt64
    ) -> SnapshotLargeFile {
        SnapshotLargeFile(
            recordVersion: 1,
            node: node(
                id: id,
                parentID: 1,
                depth: 2,
                kind: .file,
                name: name,
                logicalBytes: logicalBytes,
                fileCount: 1
            ),
            parentContext: [SnapshotNodeName(
                encoding: .unixBytes,
                encodedBytes: Data("Downloads".utf8),
                display: "Downloads"
            )],
            contextTruncated: false
        )
    }

    private func node(
        id: UInt64,
        parentID: UInt64?,
        depth: UInt32,
        kind: SnapshotNodeKind,
        name: String,
        category: SnapshotStorageCategory = .unclassified,
        childCount: UInt64 = 0,
        logicalBytes: UInt64 = 10,
        fileCount: UInt64? = nil
    ) -> SnapshotNode {
        node(
            id: id,
            parentID: parentID,
            depth: depth,
            kind: kind,
            nameBytes: Data(name.utf8),
            display: name,
            category: category,
            childCount: childCount,
            logicalBytes: logicalBytes,
            fileCount: fileCount
        )
    }

    private func node(
        id: UInt64,
        parentID: UInt64?,
        depth: UInt32,
        kind: SnapshotNodeKind,
        nameBytes: Data,
        display: String,
        category: SnapshotStorageCategory = .unclassified,
        childCount: UInt64 = 0,
        logicalBytes: UInt64 = 10,
        fileCount: UInt64? = nil,
        encoding: SnapshotNameEncoding = .unixBytes
    ) -> SnapshotNode {
        SnapshotNode(
            recordVersion: 2,
            id: id,
            parentId: parentID,
            depth: depth,
            kind: kind,
            category: category,
            name: SnapshotNodeName(
                encoding: encoding,
                encodedBytes: nameBytes,
                display: display
            ),
            logicalBytes: logicalBytes,
            allocatedBytes: 16,
            fileCount: fileCount ?? (kind == .directory ? 1 : 0),
            childCount: childCount,
            modifiedAt: SnapshotNodeTimestamp(secondsSinceUnixEpoch: 1, nanoseconds: 2),
            accessedAt: nil,
            scanFlags: flags()
        )
    }

    private func flags() -> SnapshotNodeScanFlags {
        SnapshotNodeScanFlags(
            inaccessible: false,
            timedOut: false,
            hardLinkDuplicate: false,
            mountBoundary: false
        )
    }
}
