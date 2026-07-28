@testable import DUX
import XCTest

final class ExplorerCandidateDetailTests: XCTestCase {
    func testRustTargetPlanReviewMapsExactCurrentPermanentSafeObservation() throws {
        let record = rustTargetPlanReviewRecord()

        let mapped = try ExplorerRustTargetPlanReviewAdapter.map(
            record,
            expectedScanID: "scan:example",
            expectedCandidateID: "candidate:example",
            now: ExplorerSnapshotTimestamp(
                secondsSinceUnixEpoch: 1_700_000_100,
                nanoseconds: 0
            )
        )

        XCTAssertEqual(mapped.planID, "plan:example")
        XCTAssertEqual(mapped.category, .developerArtifact)
        XCTAssertEqual(mapped.itemCount, 1)
        XCTAssertEqual(mapped.pathCount, 1)
        XCTAssertEqual(mapped.target.display, "/Users/example/project/target")
    }

    func testRustTargetPlanReviewRejectsCrossCandidateAndOversizedLifetime() {
        let record = rustTargetPlanReviewRecord(
            effectiveExpiresAt: ExplorerSnapshotTimestamp(
                secondsSinceUnixEpoch: 1_700_000_600,
                nanoseconds: 1
            )
        )

        XCTAssertThrowsError(
            try ExplorerRustTargetPlanReviewAdapter.map(
                record,
                expectedScanID: "scan:example",
                expectedCandidateID: "candidate:other",
                now: ExplorerSnapshotTimestamp(
                    secondsSinceUnixEpoch: 1_700_000_100,
                    nanoseconds: 0
                )
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerRustTargetPlanReviewError, .invalidResponse)
        }
        XCTAssertThrowsError(
            try ExplorerRustTargetPlanReviewAdapter.map(
                record,
                expectedScanID: "scan:example",
                expectedCandidateID: "candidate:example",
                now: ExplorerSnapshotTimestamp(
                    secondsSinceUnixEpoch: 1_700_000_100,
                    nanoseconds: 0
                )
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerRustTargetPlanReviewError, .invalidResponse)
        }
    }

    func testRustTargetPlanReviewRejectsNonUnixAndNulTargetBytes() {
        for target in [
            ExplorerRustTargetPlanReviewPath(
                encoding: .windowsUTF16LittleEndian,
                encodedBytes: Data("/Users/example/target".utf8),
                display: "/Users/example/target"
            ),
            ExplorerRustTargetPlanReviewPath(
                encoding: .unixBytes,
                encodedBytes: Data([0x2f, 0x74, 0]),
                display: "/t"
            ),
            ExplorerRustTargetPlanReviewPath(
                encoding: .unixBytes,
                encodedBytes: Data("/Users/example/project/target".utf8),
                display: "/Users/example/project/\ntarget"
            ),
        ] {
            XCTAssertThrowsError(
                try ExplorerRustTargetPlanReviewAdapter.map(
                    rustTargetPlanReviewRecord(target: target),
                    expectedScanID: "scan:example",
                    expectedCandidateID: "candidate:example",
                    now: ExplorerSnapshotTimestamp(
                        secondsSinceUnixEpoch: 1_700_000_100,
                        nanoseconds: 0
                    )
                )
            ) { error in
                XCTAssertEqual(
                    error as? ExplorerRustTargetPlanReviewError,
                    .invalidResponse
                )
            }
        }
    }

