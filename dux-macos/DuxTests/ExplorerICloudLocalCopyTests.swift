import XCTest
@testable import DUX

final class ExplorerICloudLocalCopyTests: XCTestCase {
    func testMapsBoundedAllocationRankedPathFreeObservationSource() throws {
        let raw = observationSource(
            maxResults: 3,
            totalRankedFiles: 4,
            hasMore: true,
            targets: [
                observationTarget(rank: 0, id: 10, allocatedBytes: 30, logicalBytes: 10),
                observationTarget(rank: 1, id: 11, allocatedBytes: 20, logicalBytes: 40),
                observationTarget(rank: 2, id: 12, allocatedBytes: 20, logicalBytes: 40),
            ]
        )

        let source = try ExplorerICloudObservationSourceAdapter.map(
            raw,
            expectedScanID: "scan:one",
            expectedScopeNodeID: 7,
            requestedMaxResults: 3
        )

        XCTAssertEqual(source.scanID, "scan:one")
        XCTAssertEqual(source.scopeNodeID, 7)
        XCTAssertEqual(source.visitedNodeCount, 9)
        XCTAssertEqual(source.totalRankedFiles, 4)
        XCTAssertTrue(source.hasMore)
        XCTAssertEqual(source.targets.map(\.rank), [0, 1, 2])
        XCTAssertEqual(source.targets.map(\.id), [10, 11, 12])
        XCTAssertEqual(source.targets.map(\.parentDisplay), [
            "Documents",
            "Documents",
            "Documents",
        ])
    }

    func testObservationSourceRequestEnforcesHardCap() throws {
        let request = try ExplorerICloudObservationSourceAdapter.request(
            scopeNodeID: 0,
            maxResults: 32
        )
        XCTAssertEqual(request.recordVersion, 1)
        XCTAssertEqual(request.scopeNodeId, 0)
        XCTAssertEqual(request.maxResults, 32)

        for maxResults: UInt16 in [0, 33, .max] {
            XCTAssertThrowsError(
                try ExplorerICloudObservationSourceAdapter.request(
                    scopeNodeID: 0,
                    maxResults: maxResults
                )
            ) { error in
                XCTAssertEqual(
                    error as? ExplorerICloudObservationSourceError,
                    .invalidRequest
                )
            }
        }
    }

    func testMapsEmptyDirectoryObservationSourceWithZeroVisitedNodes() throws {
        let source = try ExplorerICloudObservationSourceAdapter.map(
            observationSource(
                visitedNodeCount: 0,
                totalRankedFiles: 0,
                targets: []
            ),
            expectedScanID: "scan:one",
            expectedScopeNodeID: 7,
            requestedMaxResults: 1
        )

        XCTAssertEqual(source.visitedNodeCount, 0)
        XCTAssertEqual(source.totalRankedFiles, 0)
        XCTAssertFalse(source.hasMore)
        XCTAssertTrue(source.targets.isEmpty)
    }

    func testRejectsMalformedObservationSourceEnvelope() {
        let target = observationTarget(rank: 0, id: 10)
        let malformed = [
            observationSource(recordVersion: 2, targets: [target]),
            observationSource(scanID: "scan:other", targets: [target]),
            observationSource(scopeNodeID: 8, targets: [target]),
            observationSource(visitedNodeCount: 200_001, targets: [target]),
            observationSource(
                visitedNodeCount: 1,
                totalRankedFiles: 2,
                hasMore: true,
                targets: [target]
            ),
            observationSource(totalRankedFiles: 2, hasMore: false, targets: [target]),
            observationSource(totalRankedFiles: 1, hasMore: true, targets: [target]),
            observationSource(
                maxResults: 2,
                totalRankedFiles: 2,
                hasMore: false,
                targets: [target]
            ),
        ]

        for raw in malformed {
            assertInvalidSource(raw)
        }
        assertInvalidSource(
            observationSource(maxResults: 2, targets: [target]),
            expectedMaxResults: 1
        )
    }

