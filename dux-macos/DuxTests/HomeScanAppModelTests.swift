import Foundation
import XCTest
@testable import DUX

@MainActor
final class HomeScanAppModelTests: XCTestCase {
    func testInitialLoadNeverStartsDiscoveryScan() async {
        let service = HomeScanServiceSpy(responses: [])
        let model = AppModel(
            engineService: HomeScanEngineStub(),
            volumeMonitor: HomeScanVolumeMonitorStub(),
            homeScanService: service
        )

        await model.loadInitialState()

        let startCount = await service.startCount()
        XCTAssertEqual(startCount, 0)
        XCTAssertEqual(model.scanState, .idle)
    }

    func testConcurrentStartsCoalesceAndPublishOnlyMeasuredProgressThenSuccess() async {
        let facts = ScanProgressFacts(
            files: 4,
            directories: 2,
            knownAllocatedBytes: 1_024,
            issueCount: 1
        )
        let summary = successfulResult(progress: facts)
        let task = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: nil)),
                .success(activePoll(revision: 3, progress: facts)),
                .success(successPoll(revision: 6, result: summary)),
            ]
        )
        let service = HomeScanServiceSpy(responses: [.success(.started(task))])
        let clock = ManualHomeScanClock()
        let model = model(service: service, clock: clock)

        let first = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)
        XCTAssertEqual(model.scanState.phase, .scanning(nil))

        let second = Task { @MainActor in await model.startHomeScan() }
        await Task.yield()
        let startCount = await service.startCount()
        XCTAssertEqual(startCount, 1)

        await clock.advance()
        await clock.waitForSleepCount(2)
        XCTAssertEqual(model.scanState.phase, .scanning(facts))
        await clock.advance()
        await first.value
        await second.value

        let expected = try! XCTUnwrap(summary.successfulSummary)
        XCTAssertEqual(model.scanState.phase, .succeeded(expected))
        XCTAssertEqual(model.scanState.lastSuccessful, expected)
        let pollCount = await task.pollCount()
        XCTAssertEqual(pollCount, 3)
    }

    func testCancellationRemainsRequestedUntilRustReportsTerminal() async {
        let facts = ScanProgressFacts(
            files: 8,
            directories: 3,
            knownAllocatedBytes: 2_048,
            issueCount: 0
        )
        let task = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: facts)),
                .success(
                    activePoll(
                        revision: 3,
                        progress: facts,
                        cancellationRequested: true
                    )
                ),
                .success(cancelledPoll(revision: 5)),
            ]
        )
        let clock = ManualHomeScanClock()
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: clock
        )
        let scan = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)

        await model.cancelHomeScan()
        XCTAssertEqual(model.scanState.phase, .cancellationRequested(facts))
        let cancelCount = await task.cancelCount()
        XCTAssertEqual(cancelCount, 1)

        await clock.advance()
        await clock.waitForSleepCount(2)
        XCTAssertEqual(model.scanState.phase, .cancellationRequested(facts))
        await clock.advance()
        await scan.value
        XCTAssertEqual(model.scanState.phase, .cancelled)
    }

    func testLateSuccessAfterCancellationIsTruthfulAndRetainsPreviousSuccessOnRetryFailure() async {
        let firstFacts = ScanProgressFacts(
            files: 1,
            directories: 1,
            knownAllocatedBytes: 64,
            issueCount: 0
        )
        let firstResult = successfulResult(scanID: "scan:first", progress: firstFacts)
        let firstTask = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: firstFacts)),
                .success(successPoll(revision: 4, result: firstResult, cancelled: true)),
            ]
        )
        let service = HomeScanServiceSpy(
            responses: [
                .success(.started(firstTask)),
                .failure(.queueFull),
            ]
        )
        let clock = ManualHomeScanClock()
        let model = model(service: service, clock: clock)
        let scan = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)
        await model.cancelHomeScan()
        await clock.advance()
        await scan.value

        let expected = try! XCTUnwrap(firstResult.successfulSummary)
        XCTAssertEqual(model.scanState.phase, .succeeded(expected))
        XCTAssertEqual(model.scanState.lastSuccessful, expected)

        await model.startHomeScan()
        XCTAssertEqual(model.scanState.phase, .failed(.busy))
        XCTAssertEqual(model.scanState.lastSuccessful, expected)
        let startCount = await service.startCount()
        XCTAssertEqual(startCount, 2)
    }

    func testShutdownCancelsTaskDrainsDriverAndRejectsLatePublication() async {
        let facts = ScanProgressFacts(
            files: 10,
            directories: 2,
            knownAllocatedBytes: 512,
            issueCount: 0
        )
        let task = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: facts)),
                .success(successPoll(revision: 4, result: successfulResult(progress: facts))),
            ]
        )
        let clock = ManualHomeScanClock()
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: clock
        )
        let scan = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)
        XCTAssertEqual(model.scanState.phase, .scanning(facts))

        await model.shutdownHomeScan()
        await scan.value

        let cancelCount = await task.cancelCount()
        let pollCount = await task.pollCount()
        XCTAssertEqual(cancelCount, 1)
        XCTAssertEqual(pollCount, 1)
        XCTAssertEqual(model.scanState.phase, .scanning(facts))
        await model.startHomeScan()
        let finalPollCount = await task.pollCount()
        XCTAssertEqual(finalPollCount, 1)
    }

    func testPollFailureRequestsCancellationBeforeReleasingActiveTask() async {
        let task = HomeScanTaskSpy(polls: [.failure(.invalidResponse)])
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: ManualHomeScanClock()
        )

        await model.startHomeScan()

        XCTAssertEqual(model.scanState.phase, .failed(.invalidResponse))
        let cancelCount = await task.cancelCount()
        XCTAssertEqual(cancelCount, 1)
    }

    func testSubtreeSuccessPreservesLastSuccessfulHomeBaseline() async {
        let homeResult = successfulResult(scanID: "scan:home")
        let subtreeResult = successfulResult(scanID: "scan:folder")
        let homeService = HomeScanServiceSpy(
            responses: [.success(.started(HomeScanTaskSpy(polls: [
                .success(successPoll(revision: 1, result: homeResult)),
            ])))]
        )
        let model = model(service: homeService, clock: ManualHomeScanClock())
        await model.startHomeScan()
        let homeSummary = try! XCTUnwrap(homeResult.successfulSummary)

        let subtreeService = SubtreeScanServiceSpy(
            response: .success(.started(HomeScanTaskSpy(polls: [
                .success(successPoll(revision: 1, result: subtreeResult)),
            ])))
        )
        let outcome = await model.startSubtreeScan(
            sourceScanID: "scan:home",
            nodeID: 42,
            displayName: "Caches",
            using: subtreeService
        )

        let subtreeSummary = try! XCTUnwrap(subtreeResult.successfulSummary)
        XCTAssertEqual(outcome, .succeeded(subtreeSummary))
        XCTAssertEqual(model.scanState.phase, .succeeded(subtreeSummary))
        XCTAssertEqual(model.scanState.scope, .subtree(displayName: "Caches"))
        XCTAssertEqual(model.scanState.lastSuccessful, homeSummary)
        let request = await subtreeService.lastRequest()
        XCTAssertEqual(request?.scanID, "scan:home")
        XCTAssertEqual(request?.nodeID, 42)
    }

    func testDifferentScanRequestIsBusyWithoutReplacingActiveScope() async {
        let task = HomeScanTaskSpy(polls: [
            .success(activePoll(revision: 1, progress: nil)),
            .success(successPoll(revision: 2, result: successfulResult())),
        ])
        let clock = ManualHomeScanClock()
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: clock
        )
        let home = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)

        let outcome = await model.startSubtreeScan(
            sourceScanID: "scan:home",
            nodeID: 7,
            displayName: "Build",
            using: SubtreeScanServiceSpy(response: .failure(.busy))
        )

        XCTAssertEqual(outcome, .failed(.busy))
        XCTAssertEqual(model.scanState.scope, .home)
        XCTAssertEqual(model.scanState.phase, .scanning(nil))
        await clock.advance()
        await home.value
    }

    func testExactSubtreeStartsCoalesceOntoOneTask() async {
        let task = HomeScanTaskSpy(polls: [
            .success(activePoll(revision: 1, progress: nil)),
            .success(successPoll(revision: 2, result: successfulResult(scanID: "scan:new"))),
        ])
        let service = SubtreeScanServiceSpy(response: .success(.started(task)))
        let clock = ManualHomeScanClock()
        let model = model(service: HomeScanServiceSpy(responses: []), clock: clock)

        let first = Task { @MainActor in
            await model.startSubtreeScan(
                sourceScanID: "scan:old",
                nodeID: 9,
                displayName: "Caches",
                using: service
            )
        }
        await clock.waitForSleepCount(1)
        let second = Task { @MainActor in
            await model.startSubtreeScan(
                sourceScanID: "scan:old",
                nodeID: 9,
                displayName: "Caches",
                using: service
            )
        }
        await Task.yield()
        let startsBeforeCompletion = await service.startCount()
        XCTAssertEqual(startsBeforeCompletion, 1)
        await clock.advance()
        let firstOutcome = await first.value
        let secondOutcome = await second.value
        let finalStartCount = await service.startCount()
        XCTAssertEqual(firstOutcome, secondOutcome)
        XCTAssertEqual(finalStartCount, 1)
    }

    private func model(
        service: HomeScanServiceSpy,
        clock: ManualHomeScanClock
    ) -> AppModel {
        AppModel(
            engineService: HomeScanEngineStub(),
            homeScanService: service,
            homeScanClock: clock
        )
    }
}