    func testRustTargetPlanReviewRejectsFutureCreationTime() {
        XCTAssertThrowsError(
            try ExplorerRustTargetPlanReviewAdapter.map(
                rustTargetPlanReviewRecord(
                    createdAt: ExplorerSnapshotTimestamp(
                        secondsSinceUnixEpoch: 1_700_000_101,
                        nanoseconds: 0
                    )
                ),
                expectedScanID: "scan:example",
                expectedCandidateID: "candidate:example",
                now: ExplorerSnapshotTimestamp(
                    secondsSinceUnixEpoch: 1_700_000_100,
                    nanoseconds: 0
                )
            )
        ) { error in
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .invalidResponse
            )
        }
    }

    func testRustTargetPlanReviewAcceptsLosslessEscapedUnixDisplay() throws {
        let bytes = Data([
            0x2f, 0x74, 0x6d, 0x70, 0x2f, 0xff, 0x5c, 0x0a,
            0x2f, 0x74, 0x61, 0x72, 0x67, 0x65, 0x74,
        ])
        let mapped = try ExplorerRustTargetPlanReviewAdapter.map(
            rustTargetPlanReviewRecord(
                target: ExplorerRustTargetPlanReviewPath(
                    encoding: .unixBytes,
                    encodedBytes: bytes,
                    display: "unix-bytes:/tmp/\\xff\\\\\\x0a/target"
                )
            ),
            expectedScanID: "scan:example",
            expectedCandidateID: "candidate:example",
            now: ExplorerSnapshotTimestamp(
                secondsSinceUnixEpoch: 1_700_000_100,
                nanoseconds: 0
            )
        )

        XCTAssertEqual(mapped.target.encodedBytes, bytes)
        XCTAssertEqual(mapped.target.encoding, .unixBytes)
        XCTAssertEqual(mapped.target.display, "unix-bytes:/tmp/\\xff\\\\\\x0a/target")
    }

    func testRustTargetPlanReviewRejectsDisplayThatDoesNotMatchExactBytes() {
        XCTAssertThrowsError(
            try ExplorerRustTargetPlanReviewAdapter.map(
                rustTargetPlanReviewRecord(
                    target: ExplorerRustTargetPlanReviewPath(
                        encoding: .unixBytes,
                        encodedBytes: Data("/Users/example/project/target".utf8),
                        display: "/Users/example/another/target"
                    )
                ),
                expectedScanID: "scan:example",
                expectedCandidateID: "candidate:example",
                now: ExplorerSnapshotTimestamp(
                    secondsSinceUnixEpoch: 1_700_000_100,
                    nanoseconds: 0
                )
            )
        ) { error in
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .invalidResponse
            )
        }
    }

    func testRustTargetPlanReviewRejectsCanonicallyEquivalentDisplayBytes() {
        let decomposed = "/tmp/cafe\u{301}/target"
        let precomposed = "/tmp/caf\u{e9}/target"
        XCTAssertEqual(decomposed, precomposed)
        XCTAssertNotEqual(Data(decomposed.utf8), Data(precomposed.utf8))

        XCTAssertThrowsError(
            try ExplorerRustTargetPlanReviewAdapter.map(
                rustTargetPlanReviewRecord(
                    target: ExplorerRustTargetPlanReviewPath(
                        encoding: .unixBytes,
                        encodedBytes: Data(decomposed.utf8),
                        display: precomposed
                    )
                ),
                expectedScanID: "scan:example",
                expectedCandidateID: "candidate:example",
                now: ExplorerSnapshotTimestamp(
                    secondsSinceUnixEpoch: 1_700_000_100,
                    nanoseconds: 0
                )
            )
        ) { error in
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .invalidResponse
            )
        }
    }

    func testRustTargetPlanReviewMatchesEscapedBidiDisplayParity() throws {
        let bytes = Data("/tmp/\u{202e}/target".utf8)
        let mapped = try ExplorerRustTargetPlanReviewAdapter.map(
            rustTargetPlanReviewRecord(
                target: ExplorerRustTargetPlanReviewPath(
                    encoding: .unixBytes,
                    encodedBytes: bytes,
                    display: "unix-bytes:/tmp/\\xe2\\x80\\xae/target"
                )
            ),
            expectedScanID: "scan:example",
            expectedCandidateID: "candidate:example",
            now: ExplorerSnapshotTimestamp(
                secondsSinceUnixEpoch: 1_700_000_100,
                nanoseconds: 0
            )
        )

        XCTAssertEqual(
            mapped.target.display,
            "unix-bytes:/tmp/\\xe2\\x80\\xae/target"
        )
    }

    func testRustTargetPlanReviewMatchesEscapedArabicLetterMarkParity() throws {
        let bytes = Data("/tmp/\u{061c}/target".utf8)
        let mapped = try ExplorerRustTargetPlanReviewAdapter.map(
            rustTargetPlanReviewRecord(
                target: ExplorerRustTargetPlanReviewPath(
                    encoding: .unixBytes,
                    encodedBytes: bytes,
                    display: "unix-bytes:/tmp/\\xd8\\x9c/target"
                )
            ),
            expectedScanID: "scan:example",
            expectedCandidateID: "candidate:example",
            now: ExplorerSnapshotTimestamp(
                secondsSinceUnixEpoch: 1_700_000_100,
                nanoseconds: 0
            )
        )

        XCTAssertEqual(
            mapped.target.display,
            "unix-bytes:/tmp/\\xd8\\x9c/target"
        )
    }

    func testRustTargetPlanReviewMatchesEscapedDefaultIgnorableParity() throws {
        for (scalar, escapedBytes) in [
            ("\u{180f}", "\\xe1\\xa0\\x8f"),
            ("\u{fe0f}", "\\xef\\xb8\\x8f"),
        ] {
            let bytes = Data("/tmp/\(scalar)/target".utf8)
            let expectedDisplay = "unix-bytes:/tmp/\(escapedBytes)/target"
            let mapped = try ExplorerRustTargetPlanReviewAdapter.map(
                rustTargetPlanReviewRecord(
                    target: ExplorerRustTargetPlanReviewPath(
                        encoding: .unixBytes,
                        encodedBytes: bytes,
                        display: expectedDisplay
                    )
                ),
                expectedScanID: "scan:example",
                expectedCandidateID: "candidate:example",
                now: ExplorerSnapshotTimestamp(
                    secondsSinceUnixEpoch: 1_700_000_100,
                    nanoseconds: 0
                )
            )

            XCTAssertEqual(mapped.target.display, expectedDisplay)
        }
    }

    func testRustTargetPlanReviewRejectsNonNormalizedOrNonTargetPath() {
        for display in [
            "/Users/example//target",
            "/Users/example/./target",
            "/Users/example/../target",
            "/Users/example/target/",
            "/Users/example/build",
        ] {
            XCTAssertThrowsError(
                try ExplorerRustTargetPlanReviewAdapter.map(
                    rustTargetPlanReviewRecord(
                        target: ExplorerRustTargetPlanReviewPath(
                            encoding: .unixBytes,
                            encodedBytes: Data(display.utf8),
                            display: display
                        )
                    ),
                    expectedScanID: "scan:example",
                    expectedCandidateID: "candidate:example",
                    now: ExplorerSnapshotTimestamp(
                        secondsSinceUnixEpoch: 1_700_000_100,
                        nanoseconds: 0
                    )
                )
            ) { error in
                XCTAssertEqual(
                    error as? ExplorerRustTargetPlanReviewError,
                    .invalidResponse
                )
            }
        }
    }

    func testReviewIntentMapsOnlyTheBoundCandidateAndScan() throws {
        let raw = CandidateReviewResult(
            recordVersion: 1,
            scanId: "scan:example",
            candidateId: "candidate:example",
            status: .selected
        )

        let mapped = try ExplorerCandidateDetailAdapter.mapReviewResult(
            raw,
            expectedScanID: "scan:example",
            expectedCandidateID: "candidate:example"
        )
        XCTAssertEqual(
            mapped,
            ExplorerCandidateReviewResult(
                scanID: "scan:example",
                candidateID: "candidate:example",
                status: .selected
            )
        )
        XCTAssertEqual(
            ExplorerCandidateDetailAdapter.ffiCommand(.dismiss),
            .dismiss
        )

        XCTAssertThrowsError(
            try ExplorerCandidateDetailAdapter.mapReviewResult(
                raw,
                expectedScanID: "scan:other",
                expectedCandidateID: "candidate:example"
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerCandidateDetailError, .invalidResponse)
        }
    }

    func testPathPageMapsBoundedHistoricalCandidateDisclosure() throws {
        let timestamp = SnapshotNodeTimestamp(secondsSinceUnixEpoch: 1_700_000_000, nanoseconds: 0)
        let path = CandidateObservedPath(
            encoding: .utf8,
            encodedBytes: Data("/Users/example/Library/Caches/project".utf8),
            display: "/Users/example/Library/Caches/project"
        )
        let summary = CandidateSummary(
            recordVersion: 1,
            candidateId: "candidate:example",
            ruleId: "developer.rust.target",
            ruleRevision: 2,
            category: .developerArtifact,
            estimatedBytes: 42,
            newestMtime: timestamp,
            safety: .safeRegenerable,
            action: .removeKnownRegenerableContents,
            ruleScheduleEligible: false,
            pathCount: 1,
            evidenceKinds: [.matchedPath],
            blockers: [.protectedPath],
            createdAt: timestamp,
            status: .discovered
        )
        let page = CandidatePathPage(
            recordVersion: 1,
            scanId: "scan:example",
            candidate: summary,
            cursor: 0,
            nextCursor: nil,
            totalPaths: 1,
            paths: [path]
        )

        let mapped = try ExplorerCandidateDetailAdapter.mapPaths(
            page,
            expectedScanID: "scan:example",
            expectedCandidateID: "candidate:example",
            expectedCursor: 0,
            requestedLimit: 64
        )

        XCTAssertEqual(mapped.scanID, "scan:example")
        XCTAssertEqual(mapped.candidate.category, .developerArtifact)
        XCTAssertEqual(mapped.paths.first?.encodedBytes, path.encodedBytes)
        XCTAssertEqual(mapped.candidate.blockers, [.protectedPath])

        let oversizedPathPage = CandidatePathPage(
            recordVersion: 1,
            scanId: "scan:example",
            candidate: summary,
            cursor: 0,
            nextCursor: nil,
            totalPaths: 1,
            paths: [
                CandidateObservedPath(
                    encoding: .utf8,
                    encodedBytes: Data(repeating: 0x61, count: 65_537),
                    display: "/Users/example/target"
                ),
            ]
        )
        XCTAssertThrowsError(
            try ExplorerCandidateDetailAdapter.mapPaths(
                oversizedPathPage,
                expectedScanID: "scan:example",
                expectedCandidateID: "candidate:example",
                expectedCursor: 0,
                requestedLimit: 64
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerCandidateDetailError, .invalidResponse)
        }

        let summaries = CandidateSummaryPage(
            recordVersion: 1,
            scanId: "scan:example",
            cursor: 0,
            nextCursor: nil,
            totalCandidates: 1,
            candidates: [summary]
        )
        let mappedSummaries = try ExplorerCandidateDetailAdapter.mapSummaries(
            summaries,
            expectedScanID: "scan:example",
            expectedCursor: 0,
            requestedLimit: 64
        )
        XCTAssertEqual(mappedSummaries.totalCandidates, 1)
        XCTAssertEqual(mappedSummaries.candidates.map(\.candidateID), ["candidate:example"])
    }

    func testEvidencePageRejectsContradictoryOptionalFields() {
        let timestamp = SnapshotNodeTimestamp(secondsSinceUnixEpoch: 1_700_000_000, nanoseconds: 0)
        let summary = CandidateSummary(
            recordVersion: 1,
            candidateId: "candidate:example",
            ruleId: "developer.rust.target",
            ruleRevision: 2,
            category: .developerArtifact,
            estimatedBytes: 42,
            newestMtime: nil,
            safety: .safeRegenerable,
            action: .removeKnownRegenerableContents,
            ruleScheduleEligible: false,
            pathCount: 1,
            evidenceKinds: [.matchedPath],
            blockers: [],
            createdAt: timestamp,
            status: .discovered
        )
        let page = CandidateEvidencePage(
            recordVersion: 1,
            scanId: "scan:example",
            candidate: summary,
            cursor: 0,
            nextCursor: nil,
            totalEvidence: 1,
            evidence: [
                CandidateEvidenceRecord(
                    recordVersion: 1,
                    ordinal: 0,
                    kind: .matchedPath,
                    path: nil,
                    identifier: "unexpected",
                    newestMtime: nil,
                    minimumAgeSeconds: nil,
                    minimumAgeNanoseconds: nil,
                    observedBytes: nil,
                    minimumBytes: nil
                ),
            ]
        )

        XCTAssertThrowsError(
            try ExplorerCandidateDetailAdapter.mapEvidence(
                page,
                expectedScanID: "scan:example",
                expectedCandidateID: "candidate:example",
                expectedCursor: 0,
                requestedLimit: 64
            )
        ) { error in
            XCTAssertEqual(error as? ExplorerCandidateDetailError, .invalidResponse)
        }
    }

    private func rustTargetPlanReviewRecord(
        createdAt: ExplorerSnapshotTimestamp = ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: 1_700_000_000,
            nanoseconds: 0
        ),
        effectiveExpiresAt: ExplorerSnapshotTimestamp = ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: 1_700_000_600,
            nanoseconds: 0
        ),
        target: ExplorerRustTargetPlanReviewPath = ExplorerRustTargetPlanReviewPath(
            encoding: .unixBytes,
            encodedBytes: Data("/Users/example/project/target".utf8),
            display: "/Users/example/project/target"
        )
    ) -> ExplorerRustTargetPlanReviewRecord {
        ExplorerRustTargetPlanReviewRecord(
            recordVersion: 1,
            planID: "plan:example",
            sourceScanID: "scan:example",
            candidateID: "candidate:example",
            ruleID: "developer.rust.target",
            ruleRevision: 2,
            category: .developerArtifact,
            mode: .permanentSafe,
            safety: .safeRegenerable,
            action: .removeKnownRegenerableContents,
            estimatedBytes: 42,
            itemCount: 1,
            pathCount: 1,
            warnings: [
                .estimatedBytesUnverified,
                .permanentRemovalCannotBeUndone,
            ],
            createdAt: createdAt,
            effectiveExpiresAt: effectiveExpiresAt,
            scheduleEligible: false,
            target: target
        )
    }
}
