import Foundation
import XCTest
@testable import DUX

final class TargetedReclaimScanPresentationTests: XCTestCase {
    func testProgressAndCompletedCopyExposeTextAlternatives() {
        let context = scanContext(rootCount: 2)
        let root = configuredRoot("/Users/example/Project", ordinal: 0)
        let result = successfulResult(scanID: "scan:one", candidateCount: 3)
        let progress = TargetedReclaimScanProgress(
            context: context,
            completed: [
                TargetedReclaimRootResult(
                    ordinal: 0,
                    root: root,
                    source: .focusedRun,
                    result: result
                ),
            ],
            failed: [],
            activeOrdinal: 1,
            activeProgress: nil
        )

        let active = TargetedReclaimScanPresentation.make(.scanning(progress))
        XCTAssertEqual(active.progressValue, 0.5)
        XCTAssertEqual(active.progressLabel, "1 of 2 locations finished")
        XCTAssertTrue(active.canCancel)

        let batch = TargetedReclaimScanBatch(
            context: context,
            completed: progress.completed,
            failed: [
                TargetedReclaimFailedRoot(
                    ordinal: 1,
                    root: configuredRoot("/Users/example/Other", ordinal: 1),
                    failure: .accessDenied
                ),
            ]
        )
        let completed = TargetedReclaimScanPresentation.make(.completed(batch))
        XCTAssertTrue(completed.detail.contains("3 candidates"))
        XCTAssertEqual(completed.rows.count, 2)
        XCTAssertTrue(batch.isComplete)
        XCTAssertTrue(completed.canRetry)
        XCTAssertFalse(completed.canCancel)

        let partial = TargetedReclaimScanBatch(
            context: context,
            completed: progress.completed,
            failed: []
        )
        XCTAssertFalse(partial.isComplete)
    }

    @MainActor
    func testAccessibilityIdentifiersAreUniqueAndOrdinalBound() {
        XCTAssertEqual(
            Set(ExplorerAccessibility.allIdentifiers).count,
            ExplorerAccessibility.allIdentifiers.count
        )
        XCTAssertNotEqual(
            ExplorerAccessibility.targetedReclaimScanRoot(ordinal: 0),
            ExplorerAccessibility.targetedReclaimScanRoot(ordinal: 1)
        )
        XCTAssertEqual(
            Set(MenuBarPopoverAccessibility.allIdentifiers).count,
            MenuBarPopoverAccessibility.allIdentifiers.count
        )
    }
}

