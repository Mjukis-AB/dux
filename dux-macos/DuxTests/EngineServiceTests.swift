import Darwin
import Foundation
import XCTest
@testable import DUX

private func canonicalTestPath(_ url: URL) -> String {
    var buffer = [CChar](repeating: 0, count: Int(PATH_MAX))
    let resolved = buffer.withUnsafeMutableBufferPointer { output in
        url.path.withCString { realpath($0, output.baseAddress) }
    }
    precondition(resolved != nil)
    let end = buffer.firstIndex(of: 0) ?? buffer.endIndex
    return String(decoding: buffer[..<end].map(UInt8.init(bitPattern:)), as: UTF8.self)
}

final class EngineServiceTests: XCTestCase {
    func testRustTargetPlanReviewBridgeUsesBoundParentAndMapsExactRecord() async throws {
        let parent = RecordingGeneratedSnapshotReview()
        let plan = RecordingGeneratedRustTargetPlanReview(
            info: generatedRustTargetPlanReviewInfo()
        )
        let engine = RecordingRustTargetPlanReviewEngine(
            parent: parent,
            plan: plan
        )
        let service = EngineService(engine: engine)
        let lease = try await service.acquireExplorerReview(scanID: "scan:example")

        let session = try await lease.prepareRustTargetPlanReview(
            candidateID: "candidate:example"
        )
        let info = try await session.info()

        XCTAssertEqual(engine.receivedScanID, "scan:example")
        XCTAssertTrue(engine.receivedParent === parent)
        XCTAssertEqual(engine.receivedRequest?.recordVersion, 1)
        XCTAssertEqual(engine.receivedRequest?.candidateId, "candidate:example")
        XCTAssertEqual(session.scanID, "scan:example")
        XCTAssertEqual(session.candidateID, "candidate:example")
        XCTAssertEqual(info.createdAt.secondsSinceUnixEpoch, 1_700_000_000)
        XCTAssertEqual(info.createdAt.nanoseconds, 123_000_000)
        XCTAssertEqual(info.target.encoding, .unixBytes)
        XCTAssertEqual(info.target.encodedBytes, Data("/Users/example/project/target".utf8))

        await session.release()
        XCTAssertEqual(plan.releaseCount, 1)
        await lease.release()
        XCTAssertEqual(parent.releaseCount, 1)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testRustTargetPlanReviewBridgeMapsEveryGeneratedFailure() {
        let cases: [(RustTargetPlanReviewError, ExplorerRustTargetPlanReviewError)] = [
            (.Closed, .closed),
            (.WrongEngine, .invalidResponse),
            (.InvalidRecordVersion, .invalidResponse),
            (.ParentReviewUnavailable, .parentReviewUnavailable),
            (.ReviewExpired, .reviewExpired),
            (.CandidateUnavailable, .candidateUnavailable),
            (.CargoNotEnrolled, .cargoNotEnrolled),
            (.ActiveProcesses, .activeProcesses),
            (.ChangedDuringReview, .changedDuringReview),
            (.UnsupportedPlatform, .unsupportedPlatform),
            (.BudgetExceeded, .budgetExceeded),
            (.Busy, .busy),
            (.UnsafeStorage, .unsafeStorage),
            (.CorruptData, .corruptData),
            (.Unavailable, .unavailable),
            (.ReviewBusy, .busy),
            (.ReviewUnavailable, .unavailable),
            (.InternalState, .invalidResponse),
        ]

        for (ffi, expected) in cases {
            XCTAssertEqual(EngineRustTargetPlanReviewAdapter.map(ffi), expected)
        }
    }

    func testRustTargetPlanReviewBridgeRejectsNegativeGeneratedTimestamp() {
        XCTAssertThrowsError(
            try EngineRustTargetPlanReviewAdapter.map(
                generatedRustTargetPlanReviewInfo(createdAtUnixMS: -1)
            )
        ) { error in
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .invalidResponse
            )
        }
    }

    func testSuspendedRustTargetPrepareDoesNotBlockEngineClose() async throws {
        let parent = RecordingGeneratedSnapshotReview()
        let engine = SuspendedRustTargetPlanReviewEngine(parent: parent)
        let service = EngineService(engine: engine)
        let lease = try await service.acquireExplorerReview(scanID: "scan:example")

        let preparing = Task {
            try await lease.prepareRustTargetPlanReview(
                candidateID: "candidate:example"
            )
        }
        for _ in 0 ..< 2_000 where !engine.prepareStarted {
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTAssertTrue(engine.prepareStarted)

        let closed = await service.close()

        XCTAssertTrue(closed)
        XCTAssertTrue(engine.closeReached)
        engine.resumePrepare()
        do {
            _ = try await preparing.value
            XCTFail("Expected the closed raw engine to reject late prepare")
        } catch {
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .closed
            )
        }
    }

