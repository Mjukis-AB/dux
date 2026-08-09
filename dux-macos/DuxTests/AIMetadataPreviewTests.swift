import Foundation
import XCTest
@testable import DUX

final class AIMetadataPreviewTests: XCTestCase {
    func testEngineServiceUsesExactParentAndBoundedRequestThenDropsParent() async throws {
        var rawParent: StubAIMetadataSnapshotReviewSession? = StubAIMetadataSnapshotReviewSession()
        weak let weakParent = rawParent
        let rawPreview = StubAIMetadataPreviewSession(
            infoResult: .success(validAIMetadataPreview())
        )
        let engine = StubAIMetadataPreviewEngine(
            nextParent: rawParent!,
            previewResult: .success(rawPreview)
        )
        let service = EngineService(engine: engine)
        var parent: (any DuxSnapshotReviewLease)? = try await service.acquireExplorerReview(
            scanID: "scan-ai-preview"
        )
        rawParent = nil

        let preview = try await parent!.prepareAIMetadataPreview(nodeID: 42)
        XCTAssertTrue(engine.preparedParent === weakParent)
        XCTAssertEqual(
            engine.preparedRequest,
            AiMetadataPreviewRequest(recordVersion: 1, selectedNodeId: 42)
        )
        let reread = try await preview.readInfo()
        XCTAssertEqual(preview.preview, reread)
        XCTAssertEqual(rawPreview.infoCallCount, 2)

        parent = nil
        XCTAssertNotNil(weakParent)
        await preview.release()
        await preview.release()
        XCTAssertNil(weakParent)
        XCTAssertEqual(rawPreview.releaseCallCount, 1)
        do {
            _ = try await preview.readInfo()
            XCTFail("released preview was readable")
        } catch {
            XCTAssertEqual(error as? ExplorerAIMetadataPreviewError, .previewUnavailable)
        }
        XCTAssertEqual(rawPreview.infoCallCount, 2)
    }

    func testEngineServiceRollsBackMalformedInitialInfoExactlyOnce() async throws {
        let rawParent = StubAIMetadataSnapshotReviewSession()
        let rawPreview = StubAIMetadataPreviewSession(
            infoResult: .success(validAIMetadataPreview(rootLabel: "Source folder"))
        )
        let engine = StubAIMetadataPreviewEngine(
            nextParent: rawParent,
            previewResult: .success(rawPreview)
        )
        let service = EngineService(engine: engine)
        let parent = try await service.acquireExplorerReview(scanID: "scan-ai-preview")

        do {
            _ = try await parent.prepareAIMetadataPreview(nodeID: 7)
            XCTFail("malformed preview was published")
        } catch {
            XCTAssertEqual(error as? ExplorerAIMetadataPreviewError, .invalidResponse)
        }
        XCTAssertTrue(engine.preparedParent === rawParent)
        XCTAssertEqual(
            engine.preparedRequest,
            AiMetadataPreviewRequest(recordVersion: 1, selectedNodeId: 7)
        )
        XCTAssertEqual(rawPreview.infoCallCount, 1)
        XCTAssertEqual(rawPreview.releaseCallCount, 1)
    }

    func testValidPreviewMapsExactPathFreeDisclosure() throws {
        let raw = validAIMetadataPreview()
        let preview = try ExplorerAIMetadataPreviewAdapter.map(raw)

        XCTAssertEqual(preview.inputSchemaVersion, 1)
        XCTAssertEqual(preview.privacyPolicyRevision, 1)
        XCTAssertEqual(preview.expiresAtUnixMilliseconds - preview.preparedAtUnixMilliseconds, 120_000)
        XCTAssertEqual(preview.inputDigestSHA256, String(repeating: "a", count: 64))
        XCTAssertEqual(preview.rootLabel, "Selected folder")
        XCTAssertEqual(preview.totalLogicalBytes, 10)
        XCTAssertEqual(preview.inspectedNodeCount, 3)
        XCTAssertEqual(preview.includedDirectChildCount, 1)
        XCTAssertEqual(preview.excludedSensitiveDirectChildCount, 1)
        XCTAssertEqual(preview.omittedEligibleDirectChildCount, 0)
        XCTAssertTrue(preview.childrenComplete)
        XCTAssertEqual(
            preview.children,
            [
                ExplorerAIMetadataPreviewChild(
                    inputNodeID: "n-1",
                    label: "File 1",
                    kind: .file,
                    logicalBytes: 10,
                    ageSummary: nativePreviewAge(within7Days: 10)
                ),
            ]
        )
        XCTAssertEqual(preview.encodedInputJSONUTF8, raw.encodedInputJsonUtf8)
        for forbidden in [
            "/Users/example", "private-name", "scan-id", "provider", "candidate", "cleanup",
        ] {
            XCTAssertFalse(preview.encodedInputJSON.contains(forbidden))
        }
    }