@MainActor
final class TargetedReclaimScanAppModelTests: XCTestCase {
    func testWarningRunsKnownCacheBeforeConfiguredRootAndReusesDurableResult() async {
        let context = scanContext(
            rootCount: 2,
            knownUserLibraryCachesIncluded: true
        )
        let firstRoot = knownUserCacheRoot()
        let secondRoot = configuredRoot("/Users/example/Beta", ordinal: 1)
        let firstResult = successfulResult(scanID: "scan:current", candidateCount: 2)
        let secondResult = successfulResult(scanID: "scan:new", candidateCount: 4)
        let task = TargetedHomeScanTaskSpy(
            polls: [
                activePoll(),
                successfulPoll(secondResult),
            ]
        )
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: firstRoot,
                    disposition: .current(firstResult)
                ),
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 1,
                    root: secondRoot,
                    disposition: .started(task)
                ),
            ]
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service,
            homeScanClock: ImmediateTargetedScanClock()
        )

        await model.reconcileTargetedReclaimScan(for: lowSpaceSnapshot())

        guard case let .completed(batch) = model.targetedReclaimScanState else {
            return XCTFail("Expected completed focused batch")
        }
        XCTAssertEqual(batch.context, context)
        XCTAssertEqual(batch.completed.map(\.source), [.currentDurable, .focusedRun])
        XCTAssertEqual(
            batch.completed.map(\.root.kind),
            [.knownUserLibraryCaches, .configuredProject]
        )
        XCTAssertEqual(batch.candidateCount, 6)
        XCTAssertTrue(batch.failed.isEmpty)
        let calls = await service.calls()
        XCTAssertEqual(calls.count, 2)
        XCTAssertNil(calls[0].expectedRootsRevision)
        XCTAssertEqual(calls[1].expectedRootsRevision, context.rootsRevision)
        XCTAssertNil(calls[0].expectedRootCatalogDigestSHA256)
        XCTAssertEqual(
            calls[1].expectedRootCatalogDigestSHA256,
            context.rootCatalogDigestSHA256
        )
        XCTAssertEqual(calls.map(\.ordinal), [0, 1])
        let checkpoints = await service.checkpoints()
        XCTAssertEqual(checkpoints, 1)
    }

    func testRepeatedSampleForSameDurableBatchDoesNotRepeatTraversal() async {
        let context = scanContext(rootCount: 1)
        let root = configuredRoot("/Users/example/Alpha", ordinal: 0)
        let result = successfulResult(scanID: "scan:current", candidateCount: 1)
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: root,
                    disposition: .current(result)
                ),
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: root,
                    disposition: .current(result)
                ),
            ]
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service
        )
        let snapshot = lowSpaceSnapshot()

        await model.reconcileTargetedReclaimScan(for: snapshot)
        await model.reconcileTargetedReclaimScan(for: snapshot)

        guard case let .completed(batch) = model.targetedReclaimScanState else {
            return XCTFail("Expected retained focused batch")
        }
        XCTAssertEqual(batch.completed.count, 1)
        let callCount = await service.calls().count
        XCTAssertEqual(callCount, 2)
    }

    func testUnavailableRootDoesNotPreventLaterRootScan() async {
        let context = scanContext(rootCount: 2)
        let missing = configuredRoot("/Users/example/Missing", ordinal: 0)
        let available = configuredRoot("/Users/example/Available", ordinal: 1)
        let result = successfulResult(scanID: "scan:available", candidateCount: 2)
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: missing,
                    disposition: .unavailable(.rootMissing)
                ),
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 1,
                    root: available,
                    disposition: .current(result)
                ),
            ]
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service
        )

        await model.reconcileTargetedReclaimScan(for: lowSpaceSnapshot())

        guard case let .completed(batch) = model.targetedReclaimScanState else {
            return XCTFail("Expected partial focused batch")
        }
        XCTAssertEqual(batch.failed.map(\.failure), [.rootMissing])
        XCTAssertEqual(batch.completed.map(\.result.scanID), ["scan:available"])
    }

    func testRetryRechecksPreviouslyUnavailableRoot() async {
        let context = scanContext(rootCount: 1)
        let root = configuredRoot("/Users/example/Retry", ordinal: 0)
        let result = successfulResult(scanID: "scan:retry", candidateCount: 2)
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: root,
                    disposition: .unavailable(.rootMissing)
                ),
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: root,
                    disposition: .current(result)
                ),
            ]
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service
        )
        let snapshot = lowSpaceSnapshot()

        await model.reconcileTargetedReclaimScan(for: snapshot)
        await model.reconcileTargetedReclaimScan(for: snapshot)

        guard case let .completed(batch) = model.targetedReclaimScanState else {
            return XCTFail("Expected retry to replace the unavailable root")
        }
        XCTAssertTrue(batch.failed.isEmpty)
        XCTAssertEqual(batch.completed.map(\.result.scanID), ["scan:retry"])
        let calls = await service.calls()
        XCTAssertEqual(calls.map(\.ordinal), [0, 0])
    }

    func testHealthySampleCancelsActiveFocusedTaskWithoutCleanup() async {
        let context = scanContext(rootCount: 1)
        let task = TargetedHomeScanTaskSpy(polls: [activePoll()], suspendsAfterPolls: true)
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: configuredRoot("/Users/example/Alpha", ordinal: 0),
                    disposition: .started(task)
                ),
            ]
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service,
            homeScanClock: ImmediateTargetedScanClock()
        )
        let run = Task { @MainActor in
            await model.reconcileTargetedReclaimScan(for: lowSpaceSnapshot())
        }
        await task.waitForPoll()

        await model.reconcileTargetedReclaimScan(for: healthySnapshot())
        await task.resumeSuspendedPoll()
        await run.value

        let cancellationRequested = await task.wasCancellationRequested()
        XCTAssertTrue(cancellationRequested)
        XCTAssertFalse(model.targetedReclaimScanState.isActive)
    }

    func testStoppingNonOwningObservationDoesNotCancelEngineTask() async {
        let context = scanContext(rootCount: 1)
        let task = TargetedHomeScanTaskSpy(polls: [activePoll()])
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: configuredRoot("/Users/example/Alpha", ordinal: 0),
                    disposition: .observing(task)
                ),
            ]
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service
        )
        let run = Task { @MainActor in
            await model.reconcileTargetedReclaimScan(for: lowSpaceSnapshot())
        }
        await task.waitForPoll()

        await model.cancelTargetedReclaimScan()
        await run.value

        let cancellationRequested = await task.wasCancellationRequested()
        XCTAssertFalse(cancellationRequested)
        XCTAssertFalse(model.targetedReclaimScanState.isActive)
    }

    func testChangedRegistryFailsClosedWithoutStartingAPath() async {
        let service = TargetedReclaimScanServiceSpy(
            admissions: [],
            errors: [.configuredRootsChanged]
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service
        )

        await model.reconcileTargetedReclaimScan(for: lowSpaceSnapshot())

        XCTAssertEqual(
            model.targetedReclaimScanState,
            .failed(.configuredRootsChanged, previous: nil)
        )
    }

    func testAdmissionPressureMustMatchTheCapacityObservation() async {
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: scanContext(rootCount: 1, pressure: .critical),
                    ordinal: 0,
                    root: configuredRoot("/Users/example/Alpha", ordinal: 0),
                    disposition: .current(
                        successfulResult(
                            scanID: "scan:wrong-pressure",
                            candidateCount: 1,
                            startedAt: Date(timeIntervalSince1970: 1_700_000_060)
                        )
                    )
                ),
            ]
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service
        )

        await model.reconcileTargetedReclaimScan(for: lowSpaceSnapshot())

        XCTAssertEqual(
            model.targetedReclaimScanState,
            .failed(.invalidResponse, previous: nil)
        )
        let checkpoints = await service.checkpoints()
        XCTAssertEqual(checkpoints, 0)
    }

    func testChangedRegistryAtFinalCheckpointDoesNotPublishCompletedBatch() async {
        let context = scanContext(rootCount: 1)
        let result = successfulResult(scanID: "scan:checkpoint", candidateCount: 1)
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: context,
                    ordinal: 0,
                    root: configuredRoot("/Users/example/Alpha", ordinal: 0),
                    disposition: .current(result)
                ),
            ],
            checkpointError: .configuredRootsChanged
        )
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service
        )

        await model.reconcileTargetedReclaimScan(for: lowSpaceSnapshot())

        guard case let .failed(failure, previous) = model.targetedReclaimScanState else {
            return XCTFail("Expected final checkpoint failure")
        }
        XCTAssertEqual(failure, .configuredRootsChanged)
        XCTAssertEqual(previous?.completed.map(\.result.scanID), ["scan:checkpoint"])
    }

    func testCriticalSampleRevalidatesAfterWarningBatchFinishes() async {
        let warningContext = scanContext(rootCount: 1)
        let criticalContext = scanContext(rootCount: 1, pressure: .critical)
        let root = configuredRoot("/Users/example/Alpha", ordinal: 0)
        let warningResult = successfulResult(scanID: "scan:warning", candidateCount: 1)
        let criticalResult = successfulResult(
            scanID: "scan:critical",
            candidateCount: 2,
            startedAt: Date(timeIntervalSince1970: 1_700_000_060)
        )
        let task = TargetedHomeScanTaskSpy(
            polls: [
                activePoll(),
                successfulPoll(warningResult),
            ]
        )
        let service = TargetedReclaimScanServiceSpy(
            admissions: [
                TargetedReclaimScanAdmission(
                    context: warningContext,
                    ordinal: 0,
                    root: root,
                    disposition: .started(task)
                ),
                TargetedReclaimScanAdmission(
                    context: criticalContext,
                    ordinal: 0,
                    root: root,
                    disposition: .current(criticalResult)
                ),
            ]
        )
        let clock = GatedTargetedScanClock()
        let model = AppModel(
            engineService: TargetedEngineStub(),
            targetedReclaimScanService: service,
            homeScanClock: clock
        )
        let warningRun = Task { @MainActor in
            await model.reconcileTargetedReclaimScan(for: lowSpaceSnapshot())
        }
        await clock.waitUntilSleeping()
        let criticalRun = Task { @MainActor in
            await model.reconcileTargetedReclaimScan(for: criticalSpaceSnapshot())
        }
        await Task.yield()
        await clock.resume()
        await warningRun.value
        await criticalRun.value

        guard case let .completed(batch) = model.targetedReclaimScanState else {
            return XCTFail("Expected revalidated Critical focused batch")
        }
        XCTAssertEqual(batch.context.pressure, .critical)
        let calls = await service.calls()
        XCTAssertEqual(calls.map(\.anchorAt), [
            lowSpaceSnapshot().sampledAt,
            criticalSpaceSnapshot().sampledAt,
        ])
    }
}

