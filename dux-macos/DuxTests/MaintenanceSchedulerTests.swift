import XCTest
@testable import DUX

final class MaintenanceSchedulerTests: XCTestCase {
    func testInitialGraceRunsOneFullFairCycleBeforeNormalCadence() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await scheduler.signal(.applicationBecameActive)
        await clock.advance(by: 9)
        let startsBeforeGrace = await service.startedKinds()
        XCTAssertEqual(startsBeforeGrace, [])

        await clock.advance(by: 1)
        try await eventually {
            let count = await service.startedKinds().count
            let inFlight = await scheduler.snapshot().inFlightKind
            return count == 1 && inFlight == nil
        }
        let firstStarts = await service.startedKinds()
        XCTAssertEqual(firstStarts, [.snapshotTerminalTemp])

        for expectedCount in 2 ... 6 {
            await clock.advance(by: 2)
            try await eventually {
                let count = await service.startedKinds().count
                let inFlight = await scheduler.snapshot().inFlightKind
                return count == expectedCount && inFlight == nil
            }
        }
        let allStarts = await service.startedKinds()
        XCTAssertEqual(allStarts, DuxMaintenanceKind.allCases)
        let afterCycle = await scheduler.snapshot()
        XCTAssertEqual(afterCycle.nextDeadline?.milliseconds, 120)
        await scheduler.stop()
    }

    func testDeferredOutcomeUsesDeferralBackoffInsteadOfHasMoreDelay() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(
            .started(StubDuxMaintenanceTask(polls: [
                .finished(DuxMaintenanceBatchSignal(hasMore: true, deferral: .active)),
            ]))
        )
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await waitForCompletedAttempt(scheduler, service: service, count: 1)
        await clock.advance(by: 29)
        let countBeforeDeferralDeadline = await service.startedKinds().count
        XCTAssertEqual(countBeforeDeferralDeadline, 1)
        await clock.advance(by: 1)
        try await eventually { await service.startedKinds().count == 2 }
        await scheduler.stop()
    }

    func testHasMoreUsesShortDelayAndMovesKindToRotationTail() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(
            .started(StubDuxMaintenanceTask(polls: [
                .finished(DuxMaintenanceBatchSignal(hasMore: true)),
            ]))
        )
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually {
            let count = await service.startedKinds().count
            let inFlight = await scheduler.snapshot().inFlightKind
            return count == 1 && inFlight == nil
        }

        await clock.advance(by: 19)
        let countBeforeDelay = await service.startedKinds().count
        XCTAssertEqual(countBeforeDelay, 1)
        await clock.advance(by: 1)
        try await eventually {
            let count = await service.startedKinds().count
            let inFlight = await scheduler.snapshot().inFlightKind
            return count == 2 && inFlight == nil
        }
        let starts = await service.startedKinds()
        XCTAssertEqual(starts, [.snapshotTerminalTemp, .snapshotUnleasedTemp])
        await scheduler.stop()
    }

    func testDeferredBusyRetriesSameKindWithCappedBackoff() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(.deferredBusy)
        await service.enqueue(.deferredBusy)
        await service.enqueue(.deferredBusy)
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await waitForCompletedAttempt(scheduler, service: service, count: 1)
        await clock.advance(by: 5)
        try await waitForCompletedAttempt(scheduler, service: service, count: 2)
        await clock.advance(by: 9)
        let countBeforeCap = await service.startedKinds().count
        XCTAssertEqual(countBeforeCap, 2)
        await clock.advance(by: 1)
        try await waitForCompletedAttempt(scheduler, service: service, count: 3)

        let starts = await service.startedKinds()
        XCTAssertEqual(
            starts,
            Array(repeating: DuxMaintenanceKind.snapshotTerminalTemp, count: 3)
        )
        await scheduler.stop()
    }

    func testRetryableTaskFailureUsesFailureBackoffWithoutAdvancing() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(
            .started(StubDuxMaintenanceTask(polls: [.failed(.retryable)]))
        )
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await waitForCompletedAttempt(scheduler, service: service, count: 1)
        await scheduler.signal(.energyPolicyChanged)
        await clock.advance(by: 24)
        let countBeforeRetry = await service.startedKinds().count
        XCTAssertEqual(countBeforeRetry, 1)
        await clock.advance(by: 1)
        try await waitForCompletedAttempt(scheduler, service: service, count: 2)
        let starts = await service.startedKinds()
        XCTAssertEqual(starts, [.snapshotTerminalTemp, .snapshotTerminalTemp])
        await scheduler.stop()
    }

    func testTriggerDuringTaskCannotBypassFailureBackoff() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(
            .started(StubDuxMaintenanceTask(polls: [.running, .failed(.retryable)]))
        )
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually {
            await scheduler.snapshot().inFlightKind == .snapshotTerminalTemp
        }
        try await eventually { await service.firstTaskPollCount() == 1 }
        await scheduler.signal(.wake)
        await clock.advance(by: 1)
        try await waitForCompletedAttempt(scheduler, service: service, count: 1)

        await clock.advance(by: 24)
        let beforeBackoff = await service.startedKinds().count
        XCTAssertEqual(beforeBackoff, 1)
        await clock.advance(by: 1)
        try await waitForCompletedAttempt(scheduler, service: service, count: 2)
        await scheduler.stop()
    }

    func testBlockedFailureSkipsKindUntilRestart() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(
            .started(StubDuxMaintenanceTask(polls: [.failed(.blockedUntilRestart)]))
        )
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await waitForCompletedAttempt(scheduler, service: service, count: 1)
        await clock.advance(by: 2)
        try await waitForCompletedAttempt(scheduler, service: service, count: 2)
        let starts = await service.startedKinds()
        XCTAssertEqual(starts, [.snapshotTerminalTemp, .snapshotUnleasedTemp])
        await scheduler.stop()
    }

    func testBlockedKindDoesNotLeakBusyBackoffIntoNextKind() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(.deferredBusy)
        await service.enqueue(
            .started(StubDuxMaintenanceTask(polls: [.failed(.blockedUntilRestart)]))
        )
        await service.enqueue(.deferredBusy)
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await waitForCompletedAttempt(scheduler, service: service, count: 1)
        await clock.advance(by: 5)
        try await waitForCompletedAttempt(scheduler, service: service, count: 2)
        await clock.advance(by: 2)
        try await waitForCompletedAttempt(scheduler, service: service, count: 3)

        await clock.advance(by: 4)
        let countBeforeFreshBackoff = await service.startedKinds().count
        XCTAssertEqual(countBeforeFreshBackoff, 3)
        await clock.advance(by: 1)
        try await waitForCompletedAttempt(scheduler, service: service, count: 4)
        let starts = await service.startedKinds()
        XCTAssertEqual(
            starts,
            [
                .snapshotTerminalTemp,
                .snapshotTerminalTemp,
                .snapshotUnleasedTemp,
                .snapshotUnleasedTemp,
            ]
        )
        await scheduler.stop()
    }

    func testWakeAndActivationCoalesceWhileTaskRunsWithoutCancellingIt() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let task = StubDuxMaintenanceTask(polls: [.running])
        await service.enqueue(.started(task))
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually {
            await scheduler.snapshot().inFlightKind == .snapshotTerminalTemp
        }
        await scheduler.signal(.wake)
        await scheduler.signal(.applicationBecameActive)
        await scheduler.signal(.significantTimeChange)
        let cancellationsBeforeFinish = await task.cancellationCount()
        let coalescedBeforeFinish = await scheduler.snapshot().hasCoalescedTrigger
        XCTAssertEqual(cancellationsBeforeFinish, 0)
        XCTAssertTrue(coalescedBeforeFinish)

        await task.append(.finished(DuxMaintenanceBatchSignal(hasMore: false)))
        await clock.advance(by: 1)
        try await eventually { await scheduler.snapshot().inFlightKind == nil }
        let startsBeforeDelay = await service.startedKinds()
        XCTAssertEqual(startsBeforeDelay, [.snapshotTerminalTemp])
        await clock.advance(by: 1)
        let startsStillBeforeDelay = await service.startedKinds()
        XCTAssertEqual(startsStillBeforeDelay, [.snapshotTerminalTemp])
        await clock.advance(by: 1)
        try await eventually { await service.startedKinds().count == 2 }
        let starts = await service.startedKinds()
        let cancellationsAfterFinish = await task.cancellationCount()
        XCTAssertEqual(starts, [.snapshotTerminalTemp, .snapshotUnleasedTemp])
        XCTAssertEqual(cancellationsAfterFinish, 0)
        await scheduler.stop()
    }

    func testEnergyGateDefersUntilPolicyChangeTrigger() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let energy = StubDuxMaintenanceEnergyPolicy(permitted: false)
        let scheduler = makeScheduler(service: service, clock: clock, energy: energy)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually {
            await scheduler.snapshot().nextDeadline?.milliseconds == 50
        }
        let startsWhileBlocked = await service.startedKinds()
        XCTAssertEqual(startsWhileBlocked, [])

        await energy.setPermitted(true)
        await scheduler.signal(.energyPolicyChanged)
        try await waitForCompletedAttempt(scheduler, service: service, count: 1)
        let starts = await service.startedKinds()
        XCTAssertEqual(starts, [.snapshotTerminalTemp])
        await scheduler.stop()
    }

    func testAlreadyActiveTaskIsObservedWithoutAnotherAdmission() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(
            .alreadyActive(StubDuxMaintenanceTask(polls: [
                .finished(DuxMaintenanceBatchSignal(hasMore: false)),
            ]))
        )
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually {
            await scheduler.snapshot().nextKind == .snapshotUnleasedTemp
        }
        let starts = await service.startedKinds()
        XCTAssertEqual(starts, [.snapshotTerminalTemp])
        await scheduler.stop()
    }

    func testStopCancelsInFlightTaskAndPreventsFutureAdmissions() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let task = StubDuxMaintenanceTask(polls: [.running])
        await service.enqueue(.started(task))
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually {
            await scheduler.snapshot().inFlightKind == .snapshotTerminalTemp
        }
        await scheduler.stop()
        let cancellations = await task.cancellationCount()
        XCTAssertEqual(cancellations, 1)

        await clock.advance(by: 1_000)
        await Task.yield()
        let starts = await service.startedKinds()
        let isStarted = await scheduler.snapshot().isStarted
        XCTAssertEqual(starts, [.snapshotTerminalTemp])
        XCTAssertFalse(isStarted)
    }

    func testStopWhileEnergyCheckIsSuspendedPreventsStaleAdmission() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let energy = SuspendedDuxMaintenanceEnergyPolicy()
        let scheduler = makeScheduler(service: service, clock: clock, energy: energy)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually { await energy.hasSuspendedCheck() }

        let stopTask = Task {
            await scheduler.stop()
        }
        try await eventually { !(await scheduler.snapshot().isStarted) }
        await energy.resume(permitted: false)
        await stopTask.value

        let starts = await service.startedKinds()
        let snapshot = await scheduler.snapshot()
        XCTAssertEqual(starts, [])
        XCTAssertFalse(snapshot.isStarted)
        XCTAssertNil(snapshot.inFlightKind)
        XCTAssertNil(snapshot.nextDeadline)
    }

    func testStopWhilePollIsSuspendedIgnoresStaleCompletion() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let task = SuspendedDuxMaintenanceTask()
        await service.enqueue(.started(task))
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually { await task.hasSuspendedPoll() }

        let stopTask = Task {
            await scheduler.stop()
        }
        try await eventually { await task.cancellationCount() == 1 }

        await task.resumePoll(
            .finished(DuxMaintenanceBatchSignal(hasMore: false))
        )
        await stopTask.value
        await clock.advance(by: 1_000)
        await Task.yield()

        let starts = await service.startedKinds()
        let snapshot = await scheduler.snapshot()
        XCTAssertEqual(starts, [.snapshotTerminalTemp])
        XCTAssertFalse(snapshot.isStarted)
        XCTAssertNil(snapshot.inFlightKind)
        XCTAssertNil(snapshot.nextDeadline)
    }

    private func makeScheduler(
        service: StubDuxMaintenanceService,
        clock: ManualDuxMaintenanceClock,
        energy: any DuxMaintenanceEnergyPolicy = AlwaysPermittedDuxMaintenanceEnergyPolicy()
    ) -> DuxMaintenanceScheduler {
        DuxMaintenanceScheduler(
            service: service,
            clock: clock,
            energyPolicy: energy,
            timing: DuxMaintenanceSchedulerTiming(
                initialGraceMilliseconds: 10,
                interBatchDelayMilliseconds: 2,
                normalCadenceMilliseconds: 100,
                hasMoreDelayMilliseconds: 20,
                deferralDelayMilliseconds: 30,
                energyRetryMilliseconds: 40,
                pollIntervalMilliseconds: 1,
                busyBackoffMilliseconds: [5, 10],
                failureBackoffMilliseconds: [25, 50]
            )
        )
    }

    private func eventually(
        _ condition: @escaping @Sendable () async -> Bool,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws {
        for _ in 0 ..< 1_000 {
            if await condition() {
                return
            }
            await Task.yield()
        }
        XCTFail("Condition did not become true", file: file, line: line)
    }

    private func waitForCompletedAttempt(
        _ scheduler: DuxMaintenanceScheduler,
        service: StubDuxMaintenanceService,
        count: Int,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws {
        try await eventually({
            let starts = await service.startedKinds().count
            let inFlight = await scheduler.snapshot().inFlightKind
            return starts == count && inFlight == nil
        }, file: file, line: line)
    }
}

private actor ManualDuxMaintenanceClock: DuxMaintenanceSchedulingClock {
    private struct Sleeper {
        let deadline: DuxMaintenanceInstant
        let continuation: CheckedContinuation<Void, any Error>
    }

    private var current = DuxMaintenanceInstant(milliseconds: 0)
    private var sleepers: [UUID: Sleeper] = [:]

    func now() -> DuxMaintenanceInstant {
        current
    }

    func sleep(until deadline: DuxMaintenanceInstant) async throws {
        try Task.checkCancellation()
        guard current < deadline else {
            return
        }

        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation {
                (continuation: CheckedContinuation<Void, any Error>) in
                if Task.isCancelled {
                    continuation.resume(throwing: CancellationError())
                } else {
                    sleepers[id] = Sleeper(deadline: deadline, continuation: continuation)
                }
            }
        } onCancel: {
            Task {
                await self.cancelSleeper(id)
            }
        }
    }

    func advance(by milliseconds: Int64) {
        current = current.advanced(by: milliseconds)
        let ready = sleepers.filter { $0.value.deadline <= current }
        for (id, sleeper) in ready {
            sleepers.removeValue(forKey: id)
            sleeper.continuation.resume()
        }
    }

    private func cancelSleeper(_ id: UUID) {
        sleepers.removeValue(forKey: id)?.continuation.resume(throwing: CancellationError())
    }
}