    func testRecordVersionsTimingDigestFlagsAndBudgetsAreStrict() {
        let unsafe = ExplorerAIMetadataPreviewAdapter.maximumExactJSONInteger + 1
        let cases = [
            validAIMetadataPreview(recordVersion: 2),
            validAIMetadataPreview(inputSchemaVersion: 2),
            validAIMetadataPreview(privacyPolicyRevision: 2),
            validAIMetadataPreview(preparedAtUnixMs: -1),
            validAIMetadataPreview(preparedAtUnixMs: 1_000, expiresAtUnixMs: 1_000),
            validAIMetadataPreview(preparedAtUnixMs: 1_000, expiresAtUnixMs: 121_001),
            validAIMetadataPreview(inputDigestSha256: String(repeating: "A", count: 64)),
            validAIMetadataPreview(inputDigestSha256: String(repeating: "a", count: 63)),
            validAIMetadataPreview(contentIncluded: true),
            validAIMetadataPreview(sourceNamesIncluded: true),
            validAIMetadataPreview(sourcePathsIncluded: true),
            validAIMetadataPreview(rootLabel: "Source folder"),
            validAIMetadataPreview(inspectedNodeCount: 200_001),
            validAIMetadataPreview(totalLogicalBytes: unsafe),
        ]

        for raw in cases {
            assertInvalid(raw)
        }
    }

    func testDisclosureOmissionAndCheckedAgeAccountingAreExact() {
        let wrongAge = generatedPreviewAge(within7Days: 9)
        let wrongChild = previewChild(logicalBytes: 10, ageSummary: wrongAge)
        let tooManyChildren = (1 ... 129).map { index in
            previewChild(
                inputNodeID: "n-\(index)",
                label: "File \(index)",
                logicalBytes: 0,
                ageSummary: generatedPreviewAge()
            )
        }
        let cases = [
            validAIMetadataPreview(includedDirectChildCount: 2),
            validAIMetadataPreview(omittedEligibleDirectChildCount: 1),
            validAIMetadataPreview(inspectedNodeCount: 2),
            validAIMetadataPreview(childrenComplete: false),
            validAIMetadataPreview(omittedChildCount: 1),
            validAIMetadataPreview(omittedLogicalBytes: 1),
            validAIMetadataPreview(ageSummary: generatedPreviewAge(within7Days: 9)),
            validAIMetadataPreview(children: [wrongChild]),
            validAIMetadataPreview(
                children: [previewChild(inputNodeID: "n-2")]
            ),
            validAIMetadataPreview(
                children: [previewChild(label: "private-name")]
            ),
            validAIMetadataPreview(
                includedDirectChildCount: 129,
                totalLogicalBytes: 0,
                ageSummary: generatedPreviewAge(),
                children: tooManyChildren
            ),
        ]

        for raw in cases {
            assertInvalid(raw)
        }
    }

    func testExactJSONEnvelopeMustMatchProjection() {
        let malformed = AiMetadataPreviewInfo(
            recordVersion: 1,
            inputSchemaVersion: 1,
            privacyPolicyRevision: 1,
            preparedAtUnixMs: 1_000,
            expiresAtUnixMs: 121_000,
            inputDigestSha256: String(repeating: "a", count: 64),
            encodedInputJsonUtf8: Data([0xFF]),
            inspectedNodeCount: 3,
            includedDirectChildCount: 1,
            excludedSensitiveDirectChildCount: 1,
            omittedEligibleDirectChildCount: 0,
            rootLabel: "Selected folder",
            totalLogicalBytes: 10,
            ageSummary: generatedPreviewAge(within7Days: 10),
            childrenComplete: true,
            omittedChildCount: 0,
            omittedLogicalBytes: 0,
            omittedAgeSummary: generatedPreviewAge(),
            children: [previewChild()],
            contentIncluded: false,
            sourceNamesIncluded: false,
            sourcePathsIncluded: false
        )
        assertInvalid(malformed)

        let mutations: [(inout [String: Any]) -> Void] = [
            { $0["unknown"] = 1 },
            { $0["schema_version"] = 2 },
            { $0["task"] = "delete_storage" },
            { $0["input_digest_sha256"] = String(repeating: "b", count: 64) },
            { ($0["metadata"] as? NSMutableDictionary)?["root_label"] = "Source folder" },
            { ($0["metadata"] as? NSMutableDictionary)?["coverage"] = "partial" },
            { ($0["metadata"] as? NSMutableDictionary)?["protected"] = true },
            { ($0["metadata"] as? NSMutableDictionary)?["content_included"] = true },
            {
                ($0["metadata"] as? NSMutableDictionary)?["known_classifications"] = [
                    ["input_node_id": "n-1", "classification_id": "x", "label": "x"],
                ]
            },
            {
                guard
                    let metadata = $0["metadata"] as? NSMutableDictionary,
                    let children = metadata["children"] as? NSMutableArray,
                    let child = children.firstObject as? NSMutableDictionary
                else { return }
                child["coverage"] = "partial"
            },
        ]

        for mutation in mutations {
            assertInvalid(validAIMetadataPreview(mutateJSON: mutation))
        }
    }