private actor TargetedReclaimScanServiceSpy: DuxTargetedReclaimScanServing {
    struct Call: Equatable, Sendable {
        let stableVolumeID: String
        let anchorAt: Date
        let ordinal: UInt16
        let expectedRootsRevision: UInt64?
        let expectedRootCatalogDigestSHA256: Data?
    }

    private var admissions: [TargetedReclaimScanAdmission]
    private var errors: [TargetedReclaimScanServiceError]
    private var recordedCalls: [Call] = []
    private let checkpointContext: TargetedReclaimScanContext?
    private let checkpointError: TargetedReclaimScanServiceError?
    private var checkpointCount = 0

    init(
        admissions: [TargetedReclaimScanAdmission],
        errors: [TargetedReclaimScanServiceError] = [],
        checkpointError: TargetedReclaimScanServiceError? = nil
    ) {
        self.admissions = admissions
        self.errors = errors
        checkpointContext = admissions.compactMap(\.context).last
        self.checkpointError = checkpointError
    }

    func startTargetedReclaimScan(
        stableVolumeID: String,
        anchorAt: Date,
        ordinal: UInt16,
        expectedRootsRevision: UInt64?,
        expectedRootCatalogDigestSHA256: Data?
    ) async throws -> TargetedReclaimScanAdmission {
        recordedCalls.append(
            Call(
                stableVolumeID: stableVolumeID,
                anchorAt: anchorAt,
                ordinal: ordinal,
                expectedRootsRevision: expectedRootsRevision,
                expectedRootCatalogDigestSHA256: expectedRootCatalogDigestSHA256
            )
        )
        if !errors.isEmpty {
            throw errors.removeFirst()
        }
        guard !admissions.isEmpty else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        return admissions.removeFirst()
    }

    func calls() -> [Call] {
        recordedCalls
    }

    func validateTargetedReclaimScan(
        _ context: TargetedReclaimScanContext
    ) async throws -> TargetedReclaimScanContext {
        checkpointCount += 1
        if let checkpointError {
            throw checkpointError
        }
        guard checkpointContext == context else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        return context
    }

    func checkpoints() -> Int {
        checkpointCount
    }
}