    func testRejectsMalformedObservationTargetsAndRanking() {
        let first = observationTarget(rank: 0, id: 10, allocatedBytes: 20)
        let malformed = [
            observationSource(targets: [
                observationTarget(rank: 1, id: 10, allocatedBytes: 20),
            ]),
            observationSource(targets: [
                observationTarget(rank: 0, id: 0, allocatedBytes: 20),
            ]),
            observationSource(targets: [
                observationTarget(rank: 0, id: 10, allocatedBytes: nil),
            ]),
            observationSource(targets: [
                observationTarget(rank: 0, id: 10, allocatedBytes: 0),
            ]),
            observationSource(targets: [
                observationTarget(
                    rank: 0,
                    id: 10,
                    allocatedBytes: 20,
                    kind: .directory
                ),
            ]),
            observationSource(targets: [
                observationTarget(
                    rank: 0,
                    id: 10,
                    allocatedBytes: 20,
                    scanFlags: SnapshotNodeScanFlags(
                        inaccessible: false,
                        timedOut: false,
                        hardLinkDuplicate: true,
                        mountBoundary: false
                    )
                ),
            ]),
            observationSource(targets: [
                observationTarget(
                    rank: 0,
                    id: 10,
                    allocatedBytes: 20,
                    parentContext: []
                ),
            ]),
            observationSource(
                maxResults: 2,
                totalRankedFiles: 2,
                targets: [
                    first,
                    observationTarget(rank: 1, id: 10, allocatedBytes: 10),
                ]
            ),
            observationSource(
                maxResults: 2,
                totalRankedFiles: 2,
                targets: [
                    first,
                    observationTarget(rank: 1, id: 11, allocatedBytes: 30),
                ]
            ),
        ]

        for raw in malformed {
            assertInvalidSource(raw)
        }
    }

    func testPresentationUsesObservationOnlyEvictionDisclosure() throws {
        let eligible = try ExplorerICloudLocalCopyAssessmentAdapter.map(assessment())
        XCTAssertEqual(eligible.reviewTitle, "Currently supports review")
        XCTAssertTrue(eligible.reviewDetail.contains("does not authorize cleanup"))
        XCTAssertEqual(eligible.factRows.count, 10)
        XCTAssertEqual(
            ExplorerICloudLocalCopyDisclosure.action,
            "Remove local copy"
        )
        XCTAssertEqual(
            ExplorerICloudLocalCopyDisclosure.retention,
            "Stays in iCloud"
        )
        XCTAssertEqual(
            ExplorerICloudLocalCopyDisclosure.redownload,
            "Requires a network connection to download again"
        )
        XCTAssertTrue(
            ExplorerICloudLocalCopyDisclosure.unavailable
                .contains("No cleanup action is available yet")
        )

        let blocked = try ExplorerICloudLocalCopyAssessmentAdapter.map(assessment(
            uploaded: .unknown,
            eligible: false,
            blockers: [.uploadStateUnknown]
        ))
        XCTAssertEqual(blocked.reviewTitle, "Not currently ready for review")
        XCTAssertTrue(blocked.reviewDetail.contains("failed closed"))
        XCTAssertEqual(
            blocked.blockers.map(\.displayText),
            ["Upload completion is unknown."]
        )
    }

    func testEveryBlockReasonHasNonemptyPresentationCopy() {
        let reasons: [ExplorerICloudLocalCopyBlockReason] = [
            .unsupportedItemKind,
            .ubiquityUnknown,
            .notUbiquitous,
            .uploadStateUnknown,
            .uploadIncomplete,
            .uploadActivityUnknown,
            .uploadInProgress,
            .uploadErrorUnknown,
            .uploadErrorPresent,
            .conflictStateUnknown,
            .unresolvedConflicts,
            .localCopyStateUnknown,
            .staleLocalCopy,
            .noLocalCopy,
            .downloadRequestUnknown,
            .downloadRequested,
            .downloadActivityUnknown,
            .downloadInProgress,
            .downloadErrorUnknown,
            .downloadErrorPresent,
            .syncExclusionUnknown,
            .excludedFromSync,
            .allocationUnknown,
            .noLocalAllocation,
            .invalidObservationTime,
        ]

        XCTAssertEqual(reasons.count, 25)
        XCTAssertTrue(reasons.allSatisfy { !$0.displayText.isEmpty })
        XCTAssertEqual(Set(reasons.map(\.displayText)).count, reasons.count)
    }

