import XCTest
@testable import DUX

final class ExplorerICloudLocalCopyTests: XCTestCase {
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