private actor StubDuxMaintenanceEnergyPolicy: DuxMaintenanceEnergyPolicy {
    private var permitted: Bool

    init(permitted: Bool) {
        self.permitted = permitted
    }

    func permitsMaintenance() -> Bool {
        permitted
    }

    func setPermitted(_ permitted: Bool) {
        self.permitted = permitted
    }
}

private actor SuspendedDuxMaintenanceEnergyPolicy: DuxMaintenanceEnergyPolicy {
    private var continuation: CheckedContinuation<Bool, Never>?

    func permitsMaintenance() async -> Bool {
        await withCheckedContinuation { continuation in
            precondition(self.continuation == nil)
            self.continuation = continuation
        }
    }

    func hasSuspendedCheck() -> Bool {
        continuation != nil
    }

    func resume(permitted: Bool) {
        let pending = continuation
        continuation = nil
        pending?.resume(returning: permitted)
    }
}

private actor StubDuxMaintenanceService: DuxMaintenanceServing {
    private var admissions: [DuxMaintenanceStartDisposition] = []
    private var starts: [DuxMaintenanceKind] = []
    private var observedTasks: [any DuxMaintenanceTask] = []

    func enqueue(_ admission: DuxMaintenanceStartDisposition) {
        admissions.append(admission)
    }

    func startMaintenance(_ kind: DuxMaintenanceKind) -> DuxMaintenanceStartDisposition {
        starts.append(kind)
        let admission: DuxMaintenanceStartDisposition
        if admissions.isEmpty {
            admission = .started(
                StubDuxMaintenanceTask(polls: [
                    .finished(DuxMaintenanceBatchSignal(hasMore: false)),
                ])
            )
        } else {
            admission = admissions.removeFirst()
        }
        if case let .started(task) = admission {
            observedTasks.append(task)
        } else if case let .alreadyActive(task) = admission {
            observedTasks.append(task)
        }
        return admission
    }

    func startedKinds() -> [DuxMaintenanceKind] {
        starts
    }

    func firstTaskPollCount() async -> Int {
        await (observedTasks.first as? StubDuxMaintenanceTask)?.pollCount() ?? 0
    }
}

