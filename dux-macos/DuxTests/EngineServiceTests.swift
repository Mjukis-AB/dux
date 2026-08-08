import Darwin
@testable import DUX
import Foundation
import XCTest

private func canonicalTestPath(_ url: URL) -> String {
    var buffer = [CChar](repeating: 0, count: Int(PATH_MAX))
    let resolved = buffer.withUnsafeMutableBufferPointer { output in
        url.path.withCString { realpath($0, output.baseAddress) }
    }
    precondition(resolved != nil)
    let end = buffer.firstIndex(of: 0) ?? buffer.endIndex
    return String(decoding: buffer[..<end].map(UInt8.init(bitPattern:)), as: UTF8.self)
}

private let targetedTestVolumeID =
    "volume:macos:01234567-89ab-cdef-0123-456789abcdef"

private let targetedTestCatalogDigest = Data(repeating: 0xA5, count: 32)

private func generatedTargetedRootCatalog() -> TargetedReclaimRootCatalog {
    TargetedReclaimRootCatalog(
        recordVersion: 1,
        knownRootsPolicyRevision: 1,
        configuredRootsRevision: 7,
        knownUserLibraryCachesIncluded: false,
        rootCount: 4,
        digestSha256: targetedTestCatalogDigest
    )
}

private func generatedTargetedPressure(
    anchorUnixMS: Int64 = 4000,
    pressure: TargetedProjectScanPressure = .warning
) -> TargetedProjectScanPressureContext {
    TargetedProjectScanPressureContext(
        recordVersion: 1,
        stableVolumeId: targetedTestVolumeID,
        capacityAnchorUnixMs: anchorUnixMS,
        pressure: pressure,
        currentEpisodeStartedAtUnixMs: 2000,
        pressureStartedAtUnixMs: 1000,
        policyRevision: 9
    )
}

private func generatedTargetedAdmission(
    pressure: TargetedProjectScanPressureContext,
    disposition: TargetedProjectScanDisposition,
    task: ScanTask? = nil,
    existingPhase: TaskPhase? = nil,
    rootUnavailableReason: TargetedProjectScanRootUnavailableReason? = nil,
    currentResult: ScanTaskResult? = nil,
    maxNodes: UInt32 = 50000,
    rootCatalog: TargetedReclaimRootCatalog = generatedTargetedRootCatalog(),
    kind: TargetedReclaimRootKind = .configuredProject,
    rootPath: String = "/Users/example/project"
) -> TargetedProjectScanAdmission {
    TargetedProjectScanAdmission(
        recordVersion: 1,
        configuredRootsRevision: rootCatalog.configuredRootsRevision,
        rootCount: rootCatalog.rootCount,
        rootCatalog: rootCatalog,
        selection: TargetedProjectScanSelection(
            recordVersion: 1,
            ordinal: 0,
            kind: kind,
            root: ConfiguredProjectRootPath(
                encoding: .unixBytes,
                encodedBytes: Data(rootPath.utf8)
            ),
            maxNodes: maxNodes
        ),
        pressure: pressure,
        disposition: disposition,
        rootUnavailableReason: rootUnavailableReason,
        currentResult: currentResult,
        task: task,
        existingTaskObservedPhase: existingPhase
    )
}

private func generatedTargetedCurrentResult() -> ScanTaskResult {
    ScanTaskResult(
        recordVersion: 1,
        scanId: "scan:targeted:fixture",
        startedAtUnixMs: 2500,
        completedAtUnixMs: 3000,
        status: .succeeded,
        directoryCount: 4,
        fileCount: 8,
        logicalBytes: 4096,
        allocatedBytes: 3072,
        snapshotAvailable: true,
        coverage: ScanCoverageSummary(
            recordVersion: 1,
            status: .complete,
            measuredPermille: 1000,
            issueRecordCount: 0,
            issueOccurrenceCount: 0
        ),
        candidateEvaluation: ScanCandidateEvaluationSummary(
            recordVersion: 1,
            status: .succeeded,
            candidateCount: 2,
            failure: nil
        )
    )
}

private func generatedEmergencyRecoveryOrdering(
    pressure: TargetedProjectScanPressureContext,
    recordVersion: UInt32 = 1,
    observedAtUnixMS: Int64 = 5000
) -> EmergencyRecoveryOrdering {
    EmergencyRecoveryOrdering(
        recordVersion: recordVersion,
        policyRevision: 1,
        pressure: pressure,
        rootCatalog: generatedTargetedRootCatalog(),
        observedRootCount: 3,
        candidateEvaluatedRootCount: 2,
        unavailableRootCount: 1,
        groups: [
            EmergencyRecoveryGroup(
                recordVersion: 1,
                rank: 0,
                lane: .staleSafeRegenerable,
                ruleId: "cargo-target",
                ruleRevision: 3,
                category: .developerArtifact,
                unavailableRootCount: 0,
                sources: [
                    EmergencyRecoverySource(
                        recordVersion: 1,
                        rootOrdinal: 0,
                        scanId: "scan:targeted:cargo",
                        observedAtUnixMs: observedAtUnixMS,
                        candidateCount: 4,
                        blockedCandidateCount: 1,
                        permissionIssueCount: nil
                    ),
                ]
            ),
            EmergencyRecoveryGroup(
                recordVersion: 1,
                rank: 1,
                lane: .guidedExploration,
                ruleId: nil,
                ruleRevision: nil,
                category: nil,
                unavailableRootCount: 0,
                sources: [
                    EmergencyRecoverySource(
                        recordVersion: 1,
                        rootOrdinal: 1,
                        scanId: "scan:targeted:explore",
                        observedAtUnixMs: observedAtUnixMS + 500,
                        candidateCount: nil,
                        blockedCandidateCount: nil,
                        permissionIssueCount: nil
                    ),
                ]
            ),
            EmergencyRecoveryGroup(
                recordVersion: 1,
                rank: 2,
                lane: .permissionGap,
                ruleId: nil,
                ruleRevision: nil,
                category: nil,
                unavailableRootCount: 1,
                sources: []
            ),
        ]
    )
}