    func testEveryGeneratedFailureMapsWithoutGrantingAuthority() {
        let mappings: [(AiMetadataPreviewError, ExplorerAIMetadataPreviewError)] = [
            (.Closed, .closed),
            (.InvalidRecordVersion, .invalidResponse),
            (.WrongReview, .wrongReview),
            (.ReviewUnavailable, .reviewUnavailable),
            (.IncompleteCoverage, .incompleteCoverage),
            (.SelectionUnavailable, .selectionUnavailable),
            (.SelectionNotDirectory, .selectionNotDirectory),
            (.SensitiveSelection, .sensitiveSelection),
            (.UnsupportedObservation, .unsupportedObservation),
            (.BudgetExceeded, .budgetExceeded),
            (.InvalidClock, .invalidClock),
            (.UnsafeStorage, .unsafeStorage),
            (.CorruptData, .corruptData),
            (.Busy, .busy),
            (.PreviewUnavailable, .previewUnavailable),
            (.Unavailable, .unavailable),
            (.InternalState, .invalidResponse),
        ]
        for (raw, expected) in mappings {
            XCTAssertEqual(ExplorerAIMetadataPreviewAdapter.map(raw), expected)
        }

        let labels = Set(Mirror(reflecting: try! ExplorerAIMetadataPreviewAdapter.map(
            validAIMetadataPreview()
        )).children.compactMap(\.label))
        XCTAssertFalse(labels.contains("path"))
        XCTAssertFalse(labels.contains("provider"))
        XCTAssertFalse(labels.contains("candidate"))
        XCTAssertFalse(labels.contains("approval"))
        XCTAssertFalse(labels.contains("cleanup"))
    }

    func testLifecycleReleasesExactlyOnceAndDropsParent() {
        let session = StubAIMetadataPreviewSession(infoResult: .success(validAIMetadataPreview()))
        var parent: NSObject? = NSObject()
        weak let weakParent = parent
        let lifecycle = AIMetadataPreviewLeaseLifecycle(session: session, parent: parent!)
        parent = nil

        XCTAssertNotNil(weakParent)
        XCTAssertNoThrow(try lifecycle.readInfo())
        lifecycle.release()
        lifecycle.release()

        XCTAssertNil(weakParent)
        XCTAssertEqual(session.releaseCallCount, 1)
        XCTAssertThrowsError(try lifecycle.readInfo()) { error in
            XCTAssertEqual(error as? AiMetadataPreviewError, .PreviewUnavailable)
        }
        XCTAssertEqual(session.infoCallCount, 1)
    }

    func testTerminalReadFailureReleasesOnceAndDropsParent() {
        let errors: [AiMetadataPreviewError] = [
            .Closed, .InvalidRecordVersion, .WrongReview, .ReviewUnavailable,
            .IncompleteCoverage, .SelectionUnavailable, .SelectionNotDirectory,
            .SensitiveSelection, .UnsupportedObservation, .BudgetExceeded,
            .InvalidClock, .UnsafeStorage, .CorruptData, .Busy,
            .PreviewUnavailable, .Unavailable, .InternalState,
        ]
        for expectedError in errors {
            let session = StubAIMetadataPreviewSession(infoResult: .failure(expectedError))
            var parent: NSObject? = NSObject()
            weak let weakParent = parent
            let lifecycle = AIMetadataPreviewLeaseLifecycle(session: session, parent: parent!)
            parent = nil

            XCTAssertThrowsError(try lifecycle.readInfo()) { error in
                XCTAssertEqual(error as? AiMetadataPreviewError, expectedError)
            }
            XCTAssertNil(weakParent)
            XCTAssertEqual(session.releaseCallCount, 1)
            lifecycle.release()
            XCTAssertEqual(session.releaseCallCount, 1)
        }
    }
}