private actor TargetedHomeScanTaskSpy: HomeScanTask {
    private var polls: [HomeScanTaskPoll]
    private let suspendsAfterPolls: Bool
    private var cancellationRequested = false
    private var pollCount = 0
    private var pollWaiters: [CheckedContinuation<Void, Never>] = []
    private var resumeContinuation: CheckedContinuation<Void, Never>?

    init(polls: [HomeScanTaskPoll], suspendsAfterPolls: Bool = false) {
        self.polls = polls
        self.suspendsAfterPolls = suspendsAfterPolls
    }

    func poll() async throws -> HomeScanTaskPoll {
        pollCount += 1
        let waiters = pollWaiters
        pollWaiters.removeAll()
        waiters.forEach { $0.resume() }
        if cancellationRequested {
            return cancelledPoll(revision: UInt64(pollCount))
        }
        if polls.isEmpty, suspendsAfterPolls {
            await withCheckedContinuation { continuation in
                resumeContinuation = continuation
            }
        }
        if cancellationRequested {
            return cancelledPoll(revision: UInt64(pollCount))
        }
        guard !polls.isEmpty else {
            return activePoll()
        }
        return polls.removeFirst()
    }

    func requestCancellation() async throws -> HomeScanCancelOutcome {
        cancellationRequested = true
        resumeContinuation?.resume()
        resumeContinuation = nil
        return .requested
    }

    func waitForPoll() async {
        if pollCount > 0 {
            return
        }
        await withCheckedContinuation { continuation in
            pollWaiters.append(continuation)
        }
    }

    func resumeSuspendedPoll() {
        resumeContinuation?.resume()
        resumeContinuation = nil
    }

    func wasCancellationRequested() -> Bool {
        cancellationRequested
    }

    private func cancelledPoll(revision: UInt64) -> HomeScanTaskPoll {
        HomeScanTaskPoll(
            phase: .cancelled,
            stage: .terminal,
            cancellationRequested: true,
            revision: revision,
            progress: nil,
            eventsTruncated: false,
            failure: nil,
            result: nil
        )
    }
}