    @MainActor
    func testDefaultInitializationDefersEngineOpenOffMainActor() async throws {
        let baseline = liveEngineInstanceCount()
        let service = EngineService()
        XCTAssertEqual(liveEngineInstanceCount(), baseline)

        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    @MainActor
    func testFirstRealEngineOpenIsLazyAndRunsOffMainActor() async throws {
        let fixture = try TestStorageRootsFixture()
        let dataRoot = URL(fileURLWithPath: fixture.storageRoots.dataRoot, isDirectory: true)
        XCTAssertFalse(FileManager.default.fileExists(atPath: dataRoot.path))

        let service = EngineService(storageRoots: fixture.storageRoots)
        XCTAssertFalse(FileManager.default.fileExists(atPath: dataRoot.path))

        let result = try await service.loadStatus()
        XCTAssertTrue(result.executedOffMainThread)
        XCTAssertTrue(FileManager.default.fileExists(atPath: dataRoot.path))

        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testTransientEngineOpenFailureIsNotCached() async throws {
        let fixture = try TestStorageRootsFixture()
        let dataRoot = URL(fileURLWithPath: fixture.storageRoots.dataRoot, isDirectory: true)
        XCTAssertTrue(FileManager.default.createFile(atPath: dataRoot.path, contents: Data()))
        let service = EngineService(storageRoots: fixture.storageRoots)

        do {
            _ = try await service.loadStatus()
            XCTFail("Expected the obstructed storage root to fail")
        } catch {
            XCTAssertTrue(error is EngineServiceError)
        }

        // DUX-DESTRUCTIVE: allow=test-swift-retry-obstruction-remove -- remove only this test fixture's deliberate file obstruction
        try FileManager.default.removeItem(at: dataRoot)
        let status = try await service.loadStatus()
        XCTAssertEqual(status.ffiContractVersion, 31)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testLoadsTypedRustValuesOffTheMainThread() async throws {
        let fixture = try TestEngineFixture()
        let result = try await EngineService(engine: fixture.engine).loadStatus()

        XCTAssertEqual(result.libraryVersion, "0.5.0")
        XCTAssertEqual(result.ffiContractVersion, 31)
        XCTAssertTrue(result.executedOffMainThread)
    }

    func testCleanupHistorySummaryFeedIsBoundedAndReadOnly() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)

        let page = try await service.loadRecentCleanupHistory(cursor: nil, limit: 64)
        XCTAssertTrue(page.records.isEmpty)
        XCTAssertNil(page.nextCursor)

        do {
            _ = try await service.loadRecentCleanupHistory(cursor: nil, limit: 0)
            XCTFail("Expected the history page limit to be rejected")
        } catch let error as CleanupHistoryServiceError {
            XCTAssertEqual(error, .invalidLimit)
        }

        do {
            _ = try await service.loadCleanupHistorySession(sessionID: "../not-a-token")
            XCTFail("Expected the history session token to be rejected")
        } catch let error as CleanupHistoryServiceError {
            XCTAssertEqual(error, .invalidSessionID)
        }

        do {
            _ = try await service.loadCleanupHistorySession(sessionID: "session:missing")
            XCTFail("Expected the absent history session to be reported")
        } catch let error as CleanupHistoryServiceError {
            XCTAssertEqual(error, .sessionNotFound)
        }

        let closed = await service.close()
        XCTAssertTrue(closed)
        do {
            _ = try await service.loadRecentCleanupHistory(cursor: nil, limit: 1)
            XCTFail("Expected the closed engine to reject history reads")
        } catch let error as CleanupHistoryServiceError {
            XCTAssertEqual(error, .closed)
        }
        do {
            _ = try await service.loadCleanupHistorySession(sessionID: "session:missing")
            XCTFail("Expected the closed engine to reject exact-session history reads")
        } catch let error as CleanupHistoryServiceError {
            XCTAssertEqual(error, .closed)
        }
    }

    func testCleanupHistoryAdapterRejectsMalformedSummaryBeforePresentation() throws {
        let emptyCounts = CleanupStatusCounts(
            planned: 0,
            validating: 0,
            dryRun: 0,
            effectStarted: 0,
            trashed: 0,
            removed: 0,
            evicted: 0,
            skipped: 0,
            rejected: 0,
            failed: 0,
            changedSincePlan: 0,
            interrupted: 0,
            unavailable: 0,
            outcomeUnknown: 0,
            total: 0
        )
        let summary = CleanupSessionSummary(
            recordVersion: 1,
            sessionId: "session:test",
            planId: "plan:test",
            format: .legacyIncomplete,
            sourceScanId: nil,
            startedAtUnixMs: 1_000,
            completedAtUnixMs: nil,
            planCreatedAtUnixMs: nil,
            planExpiresAtUnixMs: nil,
            mode: .dryRun,
            trigger: .manual,
            status: .dryRun,
            estimatedBytes: 0,
            verifiedCapacityDeltaBytes: nil,
            cancellationRequested: nil,
            itemTotal: 0,
            pathTotal: 0,
            evidenceTotal: 0,
            itemStatusCounts: emptyCounts,
            pathStatusCounts: emptyCounts
        )
        let validPage = CleanupHistoryPage(
            recordVersion: 1,
            records: [summary],
            nextCursor: nil
        )
        XCTAssertEqual(try CleanupHistoryAdapter.map(validPage).records.count, 1)

        let earlierDuplicate = CleanupSessionSummary(
            recordVersion: 1,
            sessionId: "session:test",
            planId: "plan:test",
            format: .legacyIncomplete,
            sourceScanId: nil,
            startedAtUnixMs: 999,
            completedAtUnixMs: nil,
            planCreatedAtUnixMs: nil,
            planExpiresAtUnixMs: nil,
            mode: .dryRun,
            trigger: .manual,
            status: .dryRun,
            estimatedBytes: 0,
            verifiedCapacityDeltaBytes: nil,
            cancellationRequested: nil,
            itemTotal: 0,
            pathTotal: 0,
            evidenceTotal: 0,
            itemStatusCounts: emptyCounts,
            pathStatusCounts: emptyCounts
        )
        XCTAssertThrowsError(
            try CleanupHistoryAdapter.map(
                CleanupHistoryPage(
                    recordVersion: 1,
                    records: [summary, earlierDuplicate],
                    nextCursor: nil
                )
            )
        ) { error in
            XCTAssertEqual(error as? CleanupHistoryServiceError, .invalidResponse)
        }

        let malformed = CleanupHistoryPage(
            recordVersion: 2,
            records: [summary],
            nextCursor: nil
        )
        XCTAssertThrowsError(try CleanupHistoryAdapter.map(malformed)) { error in
            XCTAssertEqual(error as? CleanupHistoryServiceError, .invalidResponse)
        }

        let inconsistentCounts = CleanupStatusCounts(
            planned: 1,
            validating: 0,
            dryRun: 0,
            effectStarted: 0,
            trashed: 0,
            removed: 0,
            evicted: 0,
            skipped: 0,
            rejected: 0,
            failed: 0,
            changedSincePlan: 0,
            interrupted: 0,
            unavailable: 0,
            outcomeUnknown: 0,
            total: 0
        )
        let inconsistentSummary = CleanupSessionSummary(
            recordVersion: 1,
            sessionId: "session:test",
            planId: "plan:test",
            format: .legacyIncomplete,
            sourceScanId: nil,
            startedAtUnixMs: 1_000,
            completedAtUnixMs: nil,
            planCreatedAtUnixMs: nil,
            planExpiresAtUnixMs: nil,
            mode: .dryRun,
            trigger: .manual,
            status: .dryRun,
            estimatedBytes: 0,
            verifiedCapacityDeltaBytes: nil,
            cancellationRequested: nil,
            itemTotal: 0,
            pathTotal: 0,
            evidenceTotal: 0,
            itemStatusCounts: inconsistentCounts,
            pathStatusCounts: emptyCounts
        )
        XCTAssertThrowsError(
            try CleanupHistoryAdapter.map(
                CleanupHistoryPage(recordVersion: 1, records: [inconsistentSummary], nextCursor: nil)
            )
        ) { error in
            XCTAssertEqual(error as? CleanupHistoryServiceError, .invalidResponse)
        }
    }

    func testCleanupHistorySessionAdapterMapsSignedCapacityAndRejectsMalformedGraphs() throws {
        let valid = generatedCleanupSessionHistory()
        let mapped = try CleanupHistoryAdapter.mapSession(
            valid,
            requestedSessionID: "session:test"
        )
        XCTAssertEqual(mapped.summary.verifiedCapacityDeltaBytes, -512)
        XCTAssertEqual(mapped.items.map(\.ordinal), [0])
        XCTAssertEqual(mapped.items.first?.category, .developerArtifact)
        XCTAssertEqual(mapped.items.first?.status, .removed)
        XCTAssertEqual(mapped.items.first?.pathCount, 2)
        XCTAssertEqual(
            mapped.warnings,
            [
                .estimatedBytesUnverified,
                .permanentRemovalCannotBeUndone,
            ]
        )
        let evicted = try CleanupHistoryAdapter.mapSession(
            generatedCleanupSessionHistory(
                summary: generatedCleanupSummary(mode: .evictLocalCopy),
                item: generatedCleanupItem(action: .evictLocalCopy),
                warnings: [
                    .estimatedBytesUnverified,
                    .cloudEvictionRequiresNetworkToRedownload,
                ]
            ),
            requestedSessionID: "session:test"
        )
        XCTAssertEqual(evicted.items.first?.action, .evictLocalCopy)
        XCTAssertEqual(
            evicted.warnings,
            [.estimatedBytesUnverified, .cloudEvictionRequiresNetworkToRedownload]
        )
        XCTAssertTrue(
            try CleanupHistoryAdapter.mapSession(
                generatedLegacyCleanupSessionHistory(),
                requestedSessionID: "session:legacy"
            ).warnings.isEmpty
        )

        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(recordVersion: 2)
        )
        XCTAssertThrowsError(
            try CleanupHistoryAdapter.mapSession(
                valid,
                requestedSessionID: "session:different"
            )
        ) { error in
            XCTAssertEqual(error as? CleanupHistoryServiceError, .invalidResponse)
        }
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                item: generatedCleanupItem(ordinal: 1)
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                item: generatedCleanupItem(category: nil)
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                item: generatedCleanupItem(estimatedBytes: 2_048)
            )
        )
        let overflowingSummary = CleanupSessionSummary(
            recordVersion: 1,
            sessionId: "session:test",
            planId: "plan:test",
            format: .complete,
            sourceScanId: "scan:test",
            startedAtUnixMs: 2_000,
            completedAtUnixMs: 3_000,
            planCreatedAtUnixMs: 1_000,
            planExpiresAtUnixMs: 4_000,
            mode: .permanentSafe,
            trigger: .manual,
            status: .completed,
            estimatedBytes: .max,
            verifiedCapacityDeltaBytes: -512,
            cancellationRequested: false,
            itemTotal: 2,
            pathTotal: 4,
            evidenceTotal: 2,
            itemStatusCounts: generatedCleanupCounts(removed: 2),
            pathStatusCounts: generatedCleanupCounts(removed: 4)
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                summary: overflowingSummary,
                items: [
                    generatedCleanupItem(estimatedBytes: .max),
                    generatedCleanupItem(ordinal: 1, estimatedBytes: 1),
                ]
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                item: generatedCleanupItem(errorRecorded: false, errorCategory: "io")
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                item: generatedCleanupItem(errorRecorded: true, errorCategory: "not/typed")
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                summary: generatedCleanupSummary(
                    itemStatusCounts: generatedCleanupCounts(planned: 1)
                )
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                summary: generatedCleanupSummary(planExpiresAtUnixMs: 2_000)
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                summary: generatedCleanupSummary(
                    completedAtUnixMs: nil,
                    status: .running,
                    verifiedCapacityDeltaBytes: -512
                )
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                summary: generatedCleanupSummary(
                    completedAtUnixMs: nil,
                    status: .planned,
                    verifiedCapacityDeltaBytes: nil,
                    cancellationRequested: true
                )
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                summary: generatedCleanupSummary(planCreatedAtUnixMs: 2_001)
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                warnings: [.estimatedBytesUnverified, .estimatedBytesUnverified]
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                warnings: [.permanentRemovalCannotBeUndone, .estimatedBytesUnverified]
            )
        )
        assertInvalidCleanupSessionHistory(
            generatedCleanupSessionHistory(
                warnings: [.estimatedBytesUnverified, .dryRunDoesNotMutate]
            )
        )
        XCTAssertThrowsError(
            try CleanupHistoryAdapter.mapSession(
                generatedLegacyCleanupSessionHistory(
                    warnings: [.estimatedBytesUnverified]
                ),
                requestedSessionID: "session:legacy"
            )
        ) { error in
            XCTAssertEqual(error as? CleanupHistoryServiceError, .invalidResponse)
        }
    }

    @MainActor
    func testRealPressurePolicyRoundTripPreservesExactValuesAndProvenanceOffMainActor() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let defaults = try await service.loadDiskPressurePolicy()
        XCTAssertEqual(defaults, testDefaultDiskPressurePolicy())

        let explicitDefault = try await service.setDiskPressurePolicy(.defaults)
        XCTAssertTrue(explicitDefault.changed)
        XCTAssertEqual(explicitDefault.policy.source, .stored)
        XCTAssertEqual(explicitDefault.policy.revision, 1)
        XCTAssertEqual(explicitDefault.policy.configuration, .defaults)

        let unchangedDefault = try await service.setDiskPressurePolicy(.defaults)
        XCTAssertFalse(unchangedDefault.changed)
        XCTAssertEqual(unchangedDefault.policy, explicitDefault.policy)

        let exact = DiskPressurePolicyConfiguration(
            criticalAvailableBytes: UInt64.max - (1 << 31),
            criticalAvailableBasisPoints: 9_998,
            warningAvailableBytes: UInt64.max - (1 << 30),
            warningAvailableBasisPoints: 9_999,
            recoveryBytes: 1,
            recoveryBasisPoints: 1
        )
        let custom = try await service.setDiskPressurePolicy(exact)
        XCTAssertTrue(custom.changed)
        XCTAssertEqual(custom.policy.configuration, exact)
        XCTAssertEqual(custom.policy.revision, 2)

        let reset = try await service.resetDiskPressurePolicy()
        XCTAssertTrue(reset.changed)
        XCTAssertEqual(reset.policy.source, .default)
        XCTAssertEqual(reset.policy.configuration, .defaults)
        XCTAssertEqual(reset.policy.revision, 3)

        let unchangedReset = try await service.resetDiskPressurePolicy()
        XCTAssertFalse(unchangedReset.changed)
        XCTAssertEqual(unchangedReset.policy, reset.policy)
    }

    func testPressurePolicyServiceMapsTypedValidationAndClosedErrors() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let invalid = DiskPressurePolicyConfiguration(
            criticalAvailableBytes: 10,
            criticalAvailableBasisPoints: 500,
            warningAvailableBytes: 9,
            warningAvailableBasisPoints: 1_000,
            recoveryBytes: 1,
            recoveryBasisPoints: 1
        )
        do {
            _ = try await service.setDiskPressurePolicy(invalid)
            XCTFail("Expected Rust policy validation to reject warning bytes")
        } catch let error as DiskPressurePolicyServiceError {
            XCTAssertEqual(error, .warningBytesBelowCritical)
        }

        let closed = await service.close()
        XCTAssertTrue(closed)
        do {
            _ = try await service.loadDiskPressurePolicy()
            XCTFail("Expected a closed policy service")
        } catch let error as DiskPressurePolicyServiceError {
            XCTAssertEqual(error, .closed)
        }
    }

    func testPressurePolicyServiceRejectsMalformedVersionedResponse() async {
        let service = EngineService(engine: InvalidPressurePolicyEngine())
        do {
            _ = try await service.loadDiskPressurePolicy()
            XCTFail("Expected malformed policy response rejection")
        } catch let error as DiskPressurePolicyServiceError {
            XCTAssertEqual(error, .invalidResponse)
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testPressurePolicyServiceRejectsMisleadingDefaultResponses() async {
        let service = EngineService(engine: MisleadingDefaultPressurePolicyEngine())
        do {
            _ = try await service.loadDiskPressurePolicy()
            XCTFail("Expected a custom configuration labeled Default to be rejected")
        } catch let error as DiskPressurePolicyServiceError {
            XCTAssertEqual(error, .invalidResponse)
        } catch {
            XCTFail("Unexpected error: \(error)")
        }

        do {
            _ = try await service.resetDiskPressurePolicy()
            XCTFail("Expected a misleading reset response to be rejected")
        } catch let error as DiskPressurePolicyServiceError {
            XCTAssertEqual(error, .invalidResponse)
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testRealHomeScanUsesInjectedTemporaryScopeAndReturnsValidatedSummary() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(
            engine: fixture.engine,
            homeScanRoot: fixture.homeScanRoot
        )
        let folder = fixture.homeScanRoot.appending(path: "folder", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        // DUX-DESTRUCTIVE: allow=test-swift-home-scan-fixture-write -- write only one nested payload inside this UUID-named temporary scan root
        try Data("scan payload".utf8).write(
            to: folder.appending(path: "nested.txt")
        )

        let start = try await service.startHomeScan()
        let task = start.task
        let deadline = ContinuousClock.now + .seconds(5)
        let terminal: HomeScanTaskPoll
        while true {
            let poll = try await task.poll()
            if poll.phase.isTerminal {
                terminal = poll
                break
            }
            XCTAssertLessThan(ContinuousClock.now, deadline)
            try await Task.sleep(for: .milliseconds(10))
        }

        XCTAssertEqual(terminal.phase, .succeeded)
        let result = try XCTUnwrap(terminal.result)
        XCTAssertTrue(result.succeeded)
        XCTAssertEqual(result.fileCount, 1)
        XCTAssertEqual(result.logicalBytes, UInt64("scan payload".utf8.count))
        XCTAssertTrue(result.snapshotAvailable)
        XCTAssertNotEqual(result.coverage, .unknown)
        XCTAssertNotNil(result.successfulSummary)

        let review = try await service.acquireLatestExplorerReview()
        XCTAssertEqual(review.scanID, result.scanID)
        let rootNode = try await review.rootNode()
        XCTAssertEqual(rootNode.id, 0)
        XCTAssertEqual(rootNode.kind, .directory)
        XCTAssertEqual(rootNode.childCount, 1)
        let page = try await review.childNodes(
            parentID: rootNode.id,
            sort: .logicalBytesDescending,
            offset: 0,
            limit: 50
        )
        XCTAssertEqual(page.totalChildren, 1)
        let folderNode = try XCTUnwrap(page.nodes.first)
        XCTAssertEqual(folderNode.kind, .directory)
        XCTAssertEqual(folderNode.name.display, "folder")
        XCTAssertFalse(page.hasMore)
        let rootTreemap = try await review.treemap(parentID: rootNode.id, maxCells: 48)
        XCTAssertEqual(rootTreemap.parentID, rootNode.id)
        XCTAssertEqual(rootTreemap.totalChildren, page.totalChildren)
        XCTAssertEqual(rootTreemap.totalChildLogicalBytes, rootNode.logicalBytes)
        XCTAssertEqual(rootTreemap.cells.map(\.node.id), [folderNode.id])
        XCTAssertEqual(rootTreemap.otherChildCount, 0)
        let nested = try await review.childNodes(
            parentID: folderNode.id,
            sort: .nameAscending,
            offset: 0,
            limit: 50
        )
        XCTAssertEqual(nested.totalChildren, 1)
        XCTAssertEqual(nested.nodes.map(\.name.display), ["nested.txt"])
        let nestedNode = try XCTUnwrap(nested.nodes.first)
        let nestedTreemap = try await review.treemap(parentID: folderNode.id, maxCells: 48)
        XCTAssertEqual(nestedTreemap.cells.map(\.node.name.display), ["nested.txt"])
        XCTAssertEqual(nestedTreemap.totalChildLogicalBytes, folderNode.logicalBytes)
        let rootLiveItem = try await review.resolveLiveItem(
            nodeID: rootNode.id,
            purpose: .reveal
        )
        XCTAssertEqual(rootLiveItem.kind, .directory)
        XCTAssertEqual(
            rootLiveItem.exactTextPath,
            canonicalTestPath(fixture.homeScanRoot)
        )
        let nestedLiveItem = try await review.resolveLiveItem(
            nodeID: nestedNode.id,
            purpose: .quickLook
        )
        XCTAssertEqual(nestedLiveItem.kind, .file)
        XCTAssertEqual(
            nestedLiveItem.exactTextPath,
            canonicalTestPath(folder.appending(path: "nested.txt"))
        )

        let subtreeTask = try await review.startSubtreeScan(nodeID: folderNode.id).task
        let subtreeDeadline = ContinuousClock.now + .seconds(5)
        let subtreeTerminal: HomeScanTaskPoll
        while true {
            let poll = try await subtreeTask.poll()
            if poll.phase.isTerminal {
                subtreeTerminal = poll
                break
            }
            XCTAssertLessThan(ContinuousClock.now, subtreeDeadline)
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(subtreeTerminal.phase, .succeeded)
        let subtreeResult = try XCTUnwrap(subtreeTerminal.result)
        XCTAssertEqual(subtreeResult.fileCount, 1)
        XCTAssertEqual(subtreeResult.directoryCount, 1)
        XCTAssertNotEqual(subtreeResult.scanID, result.scanID)

        // Starting a standalone folder snapshot must not invalidate or mutate
        // the source review that supplied the node identity.
        let retainedSourceRoot = try await review.rootNode()
        XCTAssertEqual(retainedSourceRoot, rootNode)
        let subtreeReview = try await service.acquireExplorerReview(
            scanID: subtreeResult.scanID
        )
        let subtreeRoot = try await subtreeReview.rootNode()
        XCTAssertEqual(subtreeRoot.kind, .directory)
        XCTAssertEqual(subtreeRoot.name.display, canonicalTestPath(folder))
        XCTAssertEqual(subtreeRoot.childCount, 1)
        let subtreePage = try await subtreeReview.childNodes(
            parentID: subtreeRoot.id,
            sort: .nameAscending,
            offset: 0,
            limit: 50
        )
        XCTAssertEqual(subtreePage.nodes.map(\.name.display), ["nested.txt"])
        await subtreeReview.release()

        do {
            _ = try await review.treemap(parentID: rootNode.id, maxCells: 0)
            XCTFail("Expected an invalid treemap budget")
        } catch {
            XCTAssertEqual(error as? ExplorerSnapshotTreemapError, .invalidBudget)
        }
        await review.release()
        do {
            _ = try await review.rootNode()
            XCTFail("Expected released review navigation to expire")
        } catch {
            XCTAssertEqual(error as? ExplorerSnapshotNodeError, .reviewExpired)
        }
        do {
            _ = try await review.resolveLiveItem(nodeID: nestedNode.id, purpose: .reveal)
            XCTFail("Expected released live-path resolution to expire")
        } catch {
            XCTAssertEqual(error as? ExplorerSnapshotLivePathError, .reviewExpired)
        }
    }

    func testHomeScanAdapterPassesOnlyResolvedHomeAndRunsAllFFIOffMain() async throws {
        let generatedTask = RecordingGeneratedScanTask(
            polls: [generatedActivePoll(revision: 1)]
        )
        let engine = RecordingScanEngine(task: generatedTask)
        let service = EngineService(engine: engine)

        let start = try await service.startHomeScan()
        _ = try await start.task.poll()
        _ = try await start.task.requestCancellation()

        XCTAssertEqual(
            engine.request?.root,
            FileManager.default.homeDirectoryForCurrentUser.path
        )
        XCTAssertEqual(engine.request?.recordVersion, 1)
        XCTAssertEqual(engine.calledOnMain, false)
        XCTAssertEqual(generatedTask.pollCalledOnMain, false)
        XCTAssertEqual(generatedTask.cancelCalledOnMain, false)
    }

    func testHomeScanAdapterRejectsVersionContradictionAndRevisionRegression() async throws {
        let wrongVersion = RecordingGeneratedScanTask(
            polls: [
                ScanPoll(
                    recordVersion: 2,
                    phase: .running,
                    stage: .scanning,
                    cancellationRequested: false,
                    revision: 1,
                    progress: nil,
                    events: [],
                    nextEventSequence: 0,
                    oldestAvailableEventSequence: 0,
                    eventsTruncated: false,
                    failure: nil,
                    result: nil
                ),
            ]
        )
        let wrongVersionTask = try await EngineService(
            engine: RecordingScanEngine(task: wrongVersion)
        ).startHomeScan().task
        await XCTAssertThrowsHomeScanError(.invalidResponse) {
            _ = try await wrongVersionTask.poll()
        }

        let regressing = RecordingGeneratedScanTask(
            polls: [
                ScanPoll(
                    recordVersion: 1,
                    phase: .running,
                    stage: .scanning,
                    cancellationRequested: false,
                    revision: 2,
                    progress: nil,
                    events: [],
                    nextEventSequence: 3,
                    oldestAvailableEventSequence: 1,
                    eventsTruncated: false,
                    failure: nil,
                    result: nil
                ),
                generatedActivePoll(revision: 1),
            ]
        )
        let regressingTask = try await EngineService(
            engine: RecordingScanEngine(task: regressing)
        ).startHomeScan().task
        _ = try await regressingTask.poll()
        await XCTAssertThrowsHomeScanError(.invalidResponse) {
            _ = try await regressingTask.poll()
        }
    }

    func testHomeScanAdapterAcceptsSameRevisionWithMonotonicProgress() async throws {
        let first = ScanProgress(
            recordVersion: 1,
            filesScanned: 1,
            directoriesScanned: 1,
            knownAllocatedBytes: 64,
            errorCount: 0
        )
        let second = ScanProgress(
            recordVersion: 1,
            filesScanned: 2,
            directoriesScanned: 1,
            knownAllocatedBytes: 96,
            errorCount: 0
        )
        let generated = RecordingGeneratedScanTask(
            polls: [
                generatedActivePoll(revision: 2, progress: first),
                generatedActivePoll(revision: 2, progress: second),
            ]
        )
        let task = try await EngineService(
            engine: RecordingScanEngine(task: generated)
        ).startHomeScan().task

        _ = try await task.poll()
        let advanced = try await task.poll()

        XCTAssertEqual(advanced.revision, 2)
        XCTAssertEqual(advanced.progress?.files, 2)
        XCTAssertEqual(advanced.progress?.knownAllocatedBytes, 96)
    }

    func testHomeScanAdapterPreservesTypedFailureAfterLargePartialProgress() async throws {
        let progress = ScanProgress(
            recordVersion: 1,
            filesScanned: 120_000,
            directoriesScanned: 30_000,
            knownAllocatedBytes: 64 * 1_024 * 1_024 * 1_024,
            errorCount: 20_000
        )
        let failedResult = ScanTaskResult(
            recordVersion: 1,
            scanId: "scan:rejected-production-shape",
            startedAtUnixMs: 1,
            completedAtUnixMs: 180_001,
            status: .failed,
            directoryCount: 0,
            fileCount: 0,
            logicalBytes: 0,
            allocatedBytes: nil,
            snapshotAvailable: false,
            coverage: ScanCoverageSummary(
                recordVersion: 1,
                status: .unknown,
                measuredPermille: nil,
                issueRecordCount: 0,
                issueOccurrenceCount: 0
            ),
            candidateEvaluation: ScanCandidateEvaluationSummary(
                recordVersion: 1,
                status: .notRun,
                candidateCount: 0,
                failure: nil
            )
        )
        let generated = RecordingGeneratedScanTask(
            polls: [
                generatedActivePoll(revision: 2, progress: progress),
                ScanPoll(
                    recordVersion: 1,
                    phase: .failed,
                    stage: .terminal,
                    cancellationRequested: false,
                    revision: 3,
                    progress: progress,
                    events: [],
                    nextEventSequence: 0,
                    oldestAvailableEventSequence: 0,
                    eventsTruncated: false,
                    failure: .snapshotRejected,
                    result: failedResult
                ),
            ]
        )
        let task = try await EngineService(
            engine: RecordingScanEngine(task: generated)
        ).startHomeScan().task

        _ = try await task.poll()
        let terminal = try await task.poll()

        XCTAssertEqual(terminal.phase, .failed)
        XCTAssertEqual(terminal.failure, .snapshotRejected)
        XCTAssertEqual(terminal.result?.succeeded, false)
        XCTAssertEqual(terminal.progress?.issueCount, 20_000)
    }

    func testHomeScanAdapterMapsEveryTypedStartError() async {
        let cases: [(ScanError, HomeScanServiceError)] = [
            (.Closed, .closed),
            (.InvalidRecordVersion, .invalidResponse),
            (.ForeignReview, .invalidResponse),
            (.ReviewExpired, .rootUnavailable),
            (.ReviewUnavailable, .rootUnavailable),
            (.SnapshotNodeNotFound, .rootUnavailable),
            (.SnapshotNodeNotDirectory, .rootUnavailable),
            (.InvalidRoot, .invalidRoot),
            (.RootMissing, .rootMissing),
            (.RootAccessDenied, .rootAccessDenied),
            (.RootNotDirectory, .rootNotDirectory),
            (.RootSymlink, .rootSymlink),
            (.RootChanged, .rootChanged),
            (.RootIdentityUnavailable, .rootIdentityUnavailable),
            (.UnsupportedPlatform, .unsupportedPlatform),
            (.RootUnavailable, .rootUnavailable),
            (.QueueFull, .queueFull),
            (.Busy, .busy),
            (.InputTooLarge, .inputTooLarge),
            (.ReadOnlyStore, .readOnlyStore),
            (.StorageUnavailable, .persistenceUnavailable),
            (.RegistryUnavailable, .internalState),
            (.TaskUnavailable, .taskExpired),
            (.EventHistoryUnavailable, .outcomeUnknown),
            (.WrongTaskKind, .wrongTaskKind),
            (.InternalState, .internalState),
        ]

        for (ffiError, expected) in cases {
            let service = EngineService(engine: ThrowingScanEngine(error: ffiError))
            await XCTAssertThrowsHomeScanError(expected) {
                _ = try await service.startHomeScan()
            }
        }
    }

    func testHomeScanAdapterRejectsMalformedStartRecord() async {
        let engine = RecordingScanEngine(
            task: RecordingGeneratedScanTask(polls: [generatedActivePoll(revision: 1)]),
            startRecordVersion: 2
        )
        await XCTAssertThrowsHomeScanError(.invalidResponse) {
            _ = try await EngineService(engine: engine).startHomeScan()
        }
    }

    func testHomeScanAdapterMapsSequencedTypedEventsAndCursor() async throws {
        let generated = RecordingGeneratedScanTask(
            polls: [
                ScanPoll(
                    recordVersion: 1,
                    phase: .running,
                    stage: .scanning,
                    cancellationRequested: false,
                    revision: 1,
                    progress: nil,
                    events: [
                        ScanEvent(recordVersion: 1, sequence: 1, kind: .queued),
                        ScanEvent(recordVersion: 1, sequence: 2, kind: .started),
                        ScanEvent(
                            recordVersion: 1,
                            sequence: 3,
                            kind: .scanProgress(
                                filesScanned: 4,
                                directoriesScanned: 2,
                                knownAllocatedBytes: 512,
                                errorCount: 1
                            )
                        ),
                    ],
                    nextEventSequence: 3,
                    oldestAvailableEventSequence: 1,
                    eventsTruncated: false,
                    failure: nil,
                    result: nil
                ),
                ScanPoll(
                    recordVersion: 1,
                    phase: .running,
                    stage: .scanning,
                    cancellationRequested: false,
                    revision: 2,
                    progress: nil,
                    events: [],
                    nextEventSequence: 3,
                    oldestAvailableEventSequence: 1,
                    eventsTruncated: false,
                    failure: nil,
                    result: nil
                ),
            ]
        )
        let task = try await EngineService(
            engine: RecordingScanEngine(task: generated)
        ).startHomeScan().task

        let first = try await task.poll()
        XCTAssertEqual(first.eventCursor, 3)
        XCTAssertEqual(first.events.map(\.sequence), [1, 2, 3])
        XCTAssertEqual(first.events[2].kind, .scanning(
            files: 4,
            directories: 2,
            knownAllocatedBytes: 512,
            errors: 1
        ))
        let second = try await task.poll()
        XCTAssertTrue(second.events.isEmpty)
        XCTAssertEqual(second.eventCursor, 3)
    }

    func testHomeScanAdapterRejectsMalformedNestedAndTerminalRecords() async throws {
        let malformedCoverage = ScanTaskResult(
            recordVersion: 1,
            scanId: "scan:test",
            startedAtUnixMs: 1,
            completedAtUnixMs: 2,
            status: .succeeded,
            directoryCount: 1,
            fileCount: 1,
            logicalBytes: 1,
            allocatedBytes: 1,
            snapshotAvailable: true,
            coverage: ScanCoverageSummary(
                recordVersion: 1,
                status: .complete,
                measuredPermille: 999,
                issueRecordCount: 0,
                issueOccurrenceCount: 0
            ),
            candidateEvaluation: ScanCandidateEvaluationSummary(
                recordVersion: 1,
                status: .succeeded,
                candidateCount: 0,
                failure: nil
            )
        )
        let malformed = RecordingGeneratedScanTask(
            polls: [generatedSuccessPoll(result: malformedCoverage)]
        )
        let malformedTask = try await EngineService(
            engine: RecordingScanEngine(task: malformed)
        ).startHomeScan().task
        await XCTAssertThrowsHomeScanError(.invalidResponse) {
            _ = try await malformedTask.poll()
        }

        let contradictory = RecordingGeneratedScanTask(
            polls: [
                ScanPoll(
                    recordVersion: 1,
                    phase: .running,
                    stage: .terminal,
                    cancellationRequested: false,
                    revision: 2,
                    progress: nil,
                    events: [],
                    nextEventSequence: 0,
                    oldestAvailableEventSequence: 0,
                    eventsTruncated: false,
                    failure: nil,
                    result: nil
                ),
            ]
        )
        let contradictoryTask = try await EngineService(
            engine: RecordingScanEngine(task: contradictory)
        ).startHomeScan().task
        await XCTAssertThrowsHomeScanError(.invalidResponse) {
            _ = try await contradictoryTask.poll()
        }

        let measuredProgress = ScanProgress(
            recordVersion: 1,
            filesScanned: 4,
            directoriesScanned: 3,
            knownAllocatedBytes: 128,
            errorCount: 2
        )
        let regressingResult = ScanTaskResult(
            recordVersion: 1,
            scanId: "scan:regressing-counts",
            startedAtUnixMs: 1,
            completedAtUnixMs: 2,
            status: .succeeded,
            directoryCount: 1,
            fileCount: 1,
            logicalBytes: 1,
            allocatedBytes: 1,
            snapshotAvailable: true,
            coverage: ScanCoverageSummary(
                recordVersion: 1,
                status: .complete,
                measuredPermille: 1_000,
                issueRecordCount: 0,
                issueOccurrenceCount: 0
            ),
            candidateEvaluation: ScanCandidateEvaluationSummary(
                recordVersion: 1,
                status: .succeeded,
                candidateCount: 0,
                failure: nil
            )
        )
        let regressingCounts = RecordingGeneratedScanTask(
            polls: [
                generatedActivePoll(revision: 2, progress: measuredProgress),
                generatedSuccessPoll(result: regressingResult),
            ]
        )
        let regressingCountsTask = try await EngineService(
            engine: RecordingScanEngine(task: regressingCounts)
        ).startHomeScan().task
        _ = try await regressingCountsTask.poll()
        await XCTAssertThrowsHomeScanError(.invalidResponse) {
            _ = try await regressingCountsTask.poll()
        }
    }

    func testRealEngineClassifiesAndPersistsStartupVolumeOffMainThread() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let gib: UInt64 = 1_024 * 1_024 * 1_024
        let snapshot = VolumeCapacitySnapshot(
            stableVolumeID: "01234567-89AB-CDEF-0123-456789ABCDEF",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 1_024 * gib,
            filesystemAvailableBytes: 100 * gib,
            importantAvailableBytes: 20 * gib,
            effectiveAvailableBytes: 20 * gib,
            availabilityBasis: .importantUsage,
            pressure: .unknown,
            criticalBoundaryBytes: nil,
            warningBoundaryBytes: nil,
            historyDisposition: nil,
            sampledAt: Date(timeIntervalSince1970: 3_600)
        )

        let status = try await service.observeVolumeCapacity(snapshot)

        XCTAssertEqual(status.pressure, .warning)
        XCTAssertEqual(status.criticalBoundaryBytes, 10 * gib)
        XCTAssertEqual(status.warningBoundaryBytes, 30 * gib)
        XCTAssertEqual(status.historyDisposition, .stored)
        XCTAssertEqual(
            status.stableVolumeID,
            "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        )
    }

    func testCapacityTrendRoundTripPreservesSignedChangesAndPointSources() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let volumeID = "01234567-89AB-CDEF-0123-456789ABCDEF"
        let day: TimeInterval = 86_400
        let base: TimeInterval = 1_800_000_000
        let available: [UInt64] = [900, 800, 700, 650]
        let offsets: [TimeInterval] = [0, 2 * day, 7 * day, 8 * day]
        for (offset, bytes) in zip(offsets, available) {
            _ = try await service.observeVolumeCapacity(
                VolumeCapacitySnapshot(
                    stableVolumeID: volumeID,
                    displayName: "Macintosh HD",
                    filesystem: "APFS",
                    isInternal: true,
                    isRemovable: false,
                    totalBytes: 1_000,
                    filesystemAvailableBytes: bytes,
                    importantAvailableBytes: bytes,
                    effectiveAvailableBytes: bytes,
                    availabilityBasis: .importantUsage,
                    pressure: .unknown,
                    criticalBoundaryBytes: nil,
                    warningBoundaryBytes: nil,
                    historyDisposition: nil,
                    sampledAt: Date(timeIntervalSince1970: base + offset)
                )
            )
        }

        let trend = try await service.loadCapacityTrend(
            stableVolumeID: volumeID,
            at: Date(timeIntervalSince1970: base + 8 * day + 1)
        )
        XCTAssertEqual(trend.stableVolumeID, "volume:macos:01234567-89ab-cdef-0123-456789abcdef")
        XCTAssertEqual(trend.availableBytes, 650)
        XCTAssertEqual(trend.change24h?.availableBytes, -50)
        XCTAssertEqual(trend.change7d?.availableBytes, -250)
        XCTAssertEqual(trend.points.map(\.availableBytes), [900, 800, 700, 650])
        XCTAssertEqual(trend.points.map(\.source), [.raw, .raw, .raw, .raw])
    }

    func testRealEngineRejectsMalformedVolumeIdentityWithTypedError() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let snapshot = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)

        do {
            _ = try await service.observeVolumeCapacity(snapshot)
            XCTFail("Expected malformed stable identity to be rejected")
        } catch let error as EngineServiceError {
            XCTAssertEqual(error, .invalidCapacityObservation)
        }
    }

    func testLinkedEngineRejectsUnknownStartupVolumeRecordVersion() throws {
        let fixture = try TestEngineFixture()
        let gib: UInt64 = 1_024 * 1_024 * 1_024

        XCTAssertThrowsError(
            try fixture.engine.observeStartupVolume(
                observation: StartupVolumeObservation(
                    recordVersion: 2,
                    stableVolumeId: "01234567-89AB-CDEF-0123-456789ABCDEF",
                    displayName: "Macintosh HD",
                    filesystem: "APFS",
                    isInternal: true,
                    isRemovable: false,
                    sampledAtUnixMs: 1,
                    totalBytes: 1_024 * gib,
                    ordinaryAvailableBytes: 100 * gib,
                    importantAvailableBytes: 100 * gib
                )
            )
        ) { error in
            XCTAssertEqual(error as? EngineError, .InvalidCapacityObservation)
        }
    }

    func testVolumeStatusValidationRejectsContradictoryHistoryDisposition() {
        let gib: UInt64 = 1_024 * 1_024 * 1_024
        let snapshot = VolumeCapacitySnapshot(
            stableVolumeID: "01234567-89AB-CDEF-0123-456789ABCDEF",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 1_024 * gib,
            filesystemAvailableBytes: 100 * gib,
            importantAvailableBytes: 20 * gib,
            effectiveAvailableBytes: 20 * gib,
            availabilityBasis: .importantUsage,
            pressure: .unknown,
            criticalBoundaryBytes: nil,
            warningBoundaryBytes: nil,
            historyDisposition: nil,
            sampledAt: Date(timeIntervalSince1970: 1)
        )
        let stableID = "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        let valid = StartupVolumeStatus(
            recordVersion: 1,
            stableVolumeId: stableID,
            sampledAtUnixMs: 1_000,
            totalBytes: snapshot.totalBytes,
            ordinaryAvailableBytes: snapshot.filesystemAvailableBytes,
            importantAvailableBytes: snapshot.importantAvailableBytes,
            headlineAvailableBytes: snapshot.effectiveAvailableBytes,
            headlineSource: .importantUsage,
            pressure: .warning,
            previousDurablePressure: nil,
            criticalBoundaryBytes: 10 * gib,
            warningBoundaryBytes: 30 * gib,
            historyDisposition: .stored
        )
        let contradictory = StartupVolumeStatus(
            recordVersion: valid.recordVersion,
            stableVolumeId: valid.stableVolumeId,
            sampledAtUnixMs: valid.sampledAtUnixMs,
            totalBytes: valid.totalBytes,
            ordinaryAvailableBytes: valid.ordinaryAvailableBytes,
            importantAvailableBytes: valid.importantAvailableBytes,
            headlineAvailableBytes: valid.headlineAvailableBytes,
            headlineSource: valid.headlineSource,
            pressure: valid.pressure,
            previousDurablePressure: valid.previousDurablePressure,
            criticalBoundaryBytes: valid.criticalBoundaryBytes,
            warningBoundaryBytes: valid.warningBoundaryBytes,
            historyDisposition: .notStoredMissingOrdinaryAvailability
        )

        XCTAssertTrue(EngineService.hasConsistentHistoryEvidence(valid, observation: snapshot))
        XCTAssertFalse(
            EngineService.hasConsistentHistoryEvidence(contradictory, observation: snapshot)
        )
    }

    @MainActor
    func testAppModelPublishesLoadedStateOnTheMainActor() async {
        guard let fixture = try? TestEngineFixture() else {
            return XCTFail("Expected a temporary engine")
        }
        let model = AppModel(engineService: EngineService(engine: fixture.engine))

        await model.loadEngineStatus()

        guard case let .loaded(result) = model.engineState else {
            return XCTFail("Expected the app model to publish a loaded result")
        }
        XCTAssertTrue(result.executedOffMainThread)
    }

    func testEngineCloseIsIdempotentAndRejectsUseAfterClose() throws {
        let baseline = liveEngineInstanceCount()
        weak var weakEngine: DuxEngine?

        do {
            let fixture = try TestEngineFixture()
            let engine = fixture.engine
            weakEngine = engine

            XCTAssertEqual(liveEngineInstanceCount(), baseline + 1)
            XCTAssertEqual(try engine.libraryVersion().ffiContractVersion, 31)
            XCTAssertTrue(engine.close())
            XCTAssertTrue(engine.close())
            XCTAssertThrowsError(try engine.formatSize(bytes: 1_536)) { error in
                XCTAssertEqual(error as? EngineError, .Closed)
            }
        }

        XCTAssertNil(weakEngine)
        XCTAssertEqual(liveEngineInstanceCount(), baseline)
    }

    func testEngineServiceMapsClosedError() async {
        guard let fixture = try? TestEngineFixture() else {
            return XCTFail("Expected a temporary engine")
        }
        let engine = fixture.engine
        let service = EngineService(engine: engine)

        let firstClose = await service.close()
        let secondClose = await service.close()
        XCTAssertTrue(firstClose)
        XCTAssertTrue(secondClose)

        do {
            _ = try await service.loadStatus()
            XCTFail("Expected a closed engine error")
        } catch let error as EngineServiceError {
            XCTAssertEqual(error, .closed)
        } catch {
            XCTFail("Expected EngineServiceError.closed, got \(error)")
        }
    }

    @MainActor
    func testConcurrentSceneLoadsUseOneEngineAndVolumeRequest() async {
        let engineService = CountingEngineService()
        let volumeMonitor = CountingVolumeMonitor()
        let model = AppModel(
            engineService: engineService,
            volumeMonitor: volumeMonitor
        )

        async let menuLoad: Void = model.loadInitialState()
        async let explorerLoad: Void = model.loadInitialState()
        _ = await (menuLoad, explorerLoad)
        await model.loadInitialState()

        let engineLoadCount = await engineService.currentLoadCount()
        let volumeObservationCount = await engineService.currentVolumeObservationCount()
        let volumeLoadCount = await volumeMonitor.currentLoadCount()
        XCTAssertEqual(engineLoadCount, 1)
        XCTAssertEqual(volumeObservationCount, 1)
        XCTAssertEqual(volumeLoadCount, 1)
        guard case .loaded = model.engineState else {
            return XCTFail("Expected one shared loaded engine state")
        }
        guard case .loaded = model.volumeState else {
            return XCTFail("Expected one shared loaded volume state")
        }
    }

    @MainActor
    func testConcurrentVolumeRefreshesCoalesceAndPublishOneResult() async {
        let monitor = ControllableVolumeMonitor()
        let model = AppModel(
            engineService: CountingEngineService(),
            volumeMonitor: monitor
        )
        let expected = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)

        let first = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(1)
        XCTAssertEqual(model.volumeState, .loading)

        let second = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await Task.yield()
        let requestCountWhileCoalesced = await monitor.currentRequestCount()
        XCTAssertEqual(requestCountWhileCoalesced, 1)

        await monitor.succeed(request: 1, with: expected)
        await first.value
        await second.value

        XCTAssertEqual(model.volumeState, .loaded(expected))
        let finalRequestCount = await monitor.currentRequestCount()
        XCTAssertEqual(finalRequestCount, 1)
    }

    @MainActor
    func testCachedRefreshFailureBecomesStaleAndCanRetry() async {
        let monitor = ControllableVolumeMonitor()
        let model = AppModel(
            engineService: CountingEngineService(),
            volumeMonitor: monitor
        )
        let cached = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)
        let refreshed = makeVolumeSnapshot(availableBytes: 40, sampledAt: 2)

        let initial = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(1)
        await monitor.succeed(request: 1, with: cached)
        await initial.value

        let failedRefresh = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(2)
        XCTAssertEqual(model.volumeState, .refreshing(cached))
        await monitor.fail(request: 2, with: VolumeMonitorError.invalidCapacity)
        await failedRefresh.value
        XCTAssertEqual(model.volumeState, .stale(cached, .invalidObservation))

        let retry = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(3)
        XCTAssertEqual(model.volumeState, .refreshing(cached))
        await monitor.succeed(request: 3, with: refreshed)
        await retry.value

        XCTAssertEqual(model.volumeState, .loaded(refreshed))
    }

    @MainActor
    func testUncachedFailureIsTypedAndCanRetry() async {
        let monitor = ControllableVolumeMonitor()
        let model = AppModel(
            engineService: CountingEngineService(),
            volumeMonitor: monitor
        )
        let expected = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)

        let failedRefresh = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(1)
        await monitor.fail(request: 1, with: VolumeMonitorError.missingAvailableCapacity)
        await failedRefresh.value
        XCTAssertEqual(model.volumeState, .failed(.unavailable))

        let retry = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(2)
        XCTAssertEqual(model.volumeState, .loading)
        await monitor.succeed(request: 2, with: expected)
        await retry.value
        XCTAssertEqual(model.volumeState, .loaded(expected))
    }

    @MainActor
    func testCancellationRestoresCacheAndRejectsLatePublication() async {
        let monitor = ControllableVolumeMonitor()
        let model = AppModel(
            engineService: CountingEngineService(),
            volumeMonitor: monitor
        )
        let cached = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)
        let cancelledResult = makeVolumeSnapshot(availableBytes: 5, sampledAt: 2)
        let current = makeVolumeSnapshot(availableBytes: 40, sampledAt: 3)

        let initial = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(1)
        await monitor.succeed(request: 1, with: cached)
        await initial.value

        let cancelledRefresh = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(2)
        XCTAssertEqual(model.volumeState, .refreshing(cached))
        model.cancelVolumeRefresh()
        XCTAssertEqual(model.volumeState, .loaded(cached))

        let replacementRefresh = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(3)
        await monitor.succeed(request: 3, with: current)
        await replacementRefresh.value
        XCTAssertEqual(model.volumeState, .loaded(current))

        await monitor.succeed(request: 2, with: cancelledResult)
        await cancelledRefresh.value
        XCTAssertEqual(model.volumeState, .loaded(current))
    }

    @MainActor
    func testEngineFailureCanRetryWithoutChangingVolumeState() async {
        let engineService = FlakyEngineService()
        let model = AppModel(
            engineService: engineService,
            volumeMonitor: CountingVolumeMonitor()
        )

        await model.loadEngineStatus()
        guard case .failed = model.engineState else {
            return XCTFail("Expected the first engine request to fail")
        }
        XCTAssertEqual(model.volumeState, .idle)

        await model.loadEngineStatus()
        guard case .loaded = model.engineState else {
            return XCTFail("Expected the engine request to retry")
        }
        let engineLoadCount = await engineService.currentLoadCount()
        XCTAssertEqual(engineLoadCount, 2)
        XCTAssertEqual(model.volumeState, .idle)
    }

    func testApplicationRunsAsMenuBarAgent() {
        XCTAssertEqual(Bundle.main.object(forInfoDictionaryKey: "LSUIElement") as? Bool, true)
    }
}