private actor StubDuxMaintenanceTask: DuxMaintenanceTask {
    private var polls: [DuxMaintenanceTaskPoll]
    private var cancellations = 0
    private var pollsObserved = 0

    init(polls: [DuxMaintenanceTaskPoll]) {
        precondition(!polls.isEmpty)
        self.polls = polls
    }

    func poll() -> DuxMaintenanceTaskPoll {
        pollsObserved += 1
        if polls.count > 1 {
            return polls.removeFirst()
        }
        return polls[0]
    }

    func requestCancellation() {
        cancellations += 1
    }

    func append(_ poll: DuxMaintenanceTaskPoll) {
        polls = [poll]
    }

    func cancellationCount() -> Int {
        cancellations
    }

    func pollCount() -> Int {
        pollsObserved
    }
}

private actor SuspendedDuxMaintenanceTask: DuxMaintenanceTask {
    private var continuation: CheckedContinuation<DuxMaintenanceTaskPoll, Never>?
    private var cancellations = 0

    func poll() async -> DuxMaintenanceTaskPoll {
        await withCheckedContinuation { continuation in
            precondition(self.continuation == nil)
            self.continuation = continuation
        }
    }

    func requestCancellation() {
        cancellations += 1
    }

    func hasSuspendedPoll() -> Bool {
        continuation != nil
    }

    func resumePoll(_ poll: DuxMaintenanceTaskPoll) {
        let pending = continuation
        continuation = nil
        pending?.resume(returning: poll)
    }

    func cancellationCount() -> Int {
        cancellations
    }
}