private struct ImmediateTargetedScanClock: HomeScanPollingClock {
    func sleepUntilNextPoll() async throws {
        await Task.yield()
    }
}

private actor GatedTargetedScanClock: HomeScanPollingClock {
    private var sleeping = false
    private var waiters: [CheckedContinuation<Void, Never>] = []
    private var sleepContinuation: CheckedContinuation<Void, Never>?

    func sleepUntilNextPoll() async throws {
        sleeping = true
        let waiters = waiters
        self.waiters.removeAll()
        waiters.forEach { $0.resume() }
        await withCheckedContinuation { continuation in
            sleepContinuation = continuation
        }
    }

    func waitUntilSleeping() async {
        if sleeping {
            return
        }
        await withCheckedContinuation { continuation in
            waiters.append(continuation)
        }
    }

    func resume() {
        sleeping = false
        sleepContinuation?.resume()
        sleepContinuation = nil
    }
}

private struct TargetedEngineStub: EngineServing {
    func loadStatus() async throws -> EngineStatus {
        throw EngineServiceError.unavailable
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        throw EngineServiceError.unavailable
    }

    func setDiskPressurePolicy(
        _: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        throw EngineServiceError.unavailable
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        throw EngineServiceError.unavailable
    }
}

private func scanContext(
    rootCount: UInt16,
    pressure: TargetedReclaimPressure = .warning,
    knownUserLibraryCachesIncluded: Bool = false
) -> TargetedReclaimScanContext {
    TargetedReclaimScanContext(
        stableVolumeID: "volume:test",
        capacityAnchorAt: pressure == .critical
            ? criticalSpaceSnapshot().sampledAt
            : lowSpaceSnapshot().sampledAt,
        pressure: pressure,
        pressureEpisodeStartedAt: pressure == .critical
            ? Date(timeIntervalSince1970: 1_700_000_050)
            : Date(timeIntervalSince1970: 1_700_000_000),
        lowPressureSequenceStartedAt: Date(timeIntervalSince1970: 1_700_000_000),
        policyRevision: 3,
        rootsRevision: 7,
        knownRootsPolicyRevision: 1,
        knownUserLibraryCachesIncluded: knownUserLibraryCachesIncluded,
        rootCatalogDigestSHA256: targetedRootCatalogDigest,
        rootCount: rootCount
    )
}

private let targetedRootCatalogDigest = Data((0 ..< 32).map(UInt8.init))

private func configuredRoot(
    _ path: String,
    ordinal: UInt16
) -> TargetedReclaimScanRoot {
    TargetedReclaimScanRoot(
        ordinal: ordinal,
        kind: .configuredProject,
        path: observedPath(path)
    )
}