private actor SubtreeScanServiceSpy: DuxSnapshotSubtreeScanServing {
    struct Request: Equatable, Sendable {
        let scanID: String
        let nodeID: UInt64
    }

    private let response: Result<HomeScanStartDisposition, HomeScanServiceError>
    private var requests: [Request] = []

    init(response: Result<HomeScanStartDisposition, HomeScanServiceError>) {
        self.response = response
    }

    func startSubtreeScan(
        sourceScanID: String,
        nodeID: UInt64
    ) async throws -> HomeScanStartDisposition {
        requests.append(Request(scanID: sourceScanID, nodeID: nodeID))
        return try response.get()
    }

    func lastRequest() -> Request? {
        requests.last
    }

    func startCount() -> Int {
        requests.count
    }
}

private actor HomeScanServiceSpy: HomeScanServing {
    private var responses: [Result<HomeScanStartDisposition, HomeScanServiceError>]
    private var starts = 0

    init(responses: [Result<HomeScanStartDisposition, HomeScanServiceError>]) {
        self.responses = responses
    }

    func startHomeScan() async throws -> HomeScanStartDisposition {
        starts += 1
        guard !responses.isEmpty else {
            throw HomeScanServiceError.invalidResponse
        }
        return try responses.removeFirst().get()
    }

    func startCount() -> Int { starts }
}