private func assertInvalidCleanupSessionHistory(
    _ history: CleanupSessionHistory,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertThrowsError(
        try CleanupHistoryAdapter.mapSession(
            history,
            requestedSessionID: "session:test"
        ),
        file: file,
        line: line
    ) { error in
        XCTAssertEqual(
            error as? CleanupHistoryServiceError,
            .invalidResponse,
            file: file,
            line: line
        )
    }
}

private func generatedCleanupSessionHistory(
    recordVersion: UInt32 = 1,
    summary: CleanupSessionSummary = generatedCleanupSummary(),
    item: CleanupItemSummary = generatedCleanupItem(),
    items: [CleanupItemSummary]? = nil,
    warnings: [CleanupWarning] = [
        .estimatedBytesUnverified,
        .permanentRemovalCannotBeUndone,
    ]
) -> CleanupSessionHistory {
    CleanupSessionHistory(
        recordVersion: recordVersion,
        summary: summary,
        items: items ?? [item],
        warnings: warnings
    )
}

private func generatedCleanupSummary(
    completedAtUnixMs: Int64? = 3_000,
    planCreatedAtUnixMs: Int64? = 1_000,
    planExpiresAtUnixMs: Int64? = 4_000,
    itemStatusCounts: CleanupStatusCounts = generatedCleanupCounts(removed: 1),
    mode: CleanupMode = .permanentSafe,
    status: CleanupSessionStatus = .completed,
    verifiedCapacityDeltaBytes: Int64? = -512,
    cancellationRequested: Bool? = false
) -> CleanupSessionSummary {
    CleanupSessionSummary(
        recordVersion: 1,
        sessionId: "session:test",
        planId: "plan:test",
        format: .complete,
        sourceScanId: "scan:test",
        startedAtUnixMs: 2_000,
        completedAtUnixMs: completedAtUnixMs,
        planCreatedAtUnixMs: planCreatedAtUnixMs,
        planExpiresAtUnixMs: planExpiresAtUnixMs,
        mode: mode,
        trigger: .manual,
        status: status,
        estimatedBytes: 1_024,
        verifiedCapacityDeltaBytes: verifiedCapacityDeltaBytes,
        cancellationRequested: cancellationRequested,
        itemTotal: 1,
        pathTotal: 2,
        evidenceTotal: 1,
        itemStatusCounts: itemStatusCounts,
        pathStatusCounts: generatedCleanupCounts(removed: 2)
    )
}