private func generatedPreviewAge(
    within7Days: UInt64 = 0,
    days8To30: UInt64 = 0,
    days31To90: UInt64 = 0,
    olderThan90Days: UInt64 = 0,
    unknownAge: UInt64 = 0
) -> AiMetadataPreviewAgeSummary {
    AiMetadataPreviewAgeSummary(
        within7DaysLogicalBytes: within7Days,
        days8To30LogicalBytes: days8To30,
        days31To90LogicalBytes: days31To90,
        olderThan90DaysLogicalBytes: olderThan90Days,
        unknownAgeLogicalBytes: unknownAge
    )
}

private func nativePreviewAge(
    within7Days: UInt64 = 0,
    days8To30: UInt64 = 0,
    days31To90: UInt64 = 0,
    olderThan90Days: UInt64 = 0,
    unknownAge: UInt64 = 0
) -> ExplorerAIMetadataPreviewAgeSummary {
    ExplorerAIMetadataPreviewAgeSummary(
        within7DaysLogicalBytes: within7Days,
        days8To30LogicalBytes: days8To30,
        days31To90LogicalBytes: days31To90,
        olderThan90DaysLogicalBytes: olderThan90Days,
        unknownAgeLogicalBytes: unknownAge
    )
}

private func previewChild(
    inputNodeID: String = "n-1",
    label: String = "File 1",
    kind: AiMetadataPreviewNodeKind = .file,
    logicalBytes: UInt64 = 10,
    ageSummary: AiMetadataPreviewAgeSummary = generatedPreviewAge(within7Days: 10)
) -> AiMetadataPreviewChild {
    AiMetadataPreviewChild(
        inputNodeId: inputNodeID,
        label: label,
        kind: kind,
        logicalBytes: logicalBytes,
        ageSummary: ageSummary
    )
}

private func validAIMetadataPreview(
    recordVersion: UInt32 = 1,
    inputSchemaVersion: UInt64 = 1,
    privacyPolicyRevision: UInt64 = 1,
    preparedAtUnixMs: Int64 = 1_000,
    expiresAtUnixMs: Int64 = 121_000,
    inputDigestSha256: String = String(repeating: "a", count: 64),
    inspectedNodeCount: UInt64 = 3,
    includedDirectChildCount: UInt64 = 1,
    excludedSensitiveDirectChildCount: UInt64 = 1,
    omittedEligibleDirectChildCount: UInt64 = 0,
    rootLabel: String = "Selected folder",
    totalLogicalBytes: UInt64 = 10,
    ageSummary: AiMetadataPreviewAgeSummary = generatedPreviewAge(within7Days: 10),
    childrenComplete: Bool = true,
    omittedChildCount: UInt64 = 0,
    omittedLogicalBytes: UInt64 = 0,
    omittedAgeSummary: AiMetadataPreviewAgeSummary = generatedPreviewAge(),
    children: [AiMetadataPreviewChild] = [previewChild()],
    contentIncluded: Bool = false,
    sourceNamesIncluded: Bool = false,
    sourcePathsIncluded: Bool = false,
    mutateJSON: ((inout [String: Any]) -> Void)? = nil
) -> AiMetadataPreviewInfo {
    var document: [String: Any] = [
        "schema_version": inputSchemaVersion,
        "task": "explain_storage_cluster",
        "input_digest_sha256": inputDigestSha256,
        "metadata": NSMutableDictionary(dictionary: [
            "root_label": rootLabel,
            "total_logical_bytes": totalLogicalBytes,
            "age_summary": jsonAge(ageSummary),
            "coverage": "complete",
            "children_complete": childrenComplete,
            "omitted_child_count": omittedChildCount,
            "omitted_logical_bytes": omittedLogicalBytes,
            "omitted_age_summary": jsonAge(omittedAgeSummary),
            "children": NSMutableArray(array: children.map(jsonChild)),
            "known_classifications": [],
            "protected": false,
            "content_included": false,
        ]),
    ]
    mutateJSON?(&document)
    let encoded = try! JSONSerialization.data(withJSONObject: document, options: [.sortedKeys])
    return AiMetadataPreviewInfo(
        recordVersion: recordVersion,
        inputSchemaVersion: inputSchemaVersion,
        privacyPolicyRevision: privacyPolicyRevision,
        preparedAtUnixMs: preparedAtUnixMs,
        expiresAtUnixMs: expiresAtUnixMs,
        inputDigestSha256: inputDigestSha256,
        encodedInputJsonUtf8: encoded,
        inspectedNodeCount: inspectedNodeCount,
        includedDirectChildCount: includedDirectChildCount,
        excludedSensitiveDirectChildCount: excludedSensitiveDirectChildCount,
        omittedEligibleDirectChildCount: omittedEligibleDirectChildCount,
        rootLabel: rootLabel,
        totalLogicalBytes: totalLogicalBytes,
        ageSummary: ageSummary,
        childrenComplete: childrenComplete,
        omittedChildCount: omittedChildCount,
        omittedLogicalBytes: omittedLogicalBytes,
        omittedAgeSummary: omittedAgeSummary,
        children: children,
        contentIncluded: contentIncluded,
        sourceNamesIncluded: sourceNamesIncluded,
        sourcePathsIncluded: sourcePathsIncluded
    )
}