    func testMapsEligiblePathFreeAssessment() throws {
        let mapped = try ExplorerICloudLocalCopyAssessmentAdapter.map(assessment())

        XCTAssertEqual(mapped.localAllocatedBytes, 4096)
        XCTAssertEqual(mapped.observedAtUnixMilliseconds, 1_234)
        XCTAssertEqual(mapped.ubiquitous, .yes)
        XCTAssertEqual(mapped.uploaded, .yes)
        XCTAssertEqual(mapped.uploading, .no)
        XCTAssertEqual(mapped.uploadError, .absent)
        XCTAssertEqual(mapped.unresolvedConflicts, .no)
        XCTAssertEqual(mapped.localCopyState, .current)
        XCTAssertEqual(mapped.downloadRequested, .no)
        XCTAssertEqual(mapped.downloading, .no)
        XCTAssertEqual(mapped.downloadError, .absent)
        XCTAssertEqual(mapped.excludedFromSync, .no)
        XCTAssertTrue(mapped.isEligibleObservation)
        XCTAssertTrue(mapped.blockers.isEmpty)
    }

    func testPreservesAllUnknownFactsAndRequiresCoreBlockerOrder() throws {
        let blockers: [ICloudLocalCopyBlockReason] = [
            .ubiquityUnknown,
            .uploadStateUnknown,
            .uploadActivityUnknown,
            .uploadErrorUnknown,
            .conflictStateUnknown,
            .localCopyStateUnknown,
            .downloadRequestUnknown,
            .downloadActivityUnknown,
            .downloadErrorUnknown,
            .syncExclusionUnknown,
        ]
        let mapped = try ExplorerICloudLocalCopyAssessmentAdapter.map(assessment(
            ubiquitous: .unknown,
            uploaded: .unknown,
            uploading: .unknown,
            uploadError: .unknown,
            unresolvedConflicts: .unknown,
            localCopyState: .unknown,
            downloadRequested: .unknown,
            downloading: .unknown,
            downloadError: .unknown,
            excludedFromSync: .unknown,
            eligible: false,
            blockers: blockers
        ))

        XCTAssertEqual(mapped.ubiquitous, .unknown)
        XCTAssertEqual(mapped.localCopyState, .unknown)
        XCTAssertEqual(mapped.blockers, [
            .ubiquityUnknown,
            .uploadStateUnknown,
            .uploadActivityUnknown,
            .uploadErrorUnknown,
            .conflictStateUnknown,
            .localCopyStateUnknown,
            .downloadRequestUnknown,
            .downloadActivityUnknown,
            .downloadErrorUnknown,
            .syncExclusionUnknown,
        ])
    }

    func testRejectsContradictoryEligibilityAndBlockers() {
        assertInvalid(assessment(eligible: false))
        assertInvalid(assessment(blockers: [.uploadIncomplete]))
        assertInvalid(assessment(
            uploaded: .`false`,
            eligible: false,
            blockers: [.uploadStateUnknown]
        ))
        assertInvalid(assessment(
            uploaded: .`false`,
            eligible: false,
            blockers: [.uploadIncomplete, .uploadIncomplete]
        ))
        assertInvalid(assessment(
            uploaded: .`false`,
            eligible: false,
            blockers: [.unsupportedItemKind]
        ))
    }

    func testRejectsMalformedCoreOwnedEnvelope() {
        assertInvalid(assessment(recordVersion: 2))
        assertInvalid(assessment(localAllocatedBytes: 0))
        assertInvalid(assessment(observedAtUnixMs: -1))
        assertInvalid(assessment(
            eligible: false,
            blockers: [.allocationUnknown]
        ))
        assertInvalid(assessment(
            eligible: false,
            blockers: [.invalidObservationTime]
        ))
    }

    private func assertInvalid(
        _ raw: ICloudLocalCopyAssessment,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertThrowsError(
            try ExplorerICloudLocalCopyAssessmentAdapter.map(raw),
            file: file,
            line: line
        ) { error in
            XCTAssertEqual(
                error as? ExplorerICloudLocalCopyProbeError,
                .invalidResponse,
                file: file,
                line: line
            )
        }
    }