private func generatedCleanupItem(
    ordinal: UInt16 = 0,
    category: CandidateCategory? = .developerArtifact,
    action: CandidateAction? = .removeKnownRegenerableContents,
    estimatedBytes: UInt64 = 1_024,
    errorRecorded: Bool = false,
    errorCategory: String? = nil
) -> CleanupItemSummary {
    CleanupItemSummary(
        recordVersion: 1,
        ordinal: ordinal,
        ruleId: "developer.rust.target",
        ruleRevision: 2,
        category: category,
        safety: .safeRegenerable,
        action: action,
        ruleScheduleEligible: true,
        newestMtimeUnixMs: 1_500,
        estimatedBytes: estimatedBytes,
        status: .removed,
        errorRecorded: errorRecorded,
        errorCategory: errorCategory,
        pathCount: 2,
        evidenceCount: 1
    )
}

private func generatedLegacyCleanupSessionHistory(
    warnings: [CleanupWarning] = []
) -> CleanupSessionHistory {
    let summary = CleanupSessionSummary(
        recordVersion: 1,
        sessionId: "session:legacy",
        planId: "plan:legacy",
        format: .legacyIncomplete,
        sourceScanId: nil,
        startedAtUnixMs: 2_000,
        completedAtUnixMs: 3_000,
        planCreatedAtUnixMs: nil,
        planExpiresAtUnixMs: nil,
        mode: .trash,
        trigger: .cli,
        status: .completed,
        estimatedBytes: 1_024,
        verifiedCapacityDeltaBytes: -512,
        cancellationRequested: nil,
        itemTotal: 1,
        pathTotal: 2,
        evidenceTotal: 0,
        itemStatusCounts: generatedCleanupCounts(removed: 1),
        pathStatusCounts: generatedCleanupCounts(removed: 2)
    )
    let item = CleanupItemSummary(
        recordVersion: 1,
        ordinal: 0,
        ruleId: "legacy.rule",
        ruleRevision: 1,
        category: nil,
        safety: nil,
        action: nil,
        ruleScheduleEligible: nil,
        newestMtimeUnixMs: nil,
        estimatedBytes: 1_024,
        status: .removed,
        errorRecorded: true,
        errorCategory: nil,
        pathCount: 2,
        evidenceCount: 0
    )
    return CleanupSessionHistory(
        recordVersion: 1,
        summary: summary,
        items: [item],
        warnings: warnings
    )
}

