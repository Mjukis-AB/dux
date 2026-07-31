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
        XCTAssertEqual(firstStarts, [.scanRecovery])

        for expectedCount in 2 ... 8 {
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
        XCTAssertEqual(afterCycle.nextDeadline?.milliseconds, 124)
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
        XCTAssertEqual(starts, [.scanRecovery, .candidateEvaluationRecovery])
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
            Array(repeating: DuxMaintenanceKind.scanRecovery, count: 3)
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
        XCTAssertEqual(starts, [.scanRecovery, .scanRecovery])
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
            await scheduler.snapshot().inFlightKind == .scanRecovery
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
        XCTAssertEqual(starts, [.scanRecovery, .candidateEvaluationRecovery])
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
                .scanRecovery,
                .scanRecovery,
                .candidateEvaluationRecovery,
                .candidateEvaluationRecovery,
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
            await scheduler.snapshot().inFlightKind == .scanRecovery
        }
        await scheduler.signal(.wake)
        await scheduler.signal(.applicationBecameActive)
        await scheduler.signal(.significantTimeChange)
        let cancellationsBeforeFinish = await task.cancellationCount()
        let coalescedBeforeFinish = await scheduler.snapshot().hasCoalescedTrigger
        XCTAssertEqual(cancellationsBeforeFinish, 0)
        XCTAssertTrue(coalescedBeforeFinish)

        try await eventually {
            let polls = await task.pollCount()
            let isSleeping = await clock.hasSleeper(at: 11)
            return polls == 1 && isSleeping
        }
        await task.append(.finished(DuxMaintenanceBatchSignal(hasMore: false)))
        await clock.advance(by: 1)
        try await eventually { await scheduler.snapshot().inFlightKind == nil }
        try await eventually {
            let deadline = await scheduler.snapshot().nextDeadline?.milliseconds
            let isSleeping = await clock.hasSleeper(at: 13)
            return deadline == 13 && isSleeping
        }
        let startsBeforeDelay = await service.startedKinds()
        XCTAssertEqual(startsBeforeDelay, [.scanRecovery])
        await clock.advance(by: 1)
        let startsStillBeforeDelay = await service.startedKinds()
        XCTAssertEqual(startsStillBeforeDelay, [.scanRecovery])
        await clock.advance(by: 1)
        try await eventually { await service.startedKinds().count == 2 }
        let starts = await service.startedKinds()
        let cancellationsAfterFinish = await task.cancellationCount()
        XCTAssertEqual(starts, [.scanRecovery, .candidateEvaluationRecovery])
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
        XCTAssertEqual(starts, [.scanRecovery])
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
            await scheduler.snapshot().nextKind == .candidateEvaluationRecovery
        }
        let starts = await service.startedKinds()
        XCTAssertEqual(starts, [.scanRecovery])
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
            await scheduler.snapshot().inFlightKind == .scanRecovery
        }
        await scheduler.stop()
        let cancellations = await task.cancellationCount()
        XCTAssertEqual(cancellations, 1)

        await clock.advance(by: 1_000)
        await Task.yield()
        let starts = await service.startedKinds()
        let isStarted = await scheduler.snapshot().isStarted
        XCTAssertEqual(starts, [.scanRecovery])
        XCTAssertFalse(isStarted)
    }

    func testStopCancelsInFlightCandidateEvaluationRecovery() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        await service.enqueue(
            .started(StubDuxMaintenanceTask(polls: [
                .finished(DuxMaintenanceBatchSignal(hasMore: false)),
            ]))
        )
        let recoveryTask = StubDuxMaintenanceTask(polls: [.running])
        await service.enqueue(.started(recoveryTask))
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await waitForCompletedAttempt(scheduler, service: service, count: 1)
        await clock.advance(by: 2)
        try await eventually {
            await scheduler.snapshot().inFlightKind == .candidateEvaluationRecovery
        }

        await scheduler.stop()

        let cancellationCount = await recoveryTask.cancellationCount()
        let startedKinds = await service.startedKinds()
        let isStarted = await scheduler.snapshot().isStarted
        XCTAssertEqual(cancellationCount, 1)
        XCTAssertEqual(startedKinds, [.scanRecovery, .candidateEvaluationRecovery])
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
        XCTAssertEqual(starts, [.scanRecovery])
        XCTAssertFalse(snapshot.isStarted)
        XCTAssertNil(snapshot.inFlightKind)
        XCTAssertNil(snapshot.nextDeadline)
    }

    func testTerminalQuiescenceCoalescesAndJoinsSuspendedPoll() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let task = SuspendedDuxMaintenanceTask()
        let completion = MaintenanceTerminalCompletionProbe()
        await service.enqueue(.started(task))
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually { await task.hasSuspendedPoll() }

        let first = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }
        let second = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }
        try await eventually { await task.cancellationCount() == 1 }
        let completionCountBeforeRelease = await completion.completedCount()
        XCTAssertEqual(completionCountBeforeRelease, 0)

        await scheduler.start()
        await scheduler.signal(.wake)
        await task.resumePoll(.cancelled)
        await first.value
        await second.value
        await clock.advance(by: 1_000)
        await Task.yield()

        let completionCountAfterRelease = await completion.completedCount()
        let cancellationCount = await task.cancellationCount()
        let startedKinds = await service.startedKinds()
        XCTAssertEqual(completionCountAfterRelease, 2)
        XCTAssertEqual(cancellationCount, 1)
        XCTAssertEqual(startedKinds, [.scanRecovery])
        let snapshot = await scheduler.snapshot()
        XCTAssertFalse(snapshot.isStarted)
        XCTAssertNil(snapshot.inFlightKind)
        XCTAssertNil(snapshot.nextDeadline)
    }

    func testTerminalQuiescenceJoinsSuspendedEnergyAdmissionAndRejectsLateWork() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let energy = SuspendedDuxMaintenanceEnergyPolicy()
        let completion = MaintenanceTerminalCompletionProbe()
        let scheduler = makeScheduler(service: service, clock: clock, energy: energy)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually { await energy.hasSuspendedCheck() }

        let terminal = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }
        await Task.yield()
        let completionCountBeforeRelease = await completion.completedCount()
        XCTAssertEqual(completionCountBeforeRelease, 0)
        await scheduler.start()
        await scheduler.signal(.applicationBecameActive)

        await energy.resume(permitted: true)
        await terminal.value
        await clock.advance(by: 1_000)
        await Task.yield()

        let completionCountAfterRelease = await completion.completedCount()
        let startedKinds = await service.startedKinds()
        XCTAssertEqual(completionCountAfterRelease, 1)
        XCTAssertEqual(startedKinds, [])
        let snapshot = await scheduler.snapshot()
        XCTAssertFalse(snapshot.isStarted)
        XCTAssertNil(snapshot.inFlightKind)
        XCTAssertNil(snapshot.nextDeadline)
    }

    func testTerminalQuiescenceJoinsSuspendedStartAndSurvivesCallerCancellation() async throws {
        let clock = SuspendedMaintenanceNowClock()
        let service = StubDuxMaintenanceService()
        let scheduler = makeScheduler(service: service, clock: clock)
        let completion = MaintenanceTerminalCompletionProbe()

        let start = Task {
            await scheduler.start()
        }
        try await eventually { await clock.hasSuspendedNow() }

        let first = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }
        try await eventually { !(await scheduler.snapshot().isStarted) }
        first.cancel()
        let second = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }
        await Task.yield()
        let completionsBeforeRelease = await completion.completedCount()
        XCTAssertEqual(completionsBeforeRelease, 0)

        await clock.resumeNow()
        await start.value
        await first.value
        await second.value

        let completions = await completion.completedCount()
        let starts = await service.startedKinds()
        XCTAssertEqual(completions, 2)
        XCTAssertEqual(starts, [])
    }

    func testTerminalQuiescenceJoinsSignalSuspendedBeforeDriverAdmission() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let scheduler = makeScheduler(service: service, clock: clock)
        let completion = MaintenanceTerminalCompletionProbe()

        await scheduler.start()
        await clock.suspendNextNowCall()
        let signal = Task {
            await scheduler.signal(.wake)
        }
        try await eventually { await clock.hasSuspendedNow() }

        let terminal = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }
        await Task.yield()
        let completedBeforeRelease = await completion.completedCount()
        XCTAssertEqual(completedBeforeRelease, 0)

        await clock.resumeNow()
        await signal.value
        await terminal.value

        let completed = await completion.completedCount()
        let starts = await service.startedKinds()
        XCTAssertEqual(completed, 1)
        XCTAssertEqual(starts, [])
    }

    func testTerminalQuiescenceCannotOvertakeAcceptedOrdinaryStopCancellation() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let task = SuspendedCancellationDuxMaintenanceTask()
        let completion = MaintenanceTerminalCompletionProbe()
        await service.enqueue(.started(task))
        let scheduler = makeScheduler(service: service, clock: clock)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually { await task.hasSuspendedPoll() }

        let stop = Task {
            await scheduler.stop()
        }
        try await eventually { await task.hasSuspendedCancellation() }
        let terminal = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }

        await task.resumePoll(.cancelled)
        await Task.yield()
        let completedBeforeCancellation = await completion.completedCount()
        XCTAssertEqual(completedBeforeCancellation, 0)

        await task.resumeCancellation()
        await stop.value
        await terminal.value

        let completed = await completion.completedCount()
        let cancellationCount = await task.cancellationCount()
        XCTAssertEqual(completed, 1)
        XCTAssertEqual(cancellationCount, 1)
    }

    func testReentrantCancellationGetsNoProofAndOuterTerminalDrainCompletes() async throws {
        let clock = ManualDuxMaintenanceClock()
        let service = StubDuxMaintenanceService()
        let task = ReentrantCancellationDuxMaintenanceTask()
        await service.enqueue(.started(task))
        let scheduler = makeScheduler(service: service, clock: clock)
        await task.attach(scheduler)

        await scheduler.start()
        await clock.advance(by: 10)
        try await eventually { await task.hasSuspendedPoll() }

        let outer = await scheduler.terminalQuiescenceObservation()
        let reentrant = await task.observation()
        let cancellationCount = await task.cancellationCount()

        XCTAssertEqual(outer, .completed)
        XCTAssertEqual(reentrant, .reentrantNoProof)
        XCTAssertEqual(cancellationCount, 1)
    }

    private func makeScheduler(
        service: StubDuxMaintenanceService,
        clock: any DuxMaintenanceSchedulingClock,
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

private actor MaintenanceTerminalCompletionProbe {
    private var count = 0

    func markCompleted() {
        count += 1
    }

    func completedCount() -> Int {
        count
    }
}

private actor ManualDuxMaintenanceClock: DuxMaintenanceSchedulingClock {
    private struct Sleeper {
        let deadline: DuxMaintenanceInstant
        let continuation: CheckedContinuation<Void, any Error>
    }

    private var current = DuxMaintenanceInstant(milliseconds: 0)
    private var sleepers: [UUID: Sleeper] = [:]
    private var suspendNextNow = false
    private var nowContinuation: CheckedContinuation<DuxMaintenanceInstant, Never>?

    func now() async -> DuxMaintenanceInstant {
        if suspendNextNow {
            suspendNextNow = false
            return await withCheckedContinuation { continuation in
                precondition(nowContinuation == nil)
                nowContinuation = continuation
            }
        }
        return current
    }

    func suspendNextNowCall() {
        precondition(nowContinuation == nil)
        suspendNextNow = true
    }

    func hasSuspendedNow() -> Bool {
        nowContinuation != nil
    }

    func resumeNow() {
        let pending = nowContinuation
        nowContinuation = nil
        pending?.resume(returning: current)
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

    func hasSleeper(at milliseconds: Int64) -> Bool {
        sleepers.values.contains {
            $0.deadline == DuxMaintenanceInstant(milliseconds: milliseconds)
        }
    }

    private func cancelSleeper(_ id: UUID) {
        sleepers.removeValue(forKey: id)?.continuation.resume(throwing: CancellationError())
    }
}

private actor SuspendedMaintenanceNowClock: DuxMaintenanceSchedulingClock {
    private var continuation: CheckedContinuation<DuxMaintenanceInstant, Never>?

    func now() async -> DuxMaintenanceInstant {
        await withCheckedContinuation { continuation in
            precondition(self.continuation == nil)
            self.continuation = continuation
        }
    }

    func sleep(until _: DuxMaintenanceInstant) async throws {
        throw CancellationError()
    }

    func hasSuspendedNow() -> Bool {
        continuation != nil
    }

    func resumeNow() {
        let pending = continuation
        continuation = nil
        pending?.resume(returning: DuxMaintenanceInstant(milliseconds: 0))
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

private actor SuspendedCancellationDuxMaintenanceTask: DuxMaintenanceTask {
    private var pollContinuation: CheckedContinuation<DuxMaintenanceTaskPoll, Never>?
    private var cancellationContinuation: CheckedContinuation<Void, Never>?
    private var cancellations = 0

    func poll() async -> DuxMaintenanceTaskPoll {
        await withCheckedContinuation { continuation in
            precondition(pollContinuation == nil)
            pollContinuation = continuation
        }
    }

    func requestCancellation() async {
        cancellations += 1
        await withCheckedContinuation { continuation in
            precondition(cancellationContinuation == nil)
            cancellationContinuation = continuation
        }
    }

    func hasSuspendedPoll() -> Bool {
        pollContinuation != nil
    }

    func hasSuspendedCancellation() -> Bool {
        cancellationContinuation != nil
    }

    func resumePoll(_ poll: DuxMaintenanceTaskPoll) {
        let pending = pollContinuation
        pollContinuation = nil
        pending?.resume(returning: poll)
    }

    func resumeCancellation() {
        let pending = cancellationContinuation
        cancellationContinuation = nil
        pending?.resume()
    }

    func cancellationCount() -> Int {
        cancellations
    }
}

private actor ReentrantCancellationDuxMaintenanceTask: DuxMaintenanceTask {
    private weak var scheduler: DuxMaintenanceScheduler?
    private var pollContinuation: CheckedContinuation<DuxMaintenanceTaskPoll, Never>?
    private var terminalObservation: DuxMaintenanceTerminalQuiescenceObservation?
    private var cancellations = 0

    func attach(_ scheduler: DuxMaintenanceScheduler) {
        self.scheduler = scheduler
    }

    func poll() async -> DuxMaintenanceTaskPoll {
        await withCheckedContinuation { continuation in
            precondition(pollContinuation == nil)
            pollContinuation = continuation
        }
    }

    func requestCancellation() async {
        cancellations += 1
        terminalObservation = await scheduler?.terminalQuiescenceObservation()
        let pending = pollContinuation
        pollContinuation = nil
        pending?.resume(returning: .cancelled)
    }

    func hasSuspendedPoll() -> Bool {
        pollContinuation != nil
    }

    func observation() -> DuxMaintenanceTerminalQuiescenceObservation? {
        terminalObservation
    }

    func cancellationCount() -> Int {
        cancellations
    }
}