    private func assertInvalidSource(
        _ raw: SnapshotICloudObservationSource,
        expectedMaxResults: UInt16? = nil,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertThrowsError(
            try ExplorerICloudObservationSourceAdapter.map(
                raw,
                expectedScanID: "scan:one",
                expectedScopeNodeID: 7,
                requestedMaxResults: expectedMaxResults ?? raw.requestedMaxResults
            ),
            file: file,
            line: line
        ) { error in
            XCTAssertEqual(
                error as? ExplorerICloudObservationSourceError,
                .invalidResponse,
                file: file,
                line: line
            )
        }
    }

    private func observationSource(
        recordVersion: UInt32 = 1,
        scanID: String = "scan:one",
        scopeNodeID: UInt64 = 7,
        maxResults: UInt16 = 1,
        visitedNodeCount: UInt64 = 9,
        totalRankedFiles: UInt64 = 1,
        hasMore: Bool = false,
        targets: [SnapshotICloudObservationTarget]
    ) -> SnapshotICloudObservationSource {
        SnapshotICloudObservationSource(
            recordVersion: recordVersion,
            scanId: scanID,
            scopeNodeId: scopeNodeID,
            requestedMaxResults: maxResults,
            visitedNodeCount: visitedNodeCount,
            totalRankedFiles: totalRankedFiles,
            hasMore: hasMore,
            targets: targets
        )
    }

    private func observationTarget(
        rank: UInt16,
        id: UInt64,
        allocatedBytes: UInt64? = 16,
        logicalBytes: UInt64 = 10,
        kind: SnapshotNodeKind = .file,
        parentContext: [SnapshotNodeName]? = nil,
        scanFlags: SnapshotNodeScanFlags? = nil
    ) -> SnapshotICloudObservationTarget {
        SnapshotICloudObservationTarget(
            recordVersion: 1,
            rank: rank,
            node: SnapshotNode(
                recordVersion: 2,
                id: id,
                parentId: 1,
                depth: 2,
                kind: kind,
                category: .unclassified,
                name: SnapshotNodeName(
                    encoding: .unixBytes,
                    encodedBytes: Data("file-\(id)".utf8),
                    display: "file-\(id)"
                ),
                logicalBytes: logicalBytes,
                allocatedBytes: allocatedBytes,
                fileCount: 1,
                childCount: 0,
                modifiedAt: nil,
                accessedAt: nil,
                scanFlags: scanFlags ?? SnapshotNodeScanFlags(
                    inaccessible: false,
                    timedOut: false,
                    hardLinkDuplicate: false,
                    mountBoundary: false
                )
            ),
            parentContext: parentContext ?? [
                SnapshotNodeName(
                    encoding: .unixBytes,
                    encodedBytes: Data("Documents".utf8),
                    display: "Documents"
                ),
            ],
            contextTruncated: false
        )
    }

    private func assessment(
        recordVersion: UInt32 = 1,
        localAllocatedBytes: UInt64 = 4096,
        observedAtUnixMs: Int64 = 1_234,
        ubiquitous: ICloudBooleanState = .`true`,
        uploaded: ICloudBooleanState = .`true`,
        uploading: ICloudBooleanState = .`false`,
        uploadError: ICloudErrorState = .absent,
        unresolvedConflicts: ICloudBooleanState = .`false`,
        localCopyState: ICloudLocalCopyState = .current,
        downloadRequested: ICloudBooleanState = .`false`,
        downloading: ICloudBooleanState = .`false`,
        downloadError: ICloudErrorState = .absent,
        excludedFromSync: ICloudBooleanState = .`false`,
        eligible: Bool = true,
        blockers: [ICloudLocalCopyBlockReason] = []
    ) -> ICloudLocalCopyAssessment {
        ICloudLocalCopyAssessment(
            recordVersion: recordVersion,
            provider: .iCloudDrive,
            itemKind: .regularFile,
            localAllocatedBytes: localAllocatedBytes,
            observedAtUnixMs: observedAtUnixMs,
            ubiquitous: ubiquitous,
            uploaded: uploaded,
            uploading: uploading,
            uploadError: uploadError,
            unresolvedConflicts: unresolvedConflicts,
            localCopyState: localCopyState,
            downloadRequested: downloadRequested,
            downloading: downloading,
            downloadError: downloadError,
            excludedFromSync: excludedFromSync,
            isEligibleObservation: eligible,
            blockers: blockers
        )
    }
}