private func generatedCleanupCounts(
    planned: UInt16 = 0,
    validating: UInt16 = 0,
    dryRun: UInt16 = 0,
    effectStarted: UInt16 = 0,
    trashed: UInt16 = 0,
    removed: UInt16 = 0,
    evicted: UInt16 = 0,
    skipped: UInt16 = 0,
    rejected: UInt16 = 0,
    failed: UInt16 = 0,
    changedSincePlan: UInt16 = 0,
    interrupted: UInt16 = 0,
    unavailable: UInt16 = 0,
    outcomeUnknown: UInt16 = 0
) -> CleanupStatusCounts {
    let total = [
        planned,
        validating,
        dryRun,
        effectStarted,
        trashed,
        removed,
        evicted,
        skipped,
        rejected,
        failed,
        changedSincePlan,
        interrupted,
        unavailable,
        outcomeUnknown,
    ].reduce(0, +)
    return CleanupStatusCounts(
        planned: planned,
        validating: validating,
        dryRun: dryRun,
        effectStarted: effectStarted,
        trashed: trashed,
        removed: removed,
        evicted: evicted,
        skipped: skipped,
        rejected: rejected,
        failed: failed,
        changedSincePlan: changedSincePlan,
        interrupted: interrupted,
        unavailable: unavailable,
        outcomeUnknown: outcomeUnknown,
        total: total
    )
}