final class EngineServiceTests: XCTestCase {
    func testICloudObservationSourceBridgeUsesBoundReviewAndExactRequest() async throws {
        let parent = RecordingGeneratedSnapshotReview(
            iCloudObservationSource: generatedICloudObservationSource()
        )
        let engine = RecordingSnapshotReviewEngine(parent: parent)
        let service = EngineService(engine: engine)
        let lease = try await service.acquireExplorerReview(scanID: "scan:example")

        let source = try await lease.icloudObservationSource(
            scopeNodeID: 42,
            maxResults: 32
        )

        XCTAssertEqual(engine.receivedScanID, "scan:example")
        XCTAssertEqual(parent.receivedICloudObservationRequest?.recordVersion, 1)
        XCTAssertEqual(parent.receivedICloudObservationRequest?.scopeNodeId, 42)
        XCTAssertEqual(parent.receivedICloudObservationRequest?.maxResults, 32)
        XCTAssertEqual(source.scanID, "scan:example")
        XCTAssertEqual(source.scopeNodeID, 42)
        XCTAssertEqual(source.requestedMaxResults, 32)
        XCTAssertTrue(source.targets.isEmpty)

        await lease.release()
        XCTAssertEqual(parent.releaseCount, 1)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testICloudObservationSourceBridgeMapsTypedFailures() async throws {
        let cases: [(EngineError, ExplorerICloudObservationSourceError)] = [
            (.ReviewExpired, .expired),
            (.SnapshotNodeNotFound, .invalidRequest),
            (.SnapshotNodeNotDirectory, .invalidRequest),
            (.InvalidSnapshotICloudObservationSourceRequest, .invalidRequest),
            (.BudgetExceeded, .budgetExceeded),
            (.StorageUnavailable, .unavailable),
            (.InternalState, .invalidResponse),
        ]

        for (ffiError, expected) in cases {
            let parent = RecordingGeneratedSnapshotReview(
                iCloudObservationSourceError: ffiError
            )
            let service = EngineService(
                engine: RecordingSnapshotReviewEngine(parent: parent)
            )
            let lease = try await service.acquireExplorerReview(scanID: "scan:example")

            do {
                _ = try await lease.icloudObservationSource(
                    scopeNodeID: 42,
                    maxResults: 32
                )
                XCTFail("Expected \(ffiError) to fail")
            } catch {
                XCTAssertEqual(
                    error as? ExplorerICloudObservationSourceError,
                    expected
                )
            }
            await lease.release()
            let closed = await service.close()
            XCTAssertTrue(closed)
        }
    }

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
        XCTAssertEqual(info.createdAt.nanoseconds, 123_456_789)
        XCTAssertEqual(info.newestMtime.secondsSinceUnixEpoch, 1_699_395_200)
        XCTAssertEqual(info.newestMtime.nanoseconds, 123_456_789)
        XCTAssertEqual(info.minimumAgeSeconds, 604_800)
        XCTAssertEqual(info.minimumAgeNanoseconds, 0)
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

    func testRustTargetCleanupBridgeConsumesExactReviewAndMapsTerminalResult() async throws {
        let parent = RecordingGeneratedSnapshotReview()
        let plan = RecordingGeneratedRustTargetPlanReview(
            info: generatedRustTargetPlanReviewInfo()
        )
        let cleanup = RecordingGeneratedRustTargetCleanupTask(
            polls: [
                RustTargetCleanupPoll(
                    recordVersion: 1,
                    phase: .succeeded,
                    cancellationRequested: false,
                    revision: 3,
                    failure: nil,
                    result: RustTargetCleanupResult(
                        recordVersion: 1,
                        sessionId: "cleanup:rust-target:0123456789abcdef0123456789abcdef",
                        status: .completed,
                        removedEntries: 7,
                        removedLogicalBytes: 4096,
                        verifiedCapacityDeltaBytes: 3000
                    )
                ),
            ]
        )
        let engine = RecordingRustTargetPlanReviewEngine(
            parent: parent,
            plan: plan,
            cleanupTask: cleanup
        )
        let service = EngineService(engine: engine)
        let lease = try await service.acquireExplorerReview(scanID: "scan:example")
        let session = try await lease.prepareRustTargetPlanReview(
            candidateID: "candidate:example"
        )

        let task = try await session.startCleanup()
        let poll = try await task.poll()
        await session.release()

        XCTAssertTrue(engine.receivedCleanupReview === plan)
        XCTAssertEqual(plan.releaseCount, 0)
        XCTAssertEqual(poll.phase, .succeeded)
        XCTAssertEqual(poll.result?.status, .completed)
        XCTAssertEqual(poll.result?.removedEntries, 7)
        XCTAssertEqual(poll.result?.removedLogicalBytes, 4096)
        XCTAssertEqual(poll.result?.verifiedCapacityDeltaBytes, 3000)
        XCTAssertEqual(
            poll.result?.sessionID,
            "cleanup:rust-target:0123456789abcdef0123456789abcdef"
        )
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testRustTargetCleanupAdapterRejectsMalformedAndRegressingPolls() throws {
        let running = try EngineRustTargetCleanupAdapter.map(
            RustTargetCleanupPoll(
                recordVersion: 1,
                phase: .running,
                cancellationRequested: true,
                revision: 2,
                failure: nil,
                result: nil
            ),
            after: nil
        )
        XCTAssertThrowsError(
            try EngineRustTargetCleanupAdapter.map(
                RustTargetCleanupPoll(
                    recordVersion: 1,
                    phase: .queued,
                    cancellationRequested: false,
                    revision: 3,
                    failure: nil,
                    result: nil
                ),
                after: running
            )
        )
        XCTAssertThrowsError(
            try EngineRustTargetCleanupAdapter.map(
                RustTargetCleanupPoll(
                    recordVersion: 1,
                    phase: .failed,
                    cancellationRequested: false,
                    revision: 1,
                    failure: .outcomeUnknown,
                    result: RustTargetCleanupResult(
                        recordVersion: 1,
                        sessionId: "cleanup:rust-target:0123456789abcdef0123456789abcdef",
                        status: .recovering,
                        removedEntries: 1,
                        removedLogicalBytes: 0,
                        verifiedCapacityDeltaBytes: nil
                    )
                ),
                after: nil
            )
        )
    }

    func testRustTargetDryRunBridgeConsumesExactReviewAndMapsPathFreeResult() async throws {
        let parent = RecordingGeneratedSnapshotReview()
        let plan = RecordingGeneratedRustTargetPlanReview(
            info: generatedRustTargetPlanReviewInfo()
        )
        let dryRun = RecordingGeneratedRustTargetDryRunTask(
            polls: [
                RustTargetDryRunPoll(
                    recordVersion: 1,
                    phase: .succeeded,
                    cancellationRequested: false,
                    revision: 3,
                    failure: nil,
                    result: RustTargetDryRunResult(
                        recordVersion: 1,
                        sessionId: "cleanup:rust-target-dry-run:0123456789abcdef0123456789abcdef",
                        status: .dryRun
                    )
                ),
            ]
        )
        let engine = RecordingRustTargetPlanReviewEngine(
            parent: parent,
            plan: plan,
            dryRunTask: dryRun
        )
        let service = EngineService(engine: engine)
        let lease = try await service.acquireExplorerReview(scanID: "scan:example")
        let session = try await lease.prepareRustTargetPlanReview(
            candidateID: "candidate:example"
        )

        let task = try await session.startDryRun()
        let poll = try await task.poll()
        await session.release()

        XCTAssertTrue(engine.receivedDryRunReview === plan)
        XCTAssertEqual(plan.releaseCount, 0)
        XCTAssertEqual(poll.phase, .succeeded)
        XCTAssertEqual(poll.result?.status, .dryRun)
        XCTAssertEqual(
            poll.result?.sessionID,
            "cleanup:rust-target-dry-run:0123456789abcdef0123456789abcdef"
        )
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testRustTargetDryRunAdapterRejectsMalformedAndRegressingPolls() throws {
        let running = try EngineRustTargetDryRunAdapter.map(
            RustTargetDryRunPoll(
                recordVersion: 1,
                phase: .running,
                cancellationRequested: true,
                revision: 2,
                failure: nil,
                result: nil
            ),
            after: nil
        )
        XCTAssertThrowsError(
            try EngineRustTargetDryRunAdapter.map(
                RustTargetDryRunPoll(
                    recordVersion: 1,
                    phase: .queued,
                    cancellationRequested: false,
                    revision: 3,
                    failure: nil,
                    result: nil
                ),
                after: running
            )
        )
        XCTAssertThrowsError(
            try EngineRustTargetDryRunAdapter.map(
                RustTargetDryRunPoll(
                    recordVersion: 1,
                    phase: .succeeded,
                    cancellationRequested: false,
                    revision: 1,
                    failure: nil,
                    result: RustTargetDryRunResult(
                        recordVersion: 1,
                        sessionId: "cleanup:rust-target:0123456789abcdef0123456789abcdef",
                        status: .dryRun
                    )
                ),
                after: nil
            )
        )
        XCTAssertThrowsError(
            try EngineRustTargetDryRunAdapter.map(
                RustTargetDryRunPoll(
                    recordVersion: 1,
                    phase: .failed,
                    cancellationRequested: false,
                    revision: 1,
                    failure: .historyUnresolved,
                    result: RustTargetDryRunResult(
                        recordVersion: 1,
                        sessionId: "cleanup:rust-target-dry-run:0123456789abcdef0123456789abcdef",
                        status: .dryRun
                    )
                ),
                after: nil
            )
        )
    }

    func testRustTargetPlanReviewBridgeRejectsInvalidGeneratedTimestamp() {
        XCTAssertThrowsError(
            try EngineRustTargetPlanReviewAdapter.map(
                generatedRustTargetPlanReviewInfo(
                    createdAt: SnapshotNodeTimestamp(
                        secondsSinceUnixEpoch: 1_700_000_000,
                        nanoseconds: 1_000_000_000
                    )
                )
            )
        ) { error in
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .invalidResponse
            )
        }
        XCTAssertThrowsError(
            try EngineRustTargetPlanReviewAdapter.map(
                generatedRustTargetPlanReviewInfo(
                    newestMtime: SnapshotNodeTimestamp(
                        secondsSinceUnixEpoch: 1_699_395_200,
                        nanoseconds: 1_000_000_000
                    )
                )
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
        for _ in 0 ..< 2000 where !engine.prepareStarted {
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
        XCTAssertEqual(status.ffiContractVersion, 55)
        XCTAssertEqual(status.databaseSchemaVersion, 18)
        XCTAssertEqual(status.snapshotFormatVersion, 1)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testLoadsTypedRustValuesOffTheMainThread() async throws {
        let fixture = try TestEngineFixture()
        let result = try await EngineService(engine: fixture.engine).loadStatus()

        XCTAssertEqual(result.libraryVersion, "0.5.0")
        XCTAssertEqual(result.ffiContractVersion, 55)
        XCTAssertEqual(result.databaseSchemaVersion, 18)
        XCTAssertEqual(result.snapshotFormatVersion, 1)
        XCTAssertTrue(result.executedOffMainThread)
    }

    func testCleanupHistorySummaryFeedIsBoundedAndReadOnly() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)

        let page = try await service.loadRecentCleanupHistory(cursor: nil, limit: 64)
        XCTAssertTrue(page.records.isEmpty)
        XCTAssertNil(page.nextCursor)
        let ranking = try await service.loadRecurringStorageThieves()
        XCTAssertEqual(ranking.permanentSafeSessionCount, 0)
        XCTAssertEqual(ranking.manualCleanupSessionCount, 0)
        XCTAssertEqual(ranking.rankedRuleCount, 0)
        XCTAssertFalse(ranking.hasOlderPermanentSafeSessions)
        XCTAssertTrue(ranking.groups.isEmpty)
        let recoveryDebt = try await service.loadPersistentRecoveryDebt()
        XCTAssertEqual(recoveryDebt.inspectedUnclaimedCount, 0)
        XCTAssertEqual(recoveryDebt.pristineUnclaimedCount, 0)
        XCTAssertEqual(recoveryDebt.unexplainedUnclaimedCount, 0)
        XCTAssertFalse(recoveryDebt.hasMore)
        let claimedProvenance = try await service.loadClaimedRunningScanProvenance()
        XCTAssertEqual(claimedProvenance.inspectedClaimedCount, 0)
        XCTAssertEqual(claimedProvenance.sameHostCurrentBootCount, 0)
        XCTAssertEqual(claimedProvenance.sameHostPriorBootCount, 0)
        XCTAssertEqual(claimedProvenance.foreignHostCount, 0)
        XCTAssertEqual(claimedProvenance.storedUnprovenCount, 0)
        XCTAssertEqual(claimedProvenance.currentContextUnavailableCount, 0)
        XCTAssertFalse(claimedProvenance.hasMore)

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
        do {
            _ = try await service.loadRecurringStorageThieves()
            XCTFail("Expected the closed engine to reject recurring-growth reads")
        } catch let error as CleanupHistoryServiceError {
            XCTAssertEqual(error, .closed)
        }
        do {
            _ = try await service.loadPersistentRecoveryDebt()
            XCTFail("Expected the closed engine to reject recovery diagnostics")
        } catch let error as PersistentRecoveryDebtServiceError {
            XCTAssertEqual(error, .closed)
        }
        do {
            _ = try await service.loadClaimedRunningScanProvenance()
            XCTFail("Expected the closed engine to reject claimed provenance diagnostics")
        } catch let error as ClaimedRunningScanProvenanceServiceError {
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
            startedAtUnixMs: 1000,
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
            startedAtUnixMs: 1000,
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
                item: generatedCleanupItem(estimatedBytes: 2048)
            )
        )
        let overflowingSummary = CleanupSessionSummary(
            recordVersion: 1,
            sessionId: "session:test",
            planId: "plan:test",
            format: .complete,
            sourceScanId: "scan:test",
            startedAtUnixMs: 2000,
            completedAtUnixMs: 3000,
            planCreatedAtUnixMs: 1000,
            planExpiresAtUnixMs: 4000,
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
                summary: generatedCleanupSummary(planExpiresAtUnixMs: 2000)
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
                summary: generatedCleanupSummary(planCreatedAtUnixMs: 2001)
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

    func testRuleOutcomeAdapterMapsEveryStateAndIneligibilityReason() throws {
        let detail = try generatedCleanupHistoryDetail()
        let millisecondsDate: (Int64) -> Date = {
            Date(timeIntervalSince1970: Double($0) / 1000)
        }
        let reasonCases: [
            (RuleOutcomeNotEligibleReason, CleanupHistoryRuleOutcomeNotEligibleReason)
        ] = [
            (.sourceCleanupIncomplete, .sourceCleanupIncomplete),
            (
                .itemNotSuccessfulPermanentRegenerable,
                .itemNotSuccessfulPermanentRegenerable
            ),
            (.sourceScanNotComparable, .sourceScanNotComparable),
            (.sourceEvaluationNotComparable, .sourceEvaluationNotComparable),
            (.sourceEvaluationAfterPlan, .sourceEvaluationAfterPlan),
            (.sourceCandidateMismatch, .sourceCandidateMismatch),
        ]
        for (raw, expected) in reasonCases {
            let mapped = try CleanupHistoryAdapter.mapRuleOutcomes(
                generatedRuleOutcomeBatch(state: .notEligible(reason: raw)),
                requestedSessionID: "session:test",
                detail: detail
            )
            XCTAssertEqual(mapped.outcomes.first?.state, .notEligible(reason: expected))
        }

        let stateCases: [(RuleOutcomeState, CleanupHistoryRuleOutcomeState)] = [
            (
                .awaitingComparableScan(cleanedAtUnixMs: 3000),
                .awaitingComparableScan(cleanedAt: millisecondsDate(3000))
            ),
            (
                .superseded(cleanedAtUnixMs: 3000, supersededAtUnixMs: 4000),
                .superseded(
                    cleanedAt: millisecondsDate(3000),
                    supersededAt: millisecondsDate(4000)
                )
            ),
            (
                .laterSizeObserved(
                    cleanedAtUnixMs: 3000,
                    observedAtUnixMs: 4000,
                    observedBytes: 2048
                ),
                .laterSizeObserved(
                    cleanedAt: millisecondsDate(3000),
                    observedAt: millisecondsDate(4000),
                    observedBytes: 2048
                )
            ),
            (
                .zeroBaselineObserved(
                    cleanedAtUnixMs: 3000,
                    observedAtUnixMs: 4000
                ),
                .zeroBaselineObserved(
                    cleanedAt: millisecondsDate(3000),
                    observedAt: millisecondsDate(4000)
                )
            ),
            (
                .regrown(
                    cleanedAtUnixMs: 3000,
                    zeroObservedAtUnixMs: 4000,
                    observedAtUnixMs: 5000,
                    observedBytes: 4096
                ),
                .regrown(
                    cleanedAt: millisecondsDate(3000),
                    zeroObservedAt: millisecondsDate(4000),
                    observedAt: millisecondsDate(5000),
                    observedBytes: 4096
                )
            ),
        ]
        for (raw, expected) in stateCases {
            let mapped = try CleanupHistoryAdapter.mapRuleOutcomes(
                generatedRuleOutcomeBatch(state: raw),
                requestedSessionID: "session:test",
                detail: detail
            )
            XCTAssertEqual(mapped.outcomes.first?.state, expected)
        }
    }

    func testRuleOutcomeAdapterBindsExactDetailAndRejectsMalformedGraphs() throws {
        let detail = try generatedCleanupHistoryDetail()
        let valid = generatedRuleOutcomeBatch(
            state: .zeroBaselineObserved(
                cleanedAtUnixMs: 3000,
                observedAtUnixMs: 4000
            )
        )
        let mapped = try CleanupHistoryAdapter.mapRuleOutcomes(
            valid,
            requestedSessionID: "session:test",
            detail: detail
        )
        XCTAssertEqual(mapped.sessionID, detail.summary.sessionID)
        XCTAssertEqual(mapped.outcomes.map(\.itemOrdinal), detail.items.map(\.ordinal))
        XCTAssertEqual(mapped.outcomes.first?.ruleID, detail.items.first?.ruleID)
        XCTAssertEqual(mapped.outcomes.first?.ruleRevision, detail.items.first?.ruleRevision)

        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(recordVersion: 2),
            detail: detail
        )
        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(outcomeRecordVersion: 2),
            detail: detail
        )
        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(sessionID: "session:different"),
            detail: detail
        )
        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(outcomes: []),
            detail: detail
        )
        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(itemOrdinal: 1),
            detail: detail
        )
        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(ruleID: "developer.other.rule"),
            detail: detail
        )
        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(ruleID: "not/a/stable/token"),
            detail: detail
        )
        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(ruleRevision: 0),
            detail: detail
        )
        assertInvalidRuleOutcomeBatch(
            generatedRuleOutcomeBatch(ruleRevision: 3),
            detail: detail
        )

        let otherDetail = try generatedCleanupHistoryDetail(sessionID: "session:other")
        assertInvalidRuleOutcomeBatch(valid, detail: otherDetail)

        let invalidChronologies: [RuleOutcomeState] = [
            .awaitingComparableScan(cleanedAtUnixMs: -1),
            .superseded(cleanedAtUnixMs: 3000, supersededAtUnixMs: 2999),
            .laterSizeObserved(
                cleanedAtUnixMs: 3000,
                observedAtUnixMs: 3000,
                observedBytes: 1
            ),
            .laterSizeObserved(
                cleanedAtUnixMs: 3000,
                observedAtUnixMs: 4000,
                observedBytes: 0
            ),
            .zeroBaselineObserved(
                cleanedAtUnixMs: 3000,
                observedAtUnixMs: 3000
            ),
            .regrown(
                cleanedAtUnixMs: 3000,
                zeroObservedAtUnixMs: 3000,
                observedAtUnixMs: 4000,
                observedBytes: 1
            ),
            .regrown(
                cleanedAtUnixMs: 3000,
                zeroObservedAtUnixMs: 4000,
                observedAtUnixMs: 4000,
                observedBytes: 1
            ),
            .regrown(
                cleanedAtUnixMs: 3000,
                zeroObservedAtUnixMs: 4000,
                observedAtUnixMs: 5000,
                observedBytes: 0
            ),
        ]
        for state in invalidChronologies {
            assertInvalidRuleOutcomeBatch(
                generatedRuleOutcomeBatch(state: state),
                detail: detail
            )
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
            criticalAvailableBasisPoints: 9998,
            warningAvailableBytes: UInt64.max - (1 << 30),
            warningAvailableBasisPoints: 9999,
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
            warningAvailableBasisPoints: 1000,
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

    func testRealSnapshotRetentionCapRoundTripPreservesZeroMaximumAndProvenance() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)

        let initial = try await service.loadSnapshotRetentionCap()
        XCTAssertEqual(
            initial,
            SnapshotRetentionCapModel(
                capBytes: 2 * DiskPressurePolicyConfiguration.bytesPerGiB,
                source: .default,
                updatedAtUnixMilliseconds: nil
            )
        )

        let zero = try await service.setSnapshotRetentionCap(0)
        XCTAssertTrue(zero.changed)
        XCTAssertEqual(zero.settings.capBytes, 0)
        XCTAssertEqual(zero.settings.source, .stored)
        XCTAssertNotNil(zero.settings.updatedAtUnixMilliseconds)

        let exactZero = try await service.setSnapshotRetentionCap(0)
        XCTAssertFalse(exactZero.changed)
        XCTAssertEqual(exactZero.settings, zero.settings)

        let maximum = try await service.setSnapshotRetentionCap(UInt64.max)
        XCTAssertTrue(maximum.changed)
        XCTAssertEqual(maximum.settings.capBytes, UInt64.max)
        XCTAssertEqual(maximum.settings.source, .stored)

        let reset = try await service.resetSnapshotRetentionCap()
        XCTAssertTrue(reset.changed)
        XCTAssertEqual(reset.settings, initial)
        let exactReset = try await service.resetSnapshotRetentionCap()
        XCTAssertFalse(exactReset.changed)
        XCTAssertEqual(exactReset.settings, initial)
    }

    func testSnapshotRetentionCapServiceMapsClosedAndRejectsMalformedResponses() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let closed = await service.close()
        XCTAssertTrue(closed)
        do {
            _ = try await service.loadSnapshotRetentionCap()
            XCTFail("Expected a closed snapshot-cap service")
        } catch let error as SnapshotRetentionCapServiceError {
            XCTAssertEqual(error, .closed)
        }

        let malformed = EngineService(engine: InvalidSnapshotRetentionCapEngine())
        do {
            _ = try await malformed.loadSnapshotRetentionCap()
            XCTFail("Expected malformed snapshot-cap response rejection")
        } catch let error as SnapshotRetentionCapServiceError {
            XCTAssertEqual(error, .invalidResponse)
        }
        do {
            _ = try await malformed.resetSnapshotRetentionCap()
            XCTFail("Expected misleading default reset rejection")
        } catch let error as SnapshotRetentionCapServiceError {
            XCTAssertEqual(error, .invalidResponse)
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

    func testMaintenanceAdapterMapsCandidateEvaluationRecoveryOutcomes() async throws {
        let cases: [(MaintenanceOutcome, UInt64, Bool)] = [
            (.candidateEvaluationRecoveryNone, 0, false),
            (.candidateEvaluationRecoveryRecovered, 12, true),
            (.candidateEvaluationRecoveryIncompatible, 0, true),
        ]

        for (outcome, candidateCount, hasMore) in cases {
            let generatedTask = RecordingGeneratedMaintenanceTask(
                polls: [
                    generatedMaintenancePoll(
                        result: generatedCandidateEvaluationRecoveryResult(
                            outcome: outcome,
                            candidateCount: candidateCount,
                            hasMore: hasMore
                        )
                    ),
                ]
            )
            let engine = RecordingMaintenanceEngine(task: generatedTask)
            let admission = await EngineService(engine: engine)
                .startMaintenance(.candidateEvaluationRecovery)
            guard case let .started(task) = admission else {
                return XCTFail("Expected candidate-evaluation recovery task")
            }

            let poll = await task.poll()
            guard case let .finished(signal) = poll else {
                return XCTFail("Expected a valid candidate-evaluation recovery result")
            }

            XCTAssertEqual(signal.hasMore, hasMore)
            XCTAssertNil(signal.deferral)
            XCTAssertEqual(engine.receivedKind, .candidateEvaluationRecovery)
            XCTAssertEqual(engine.calledOnMain, false)
            XCTAssertEqual(generatedTask.pollCalledOnMain, false)
        }
    }

    func testMaintenanceAdapterRejectsMalformedCandidateRecoveryPolls() async {
        let validResult = generatedCandidateEvaluationRecoveryResult(
            outcome: .candidateEvaluationRecoveryRecovered,
            candidateCount: 1,
            hasMore: false
        )
        let malformedPolls = [
            generatedMaintenancePoll(recordVersion: 2, result: validResult),
            generatedMaintenancePoll(kind: .history, result: validResult),
            generatedMaintenancePoll(
                result: generatedCandidateEvaluationRecoveryResult(
                    recordVersion: 2,
                    outcome: .candidateEvaluationRecoveryRecovered,
                    candidateCount: 1,
                    hasMore: false
                )
            ),
            generatedMaintenancePoll(
                result: generatedCandidateEvaluationRecoveryResult(
                    kind: .history,
                    outcome: .candidateEvaluationRecoveryRecovered,
                    candidateCount: 1,
                    hasMore: false
                )
            ),
            generatedMaintenancePoll(
                result: generatedCandidateEvaluationRecoveryResult(
                    outcome: .candidateEvaluationRecoveryRecovered,
                    candidateCount: 1,
                    hasMore: false,
                    observedAtUnixMS: -1
                )
            ),
            generatedMaintenancePoll(
                result: generatedCandidateEvaluationRecoveryResult(
                    outcome: .historyApplied,
                    candidateCount: 0,
                    hasMore: false
                )
            ),
            generatedMaintenancePoll(
                result: generatedCandidateEvaluationRecoveryResult(
                    outcome: .candidateEvaluationRecoveryRecovered,
                    candidateCount: 1,
                    hasMore: false,
                    secondaryCountBefore: 1
                )
            ),
            generatedMaintenancePoll(
                result: generatedCandidateEvaluationRecoveryResult(
                    outcome: .candidateEvaluationRecoveryRecovered,
                    candidateCount: 4097,
                    hasMore: false
                )
            ),
            generatedMaintenancePoll(
                result: generatedCandidateEvaluationRecoveryResult(
                    outcome: .candidateEvaluationRecoveryNone,
                    candidateCount: 0,
                    hasMore: true
                )
            ),
            generatedMaintenancePoll(phase: .running, result: validResult),
            generatedMaintenancePoll(phase: .failed, result: nil),
        ]

        for poll in malformedPolls {
            let generatedTask = RecordingGeneratedMaintenanceTask(polls: [poll])
            let admission = await EngineService(
                engine: RecordingMaintenanceEngine(task: generatedTask)
            ).startMaintenance(.candidateEvaluationRecovery)
            guard case let .started(task) = admission else {
                return XCTFail("Expected generated maintenance task")
            }
            guard case .failed(.blockedUntilRestart) = await task.poll() else {
                return XCTFail("Expected malformed maintenance response to fail closed")
            }
        }
    }

    func testMaintenanceAdapterRejectsMalformedStartRecord() async {
        let generatedTask = RecordingGeneratedMaintenanceTask(
            polls: [
                generatedMaintenancePoll(
                    result: generatedCandidateEvaluationRecoveryResult(
                        outcome: .candidateEvaluationRecoveryNone,
                        candidateCount: 0,
                        hasMore: false
                    )
                ),
            ]
        )
        let engine = RecordingMaintenanceEngine(
            task: generatedTask,
            startRecordVersion: 2
        )

        let admission = await EngineService(engine: engine)
            .startMaintenance(.candidateEvaluationRecovery)

        guard case .failed(.blockedUntilRestart) = admission else {
            return XCTFail("Expected malformed start record to fail closed")
        }
    }

    func testTargetedScanBridgeUsesPathFreeAdmissionAndExactFinalCheckpoint() async throws {
        let pressure = generatedTargetedPressure()
        let task = RecordingGeneratedScanTask(
            polls: [generatedActivePoll(revision: 1)]
        )
        let engine = RecordingTargetedScanEngine(
            admissions: [
                generatedTargetedAdmission(
                    pressure: pressure,
                    disposition: .started,
                    task: task
                ),
            ],
            checkpoint: TargetedProjectScanCheckpoint(
                recordVersion: 1,
                configuredRootsRevision: 7,
                rootCount: 4,
                rootCatalog: generatedTargetedRootCatalog(),
                pressure: pressure
            )
        )
        let service = EngineService(engine: engine)

        let admission = try await service.startTargetedReclaimScan(
            stableVolumeID: targetedTestVolumeID,
            anchorAt: Date(timeIntervalSince1970: 4),
            ordinal: 0,
            expectedRootsRevision: 7,
            expectedRootCatalogDigestSHA256: nil
        )

        guard
            case .started = admission.disposition,
            let context = admission.context
        else {
            return XCTFail("Expected one owned targeted scan task")
        }
        XCTAssertEqual(admission.ordinal, 0)
        XCTAssertEqual(
            admission.root?.path.encodedBytes,
            Data("/Users/example/project".utf8)
        )
        XCTAssertEqual(context.stableVolumeID, targetedTestVolumeID)
        XCTAssertEqual(context.capacityAnchorAt, Date(timeIntervalSince1970: 4))
        XCTAssertEqual(context.pressure, .warning)
        XCTAssertEqual(context.pressureEpisodeStartedAt, Date(timeIntervalSince1970: 2))
        XCTAssertEqual(context.lowPressureSequenceStartedAt, Date(timeIntervalSince1970: 1))
        XCTAssertEqual(context.policyRevision, 9)
        XCTAssertEqual(context.rootsRevision, 7)
        XCTAssertEqual(context.rootCount, 4)
        XCTAssertEqual(
            engine.startRequest,
            TargetedProjectScanRequest(
                recordVersion: 1,
                stableVolumeId: targetedTestVolumeID,
                capacityAnchorUnixMs: 4000,
                selectedRootOrdinal: 0,
                expectedConfiguredRootsRevision: 7,
                expectedRootCatalogDigestSha256: nil
            )
        )
        XCTAssertEqual(engine.startCalledOnMain, false)

        let validated = try await service.validateTargetedReclaimScan(context)

        XCTAssertEqual(validated, context)
        XCTAssertEqual(
            engine.checkpointRequest,
            TargetedProjectScanCheckpointRequest(
                recordVersion: 1,
                expectedPressure: pressure,
                expectedRootCatalog: generatedTargetedRootCatalog()
            )
        )
        XCTAssertEqual(engine.checkpointCalledOnMain, false)
        do {
            _ = try await service.validateTargetedReclaimScan(context)
            XCTFail("Expected the exact checkpoint proof to be consume-on-success")
        } catch {
            XCTAssertEqual(
                error as? TargetedReclaimScanServiceError,
                .invalidResponse
            )
        }
    }

    func testTargetedScanBridgeDistinguishesObservedTasksAndRootFailures() async throws {
        let pressure = generatedTargetedPressure(anchorUnixMS: 4000)
        let task = RecordingGeneratedScanTask(
            polls: [generatedActivePoll(revision: 1)]
        )
        let engine = RecordingTargetedScanEngine(
            admissions: [
                generatedTargetedAdmission(
                    pressure: pressure,
                    disposition: .existingTask,
                    task: task,
                    existingPhase: .running
                ),
                generatedTargetedAdmission(
                    pressure: pressure,
                    disposition: .rootUnavailable,
                    rootUnavailableReason: .volumeMismatch
                ),
            ]
        )
        let service = EngineService(engine: engine)

        let observed = try await service.startTargetedReclaimScan(
            stableVolumeID: targetedTestVolumeID,
            anchorAt: Date(timeIntervalSince1970: 4),
            ordinal: 0,
            expectedRootsRevision: 7,
            expectedRootCatalogDigestSHA256: nil
        )
        guard case .observing = observed.disposition else {
            return XCTFail("Expected an existing targeted task to be non-owning")
        }
        XCTAssertNil(observed.disposition.ownedTask)
        XCTAssertNotNil(observed.disposition.task)

        let unavailable = try await service.startTargetedReclaimScan(
            stableVolumeID: targetedTestVolumeID,
            anchorAt: Date(timeIntervalSince1970: 4),
            ordinal: 0,
            expectedRootsRevision: 7,
            expectedRootCatalogDigestSHA256: targetedTestCatalogDigest
        )
        guard case let .unavailable(failure) = unavailable.disposition else {
            return XCTFail("Expected a root-local failure")
        }
        XCTAssertEqual(failure, .differentVolume)
    }

    func testTargetedScanBridgeMapsOnlyCurrentTargetedResultsAndExactBudgets() async throws {
        let pressure = generatedTargetedPressure()
        let valid = RecordingTargetedScanEngine(
            admissions: [
                generatedTargetedAdmission(
                    pressure: pressure,
                    disposition: .current,
                    currentResult: generatedTargetedCurrentResult()
                ),
            ]
        )
        let admission = try await EngineService(engine: valid).startTargetedReclaimScan(
            stableVolumeID: targetedTestVolumeID,
            anchorAt: Date(timeIntervalSince1970: 4),
            ordinal: 0,
            expectedRootsRevision: 7,
            expectedRootCatalogDigestSHA256: nil
        )
        guard case let .current(result) = admission.disposition else {
            return XCTFail("Expected validated durable targeted evidence")
        }
        XCTAssertTrue(result.succeeded)
        XCTAssertEqual(result.scanID, "scan:targeted:fixture")
        XCTAssertEqual(result.candidateEvaluation, .succeeded(candidateCount: 2))

        let knownCatalog = TargetedReclaimRootCatalog(
            recordVersion: 1,
            knownRootsPolicyRevision: 1,
            configuredRootsRevision: 7,
            knownUserLibraryCachesIncluded: true,
            rootCount: 2,
            digestSha256: Data(repeating: 0xC3, count: 32)
        )
        let knownAdmission = try await EngineService(
            engine: RecordingTargetedScanEngine(
                admissions: [
                    generatedTargetedAdmission(
                        pressure: pressure,
                        disposition: .rootUnavailable,
                        rootUnavailableReason: .accessDenied,
                        maxNodes: 100_000,
                        rootCatalog: knownCatalog,
                        kind: .knownUserLibraryCaches,
                        rootPath: "/Users/example/Library/Caches"
                    ),
                ]
            )
        ).startTargetedReclaimScan(
            stableVolumeID: targetedTestVolumeID,
            anchorAt: Date(timeIntervalSince1970: 4),
            ordinal: 0,
            expectedRootsRevision: 7,
            expectedRootCatalogDigestSHA256: nil
        )
        XCTAssertEqual(knownAdmission.root?.kind, .knownUserLibraryCaches)
        XCTAssertEqual(
            knownAdmission.root?.path.displayText,
            "/Users/example/Library/Caches"
        )
        XCTAssertEqual(
            knownAdmission.context?.rootCatalogDigestSHA256,
            knownCatalog.digestSha256
        )

        let malformed = generatedTargetedAdmission(
            pressure: pressure,
            disposition: .current,
            currentResult: generatedTargetedCurrentResult(),
            maxNodes: 49999
        )
        do {
            _ = try await EngineService(
                engine: RecordingTargetedScanEngine(admissions: [malformed])
            ).startTargetedReclaimScan(
                stableVolumeID: targetedTestVolumeID,
                anchorAt: Date(timeIntervalSince1970: 4),
                ordinal: 0,
                expectedRootsRevision: 7,
                expectedRootCatalogDigestSHA256: nil
            )
            XCTFail("Expected a contradictory Rust node budget to fail closed")
        } catch {
            XCTAssertEqual(
                error as? TargetedReclaimScanServiceError,
                .invalidResponse
            )
        }
    }

    func testTargetedScanBridgeMapsEveryTypedAdmissionError() async {
        let cases: [(TargetedProjectScanError, TargetedReclaimScanServiceError)] = [
            (.Closed, .closed),
            (.InvalidRecordVersion, .invalidRecordVersion),
            (.InvalidVolumeIdentity, .invalidVolumeIdentity),
            (.InvalidAnchor, .invalidAnchor),
            (.InvalidOrdinal, .invalidOrdinal),
            (.InvalidCatalog, .invalidResponse),
            (.RegistryChanged, .configuredRootsChanged),
            (.CatalogChanged, .configuredRootsChanged),
            (.PressureChanged, .pressureChanged),
            (.ReadOnlyStore, .readOnlyStore),
            (.IncompatibleSchema, .incompatibleSchema),
            (.Busy, .busy),
            (.UnsafeStorage, .unsafeStorage),
            (.BudgetExceeded, .budgetExceeded),
            (.CorruptData, .corruptData),
            (.Unavailable, .storageUnavailable),
            (.OutcomeUnknown, .outcomeUnknown),
            (.QueueFull, .queueFull),
            (.TaskIdExhausted, .internalState),
            (.InternalState, .internalState),
        ]

        for (ffiError, expected) in cases {
            do {
                _ = try await EngineService(
                    engine: RecordingTargetedScanEngine(
                        admissions: [],
                        startError: ffiError
                    )
                ).startTargetedReclaimScan(
                    stableVolumeID: targetedTestVolumeID,
                    anchorAt: Date(timeIntervalSince1970: 4),
                    ordinal: 0,
                    expectedRootsRevision: nil,
                    expectedRootCatalogDigestSHA256: nil
                )
                XCTFail("Expected \(ffiError) to be mapped")
            } catch {
                XCTAssertEqual(error as? TargetedReclaimScanServiceError, expected)
            }
        }
    }

    func testEmergencyRecoveryBridgeMapsExactCriticalOrderingAndRequest() async throws {
        let pressure = generatedTargetedPressure(pressure: .critical)
        let engine = RecordingTargetedScanEngine(
            admissions: [
                generatedTargetedAdmission(
                    pressure: pressure,
                    disposition: .rootUnavailable,
                    rootUnavailableReason: .accessDenied
                ),
            ],
            emergencyRecoveryOrderings: [
                generatedEmergencyRecoveryOrdering(pressure: pressure),
            ]
        )
        let service = EngineService(engine: engine)
        let admission = try await service.startTargetedReclaimScan(
            stableVolumeID: targetedTestVolumeID,
            anchorAt: Date(timeIntervalSince1970: 4),
            ordinal: 0,
            expectedRootsRevision: 7,
            expectedRootCatalogDigestSHA256: targetedTestCatalogDigest
        )
        let context = try XCTUnwrap(admission.context)

        let ordering = try await service.finalizeEmergencyRecovery(context)

        XCTAssertEqual(
            engine.emergencyRecoveryRequests,
            [
                EmergencyRecoveryRequest(
                    recordVersion: 1,
                    expectedPressure: pressure,
                    expectedRootCatalog: generatedTargetedRootCatalog()
                ),
            ]
        )
        XCTAssertEqual(engine.emergencyRecoveryCalledOnMain, false)
        XCTAssertEqual(ordering.policyRevision, 1)
        XCTAssertEqual(ordering.context, context)
        XCTAssertEqual(ordering.observedRootCount, 3)
        XCTAssertEqual(ordering.candidateEvaluatedRootCount, 2)
        XCTAssertEqual(ordering.unavailableRootCount, 1)
        XCTAssertEqual(ordering.groups.map(\.rank), [0, 1, 2])
        XCTAssertEqual(
            ordering.groups.map(\.lane),
            [.staleSafeRegenerable, .guidedExploration, .permissionGap]
        )
        XCTAssertEqual(ordering.groups[0].ruleID, "cargo-target")
        XCTAssertEqual(ordering.groups[0].ruleRevision, 3)
        XCTAssertEqual(ordering.groups[0].category, .developerArtifact)
        XCTAssertEqual(ordering.groups[0].sources[0].rootOrdinal, 0)
        XCTAssertEqual(ordering.groups[0].sources[0].scanID, "scan:targeted:cargo")
        XCTAssertEqual(
            ordering.groups[0].sources[0].observedAt,
            Date(timeIntervalSince1970: 5)
        )
        XCTAssertEqual(ordering.groups[0].sources[0].candidateCount, 4)
        XCTAssertEqual(ordering.groups[0].sources[0].blockedCandidateCount, 1)
        XCTAssertEqual(ordering.groups[2].unavailableRootCount, 1)
        XCTAssertTrue(ordering.groups[2].sources.isEmpty)
    }

    func testEmergencyRecoveryMalformedResponseDoesNotConsumeProof() async throws {
        let pressure = generatedTargetedPressure(pressure: .critical)
        let engine = RecordingTargetedScanEngine(
            admissions: [
                generatedTargetedAdmission(
                    pressure: pressure,
                    disposition: .rootUnavailable,
                    rootUnavailableReason: .accessDenied
                ),
            ],
            emergencyRecoveryOrderings: [
                generatedEmergencyRecoveryOrdering(
                    pressure: pressure,
                    observedAtUnixMS: Int64.max - 500
                ),
                generatedEmergencyRecoveryOrdering(pressure: pressure),
            ]
        )
        let service = EngineService(engine: engine)
        let admission = try await service.startTargetedReclaimScan(
            stableVolumeID: targetedTestVolumeID,
            anchorAt: Date(timeIntervalSince1970: 4),
            ordinal: 0,
            expectedRootsRevision: 7,
            expectedRootCatalogDigestSHA256: nil
        )
        let context = try XCTUnwrap(admission.context)

        do {
            _ = try await service.finalizeEmergencyRecovery(context)
            XCTFail("Expected malformed emergency recovery response to fail closed")
        } catch {
            XCTAssertEqual(
                error as? TargetedReclaimScanServiceError,
                .invalidResponse
            )
        }

        let retry = try await service.finalizeEmergencyRecovery(context)
        XCTAssertEqual(retry.context, context)
        XCTAssertEqual(engine.emergencyRecoveryRequests.count, 2)

        do {
            _ = try await service.finalizeEmergencyRecovery(context)
            XCTFail("Expected successful retry to consume the exact proof")
        } catch {
            XCTAssertEqual(
                error as? TargetedReclaimScanServiceError,
                .invalidResponse
            )
        }
        XCTAssertEqual(engine.emergencyRecoveryRequests.count, 2)
    }

    func testEmergencyRecoveryBridgeMapsEveryTypedEngineError() async {
        let cases: [(EmergencyRecoveryError, TargetedReclaimScanServiceError)] = [
            (.Closed, .closed),
            (.InvalidRecordVersion, .invalidRecordVersion),
            (.InvalidPressureProof, .pressureChanged),
            (.InvalidCatalog, .invalidResponse),
            (.NotCritical, .pressureChanged),
            (.RegistryChanged, .configuredRootsChanged),
            (.CatalogChanged, .configuredRootsChanged),
            (.PressureChanged, .pressureChanged),
            (.ReadOnlyStore, .readOnlyStore),
            (.IncompatibleSchema, .incompatibleSchema),
            (.Busy, .busy),
            (.UnsafeStorage, .unsafeStorage),
            (.BudgetExceeded, .budgetExceeded),
            (.CorruptData, .corruptData),
            (.Unavailable, .storageUnavailable),
            (.OutcomeUnknown, .outcomeUnknown),
            (.InternalState, .internalState),
        ]

        for (ffiError, expected) in cases {
            let pressure = generatedTargetedPressure(pressure: .critical)
            let engine = RecordingTargetedScanEngine(
                admissions: [
                    generatedTargetedAdmission(
                        pressure: pressure,
                        disposition: .rootUnavailable,
                        rootUnavailableReason: .accessDenied
                    ),
                ],
                emergencyRecoveryError: ffiError
            )
            let service = EngineService(engine: engine)

            do {
                let admission = try await service.startTargetedReclaimScan(
                    stableVolumeID: targetedTestVolumeID,
                    anchorAt: Date(timeIntervalSince1970: 4),
                    ordinal: 0,
                    expectedRootsRevision: nil,
                    expectedRootCatalogDigestSHA256: nil
                )
                let context = try XCTUnwrap(admission.context)
                _ = try await service.finalizeEmergencyRecovery(context)
                XCTFail("Expected \(ffiError) to be mapped")
            } catch {
                XCTAssertEqual(
                    error as? TargetedReclaimScanServiceError,
                    expected,
                    "Unexpected mapping for \(ffiError)"
                )
            }
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
            directoriesScanned: 30000,
            knownAllocatedBytes: 64 * 1024 * 1024 * 1024,
            errorCount: 20000
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
        XCTAssertEqual(terminal.progress?.issueCount, 20000)
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
                measuredPermille: 1000,
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
        let gib: UInt64 = 1024 * 1024 * 1024
        let snapshot = VolumeCapacitySnapshot(
            stableVolumeID: "01234567-89AB-CDEF-0123-456789ABCDEF",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 1024 * gib,
            filesystemAvailableBytes: 100 * gib,
            importantAvailableBytes: 20 * gib,
            effectiveAvailableBytes: 20 * gib,
            availabilityBasis: .importantUsage,
            pressure: .unknown,
            criticalBoundaryBytes: nil,
            warningBoundaryBytes: nil,
            historyDisposition: nil,
            sampledAt: Date(timeIntervalSince1970: 3600)
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
        let day: TimeInterval = 86400
        let base: TimeInterval = 1_800_000_000
        let available: [UInt64] = [900, 800, 700, 650]
        let offsets: [TimeInterval] = [0, 2 * day, 7 * day, 8 * day]
        var observedVolumeID: String?
        for (offset, bytes) in zip(offsets, available) {
            observedVolumeID = try await service.observeVolumeCapacity(
                VolumeCapacitySnapshot(
                    stableVolumeID: volumeID,
                    displayName: "Macintosh HD",
                    filesystem: "APFS",
                    isInternal: true,
                    isRemovable: false,
                    totalBytes: 1000,
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
            ).stableVolumeID
        }

        let trend = try await service.loadCapacityTrend(
            stableVolumeID: XCTUnwrap(observedVolumeID),
            at: Date(timeIntervalSince1970: base + 8 * day + 1)
        )
        XCTAssertEqual(trend.stableVolumeID, "volume:macos:01234567-89ab-cdef-0123-456789abcdef")
        XCTAssertEqual(trend.availableBytes, 650)
        XCTAssertEqual(trend.change24h?.availableBytes, -50)
        XCTAssertEqual(trend.change7d?.availableBytes, -250)
        XCTAssertEqual(trend.points.map(\.availableBytes), [900, 800, 700, 650])
        XCTAssertEqual(trend.points.map(\.source), [.raw, .raw, .raw, .raw])
    }

    func testCadenceSuppressedAnchorUsesOlderDurableTrendAndExactPressureAnchor() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let volumeID = "01234567-89AB-CDEF-0123-456789ABCDEF"
        let base: TimeInterval = 1_800_000_000
        var observedVolumeID: String?

        for (offset, bytes) in [(0.0, UInt64(900)), (60.0, 890)] {
            observedVolumeID = try await service.observeVolumeCapacity(
                VolumeCapacitySnapshot(
                    stableVolumeID: volumeID,
                    displayName: "Macintosh HD",
                    filesystem: "APFS",
                    isInternal: true,
                    isRemovable: false,
                    totalBytes: 1000,
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
            ).stableVolumeID
        }

        let stableVolumeID = try XCTUnwrap(observedVolumeID)
        let currentAnchor = Date(timeIntervalSince1970: base + 60)
        let trend = try await service.loadCapacityTrend(
            stableVolumeID: stableVolumeID,
            at: currentAnchor
        )
        XCTAssertEqual(trend.sampledAt, Date(timeIntervalSince1970: base))
        XCTAssertLessThan(trend.sampledAt, currentAnchor)

        let history = try await service.loadPressureEpisodeHistory(
            stableVolumeID: stableVolumeID,
            at: currentAnchor,
            limit: 64
        )
        XCTAssertEqual(history.anchorAt, currentAnchor)
        XCTAssertTrue(history.episodes.isEmpty)

        do {
            _ = try await service.loadPressureEpisodeHistory(
                stableVolumeID: stableVolumeID,
                at: Date(timeIntervalSince1970: base + 30),
                limit: 64
            )
            XCTFail("Expected an unobserved between-sample anchor to fail closed")
        } catch let error as EngineServiceError {
            guard case .unexpected = error else {
                return XCTFail("Expected a typed invalid request response, got \(error)")
            }
        }
    }

    func testPressureEpisodeHistoryRoundTripIsAnchoredAndNewestFirst() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let volumeID = "01234567-89AB-CDEF-0123-456789ABCDEF"
        let base: TimeInterval = 1_800_000_000
        var observedVolumeID: String?
        _ = try fixture.engine.setDiskPressurePolicy(
            input: PressurePolicyInput(
                recordVersion: 1,
                criticalAvailableBytes: 100,
                criticalAvailableBasisPoints: 1000,
                warningAvailableBytes: 300,
                warningAvailableBasisPoints: 3000,
                recoveryBytes: 20,
                recoveryBasisPoints: 100
            )
        )
        for (offset, bytes) in [(0.0, UInt64(250)), (60.0, 50), (120.0, 900)] {
            observedVolumeID = try await service.observeVolumeCapacity(
                VolumeCapacitySnapshot(
                    stableVolumeID: volumeID,
                    displayName: "Macintosh HD",
                    filesystem: "APFS",
                    isInternal: true,
                    isRemovable: false,
                    totalBytes: 1000,
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
            ).stableVolumeID
        }

        let history = try await service.loadPressureEpisodeHistory(
            stableVolumeID: XCTUnwrap(observedVolumeID),
            at: Date(timeIntervalSince1970: base + 120),
            limit: 64
        )
        XCTAssertEqual(
            history.stableVolumeID,
            "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        )
        XCTAssertEqual(history.anchorAt, Date(timeIntervalSince1970: base + 120))
        XCTAssertFalse(history.hasMore)
        XCTAssertEqual(history.episodes.map(\.level), [.critical, .warning])
        XCTAssertEqual(history.episodes[0].enteredAt, Date(timeIntervalSince1970: base + 60))
        XCTAssertEqual(history.episodes[0].exitedAt, Date(timeIntervalSince1970: base + 120))
        XCTAssertEqual(history.episodes[0].policyRevision, 1)
        XCTAssertEqual(history.episodes[1].exitedAt, Date(timeIntervalSince1970: base + 60))

        let anchored = try await service.loadPressureEpisodeHistory(
            stableVolumeID: XCTUnwrap(observedVolumeID),
            at: Date(timeIntervalSince1970: base + 60),
            limit: 64
        )
        XCTAssertNil(anchored.episodes[0].exitedAt)
    }

    func testPressureEpisodeHistoryRejectsContradictoryTransportEnvelopes() async {
        let stableID = "01234567-89AB-CDEF-0123-456789ABCDEF"
        let canonicalID = "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        let anchorMS: Int64 = 10000
        let invalidResponses = [
            PressureEpisodeHistoryStatus(
                recordVersion: 1,
                stableVolumeId: canonicalID,
                anchorAtUnixMs: anchorMS,
                episodes: [],
                hasMore: true
            ),
            PressureEpisodeHistoryStatus(
                recordVersion: 1,
                stableVolumeId: canonicalID,
                anchorAtUnixMs: anchorMS,
                episodes: [
                    PressureEpisodeRecord(
                        recordVersion: 1,
                        level: .critical,
                        enteredAtUnixMs: 8000,
                        exitedAtUnixMs: 9000,
                        policyRevision: 1
                    ),
                    PressureEpisodeRecord(
                        recordVersion: 1,
                        level: .warning,
                        enteredAtUnixMs: 7000,
                        exitedAtUnixMs: nil,
                        policyRevision: 1
                    ),
                ],
                hasMore: false
            ),
            PressureEpisodeHistoryStatus(
                recordVersion: 1,
                stableVolumeId: canonicalID,
                anchorAtUnixMs: anchorMS,
                episodes: [
                    PressureEpisodeRecord(
                        recordVersion: 1,
                        level: .critical,
                        enteredAtUnixMs: 8000,
                        exitedAtUnixMs: 9000,
                        policyRevision: 1
                    ),
                    PressureEpisodeRecord(
                        recordVersion: 1,
                        level: .warning,
                        enteredAtUnixMs: 7000,
                        exitedAtUnixMs: 8500,
                        policyRevision: 1
                    ),
                ],
                hasMore: false
            ),
        ]

        for response in invalidResponses {
            let service = EngineService(engine: InvalidPressureHistoryEngine(response: response))
            do {
                _ = try await service.loadPressureEpisodeHistory(
                    stableVolumeID: stableID,
                    at: Date(timeIntervalSince1970: 10),
                    limit: 64
                )
                XCTFail("Expected contradictory pressure history to fail closed")
            } catch let error as EngineServiceError {
                guard case .unexpected = error else {
                    return XCTFail("Expected unexpected response error, got \(error)")
                }
            } catch {
                XCTFail("Expected EngineServiceError, got \(error)")
            }
        }
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
        let gib: UInt64 = 1024 * 1024 * 1024

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
                    totalBytes: 1024 * gib,
                    ordinaryAvailableBytes: 100 * gib,
                    importantAvailableBytes: 100 * gib
                )
            )
        ) { error in
            XCTAssertEqual(error as? EngineError, .InvalidCapacityObservation)
        }
    }

    func testVolumeStatusValidationRejectsContradictoryHistoryDisposition() {
        let gib: UInt64 = 1024 * 1024 * 1024
        let snapshot = VolumeCapacitySnapshot(
            stableVolumeID: "01234567-89AB-CDEF-0123-456789ABCDEF",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 1024 * gib,
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
            sampledAtUnixMs: 1000,
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
            XCTAssertEqual(try engine.libraryVersion().ffiContractVersion, 55)
            XCTAssertTrue(engine.close())
            XCTAssertTrue(engine.close())
            XCTAssertThrowsError(try engine.formatSize(bytes: 1536)) { error in
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
        let capacityTrendLoadCount = await engineService.currentCapacityTrendLoadCount()
        let pressureHistoryLoadCount = await engineService.currentPressureHistoryLoadCount()
        let volumeLoadCount = await volumeMonitor.currentLoadCount()
        XCTAssertEqual(engineLoadCount, 1)
        XCTAssertEqual(volumeObservationCount, 1)
        XCTAssertEqual(capacityTrendLoadCount, 1)
        XCTAssertEqual(pressureHistoryLoadCount, 1)
        XCTAssertEqual(volumeLoadCount, 1)
        guard case .loaded = model.engineState else {
            return XCTFail("Expected one shared loaded engine state")
        }
        guard case .loaded = model.volumeState else {
            return XCTFail("Expected one shared loaded volume state")
        }
        guard case .loaded = model.pressureHistoryState else {
            return XCTFail("Expected one shared loaded pressure history state")
        }
        XCTAssertEqual(
            model.capacityTrend?.sampledAt,
            Date(timeIntervalSince1970: 0.5),
            "A cadence-suppressed request may resolve to an older durable raw sample"
        )

        await model.loadCapacityTrend()
        let cachedCapacityTrendLoadCount =
            await engineService.currentCapacityTrendLoadCount()
        XCTAssertEqual(
            cachedCapacityTrendLoadCount,
            1,
            "The request anchor, not the older raw sample timestamp, keys the loaded trend"
        )
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

    func testSnapshotDiffNodeKindTransportAndReplacementAreExact() throws {
        let raw = generatedSnapshotDiffNode(
            id: 17,
            name: "replaced",
            kind: .file,
            currentKind: .file,
            baselineKind: .directory,
            change: .replaced,
            logicalChange: SnapshotDiffValue(direction: .shrinkage, magnitudeBytes: 8),
            currentLogicalBytes: 12,
            baselineLogicalBytes: 20,
            currentAllocatedBytes: 8,
            baselineAllocatedBytes: 16,
            allocatedChange: SnapshotDiffValue(direction: .shrinkage, magnitudeBytes: 8),
            currentFileCount: 1,
            baselineFileCount: 2,
            currentChildCount: 0,
            baselineChildCount: 1,
            canDescend: true
        )

        let buffer = FfiConverterTypeSnapshotDiffNode_lower(raw)
        let lifted = try FfiConverterTypeSnapshotDiffNode_lift(buffer)
        XCTAssertEqual(lifted.currentKind, .file)
        XCTAssertEqual(lifted.baselineKind, .directory)

        let page = try ExplorerSnapshotDiffAdapter.mapPage(
            generatedSnapshotDiffPage(
                parentID: 0,
                totalChildren: 1,
                totalShrinkageBytes: 8,
                replacedChildCount: 1,
                nodes: [lifted]
            ),
            expectedParentID: 0,
            expectedOffset: 0,
            requestedLimit: 1,
            sort: .nameAscending
        )
        let node = try XCTUnwrap(page.nodes.first)
        XCTAssertEqual(node.kind, .file)
        XCTAssertEqual(node.currentKind, .file)
        XCTAssertEqual(node.baselineKind, .directory)
        XCTAssertEqual(node.change, .replaced)
        XCTAssertTrue(node.canDescend)

        assertInvalidSnapshotDiffNode(
            generatedSnapshotDiffNode(
                currentKind: .file,
                baselineKind: .file,
                change: .replaced
            )
        )
    }

    func testSnapshotDiffAdapterAcceptsZeroByteAddedAndRemovedRows() throws {
        let added = generatedSnapshotDiffNode(
            id: 1,
            name: "added",
            currentKind: .file,
            baselineKind: nil,
            category: .developerArtifact,
            change: .added,
            logicalChange: SnapshotDiffValue(direction: .unchanged, magnitudeBytes: 0),
            currentLogicalBytes: 0,
            baselineLogicalBytes: nil,
            currentAllocatedBytes: 0,
            baselineAllocatedBytes: nil,
            allocatedChange: SnapshotDiffValue(direction: .unchanged, magnitudeBytes: 0),
            currentFileCount: 1,
            baselineFileCount: nil,
            currentChildCount: 0,
            baselineChildCount: nil,
            baselineScanFlags: nil
        )
        let removed = generatedSnapshotDiffNode(
            id: 2,
            name: "removed",
            currentKind: nil,
            baselineKind: .file,
            category: .unclassified,
            change: .removed,
            logicalChange: SnapshotDiffValue(direction: .unchanged, magnitudeBytes: 0),
            currentLogicalBytes: nil,
            baselineLogicalBytes: 0,
            currentAllocatedBytes: nil,
            baselineAllocatedBytes: 0,
            allocatedChange: SnapshotDiffValue(direction: .unchanged, magnitudeBytes: 0),
            currentFileCount: nil,
            baselineFileCount: 1,
            currentChildCount: nil,
            baselineChildCount: 0,
            currentScanFlags: nil
        )

        let page = try ExplorerSnapshotDiffAdapter.mapPage(
            generatedSnapshotDiffPage(
                totalChildren: 2,
                unchangedChildCount: 2,
                nodes: [added, removed]
            ),
            expectedParentID: 0,
            expectedOffset: 0,
            requestedLimit: 2,
            sort: .nameAscending
        )

        XCTAssertEqual(page.nodes.map(\.change), [.added, .removed])
        XCTAssertEqual(page.nodes.map(\.logicalChange.direction), [.unchanged, .unchanged])
        XCTAssertEqual(page.unchangedChildCount, 2)
    }

    func testSnapshotDiffAdapterAcceptsOneUnknownAllocatedSideForPairedNode() throws {
        let raw = generatedSnapshotDiffNode(
            currentAllocatedBytes: 8,
            baselineAllocatedBytes: nil,
            allocatedChange: nil
        )
        let page = try ExplorerSnapshotDiffAdapter.mapPage(
            generatedSnapshotDiffPage(
                totalChildren: 1,
                totalGrowthBytes: 10,
                nodes: [raw]
            ),
            expectedParentID: 0,
            expectedOffset: 0,
            requestedLimit: 1,
            sort: .nameAscending
        )

        let node = try XCTUnwrap(page.nodes.first)
        XCTAssertEqual(node.currentAllocatedBytes, 8)
        XCTAssertNil(node.baselineAllocatedBytes)
        XCTAssertNil(node.allocatedChange)
    }

    func testSnapshotDiffRootRequiresBothHistoricalDirectoryKinds() throws {
        let valid = generatedSnapshotDiffNode(
            id: 0,
            parentID: nil,
            depth: 0,
            name: "/",
            kind: .directory,
            currentKind: .directory,
            baselineKind: .directory,
            currentFileCount: 3,
            baselineFileCount: 2,
            currentChildCount: 2,
            baselineChildCount: 1,
            canDescend: true
        )
        let root = try ExplorerSnapshotDiffAdapter.mapRoot(valid)
        XCTAssertEqual(root.currentKind, .directory)
        XCTAssertEqual(root.baselineKind, .directory)

        assertInvalidSnapshotDiffRoot(
            generatedSnapshotDiffNode(
                id: 0,
                parentID: nil,
                depth: 0,
                name: "/",
                kind: .directory,
                currentKind: .directory,
                baselineKind: nil,
                change: .added,
                logicalChange: SnapshotDiffValue(direction: .growth, magnitudeBytes: 20),
                baselineLogicalBytes: nil,
                baselineFileCount: nil,
                baselineChildCount: nil,
                baselineScanFlags: nil,
                canDescend: true
            )
        )
        assertInvalidSnapshotDiffRoot(
            generatedSnapshotDiffNode(
                id: 0,
                parentID: nil,
                depth: 0,
                name: "/",
                kind: .directory,
                currentKind: .directory,
                baselineKind: .file,
                change: .replaced,
                currentFileCount: 3,
                currentChildCount: 2,
                canDescend: true
            )
        )
        assertInvalidSnapshotDiffRoot(
            generatedSnapshotDiffNode(
                id: 0,
                parentID: nil,
                depth: 0,
                name: "/",
                kind: .file,
                currentKind: .file,
                baselineKind: .directory,
                change: .replaced,
                baselineFileCount: 2,
                baselineChildCount: 1,
                canDescend: true
            )
        )
    }

    func testSnapshotDiffAdapterRejectsInvalidPresenceAndValueAlgebra() {
        let invalidNodes = [
            generatedSnapshotDiffNode(currentKind: nil),
            generatedSnapshotDiffNode(currentScanFlags: nil),
            generatedSnapshotDiffNode(kind: .directory),
            generatedSnapshotDiffNode(
                logicalChange: SnapshotDiffValue(direction: .growth, magnitudeBytes: 9)
            ),
            generatedSnapshotDiffNode(
                currentAllocatedBytes: 20,
                baselineAllocatedBytes: 10,
                allocatedChange: SnapshotDiffValue(direction: .growth, magnitudeBytes: 9)
            ),
            generatedSnapshotDiffNode(canDescend: true),
            generatedSnapshotDiffNode(
                currentKind: .file,
                baselineKind: .file,
                change: .replaced
            ),
            generatedSnapshotDiffNode(
                currentKind: nil,
                baselineKind: .file,
                category: .browserCache,
                change: .removed,
                logicalChange: SnapshotDiffValue(direction: .shrinkage, magnitudeBytes: 10),
                currentLogicalBytes: nil,
                baselineLogicalBytes: 10,
                currentFileCount: nil,
                baselineFileCount: 1,
                currentChildCount: nil,
                baselineChildCount: 0,
                currentScanFlags: nil
            ),
        ]

        for node in invalidNodes {
            assertInvalidSnapshotDiffNode(node)
        }
    }

    func testSnapshotDiffPageValidatesSortingPaginationTotalsAndOverflow() throws {
        let larger = generatedSnapshotDiffNode(
            id: 2,
            parentID: 7,
            depth: 2,
            name: "b",
            logicalChange: SnapshotDiffValue(direction: .growth, magnitudeBytes: 20),
            currentLogicalBytes: 30,
            baselineLogicalBytes: 10
        )
        let smaller = generatedSnapshotDiffNode(
            id: 1,
            parentID: 7,
            depth: 2,
            name: "a"
        )
        let valid = generatedSnapshotDiffPage(
            parentID: 7,
            offset: 1,
            totalChildren: 3,
            totalGrowthBytes: 30,
            nodes: [larger, smaller]
        )
        XCTAssertNoThrow(
            try ExplorerSnapshotDiffAdapter.mapPage(
                valid,
                expectedParentID: 7,
                expectedOffset: 1,
                requestedLimit: 2,
                sort: .magnitudeDescending
            )
        )

        assertInvalidSnapshotDiffPage(
            generatedSnapshotDiffPage(
                parentID: 7,
                offset: 1,
                totalChildren: 3,
                totalGrowthBytes: 30,
                nodes: [smaller, larger]
            ),
            expectedParentID: 7,
            expectedOffset: 1,
            requestedLimit: 2,
            sort: .magnitudeDescending
        )
        assertInvalidSnapshotDiffPage(
            generatedSnapshotDiffPage(
                parentID: 7,
                offset: 1,
                totalChildren: 3,
                hasMore: true,
                nodes: [larger, smaller]
            ),
            expectedParentID: 7,
            expectedOffset: 1,
            requestedLimit: 2,
            sort: .magnitudeDescending
        )
        assertInvalidSnapshotDiffPage(
            generatedSnapshotDiffPage(
                totalChildren: 2,
                totalGrowthBytes: 29,
                nodes: [largerWithParent(0, larger), largerWithParent(0, smaller)]
            ),
            expectedParentID: 0,
            expectedOffset: 0,
            requestedLimit: 2,
            sort: .magnitudeDescending
        )
        assertInvalidSnapshotDiffPage(
            generatedSnapshotDiffPage(
                parentID: 7,
                offset: UInt64.max - 1,
                totalChildren: UInt64.max,
                nodes: [larger, smaller]
            ),
            expectedParentID: 7,
            expectedOffset: UInt64.max - 1,
            requestedLimit: 2,
            sort: .magnitudeDescending
        )
    }

    func testSnapshotDiffTreemapValidatesExactRanksCountsAndOtherAccounting() throws {
        let growth = generatedSnapshotDiffNode(
            id: 1,
            parentID: 9,
            depth: 2,
            name: "growth",
            logicalChange: SnapshotDiffValue(direction: .growth, magnitudeBytes: 10),
            currentLogicalBytes: 20,
            baselineLogicalBytes: 10
        )
        let shrinkage = generatedSnapshotDiffNode(
            id: 2,
            parentID: 9,
            depth: 2,
            name: "shrinkage",
            change: .shrank,
            logicalChange: SnapshotDiffValue(direction: .shrinkage, magnitudeBytes: 8),
            currentLogicalBytes: 2,
            baselineLogicalBytes: 10
        )
        let raw = generatedSnapshotDiffTreemap(
            parentID: 9,
            totalChildren: 6,
            changedChildCount: 4,
            totalGrowthBytes: 15,
            totalShrinkageBytes: 11,
            otherGrowthChildCount: 1,
            otherGrowthBytes: 5,
            otherShrinkageChildCount: 1,
            otherShrinkageBytes: 3,
            unchangedChildCount: 2,
            cells: [
                SnapshotDiffTreemapCell(recordVersion: 1, node: growth, magnitudeRank: 0),
                SnapshotDiffTreemapCell(recordVersion: 1, node: shrinkage, magnitudeRank: 1),
            ]
        )

        let mapped = try ExplorerSnapshotDiffAdapter.mapTreemap(
            raw,
            expectedParentID: 9,
            requestedMaxCells: 2
        )
        XCTAssertEqual(mapped.cells.map(\.magnitudeRank), [0, 1])
        XCTAssertEqual(mapped.otherGrowthBytes, 5)
        XCTAssertEqual(mapped.otherShrinkageBytes, 3)
        XCTAssertEqual(mapped.changedChildCount + mapped.unchangedChildCount, mapped.totalChildren)
    }

    func testSnapshotDiffTreemapRejectsRankOrderingAccountingAndOverflow() {
        let growth = generatedSnapshotDiffNode(
            id: 1,
            parentID: 9,
            depth: 2,
            name: "growth",
            logicalChange: SnapshotDiffValue(direction: .growth, magnitudeBytes: 10),
            currentLogicalBytes: 20,
            baselineLogicalBytes: 10
        )
        let shrinkage = generatedSnapshotDiffNode(
            id: 2,
            parentID: 9,
            depth: 2,
            name: "shrinkage",
            change: .shrank,
            logicalChange: SnapshotDiffValue(direction: .shrinkage, magnitudeBytes: 8),
            currentLogicalBytes: 2,
            baselineLogicalBytes: 10
        )
        let validCells = [
            SnapshotDiffTreemapCell(recordVersion: 1, node: growth, magnitudeRank: 0),
            SnapshotDiffTreemapCell(recordVersion: 1, node: shrinkage, magnitudeRank: 1),
        ]

        assertInvalidSnapshotDiffTreemap(
            generatedSnapshotDiffTreemap(
                parentID: 9,
                totalChildren: 5,
                changedChildCount: 3,
                totalGrowthBytes: 15,
                totalShrinkageBytes: 11,
                otherGrowthChildCount: 1,
                otherGrowthBytes: 5,
                otherShrinkageChildCount: 1,
                otherShrinkageBytes: 3,
                unchangedChildCount: 2,
                cells: validCells
            ),
            expectedParentID: 9,
            requestedMaxCells: 2
        )
        assertInvalidSnapshotDiffTreemap(
            generatedSnapshotDiffTreemap(
                parentID: 9,
                totalChildren: 4,
                changedChildCount: 4,
                totalGrowthBytes: 15,
                totalShrinkageBytes: 11,
                otherGrowthChildCount: 1,
                otherGrowthBytes: 5,
                otherShrinkageChildCount: 1,
                otherShrinkageBytes: 3,
                cells: [
                    SnapshotDiffTreemapCell(recordVersion: 1, node: growth, magnitudeRank: 1),
                    SnapshotDiffTreemapCell(recordVersion: 1, node: shrinkage, magnitudeRank: 0),
                ]
            ),
            expectedParentID: 9,
            requestedMaxCells: 2
        )
        assertInvalidSnapshotDiffTreemap(
            generatedSnapshotDiffTreemap(
                parentID: 9,
                totalChildren: 4,
                changedChildCount: 4,
                totalGrowthBytes: 14,
                totalShrinkageBytes: 11,
                otherGrowthChildCount: 1,
                otherGrowthBytes: 5,
                otherShrinkageChildCount: 1,
                otherShrinkageBytes: 3,
                cells: validCells
            ),
            expectedParentID: 9,
            requestedMaxCells: 2
        )

        let tiedB = generatedSnapshotDiffNode(
            id: 3,
            parentID: 9,
            depth: 2,
            name: "b"
        )
        let tiedA = generatedSnapshotDiffNode(
            id: 4,
            parentID: 9,
            depth: 2,
            name: "a"
        )
        assertInvalidSnapshotDiffTreemap(
            generatedSnapshotDiffTreemap(
                parentID: 9,
                totalChildren: 2,
                changedChildCount: 2,
                totalGrowthBytes: 20,
                cells: [
                    SnapshotDiffTreemapCell(recordVersion: 1, node: tiedB, magnitudeRank: 0),
                    SnapshotDiffTreemapCell(recordVersion: 1, node: tiedA, magnitudeRank: 1),
                ]
            ),
            expectedParentID: 9,
            requestedMaxCells: 2
        )

        let maximum = generatedSnapshotDiffNode(
            id: 5,
            parentID: 9,
            depth: 2,
            name: "maximum",
            logicalChange: SnapshotDiffValue(direction: .growth, magnitudeBytes: UInt64.max),
            currentLogicalBytes: UInt64.max,
            baselineLogicalBytes: 0
        )
        assertInvalidSnapshotDiffTreemap(
            generatedSnapshotDiffTreemap(
                parentID: 9,
                totalChildren: 2,
                changedChildCount: 2,
                totalGrowthBytes: UInt64.max,
                otherGrowthChildCount: 1,
                otherGrowthBytes: 1,
                cells: [
                    SnapshotDiffTreemapCell(recordVersion: 1, node: maximum, magnitudeRank: 0),
                ]
            ),
            expectedParentID: 9,
            requestedMaxCells: 1
        )
    }

    func testApplicationRunsAsMenuBarAgent() {
        XCTAssertEqual(Bundle.main.object(forInfoDictionaryKey: "LSUIElement") as? Bool, true)
    }
}

private func generatedSnapshotDiffNode(
    recordVersion: UInt32 = 1,
    id: UInt64 = 1,
    parentID: UInt64? = 0,
    depth: UInt32 = 1,
    name: String = "node",
    kind: SnapshotNodeKind = .file,
    currentKind: SnapshotNodeKind? = .file,
    baselineKind: SnapshotNodeKind? = .file,
    category: SnapshotStorageCategory = .unclassified,
    change: SnapshotDiffChange = .grew,
    logicalChange: SnapshotDiffValue = SnapshotDiffValue(
        direction: .growth,
        magnitudeBytes: 10
    ),
    currentLogicalBytes: UInt64? = 20,
    baselineLogicalBytes: UInt64? = 10,
    currentAllocatedBytes: UInt64? = nil,
    baselineAllocatedBytes: UInt64? = nil,
    allocatedChange: SnapshotDiffValue? = nil,
    currentFileCount: UInt64? = 1,
    baselineFileCount: UInt64? = 1,
    currentChildCount: UInt64? = 0,
    baselineChildCount: UInt64? = 0,
    currentScanFlags: SnapshotNodeScanFlags? = generatedSnapshotDiffFlags(),
    baselineScanFlags: SnapshotNodeScanFlags? = generatedSnapshotDiffFlags(),
    canDescend: Bool = false
) -> SnapshotDiffNode {
    SnapshotDiffNode(
        recordVersion: recordVersion,
        id: id,
        parentId: parentID,
        depth: depth,
        name: SnapshotNodeName(
            encoding: .unixBytes,
            encodedBytes: Data(name.utf8),
            display: name
        ),
        kind: kind,
        currentKind: currentKind,
        baselineKind: baselineKind,
        category: category,
        change: change,
        logicalChange: logicalChange,
        currentLogicalBytes: currentLogicalBytes,
        baselineLogicalBytes: baselineLogicalBytes,
        currentAllocatedBytes: currentAllocatedBytes,
        baselineAllocatedBytes: baselineAllocatedBytes,
        allocatedChange: allocatedChange,
        currentFileCount: currentFileCount,
        baselineFileCount: baselineFileCount,
        currentChildCount: currentChildCount,
        baselineChildCount: baselineChildCount,
        currentScanFlags: currentScanFlags,
        baselineScanFlags: baselineScanFlags,
        canDescend: canDescend
    )
}

private func generatedSnapshotDiffFlags() -> SnapshotNodeScanFlags {
    SnapshotNodeScanFlags(
        inaccessible: false,
        timedOut: false,
        hardLinkDuplicate: false,
        mountBoundary: false
    )
}

private func generatedSnapshotDiffPage(
    recordVersion: UInt32 = 1,
    parentID: UInt64 = 0,
    offset: UInt64 = 0,
    totalChildren: UInt64,
    hasMore: Bool = false,
    totalGrowthBytes: UInt64 = 0,
    totalShrinkageBytes: UInt64 = 0,
    unchangedChildCount: UInt64 = 0,
    replacedChildCount: UInt64 = 0,
    nodes: [SnapshotDiffNode]
) -> SnapshotDiffNodePage {
    SnapshotDiffNodePage(
        recordVersion: recordVersion,
        parentId: parentID,
        offset: offset,
        totalChildren: totalChildren,
        hasMore: hasMore,
        totalGrowthBytes: totalGrowthBytes,
        totalShrinkageBytes: totalShrinkageBytes,
        unchangedChildCount: unchangedChildCount,
        replacedChildCount: replacedChildCount,
        nodes: nodes
    )
}

private func generatedSnapshotDiffTreemap(
    recordVersion: UInt32 = 1,
    parentID: UInt64,
    totalChildren: UInt64,
    changedChildCount: UInt64,
    totalGrowthBytes: UInt64 = 0,
    totalShrinkageBytes: UInt64 = 0,
    otherGrowthChildCount: UInt64 = 0,
    otherGrowthBytes: UInt64 = 0,
    otherShrinkageChildCount: UInt64 = 0,
    otherShrinkageBytes: UInt64 = 0,
    unchangedChildCount: UInt64 = 0,
    replacedChildCount: UInt64 = 0,
    cells: [SnapshotDiffTreemapCell]
) -> SnapshotDiffTreemap {
    SnapshotDiffTreemap(
        recordVersion: recordVersion,
        parentId: parentID,
        totalChildren: totalChildren,
        changedChildCount: changedChildCount,
        totalGrowthBytes: totalGrowthBytes,
        totalShrinkageBytes: totalShrinkageBytes,
        otherGrowthChildCount: otherGrowthChildCount,
        otherGrowthBytes: otherGrowthBytes,
        otherShrinkageChildCount: otherShrinkageChildCount,
        otherShrinkageBytes: otherShrinkageBytes,
        unchangedChildCount: unchangedChildCount,
        replacedChildCount: replacedChildCount,
        cells: cells
    )
}

private func largerWithParent(
    _ parentID: UInt64,
    _ node: SnapshotDiffNode
) -> SnapshotDiffNode {
    generatedSnapshotDiffNode(
        id: node.id,
        parentID: parentID,
        depth: 1,
        name: node.name.display,
        kind: node.kind,
        currentKind: node.currentKind,
        baselineKind: node.baselineKind,
        category: node.category,
        change: node.change,
        logicalChange: node.logicalChange,
        currentLogicalBytes: node.currentLogicalBytes,
        baselineLogicalBytes: node.baselineLogicalBytes,
        currentAllocatedBytes: node.currentAllocatedBytes,
        baselineAllocatedBytes: node.baselineAllocatedBytes,
        allocatedChange: node.allocatedChange,
        currentFileCount: node.currentFileCount,
        baselineFileCount: node.baselineFileCount,
        currentChildCount: node.currentChildCount,
        baselineChildCount: node.baselineChildCount,
        currentScanFlags: node.currentScanFlags,
        baselineScanFlags: node.baselineScanFlags,
        canDescend: node.canDescend
    )
}

private func assertInvalidSnapshotDiffNode(
    _ node: SnapshotDiffNode,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    assertInvalidSnapshotDiffPage(
        generatedSnapshotDiffPage(
            parentID: node.parentId ?? 0,
            totalChildren: 1,
            totalGrowthBytes: node.logicalChange.direction == .growth
                ? node.logicalChange.magnitudeBytes : 0,
            totalShrinkageBytes: node.logicalChange.direction == .shrinkage
                ? node.logicalChange.magnitudeBytes : 0,
            unchangedChildCount: node.logicalChange.direction == .unchanged ? 1 : 0,
            replacedChildCount: node.change == .replaced ? 1 : 0,
            nodes: [node]
        ),
        expectedParentID: node.parentId ?? 0,
        expectedOffset: 0,
        requestedLimit: 1,
        sort: .nameAscending,
        file: file,
        line: line
    )
}

private func assertInvalidSnapshotDiffRoot(
    _ root: SnapshotDiffNode,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertThrowsError(
        try ExplorerSnapshotDiffAdapter.mapRoot(root),
        file: file,
        line: line
    ) { error in
        XCTAssertEqual(
            error as? ExplorerSnapshotDiffFailure,
            .invalidResponse,
            file: file,
            line: line
        )
    }
}

private func assertInvalidSnapshotDiffPage(
    _ page: SnapshotDiffNodePage,
    expectedParentID: UInt64,
    expectedOffset: UInt64,
    requestedLimit: UInt16,
    sort: ExplorerSnapshotDiffSort,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertThrowsError(
        try ExplorerSnapshotDiffAdapter.mapPage(
            page,
            expectedParentID: expectedParentID,
            expectedOffset: expectedOffset,
            requestedLimit: requestedLimit,
            sort: sort
        ),
        file: file,
        line: line
    ) { error in
        XCTAssertEqual(
            error as? ExplorerSnapshotDiffFailure,
            .invalidResponse,
            file: file,
            line: line
        )
    }
}

private func assertInvalidSnapshotDiffTreemap(
    _ treemap: SnapshotDiffTreemap,
    expectedParentID: UInt64,
    requestedMaxCells: UInt16,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertThrowsError(
        try ExplorerSnapshotDiffAdapter.mapTreemap(
            treemap,
            expectedParentID: expectedParentID,
            requestedMaxCells: requestedMaxCells
        ),
        file: file,
        line: line
    ) { error in
        XCTAssertEqual(
            error as? ExplorerSnapshotDiffFailure,
            .invalidResponse,
            file: file,
            line: line
        )
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

private func assertInvalidRuleOutcomeBatch(
    _ batch: RuleOutcomeBatch,
    requestedSessionID: String = "session:test",
    detail: CleanupHistorySessionDetailModel,
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertThrowsError(
        try CleanupHistoryAdapter.mapRuleOutcomes(
            batch,
            requestedSessionID: requestedSessionID,
            detail: detail
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

private func generatedRuleOutcomeBatch(
    recordVersion: UInt32 = 1,
    sessionID: String = "session:test",
    outcomeRecordVersion: UInt32 = 1,
    itemOrdinal: UInt16 = 0,
    ruleID: String = "developer.rust.target",
    ruleRevision: UInt32 = 2,
    state: RuleOutcomeState = .awaitingComparableScan(cleanedAtUnixMs: 3000),
    outcomes: [RuleOutcome]? = nil
) -> RuleOutcomeBatch {
    RuleOutcomeBatch(
        recordVersion: recordVersion,
        sessionId: sessionID,
        outcomes: outcomes ?? [
            RuleOutcome(
                recordVersion: outcomeRecordVersion,
                itemOrdinal: itemOrdinal,
                ruleId: ruleID,
                ruleRevision: ruleRevision,
                state: state
            ),
        ]
    )
}

private func generatedCleanupHistoryDetail(
    sessionID: String = "session:test"
) throws -> CleanupHistorySessionDetailModel {
    try CleanupHistoryAdapter.mapSession(
        generatedCleanupSessionHistory(
            summary: generatedCleanupSummary(sessionID: sessionID)
        ),
        requestedSessionID: sessionID
    )
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
    sessionID: String = "session:test",
    completedAtUnixMs: Int64? = 3000,
    planCreatedAtUnixMs: Int64? = 1000,
    planExpiresAtUnixMs: Int64? = 4000,
    itemStatusCounts: CleanupStatusCounts = generatedCleanupCounts(removed: 1),
    mode: CleanupMode = .permanentSafe,
    status: CleanupSessionStatus = .completed,
    verifiedCapacityDeltaBytes: Int64? = -512,
    cancellationRequested: Bool? = false
) -> CleanupSessionSummary {
    CleanupSessionSummary(
        recordVersion: 1,
        sessionId: sessionID,
        planId: "plan:test",
        format: .complete,
        sourceScanId: "scan:test",
        startedAtUnixMs: 2000,
        completedAtUnixMs: completedAtUnixMs,
        planCreatedAtUnixMs: planCreatedAtUnixMs,
        planExpiresAtUnixMs: planExpiresAtUnixMs,
        mode: mode,
        trigger: .manual,
        status: status,
        estimatedBytes: 1024,
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
    estimatedBytes: UInt64 = 1024,
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
        newestMtimeUnixMs: 1500,
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
        startedAtUnixMs: 2000,
        completedAtUnixMs: 3000,
        planCreatedAtUnixMs: nil,
        planExpiresAtUnixMs: nil,
        mode: .trash,
        trigger: .cli,
        status: .completed,
        estimatedBytes: 1024,
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
        estimatedBytes: 1024,
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
    createdAt: SnapshotNodeTimestamp = SnapshotNodeTimestamp(
        secondsSinceUnixEpoch: 1_700_000_000,
        nanoseconds: 123_456_789
    ),
    newestMtime: SnapshotNodeTimestamp = SnapshotNodeTimestamp(
        secondsSinceUnixEpoch: 1_699_395_200,
        nanoseconds: 123_456_789
    )
) -> RustTargetPlanReviewInfo {
    RustTargetPlanReviewInfo(
        recordVersion: 1,
        planId: "plan:example",
        sourceScanId: "scan:example",
        candidateId: "candidate:example",
        ruleId: "developer.rust.target",
        ruleRevision: 3,
        category: .developerArtifact,
        mode: .permanentSafe,
        safety: .safeRegenerable,
        action: .removeKnownRegenerableContents,
        estimatedBytes: 42,
        newestMtime: newestMtime,
        minimumAgeSeconds: 604_800,
        minimumAgeNanoseconds: 0,
        warnings: [
            .estimatedBytesUnverified,
            .permanentRemovalCannotBeUndone,
        ],
        createdAt: createdAt,
        effectiveExpiresAt: SnapshotNodeTimestamp(
            secondsSinceUnixEpoch: 1_700_000_600,
            nanoseconds: 123_456_789
        ),
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

private func generatedICloudObservationSource() -> SnapshotICloudObservationSource {
    SnapshotICloudObservationSource(
        recordVersion: 1,
        scanId: "scan:example",
        scopeNodeId: 42,
        requestedMaxResults: 32,
        visitedNodeCount: 0,
        totalRankedFiles: 0,
        hasMore: false,
        targets: []
    )
}

private final class RecordingSnapshotReviewEngine: DuxEngine, @unchecked Sendable {
    private let parent: SnapshotReviewSession
    private(set) var receivedScanID: String?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingSnapshotReviewEngine cannot be lifted: \(handle)")
    }

    init(parent: SnapshotReviewSession) {
        self.parent = parent
        super.init(noHandle: NoHandle())
    }

    override func acquireExplorerSnapshotReview(scanId: String) throws
        -> SnapshotReviewSession
    {
        receivedScanID = scanId
        return parent
    }

    override func close() -> Bool {
        true
    }
}

private final class RecordingRustTargetPlanReviewEngine: DuxEngine, @unchecked Sendable {
    private let parent: SnapshotReviewSession
    private let plan: RustTargetPlanReviewSession
    private let cleanupTask: RustTargetCleanupTask?
    private let dryRunTask: RustTargetDryRunTask?
    private(set) var receivedScanID: String?
    private(set) var receivedParent: SnapshotReviewSession?
    private(set) var receivedRequest: RustTargetPlanReviewRequest?
    private(set) var receivedCleanupReview: RustTargetPlanReviewSession?
    private(set) var receivedDryRunReview: RustTargetPlanReviewSession?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingRustTargetPlanReviewEngine cannot be lifted: \(handle)")
    }

    init(
        parent: SnapshotReviewSession,
        plan: RustTargetPlanReviewSession,
        cleanupTask: RustTargetCleanupTask? = nil,
        dryRunTask: RustTargetDryRunTask? = nil
    ) {
        self.parent = parent
        self.plan = plan
        self.cleanupTask = cleanupTask
        self.dryRunTask = dryRunTask
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

    override func startPermanentSafeCleanup(
        review: RustTargetPlanReviewSession
    ) throws -> RustTargetCleanupTask {
        receivedCleanupReview = review
        guard let cleanupTask else {
            throw RustTargetCleanupStartError.Unavailable
        }
        return cleanupTask
    }

    override func startRustTargetDryRun(
        review: RustTargetPlanReviewSession
    ) throws -> RustTargetDryRunTask {
        receivedDryRunReview = review
        guard let dryRunTask else {
            throw RustTargetDryRunStartError.Unavailable
        }
        return dryRunTask
    }

    override func close() -> Bool {
        true
    }
}

private final class RecordingGeneratedRustTargetDryRunTask:
    RustTargetDryRunTask,
    @unchecked Sendable
{
    private let polls: [RustTargetDryRunPoll]
    private var index = 0

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingGeneratedRustTargetDryRunTask cannot be lifted: \(handle)")
    }

    init(polls: [RustTargetDryRunPoll]) {
        precondition(!polls.isEmpty)
        self.polls = polls
        super.init(noHandle: NoHandle())
    }

    override func poll() throws -> RustTargetDryRunPoll {
        let poll = polls[min(index, polls.count - 1)]
        index += 1
        return poll
    }

    override func cancel() throws -> RustTargetDryRunCancelOutcome {
        .requested
    }
}

private final class RecordingGeneratedRustTargetCleanupTask:
    RustTargetCleanupTask,
    @unchecked Sendable
{
    private let polls: [RustTargetCleanupPoll]
    private var index = 0

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingGeneratedRustTargetCleanupTask cannot be lifted: \(handle)")
    }

    init(polls: [RustTargetCleanupPoll]) {
        precondition(!polls.isEmpty)
        self.polls = polls
        super.init(noHandle: NoHandle())
    }

    override func poll() throws -> RustTargetCleanupPoll {
        let poll = polls[min(index, polls.count - 1)]
        index += 1
        return poll
    }

    override func cancel() throws -> RustTargetCleanupCancelOutcome {
        .requested
    }
}

private final class RecordingGeneratedSnapshotReview:
    SnapshotReviewSession,
    @unchecked Sendable
{
    private(set) var releaseCount = 0
    private(set) var receivedICloudObservationRequest: SnapshotICloudObservationSourceRequest?
    private let iCloudObservationSource: SnapshotICloudObservationSource?
    private let iCloudObservationSourceError: EngineError?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingGeneratedSnapshotReview cannot be lifted: \(handle)")
    }

    init(
        iCloudObservationSource: SnapshotICloudObservationSource? = nil,
        iCloudObservationSourceError: EngineError? = nil
    ) {
        self.iCloudObservationSource = iCloudObservationSource
        self.iCloudObservationSourceError = iCloudObservationSourceError
        super.init(noHandle: NoHandle())
    }

    override func icloudObservationSource(
        request: SnapshotICloudObservationSourceRequest
    ) throws -> SnapshotICloudObservationSource {
        receivedICloudObservationRequest = request
        if let iCloudObservationSourceError {
            throw iCloudObservationSourceError
        }
        guard let iCloudObservationSource else {
            throw EngineError.InternalState
        }
        return iCloudObservationSource
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
    private var capacityTrendLoadCount = 0
    private var pressureHistoryLoadCount = 0

    func loadStatus() async throws -> EngineStatus {
        loadCount += 1
        await Task.yield()
        return EngineStatus(
            libraryVersion: "test",
            ffiContractVersion: 12,
            databaseSchemaVersion: 16,
            snapshotFormatVersion: 1,
            executedOffMainThread: true
        )
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        volumeObservationCount += 1
        return snapshot
    }

    func loadCapacityTrend(
        stableVolumeID: String,
        at: Date
    ) async throws -> VolumeCapacityTrend {
        capacityTrendLoadCount += 1
        await Task.yield()
        return VolumeCapacityTrend(
            stableVolumeID: stableVolumeID,
            sampledAt: at.addingTimeInterval(-0.5),
            totalBytes: 100,
            availableBytes: 30,
            importantAvailableBytes: 30,
            pressure: .healthy,
            change24h: nil,
            change7d: nil,
            points: []
        )
    }

    func loadPressureEpisodeHistory(
        stableVolumeID: String,
        at: Date,
        limit _: UInt16
    ) async throws -> VolumePressureHistory {
        pressureHistoryLoadCount += 1
        await Task.yield()
        return VolumePressureHistory(
            stableVolumeID: stableVolumeID,
            anchorAt: at,
            episodes: [],
            hasMore: false
        )
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

    func currentCapacityTrendLoadCount() -> Int {
        capacityTrendLoadCount
    }

    func currentPressureHistoryLoadCount() -> Int {
        pressureHistoryLoadCount
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
                cacheRoot: root
                    .appending(path: "cache", directoryHint: .isDirectory)
                    .appending(path: "Dux", directoryHint: .isDirectory)
                    .path
            )
        )
    }

    deinit {
        _ = engine.close()
        // DUX-DESTRUCTIVE: allow=test-swift-storage-roots-fixture-remove -- remove only this fixture's UUID-named temporary root
        try? FileManager.default.removeItem(at: root)
    }
}

private final class RecordingMaintenanceEngine: DuxEngine, @unchecked Sendable {
    private let task: MaintenanceTask
    private let startRecordVersion: UInt32
    private(set) var receivedKind: MaintenanceKind?
    private(set) var calledOnMain: Bool?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingMaintenanceEngine cannot be lifted from an FFI handle: \(handle)")
    }

    init(task: MaintenanceTask, startRecordVersion: UInt32 = 1) {
        self.task = task
        self.startRecordVersion = startRecordVersion
        super.init(noHandle: NoHandle())
    }

    override func startMaintenance(kind: MaintenanceKind) throws -> MaintenanceStart {
        receivedKind = kind
        calledOnMain = Thread.isMainThread
        return MaintenanceStart(
            recordVersion: startRecordVersion,
            disposition: .started,
            task: task
        )
    }
}

private final class RecordingGeneratedMaintenanceTask:
    MaintenanceTask,
    @unchecked Sendable
{
    private var polls: [MaintenancePoll]
    private(set) var pollCalledOnMain: Bool?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError(
            "RecordingGeneratedMaintenanceTask cannot be lifted from an FFI handle: \(handle)"
        )
    }

    init(polls: [MaintenancePoll]) {
        precondition(!polls.isEmpty)
        self.polls = polls
        super.init(noHandle: NoHandle())
    }

    override func poll() throws -> MaintenancePoll {
        pollCalledOnMain = Thread.isMainThread
        if polls.count > 1 {
            return polls.removeFirst()
        }
        return polls[0]
    }

    override func cancel() throws -> MaintenanceCancelOutcome {
        .requested
    }
}

private final class RecordingTargetedScanEngine: DuxEngine, @unchecked Sendable {
    private var admissions: [TargetedProjectScanAdmission]
    private let checkpoint: TargetedProjectScanCheckpoint?
    private let startError: TargetedProjectScanError?
    private var emergencyRecoveryOrderings: [EmergencyRecoveryOrdering]
    private let emergencyRecoveryError: EmergencyRecoveryError?
    private(set) var startRequest: TargetedProjectScanRequest?
    private(set) var checkpointRequest: TargetedProjectScanCheckpointRequest?
    private(set) var emergencyRecoveryRequests: [EmergencyRecoveryRequest] = []
    private(set) var startCalledOnMain: Bool?
    private(set) var checkpointCalledOnMain: Bool?
    private(set) var emergencyRecoveryCalledOnMain: Bool?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("RecordingTargetedScanEngine cannot be lifted from handle: \(handle)")
    }

    init(
        admissions: [TargetedProjectScanAdmission],
        checkpoint: TargetedProjectScanCheckpoint? = nil,
        startError: TargetedProjectScanError? = nil,
        emergencyRecoveryOrderings: [EmergencyRecoveryOrdering] = [],
        emergencyRecoveryError: EmergencyRecoveryError? = nil
    ) {
        self.admissions = admissions
        self.checkpoint = checkpoint
        self.startError = startError
        self.emergencyRecoveryOrderings = emergencyRecoveryOrderings
        self.emergencyRecoveryError = emergencyRecoveryError
        super.init(noHandle: NoHandle())
    }

    override func startTargetedProjectScan(
        request: TargetedProjectScanRequest
    ) throws -> TargetedProjectScanAdmission {
        startRequest = request
        startCalledOnMain = Thread.isMainThread
        if let startError {
            throw startError
        }
        guard !admissions.isEmpty else {
            throw TargetedProjectScanError.InternalState
        }
        return admissions.removeFirst()
    }

    override func validateTargetedProjectScanContext(
        request: TargetedProjectScanCheckpointRequest
    ) throws -> TargetedProjectScanCheckpoint {
        checkpointRequest = request
        checkpointCalledOnMain = Thread.isMainThread
        guard let checkpoint else {
            throw TargetedProjectScanError.InternalState
        }
        return checkpoint
    }

    override func finalizeEmergencyRecovery(
        request: EmergencyRecoveryRequest
    ) throws -> EmergencyRecoveryOrdering {
        emergencyRecoveryRequests.append(request)
        emergencyRecoveryCalledOnMain = Thread.isMainThread
        if let emergencyRecoveryError {
            throw emergencyRecoveryError
        }
        guard !emergencyRecoveryOrderings.isEmpty else {
            throw EmergencyRecoveryError.InternalState
        }
        return emergencyRecoveryOrderings.removeFirst()
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

private func generatedMaintenancePoll(
    recordVersion: UInt32 = 1,
    kind: MaintenanceKind = .candidateEvaluationRecovery,
    phase: TaskPhase = .succeeded,
    failure: MaintenanceFailure? = nil,
    result: MaintenanceResult?
) -> MaintenancePoll {
    MaintenancePoll(
        recordVersion: recordVersion,
        kind: kind,
        phase: phase,
        cancellationRequested: false,
        revision: 3,
        failure: failure,
        result: result
    )
}

private func generatedCandidateEvaluationRecoveryResult(
    recordVersion: UInt32 = 1,
    kind: MaintenanceKind = .candidateEvaluationRecovery,
    outcome: MaintenanceOutcome,
    candidateCount: UInt64,
    hasMore: Bool,
    observedAtUnixMS: Int64 = 1,
    secondaryCountBefore: UInt64 = 0
) -> MaintenanceResult {
    MaintenanceResult(
        recordVersion: recordVersion,
        kind: kind,
        observedAtUnixMs: observedAtUnixMS,
        outcome: outcome,
        primaryCountBefore: 0,
        primaryCountAfter: candidateCount,
        secondaryCountBefore: secondaryCountBefore,
        secondaryCountAfter: 0,
        tertiaryCountBefore: 0,
        tertiaryCountAfter: 0,
        quaternaryCountBefore: 0,
        quaternaryCountAfter: 0,
        chargedBytesBefore: 0,
        chargedBytesAfter: 0,
        removedBytes: 0,
        capBytes: 0,
        hasMore: hasMore
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

private final class InvalidSnapshotRetentionCapEngine: DuxEngine, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) {
        super.init(unsafeFromHandle: handle)
    }

    init() {
        super.init(noHandle: NoHandle())
    }

    override func getSnapshotRetentionCap() throws -> SnapshotRetentionCapStatus {
        SnapshotRetentionCapStatus(
            recordVersion: 2,
            capBytes: 1,
            source: .default,
            updatedAtUnixMs: nil
        )
    }

    override func resetSnapshotRetentionCap() throws -> SnapshotRetentionCapUpdate {
        SnapshotRetentionCapUpdate(
            recordVersion: 1,
            settings: SnapshotRetentionCapStatus(
                recordVersion: 1,
                capBytes: 1,
                source: .default,
                updatedAtUnixMs: 0
            ),
            changed: true
        )
    }
}

private final class InvalidPressureHistoryEngine: DuxEngine, @unchecked Sendable {
    private let response: PressureEpisodeHistoryStatus

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("InvalidPressureHistoryEngine cannot be lifted: \(handle)")
    }

    init(response: PressureEpisodeHistoryStatus) {
        self.response = response
        super.init(noHandle: NoHandle())
    }

    override func getPressureEpisodeHistory(
        request _: PressureEpisodeHistoryRequest
    ) throws -> PressureEpisodeHistoryStatus {
        response
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
            cacheRoot: root
                .appending(path: "cache", directoryHint: .isDirectory)
                .appending(path: "Dux", directoryHint: .isDirectory)
                .path
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
            databaseSchemaVersion: 16,
            snapshotFormatVersion: 1,
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