private func jsonAge(_ age: AiMetadataPreviewAgeSummary) -> [String: Any] {
    [
        "within_7_days_logical_bytes": age.within7DaysLogicalBytes,
        "days_8_to_30_logical_bytes": age.days8To30LogicalBytes,
        "days_31_to_90_logical_bytes": age.days31To90LogicalBytes,
        "older_than_90_days_logical_bytes": age.olderThan90DaysLogicalBytes,
        "unknown_age_logical_bytes": age.unknownAgeLogicalBytes,
    ]
}

private func jsonChild(_ child: AiMetadataPreviewChild) -> NSMutableDictionary {
    let kind: String = switch child.kind {
    case .directory: "directory"
    case .file: "file"
    case .symlink: "symlink"
    case .other: "other"
    case .unavailable: "unavailable"
    }
    return NSMutableDictionary(dictionary: [
        "input_node_id": child.inputNodeId,
        "label": child.label,
        "kind": kind,
        "logical_bytes": child.logicalBytes,
        "age_summary": jsonAge(child.ageSummary),
        "coverage": "complete",
        "protected": false,
    ])
}

private func assertInvalid(
    _ raw: AiMetadataPreviewInfo,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertThrowsError(
        try ExplorerAIMetadataPreviewAdapter.map(raw),
        file: file,
        line: line
    ) { error in
        XCTAssertEqual(
            error as? ExplorerAIMetadataPreviewError,
            .invalidResponse,
            file: file,
            line: line
        )
    }
}

private final class StubAIMetadataPreviewSession: AiMetadataPreviewSession, @unchecked Sendable {
    private let infoResult: Result<AiMetadataPreviewInfo, AiMetadataPreviewError>
    private(set) var infoCallCount = 0
    private(set) var releaseCallCount = 0

    init(infoResult: Result<AiMetadataPreviewInfo, AiMetadataPreviewError>) {
        self.infoResult = infoResult
        super.init(noHandle: NoHandle())
    }

    required init(unsafeFromHandle _: UInt64) {
        fatalError("not supported")
    }

    override func info() throws -> AiMetadataPreviewInfo {
        infoCallCount += 1
        return try infoResult.get()
    }

    override func release() throws -> AiMetadataPreviewReleaseOutcome {
        releaseCallCount += 1
        return releaseCallCount == 1 ? .released : .alreadyUnavailable
    }
}

private final class StubAIMetadataSnapshotReviewSession:
    SnapshotReviewSession, @unchecked Sendable
{
    init() {
        super.init(noHandle: NoHandle())
    }

    required init(unsafeFromHandle _: UInt64) {
        fatalError("not supported")
    }

    override func release() throws -> ReviewReleaseOutcome {
        .released
    }
}

private final class StubAIMetadataPreviewEngine: DuxEngine, @unchecked Sendable {
    private var nextParent: SnapshotReviewSession?
    private let previewResult: Result<AiMetadataPreviewSession, AiMetadataPreviewError>
    private(set) weak var preparedParent: SnapshotReviewSession?
    private(set) var preparedRequest: AiMetadataPreviewRequest?

    init(
        nextParent: SnapshotReviewSession,
        previewResult: Result<AiMetadataPreviewSession, AiMetadataPreviewError>
    ) {
        self.nextParent = nextParent
        self.previewResult = previewResult
        super.init(noHandle: NoHandle())
    }

    required init(unsafeFromHandle _: UInt64) {
        fatalError("not supported")
    }

    override func acquireExplorerSnapshotReview(scanId _: String) throws
        -> SnapshotReviewSession
    {
        guard let nextParent else {
            throw EngineError.InternalState
        }
        self.nextParent = nil
        return nextParent
    }

    override func prepareAiMetadataPreview(
        parent: SnapshotReviewSession,
        request: AiMetadataPreviewRequest
    ) throws -> AiMetadataPreviewSession {
        preparedParent = parent
        preparedRequest = request
        return try previewResult.get()
    }
}