private func generatedRustTargetPlanReviewInfo(
    createdAtUnixMS: Int64 = 1_700_000_000_123
) -> RustTargetPlanReviewInfo {
    RustTargetPlanReviewInfo(
        recordVersion: 1,
        planId: "plan:example",
        sourceScanId: "scan:example",
        candidateId: "candidate:example",
        ruleId: "developer.rust.target",
        ruleRevision: 2,
        category: .developerArtifact,
        mode: .permanentSafe,
        safety: .safeRegenerable,
        action: .removeKnownRegenerableContents,
        estimatedBytes: 42,
        warnings: [
            .estimatedBytesUnverified,
            .permanentRemovalCannotBeUndone,
        ],
        createdAtUnixMs: createdAtUnixMS,
        effectiveExpiresAtUnixMs: 1_700_000_600_123,
        scheduleEligible: false,
        itemCount: 1,
        pathCount: 1,
        path: RustTargetPlanReviewPath(
            encoding: .unixBytes,
            encodedBytes: Data("/Users/example/project/target".utf8),
            display: "/Users/example/project/target"
        )
    )
}

private final class RecordingRustTargetPlanReviewEngine: DuxEngine, @unchecked Sendable {
    private let parent: SnapshotReviewSession
    private let plan: RustTargetPlanReviewSession
    private(set) var receivedScanID: String?
    private(set) var receivedParent: SnapshotReviewSession?
    private(set) var receivedRequest: RustTargetPlanReviewRequest?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingRustTargetPlanReviewEngine cannot be lifted: \(handle)")
    }

    init(parent: SnapshotReviewSession, plan: RustTargetPlanReviewSession) {
        self.parent = parent
        self.plan = plan
        super.init(noHandle: NoHandle())
    }

    override func acquireExplorerSnapshotReview(scanId: String) throws
        -> SnapshotReviewSession
    {
        receivedScanID = scanId
        return parent
    }

    override func prepareRustTargetPlanReview(
        parentReview: SnapshotReviewSession,
        request: RustTargetPlanReviewRequest
    ) throws -> RustTargetPlanReviewSession {
        receivedParent = parentReview
        receivedRequest = request
        return plan
    }

    override func close() -> Bool {
        true
    }
}