private func knownUserCacheRoot(
    _ path: String = "/Users/example/Library/Caches"
) -> TargetedReclaimScanRoot {
    TargetedReclaimScanRoot(
        ordinal: 0,
        kind: .knownUserLibraryCaches,
        path: observedPath(path)
    )
}

private func observedPath(_ path: String) -> ProjectDiscoveryRoot {
    ProjectDiscoveryRoot(encoding: .unixBytes, encodedBytes: Data(path.utf8))
}

private func successfulResult(
    scanID: String,
    candidateCount: UInt32,
    startedAt: Date = Date(timeIntervalSince1970: 1_700_000_010)
) -> HomeScanTaskResult {
    HomeScanTaskResult(
        scanID: scanID,
        startedAt: startedAt,
        completedAt: startedAt.addingTimeInterval(10),
        succeeded: true,
        directoryCount: 4,
        fileCount: 8,
        logicalBytes: 4_096,
        allocatedBytes: 8_192,
        coverage: .complete,
        coveragePermille: 1_000,
        issueCount: 0,
        snapshotAvailable: true,
        candidateEvaluation: .succeeded(candidateCount: candidateCount)
    )
}

private func activePoll() -> HomeScanTaskPoll {
    HomeScanTaskPoll(
        phase: .running,
        stage: .scanning,
        cancellationRequested: false,
        revision: 1,
        progress: ScanProgressFacts(
            files: 2,
            directories: 1,
            knownAllocatedBytes: 512,
            issueCount: 0
        ),
        eventsTruncated: false,
        failure: nil,
        result: nil
    )
}

private func successfulPoll(_ result: HomeScanTaskResult) -> HomeScanTaskPoll {
    HomeScanTaskPoll(
        phase: .succeeded,
        stage: .terminal,
        cancellationRequested: false,
        revision: 2,
        progress: nil,
        eventsTruncated: false,
        failure: nil,
        result: result
    )
}

private func lowSpaceSnapshot() -> VolumeCapacitySnapshot {
    VolumeCapacitySnapshot(
        stableVolumeID: "volume:test",
        displayName: "Macintosh HD",
        filesystem: "APFS",
        isInternal: true,
        isRemovable: false,
        totalBytes: 1_000,
        filesystemAvailableBytes: 100,
        importantAvailableBytes: 100,
        effectiveAvailableBytes: 100,
        availabilityBasis: .importantUsage,
        pressure: .warning,
        criticalBoundaryBytes: 50,
        warningBoundaryBytes: 150,
        historyDisposition: .stored,
        sampledAt: Date(timeIntervalSince1970: 1_700_000_100)
    )
}

private func healthySnapshot() -> VolumeCapacitySnapshot {
    VolumeCapacitySnapshot(
        stableVolumeID: "volume:test",
        displayName: "Macintosh HD",
        filesystem: "APFS",
        isInternal: true,
        isRemovable: false,
        totalBytes: 1_000,
        filesystemAvailableBytes: 500,
        importantAvailableBytes: 500,
        effectiveAvailableBytes: 500,
        availabilityBasis: .importantUsage,
        pressure: .healthy,
        criticalBoundaryBytes: 50,
        warningBoundaryBytes: 150,
        historyDisposition: .stored,
        sampledAt: Date(timeIntervalSince1970: 1_700_000_200)
    )
}

private func criticalSpaceSnapshot() -> VolumeCapacitySnapshot {
    VolumeCapacitySnapshot(
        stableVolumeID: "volume:test",
        displayName: "Macintosh HD",
        filesystem: "APFS",
        isInternal: true,
        isRemovable: false,
        totalBytes: 1_000,
        filesystemAvailableBytes: 40,
        importantAvailableBytes: 40,
        effectiveAvailableBytes: 40,
        availabilityBasis: .importantUsage,
        pressure: .critical,
        criticalBoundaryBytes: 50,
        warningBoundaryBytes: 150,
        historyDisposition: .stored,
        sampledAt: Date(timeIntervalSince1970: 1_700_000_150)
    )
}