private actor HomeScanTaskSpy: HomeScanTask {
    private var polls: [Result<HomeScanTaskPoll, HomeScanServiceError>]
    private var pollsObserved = 0
    private var cancellations = 0

    init(polls: [Result<HomeScanTaskPoll, HomeScanServiceError>]) {
        self.polls = polls
    }

    func poll() async throws -> HomeScanTaskPoll {
        pollsObserved += 1
        guard !polls.isEmpty else {
            throw HomeScanServiceError.invalidResponse
        }
        return try polls.removeFirst().get()
    }

    func requestCancellation() async throws -> HomeScanCancelOutcome {
        cancellations += 1
        return cancellations == 1 ? .requested : .alreadyRequested
    }

    func pollCount() -> Int { pollsObserved }
    func cancelCount() -> Int { cancellations }
}

private actor ManualHomeScanClock: HomeScanPollingClock {
    private struct Sleeper {
        let id: UUID
        let continuation: CheckedContinuation<Void, any Error>
    }

    private var sleepers: [Sleeper] = []
    private var cancelled: Set<UUID> = []
    private var sleepCount = 0

    func sleepUntilNextPoll() async throws {
        try Task.checkCancellation()
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation {
                (continuation: CheckedContinuation<Void, any Error>) in
                sleepCount += 1
                if cancelled.remove(id) != nil || Task.isCancelled {
                    continuation.resume(throwing: CancellationError())
                } else {
                    sleepers.append(Sleeper(id: id, continuation: continuation))
                }
            }
        } onCancel: {
            Task { await self.cancel(id) }
        }
    }

    func waitForSleepCount(_ expected: Int) async {
        while sleepCount < expected {
            await Task.yield()
        }
    }

    func advance() {
        guard !sleepers.isEmpty else {
            return
        }
        sleepers.removeFirst().continuation.resume()
    }

    private func cancel(_ id: UUID) {
        if let index = sleepers.firstIndex(where: { $0.id == id }) {
            sleepers.remove(at: index).continuation.resume(throwing: CancellationError())
        } else {
            cancelled.insert(id)
        }
    }
}