private final class RecordingGeneratedSnapshotReview:
    SnapshotReviewSession,
    @unchecked Sendable
{
    private(set) var releaseCount = 0

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingGeneratedSnapshotReview cannot be lifted: \(handle)")
    }

    init() {
        super.init(noHandle: NoHandle())
    }

    override func release() throws -> ReviewReleaseOutcome {
        releaseCount += 1
        return releaseCount == 1 ? .released : .alreadyReleased
    }
}

private final class RecordingGeneratedRustTargetPlanReview:
    RustTargetPlanReviewSession,
    @unchecked Sendable
{
    private let record: RustTargetPlanReviewInfo
    private(set) var releaseCount = 0

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingGeneratedRustTargetPlanReview cannot be lifted: \(handle)")
    }

    init(info: RustTargetPlanReviewInfo) {
        record = info
        super.init(noHandle: NoHandle())
    }

    override func info() throws -> RustTargetPlanReviewInfo {
        record
    }

    override func release() throws -> RustTargetPlanReviewReleaseOutcome {
        releaseCount += 1
        return releaseCount == 1 ? .released : .alreadyUnavailable
    }
}

private final class SuspendedRustTargetPlanReviewEngine: DuxEngine, @unchecked Sendable {
    private let parent: SnapshotReviewSession
    private let prepareGate = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var _prepareStarted = false
    private var _closeReached = false

    var prepareStarted: Bool {
        lock.withLock { _prepareStarted }
    }

    var closeReached: Bool {
        lock.withLock { _closeReached }
    }

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("SuspendedRustTargetPlanReviewEngine cannot be lifted: \(handle)")
    }

    init(parent: SnapshotReviewSession) {
        self.parent = parent
        super.init(noHandle: NoHandle())
    }

    override func acquireExplorerSnapshotReview(scanId _: String) throws
        -> SnapshotReviewSession
    {
        parent
    }

    override func prepareRustTargetPlanReview(
        parentReview _: SnapshotReviewSession,
        request _: RustTargetPlanReviewRequest
    ) throws -> RustTargetPlanReviewSession {
        lock.withLock {
            _prepareStarted = true
        }
        prepareGate.wait()
        if closeReached {
            throw RustTargetPlanReviewError.Closed
        }
        throw RustTargetPlanReviewError.InternalState
    }

    override func close() -> Bool {
        lock.withLock {
            _closeReached = true
        }
        return true
    }

    func resumePrepare() {
        prepareGate.signal()
    }
}

private actor CountingEngineService: EngineServing {
    private var loadCount = 0
    private var volumeObservationCount = 0

    func loadStatus() async throws -> EngineStatus {
        loadCount += 1
        await Task.yield()
        return EngineStatus(
            libraryVersion: "test",
            ffiContractVersion: 12,
            executedOffMainThread: true
        )
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        volumeObservationCount += 1
        return snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        testDefaultDiskPressurePolicy()
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: testStoredDiskPressurePolicy(configuration),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(policy: testDefaultDiskPressurePolicy(), changed: true)
    }

    func currentLoadCount() -> Int {
        loadCount
    }

    func currentVolumeObservationCount() -> Int {
        volumeObservationCount
    }
}

private final class TestEngineFixture {
    let engine: DuxEngine
    let homeScanRoot: URL

