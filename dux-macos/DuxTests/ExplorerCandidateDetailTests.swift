@testable import DUX
import XCTest

final class ExplorerCandidateDetailTests: XCTestCase {
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
}
