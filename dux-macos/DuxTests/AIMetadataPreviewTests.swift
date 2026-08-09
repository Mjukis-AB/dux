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

    func testExactPreviewTransfersOnceIntoFixedExplanationAttemptAndBurnsValidation() async throws {
        let rawParent = StubAIMetadataSnapshotReviewSession()
        let rawPreviewInfo = validAIMetadataPreview()
        let rawPreview = StubAIMetadataPreviewSession(infoResult: .success(rawPreviewInfo))
        let rawAttemptInfo = validAIExplanationAttemptInfo(
            preview: rawPreviewInfo,
            sourceScanID: "scan-ai-preview",
            selectedRootNodeID: 42
        )
        let rawResult = validAIExplanationResult(info: rawAttemptInfo)
        let rawAttempt = StubAIExplanationAttemptSession(
            infoResult: .success(rawAttemptInfo),
            validationResult: .success(rawResult)
        )
        let engine = StubAIMetadataPreviewEngine(
            nextParent: rawParent,
            previewResult: .success(rawPreview),
            attemptResult: .success(rawAttempt)
        )
        let service = EngineService(engine: engine)
        let parent = try await service.acquireExplorerReview(scanID: "scan-ai-preview")
        let preview = try await parent.prepareAIMetadataPreview(nodeID: 42)

        let deadlineObservation = NativeAIAnthropicMessagesV1DeadlineObservation(
            monotonicNanoseconds: DispatchTime.now().uptimeNanoseconds,
            unixMilliseconds: 1000
        )
        let nativeDeadline = deadlineObservation.monotonicNanoseconds
            + NativeAIRemoteLimits.deadlineNanoseconds
        let attempt = try await preview.consumeAnthropicMessagesV1PreviewOnce(
            deadlineNanoseconds: nativeDeadline,
            deadlineObservation: deadlineObservation
        )
        XCTAssertTrue(engine.consumedPreview === rawPreview)
        XCTAssertEqual(attempt.binding, .trusted)
        XCTAssertEqual(attempt.canonicalMetadataJSON, rawPreviewInfo.encodedInputJsonUtf8)
        XCTAssertEqual(attempt.inputDigestSHA256, rawPreviewInfo.inputDigestSha256)
        XCTAssertEqual(attempt.sourceScanID, "scan-ai-preview")
        XCTAssertEqual(attempt.selectedRootNodeID, 42)
        XCTAssertEqual(attempt.deadlineNanoseconds, nativeDeadline)

        let providerOutput = Data("{\"validated\":true}".utf8)
        let result = try await attempt.validateOnce(extractedInnerJSON: providerOutput)
        XCTAssertEqual(result.inputDigestSHA256, rawPreviewInfo.inputDigestSha256)
        XCTAssertEqual(result.sourceScanID, "scan-ai-preview")
        XCTAssertEqual(result.selectedRootNodeID, 42)
        XCTAssertEqual(result.summary, "Validated summary")
        XCTAssertEqual(result.groups.first?.snapshotNodeIDs, [7])
        XCTAssertEqual(rawAttempt.validatedBodies, [providerOutput])

        do {
            _ = try await attempt.validateOnce(extractedInnerJSON: providerOutput)
            XCTFail("one-shot attempt validated twice")
        } catch {
            XCTAssertEqual(error as? ExplorerAIMetadataPreviewError, .previewUnavailable)
        }
        XCTAssertEqual(rawAttempt.validateCallCount, 1)

        do {
            _ = try await preview.readInfo()
            XCTFail("consumed preview remained readable")
        } catch {
            XCTAssertEqual(error as? ExplorerAIMetadataPreviewError, .previewUnavailable)
        }
        attempt.release()
        attempt.release()
        XCTAssertEqual(rawAttempt.releaseCallCount, 1)
    }

    func testExplanationAttemptInfoMismatchConsumesPreviewAndReleasesAttempt() async throws {
        let rawParent = StubAIMetadataSnapshotReviewSession()
        let rawPreviewInfo = validAIMetadataPreview()
        let rawPreview = StubAIMetadataPreviewSession(infoResult: .success(rawPreviewInfo))
        let mismatched = validAIExplanationAttemptInfo(
            preview: rawPreviewInfo,
            sourceScanID: "scan-other",
            selectedRootNodeID: 42
        )
        let rawAttempt = StubAIExplanationAttemptSession(
            infoResult: .success(mismatched),
            validationResult: .success(validAIExplanationResult(info: mismatched))
        )
        let engine = StubAIMetadataPreviewEngine(
            nextParent: rawParent,
            previewResult: .success(rawPreview),
            attemptResult: .success(rawAttempt)
        )
        let service = EngineService(engine: engine)
        let parent = try await service.acquireExplorerReview(scanID: "scan-ai-preview")
        let preview = try await parent.prepareAIMetadataPreview(nodeID: 42)

        do {
            let deadlineObservation = NativeAIAnthropicMessagesV1DeadlineObservation(
                monotonicNanoseconds: DispatchTime.now().uptimeNanoseconds,
                unixMilliseconds: 1000
            )
            _ = try await preview.consumeAnthropicMessagesV1PreviewOnce(
                deadlineNanoseconds: deadlineObservation.monotonicNanoseconds
                    + NativeAIRemoteLimits.deadlineNanoseconds,
                deadlineObservation: deadlineObservation
            )
            XCTFail("mismatched core attempt escaped")
        } catch {
            XCTAssertEqual(error as? ExplorerAIMetadataPreviewError, .invalidResponse)
        }
        XCTAssertEqual(rawAttempt.releaseCallCount, 1)
        XCTAssertEqual(rawPreview.releaseCallCount, 0)
        do {
            _ = try await preview.readInfo()
            XCTFail("failed transfer restored preview authority")
        } catch {
            XCTAssertEqual(error as? ExplorerAIMetadataPreviewError, .previewUnavailable)
        }
    }

    func testExplanationAttemptPreservesEarlierCoreExpiryAndRejectsExtendedInterval() async throws {
        let rawParent = StubAIMetadataSnapshotReviewSession()
        let rawPreviewInfo = validAIMetadataPreview()
        let rawPreview = StubAIMetadataPreviewSession(infoResult: .success(rawPreviewInfo))
        let earlierInfo = validAIExplanationAttemptInfo(
            preview: rawPreviewInfo,
            sourceScanID: "scan-ai-preview",
            selectedRootNodeID: 42,
            preparedAtUnixMs: 2000,
            expiresAtUnixMs: 31000
        )
        let rawAttempt = StubAIExplanationAttemptSession(
            infoResult: .success(earlierInfo),
            validationResult: .success(validAIExplanationResult(info: earlierInfo))
        )
        let engine = StubAIMetadataPreviewEngine(
            nextParent: rawParent,
            previewResult: .success(rawPreview),
            attemptResult: .success(rawAttempt)
        )
        let service = EngineService(engine: engine)
        let parent = try await service.acquireExplorerReview(scanID: "scan-ai-preview")
        let preview = try await parent.prepareAIMetadataPreview(nodeID: 42)
        let observation = NativeAIAnthropicMessagesV1DeadlineObservation(
            monotonicNanoseconds: DispatchTime.now().uptimeNanoseconds,
            unixMilliseconds: 1000
        )
        let attempt = try await preview.consumeAnthropicMessagesV1PreviewOnce(
            deadlineNanoseconds: observation.monotonicNanoseconds
                + NativeAIRemoteLimits.deadlineNanoseconds,
            deadlineObservation: observation
        )
        XCTAssertEqual(
            attempt.deadlineNanoseconds,
            observation.monotonicNanoseconds + 30_000_000_000
        )
        attempt.release()

        let extendedParent = StubAIMetadataSnapshotReviewSession()
        let extendedPreview = StubAIMetadataPreviewSession(
            infoResult: .success(rawPreviewInfo)
        )
        let extendedInfo = validAIExplanationAttemptInfo(
            preview: rawPreviewInfo,
            sourceScanID: "scan-ai-preview",
            selectedRootNodeID: 42,
            preparedAtUnixMs: 70000,
            expiresAtUnixMs: rawPreviewInfo.expiresAtUnixMs + 1
        )
        let extendedAttempt = StubAIExplanationAttemptSession(
            infoResult: .success(extendedInfo),
            validationResult: .success(validAIExplanationResult(info: extendedInfo))
        )
        let extendedEngine = StubAIMetadataPreviewEngine(
            nextParent: extendedParent,
            previewResult: .success(extendedPreview),
            attemptResult: .success(extendedAttempt)
        )
        let extendedService = EngineService(engine: extendedEngine)
        let extendedReview = try await extendedService.acquireExplorerReview(
            scanID: "scan-ai-preview"
        )
        let extendedLease = try await extendedReview.prepareAIMetadataPreview(nodeID: 42)
        do {
            _ = try await extendedLease.consumeAnthropicMessagesV1PreviewOnce(
                deadlineNanoseconds: observation.monotonicNanoseconds
                    + NativeAIRemoteLimits.deadlineNanoseconds,
                deadlineObservation: observation
            )
            XCTFail("attempt extended beyond its preview")
        } catch {
            XCTAssertEqual(error as? ExplorerAIMetadataPreviewError, .invalidResponse)
        }
        XCTAssertEqual(extendedAttempt.releaseCallCount, 1)
    }

    func testCancelledValidationQueuedBehindEngineWorkNeverEntersFFI() async throws {
        let queueGate = BlockingAIMetadataFFIGate()
        let rawParent = StubAIMetadataSnapshotReviewSession(renewGate: queueGate)
        let rawPreviewInfo = validAIMetadataPreview()
        let rawPreview = StubAIMetadataPreviewSession(infoResult: .success(rawPreviewInfo))
        let rawAttemptInfo = validAIExplanationAttemptInfo(
            preview: rawPreviewInfo,
            sourceScanID: "scan-ai-preview",
            selectedRootNodeID: 42
        )
        let rawAttempt = StubAIExplanationAttemptSession(
            infoResult: .success(rawAttemptInfo),
            validationResult: .success(validAIExplanationResult(info: rawAttemptInfo))
        )
        let engine = StubAIMetadataPreviewEngine(
            nextParent: rawParent,
            previewResult: .success(rawPreview),
            attemptResult: .success(rawAttempt)
        )
        let service = EngineService(engine: engine)
        let parent = try await service.acquireExplorerReview(scanID: "scan-ai-preview")
        let preview = try await parent.prepareAIMetadataPreview(nodeID: 42)
        let observation = NativeAIAnthropicMessagesV1DeadlineObservation(
            monotonicNanoseconds: DispatchTime.now().uptimeNanoseconds,
            unixMilliseconds: 1000
        )
        let attempt = try await preview.consumeAnthropicMessagesV1PreviewOnce(
            deadlineNanoseconds: observation.monotonicNanoseconds
                + NativeAIRemoteLimits.deadlineNanoseconds,
            deadlineObservation: observation
        )

        let blockingOperation = Task { try await parent.renew() }
        let didBlock = await queueGate.waitUntilBlocked()
        XCTAssertTrue(didBlock)
        let validation = Task {
            try await attempt.validateOnce(
                extractedInnerJSON: Data("{\"validated\":true}".utf8)
            )
        }
        for _ in 0 ..< 1000 {
            await Task.yield()
        }
        validation.cancel()
        queueGate.unblock()
        _ = try await blockingOperation.value

        do {
            _ = try await validation.value
            XCTFail("cancelled validation succeeded")
        } catch {
            XCTAssertTrue(
                error is CancellationError
                    || (error as? ExplorerAIMetadataPreviewError) == .previewUnavailable
            )
        }
        XCTAssertEqual(rawAttempt.validateCallCount, 0)
        attempt.release()
    }

    func testExpiredValidationQueuedBehindEngineWorkNeverEntersFFI() async throws {
        let queueGate = BlockingAIMetadataFFIGate()
        let rawParent = StubAIMetadataSnapshotReviewSession(renewGate: queueGate)
        let rawPreviewInfo = validAIMetadataPreview()
        let rawPreview = StubAIMetadataPreviewSession(infoResult: .success(rawPreviewInfo))
        let rawAttemptInfo = validAIExplanationAttemptInfo(
            preview: rawPreviewInfo,
            sourceScanID: "scan-ai-preview",
            selectedRootNodeID: 42,
            preparedAtUnixMs: 1001,
            expiresAtUnixMs: 1100
        )
        let rawAttempt = StubAIExplanationAttemptSession(
            infoResult: .success(rawAttemptInfo),
            validationResult: .success(validAIExplanationResult(info: rawAttemptInfo))
        )
        let engine = StubAIMetadataPreviewEngine(
            nextParent: rawParent,
            previewResult: .success(rawPreview),
            attemptResult: .success(rawAttempt)
        )
        let service = EngineService(engine: engine)
        let parent = try await service.acquireExplorerReview(scanID: "scan-ai-preview")
        let preview = try await parent.prepareAIMetadataPreview(nodeID: 42)
        let observation = NativeAIAnthropicMessagesV1DeadlineObservation(
            monotonicNanoseconds: DispatchTime.now().uptimeNanoseconds,
            unixMilliseconds: 1000
        )
        let attempt = try await preview.consumeAnthropicMessagesV1PreviewOnce(
            deadlineNanoseconds: observation.monotonicNanoseconds
                + NativeAIRemoteLimits.deadlineNanoseconds,
            deadlineObservation: observation
        )

        let blockingOperation = Task { try await parent.renew() }
        let didBlock = await queueGate.waitUntilBlocked()
        XCTAssertTrue(didBlock)
        let validation = Task {
            try await attempt.validateOnce(
                extractedInnerJSON: Data("{\"validated\":true}".utf8)
            )
        }
        while DispatchTime.now().uptimeNanoseconds < attempt.deadlineNanoseconds {
            await Task.yield()
        }
        queueGate.unblock()
        _ = try await blockingOperation.value

        do {
            _ = try await validation.value
            XCTFail("expired queued validation succeeded")
        } catch {
            XCTAssertEqual(error as? ExplorerAIMetadataPreviewError, .previewUnavailable)
        }
        XCTAssertEqual(rawAttempt.validateCallCount, 0)
        attempt.release()
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

private func validAIExplanationAttemptInfo(
    preview: AiMetadataPreviewInfo,
    sourceScanID: String,
    selectedRootNodeID: UInt64,
    preparedAtUnixMs: Int64 = 2000,
    expiresAtUnixMs: Int64 = 62000
) -> AiExplanationAttemptInfo {
    AiExplanationAttemptInfo(
        recordVersion: 1,
        inputSchemaVersion: preview.inputSchemaVersion,
        outputSchemaVersion: 1,
        privacyPolicyRevision: preview.privacyPolicyRevision,
        providerBindingRevision: 1,
        provider: .anthropic,
        transport: .messagesV1,
        model: AnthropicMessagesV1Constants.model,
        preparedAtUnixMs: preparedAtUnixMs,
        expiresAtUnixMs: expiresAtUnixMs,
        inputDigestSha256: preview.inputDigestSha256,
        encodedInputJsonUtf8: preview.encodedInputJsonUtf8,
        sourceScanId: sourceScanID,
        selectedRootNodeId: selectedRootNodeID
    )
}

private func validAIExplanationResult(
    info: AiExplanationAttemptInfo
) -> AiExplanationResult {
    AiExplanationResult(
        recordVersion: info.recordVersion,
        inputSchemaVersion: info.inputSchemaVersion,
        outputSchemaVersion: info.outputSchemaVersion,
        privacyPolicyRevision: info.privacyPolicyRevision,
        providerBindingRevision: info.providerBindingRevision,
        provider: info.provider,
        transport: info.transport,
        model: info.model,
        inputDigestSha256: info.inputDigestSha256,
        sourceScanId: info.sourceScanId,
        selectedRootNodeId: info.selectedRootNodeId,
        summary: "Validated summary",
        labels: ["Build output"],
        groups: [
            AiExplanationGroup(
                recordVersion: 1,
                title: "Generated files",
                snapshotNodeIds: [7],
                reason: "Can be rebuilt."
            ),
        ],
        questions: ["Keep recent builds?"],
        uncertainties: ["Last use is unknown."],
        researchSuggestions: ["Inspect the owning build tool."]
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
    private let renewGate: BlockingAIMetadataFFIGate?

    init(renewGate: BlockingAIMetadataFFIGate? = nil) {
        self.renewGate = renewGate
        super.init(noHandle: NoHandle())
    }

    required init(unsafeFromHandle _: UInt64) {
        fatalError("not supported")
    }

    override func release() throws -> ReviewReleaseOutcome {
        .released
    }

    override func renew() throws -> SnapshotReviewInfo {
        renewGate?.block()
        return SnapshotReviewInfo(
            recordVersion: 1,
            scanId: "scan-ai-preview",
            expiresAtUnixMs: 121_000,
            released: false
        )
    }
}

private final class BlockingAIMetadataFFIGate: @unchecked Sendable {
    private let lock = NSLock()
    private var blocked = false
    private let resume = DispatchSemaphore(value: 0)

    func block() {
        lock.withLock { blocked = true }
        resume.wait()
    }

    func waitUntilBlocked() async -> Bool {
        let clock = ContinuousClock()
        let timeout = clock.now.advanced(by: .seconds(5))
        while !lock.withLock({ blocked }) {
            guard clock.now < timeout else { return false }
            await Task.yield()
        }
        return true
    }

    func unblock() {
        resume.signal()
    }
}

private final class StubAIExplanationAttemptSession:
    AiExplanationAttemptSession, @unchecked Sendable
{
    private let infoResult: Result<AiExplanationAttemptInfo, AiExplanationAttemptError>
    private let validationResult: Result<AiExplanationResult, AiExplanationAttemptError>
    private(set) var infoCallCount = 0
    private(set) var validateCallCount = 0
    private(set) var releaseCallCount = 0
    private(set) var validatedBodies: [Data] = []

    init(
        infoResult: Result<AiExplanationAttemptInfo, AiExplanationAttemptError>,
        validationResult: Result<AiExplanationResult, AiExplanationAttemptError>
    ) {
        self.infoResult = infoResult
        self.validationResult = validationResult
        super.init(noHandle: NoHandle())
    }

    required init(unsafeFromHandle _: UInt64) {
        fatalError("not supported")
    }

    override func info() throws -> AiExplanationAttemptInfo {
        infoCallCount += 1
        return try infoResult.get()
    }

    override func validateOnce(outputJsonUtf8: Data) throws -> AiExplanationResult {
        validateCallCount += 1
        validatedBodies.append(outputJsonUtf8)
        return try validationResult.get()
    }

    override func release() throws -> AiExplanationAttemptReleaseOutcome {
        releaseCallCount += 1
        return releaseCallCount == 1 ? .released : .alreadyUnavailable
    }
}

private final class StubAIMetadataPreviewEngine: DuxEngine, @unchecked Sendable {
    private var nextParent: SnapshotReviewSession?
    private let previewResult: Result<AiMetadataPreviewSession, AiMetadataPreviewError>
    private let attemptResult: Result<AiExplanationAttemptSession, AiExplanationAttemptError>
    private(set) weak var preparedParent: SnapshotReviewSession?
    private(set) var preparedRequest: AiMetadataPreviewRequest?
    private(set) weak var consumedPreview: AiMetadataPreviewSession?

    init(
        nextParent: SnapshotReviewSession,
        previewResult: Result<AiMetadataPreviewSession, AiMetadataPreviewError>,
        attemptResult: Result<AiExplanationAttemptSession, AiExplanationAttemptError> =
            .failure(.PreviewUnavailable)
    ) {
        self.nextParent = nextParent
        self.previewResult = previewResult
        self.attemptResult = attemptResult
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

    override func beginAnthropicMessagesV1Explanation(
        preview: AiMetadataPreviewSession
    ) throws -> AiExplanationAttemptSession {
        consumedPreview = preview
        return try attemptResult.get()
    }
}