    private let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory
            .appending(path: "dux-swift-tests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        homeScanRoot = root.appending(path: "home", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(
            at: homeScanRoot,
            withIntermediateDirectories: false
        )
        engine = try DuxEngine(
            storage: EngineStorageRoots(
                dataRoot: root.appending(path: "data", directoryHint: .isDirectory).path,
                cacheRoot: root.appending(path: "cache", directoryHint: .isDirectory).path
            )
        )
    }

    deinit {
        _ = engine.close()
        // DUX-DESTRUCTIVE: allow=test-swift-storage-roots-fixture-remove -- remove only this fixture's UUID-named temporary root
        try? FileManager.default.removeItem(at: root)
    }
}

private final class RecordingScanEngine: DuxEngine, @unchecked Sendable {
    private let task: ScanTask
    private let startRecordVersion: UInt32
    private(set) var request: ScanRequest?
    private(set) var calledOnMain: Bool?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingScanEngine cannot be lifted from an FFI handle: \(handle)")
    }

    init(task: ScanTask, startRecordVersion: UInt32 = 1) {
        self.task = task
        self.startRecordVersion = startRecordVersion
        super.init(noHandle: NoHandle())
    }

    override func startScan(request: ScanRequest) throws -> ScanStart {
        self.request = request
        calledOnMain = Thread.isMainThread
        return ScanStart(
            recordVersion: startRecordVersion,
            disposition: .started,
            task: task
        )
    }
}

private final class ThrowingScanEngine: DuxEngine, @unchecked Sendable {
    private let error: ScanError

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("ThrowingScanEngine cannot be lifted from an FFI handle: \(handle)")
    }

    init(error: ScanError) {
        self.error = error
        super.init(noHandle: NoHandle())
    }

    override func startScan(request: ScanRequest) throws -> ScanStart {
        _ = request
        throw error
    }
}

private final class RecordingGeneratedScanTask: ScanTask, @unchecked Sendable {
    private var polls: [ScanPoll]
    private(set) var pollCalledOnMain: Bool?
    private(set) var cancelCalledOnMain: Bool?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingGeneratedScanTask cannot be lifted from an FFI handle: \(handle)")
    }

    init(polls: [ScanPoll]) {
        self.polls = polls
        super.init(noHandle: NoHandle())
    }

    override func poll() throws -> ScanPoll {
        pollCalledOnMain = Thread.isMainThread
        guard !polls.isEmpty else {
            throw ScanError.InternalState
        }
        return polls.removeFirst()
    }

    override func cancel() throws -> ScanCancelOutcome {
        cancelCalledOnMain = Thread.isMainThread
        return .requested
    }
}

private func generatedActivePoll(
    revision: UInt64,
    progress: ScanProgress? = nil
) -> ScanPoll {
    ScanPoll(
        recordVersion: 1,
        phase: .running,
        stage: .scanning,
        cancellationRequested: false,
        revision: revision,
        progress: progress,
        events: [],
        nextEventSequence: 0,
        oldestAvailableEventSequence: 0,
        eventsTruncated: false,
        failure: nil,
        result: nil
    )
}

private func generatedSuccessPoll(result: ScanTaskResult) -> ScanPoll {
    ScanPoll(
        recordVersion: 1,
        phase: .succeeded,
        stage: .terminal,
        cancellationRequested: false,
        revision: 3,
        progress: nil,
        events: [],
        nextEventSequence: 0,
        oldestAvailableEventSequence: 0,
        eventsTruncated: false,
        failure: nil,
        result: result
    )
}

private func XCTAssertThrowsHomeScanError(
    _ expected: HomeScanServiceError,
    operation: () async throws -> Void,
    file: StaticString = #filePath,
    line: UInt = #line
) async {
    do {
        try await operation()
        XCTFail("Expected HomeScanServiceError.\(expected)", file: file, line: line)
    } catch let error as HomeScanServiceError {
        XCTAssertEqual(error, expected, file: file, line: line)
    } catch {
        XCTFail("Unexpected error: \(error)", file: file, line: line)
    }
}

private final class InvalidPressurePolicyEngine: DuxEngine, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) {
        super.init(unsafeFromHandle: handle)
    }

    init() {
        super.init(noHandle: NoHandle())
    }

    override func getDiskPressurePolicy() throws -> PressurePolicyStatus {
        PressurePolicyStatus(
            recordVersion: 2,
            source: .default,
            revision: 0,
            criticalAvailableBytes: 1,
            criticalAvailableBasisPoints: 1,
            warningAvailableBytes: 2,
            warningAvailableBasisPoints: 2,
            recoveryBytes: 1,
            recoveryBasisPoints: 1,
            updatedAtUnixMs: nil
        )
    }
}

private final class MisleadingDefaultPressurePolicyEngine: DuxEngine, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) {
        super.init(unsafeFromHandle: handle)
    }

    init() {
        super.init(noHandle: NoHandle())
    }

    override func getDiskPressurePolicy() throws -> PressurePolicyStatus {
        misleadingPolicy
    }

    override func resetDiskPressurePolicy() throws -> PressurePolicyUpdate {
        PressurePolicyUpdate(recordVersion: 1, policy: misleadingPolicy, changed: true)
    }

    private var misleadingPolicy: PressurePolicyStatus {
        PressurePolicyStatus(
            recordVersion: 1,
            source: .default,
            revision: 1,
            criticalAvailableBytes: 1,
            criticalAvailableBasisPoints: 1,
            warningAvailableBytes: 2,
            warningAvailableBasisPoints: 2,
            recoveryBytes: 1,
            recoveryBasisPoints: 1,
            updatedAtUnixMs: 0
        )
    }
}

private final class TestStorageRootsFixture {
    let storageRoots: EngineStorageRoots

    private let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory
            .appending(path: "dux-swift-lazy-tests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        storageRoots = EngineStorageRoots(
            dataRoot: root.appending(path: "data", directoryHint: .isDirectory).path,
            cacheRoot: root.appending(path: "cache", directoryHint: .isDirectory).path
        )
    }

    deinit {
        // DUX-DESTRUCTIVE: allow=test-swift-engine-fixture-remove -- remove only this fixture's UUID-named temporary root
        try? FileManager.default.removeItem(at: root)
    }
}

private actor CountingVolumeMonitor: VolumeMonitoring {
    private var loadCount = 0

    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot {
        loadCount += 1
        await Task.yield()
        return makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)
    }

    func currentLoadCount() -> Int {
        loadCount
    }
}

private actor ControllableVolumeMonitor: VolumeMonitoring {
    private var requestCount = 0
    private var requests: [Int: CheckedContinuation<VolumeCapacitySnapshot, any Error>] = [:]
    private var requestWaiters: [
        (expected: Int, continuation: CheckedContinuation<Void, Never>)
    ] = []

    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot {
        requestCount += 1
        let request = requestCount
        resumeSatisfiedWaiters()
        return try await withCheckedThrowingContinuation { continuation in
            requests[request] = continuation
        }
    }

    func waitForRequest(_ expected: Int) async {
        guard requestCount < expected else {
            return
        }
        await withCheckedContinuation { continuation in
            requestWaiters.append((expected, continuation))
        }
    }

    func currentRequestCount() -> Int {
        requestCount
    }

    func succeed(request: Int, with snapshot: VolumeCapacitySnapshot) {
        guard let continuation = requests.removeValue(forKey: request) else {
            preconditionFailure("No pending volume request \(request)")
        }
        continuation.resume(returning: snapshot)
    }

    func fail(request: Int, with error: any Error) {
        guard let continuation = requests.removeValue(forKey: request) else {
            preconditionFailure("No pending volume request \(request)")
        }
        continuation.resume(throwing: error)
    }

    private func resumeSatisfiedWaiters() {
        let satisfied = requestWaiters.filter { $0.expected <= requestCount }
        requestWaiters.removeAll { $0.expected <= requestCount }
        for waiter in satisfied {
            waiter.continuation.resume()
        }
    }
}

private actor FlakyEngineService: EngineServing {
    private var loadCount = 0

    func loadStatus() async throws -> EngineStatus {
        loadCount += 1
        guard loadCount > 1 else {
            throw EngineServiceError.unexpected("transient test failure")
        }
        return EngineStatus(
            libraryVersion: "test",
            ffiContractVersion: 12,
            executedOffMainThread: true
        )
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        testDefaultDiskPressurePolicy()
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: testStoredDiskPressurePolicy(configuration),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(policy: testDefaultDiskPressurePolicy(), changed: true)
    }

    func currentLoadCount() -> Int {
        loadCount
    }
}

private func makeVolumeSnapshot(
    availableBytes: UInt64,
    sampledAt: TimeInterval
) -> VolumeCapacitySnapshot {
    VolumeCapacitySnapshot(
        stableVolumeID: "test-volume",
        displayName: "Test Volume",
        filesystem: "APFS",
        isInternal: true,
        isRemovable: false,
        totalBytes: 100,
        filesystemAvailableBytes: 20,
        importantAvailableBytes: availableBytes,
        effectiveAvailableBytes: availableBytes,
        availabilityBasis: .importantUsage,
        pressure: .unknown,
        criticalBoundaryBytes: nil,
        warningBoundaryBytes: nil,
        historyDisposition: nil,
        sampledAt: Date(timeIntervalSince1970: sampledAt)
    )
}

private func testDefaultDiskPressurePolicy() -> DiskPressurePolicy {
    DiskPressurePolicy(
        source: .default,
        revision: 0,
        configuration: .defaults,
        updatedAtUnixMilliseconds: nil
    )
}

private func testStoredDiskPressurePolicy(
    _ configuration: DiskPressurePolicyConfiguration
) -> DiskPressurePolicy {
    DiskPressurePolicy(
        source: .stored,
        revision: 1,
        configuration: configuration,
        updatedAtUnixMilliseconds: 1
    )
}