private actor HomeScanEngineStub: EngineServing {
    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 12, executedOffMainThread: true)
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        DiskPressurePolicy(
            source: .default,
            revision: 0,
            configuration: .defaults,
            updatedAtUnixMilliseconds: nil
        )
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .stored,
                revision: 1,
                configuration: configuration,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .default,
                revision: 1,
                configuration: .defaults,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }
}

private struct HomeScanVolumeMonitorStub: VolumeMonitoring {
    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot {
        VolumeCapacitySnapshot(
            stableVolumeID: nil,
            displayName: "Test",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 100,
            filesystemAvailableBytes: 50,
            importantAvailableBytes: 60,
            effectiveAvailableBytes: 60,
            availabilityBasis: .importantUsage,
            pressure: .healthy,
            criticalBoundaryBytes: 5,
            warningBoundaryBytes: 10,
            historyDisposition: .notStoredMissingStableIdentity,
            sampledAt: Date(timeIntervalSince1970: 1)
        )
    }
}

private func activePoll(
    revision: UInt64,
    progress: ScanProgressFacts?,
    cancellationRequested: Bool = false
) -> HomeScanTaskPoll {
    HomeScanTaskPoll(
        phase: .running,
        stage: .scanning,
        cancellationRequested: cancellationRequested,
        revision: revision,
        progress: progress,
        eventsTruncated: false,
        failure: nil,
        result: nil
    )
}

private func successPoll(
    revision: UInt64,
    result: HomeScanTaskResult,
    cancelled: Bool = false
) -> HomeScanTaskPoll {
    HomeScanTaskPoll(
        phase: .succeeded,
        stage: .terminal,
        cancellationRequested: cancelled,
        revision: revision,
        progress: nil,
        eventsTruncated: false,
        failure: nil,
        result: result
    )
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

private func successfulResult(
    scanID: String = "scan:test",
    progress: ScanProgressFacts = ScanProgressFacts(
        files: 1,
        directories: 1,
        knownAllocatedBytes: 1,
        issueCount: 0
    )
) -> HomeScanTaskResult {
    HomeScanTaskResult(
        scanID: scanID,
        startedAt: Date(timeIntervalSince1970: 1),
        completedAt: Date(timeIntervalSince1970: 2),
        succeeded: true,
        directoryCount: progress.directories,
        fileCount: progress.files,
        logicalBytes: 2_048,
        allocatedBytes: progress.knownAllocatedBytes,
        coverage: progress.issueCount == 0 ? .complete : .partial,
        coveragePermille: progress.issueCount == 0 ? 1_000 : nil,
        issueCount: progress.issueCount,
        snapshotAvailable: true,
        candidateEvaluation: .succeeded(candidateCount: 0)
    )
}
