import XCTest
@testable import DUX

final class CapacitySamplingSchedulerTests: XCTestCase {
    func testProductionCadenceIsExactlyFiveMinutes() {
        XCTAssertEqual(
            DuxCapacitySamplingTiming.production.cadenceMilliseconds,
            5 * 60 * 1_000
        )
    }

    func testLaunchSamplesImmediatelyThenUsesFiveMinuteStyleCadence() async throws {
        let clock = ManualCapacityClock()
        let sampler = await StubCapacitySampler()
        let scheduler = makeScheduler(sampler: sampler, clock: clock)

        await scheduler.start()
        try await eventually { await sampler.sampleCount == 1 }
        let scheduled = await scheduler.snapshot()
        XCTAssertEqual(scheduled.nextDeadline?.milliseconds, 300)

        await clock.advance(by: 299)
        let countBeforeCadence = await sampler.sampleCount
        XCTAssertEqual(countBeforeCadence, 1)
        await clock.advance(by: 1)
        try await eventually { await sampler.sampleCount == 2 }
        await scheduler.stop()
    }

    func testWakeAdvancesAnIdleCadenceDeadline() async throws {
        let clock = ManualCapacityClock()
        let sampler = await StubCapacitySampler()
        let scheduler = makeScheduler(sampler: sampler, clock: clock)

        await scheduler.start()
        try await eventually { await sampler.sampleCount == 1 }
        await clock.advance(by: 20)
        await scheduler.signal(.wake)
        try await eventually { await sampler.sampleCount == 2 }
        try await eventually {
            await scheduler.snapshot().nextDeadline?.milliseconds == 320
        }
        let scheduled = await scheduler.snapshot()
        XCTAssertEqual(scheduled.nextDeadline?.milliseconds, 320)
        await scheduler.stop()
    }

    func testSignalsDuringSampleCoalesceIntoExactlyOneFollowUp() async throws {
        let clock = ManualCapacityClock()
        let sampler = await StubCapacitySampler(blockFirstSample: true)
        let scheduler = makeScheduler(sampler: sampler, clock: clock)

        await scheduler.start()
        try await eventually { await sampler.sampleCount == 1 }
        await scheduler.signal(.wake)
        await scheduler.signal(.volumesChanged)
        await scheduler.signal(.manual)
        let during = await scheduler.snapshot()
        XCTAssertTrue(during.isSampling)
        XCTAssertTrue(during.hasCoalescedTrigger)

        await sampler.releaseFirstSample()
        try await eventually { await sampler.sampleCount == 2 }
        await Task.yield()
        let countAfterFollowUp = await sampler.sampleCount
        XCTAssertEqual(countAfterFollowUp, 2)
        let after = await scheduler.snapshot()
        XCTAssertFalse(after.hasCoalescedTrigger)
        XCTAssertEqual(after.nextDeadline?.milliseconds, 300)
        await scheduler.stop()
    }

    func testStopInvalidatesInFlightCompletionAndCancelsPublication() async throws {
        let clock = ManualCapacityClock()
        let sampler = await StubCapacitySampler(blockFirstSample: true)
        let scheduler = makeScheduler(sampler: sampler, clock: clock)

        await scheduler.start()
        try await eventually { await sampler.sampleCount == 1 }
        await scheduler.stop()
        let cancellationCount = await sampler.cancelCount
        XCTAssertEqual(cancellationCount, 1)
        let stopped = await scheduler.snapshot()
        XCTAssertFalse(stopped.isStarted)
        XCTAssertFalse(stopped.isSampling)
        XCTAssertNil(stopped.nextDeadline)

        await sampler.releaseFirstSample()
        await clock.advance(by: 1_000)
        await Task.yield()
        let countAfterStop = await sampler.sampleCount
        XCTAssertEqual(countAfterStop, 1)
    }

    func testStartAndStopAreIdempotent() async throws {
        let clock = ManualCapacityClock()
        let sampler = await StubCapacitySampler()
        let scheduler = makeScheduler(sampler: sampler, clock: clock)

        await scheduler.start()
        await scheduler.start()
        try await eventually { await sampler.sampleCount == 1 }
        await scheduler.stop()
        await scheduler.stop()
        let cancellationCount = await sampler.cancelCount
        XCTAssertEqual(cancellationCount, 1)
    }

    func testTerminalQuiescenceJoinsSampleRetainedAcrossOrdinaryStop() async throws {
        let clock = ManualCapacityClock()
        let sampler = await StubCapacitySampler(blockFirstSample: true)
        let scheduler = makeScheduler(sampler: sampler, clock: clock)
        let completion = CapacityTerminalCompletionProbe()

        await scheduler.start()
        try await eventually { await sampler.sampleCount == 1 }
        await scheduler.stop()

        let terminal = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }
        try await eventually { await sampler.cancelCount == 2 }
        let completedBeforeRelease = await completion.isCompleted()
        XCTAssertFalse(completedBeforeRelease)

        await sampler.releaseFirstSample()
        await terminal.value
        let completedAfterRelease = await completion.isCompleted()
        XCTAssertTrue(completedAfterRelease)
    }

    func testTerminalQuiescencePermanentlyRejectsStartAndSignals() async throws {
        let clock = ManualCapacityClock()
        let sampler = await StubCapacitySampler()
        let scheduler = makeScheduler(sampler: sampler, clock: clock)

        await scheduler.quiesceForTerminalRuntime()
        await scheduler.quiesceForTerminalRuntime()
        await scheduler.start()
        await scheduler.signal(.wake)
        await scheduler.signal(.volumesChanged)
        await clock.advance(by: 1_000)
        await Task.yield()

        let snapshot = await scheduler.snapshot()
        XCTAssertFalse(snapshot.isStarted)
        XCTAssertFalse(snapshot.isSampling)
        XCTAssertNil(snapshot.nextDeadline)
        let sampleCount = await sampler.sampleCount
        let cancelCount = await sampler.cancelCount
        XCTAssertEqual(sampleCount, 0)
        XCTAssertEqual(cancelCount, 1)
    }

    func testTerminalQuiescenceJoinsSuspendedStartAndSurvivesCallerCancellation() async throws {
        let clock = SuspendedCapacityNowClock()
        let sampler = await StubCapacitySampler()
        let scheduler = makeScheduler(sampler: sampler, clock: clock)
        let firstCompletion = CapacityTerminalCompletionProbe()
        let secondCompletion = CapacityTerminalCompletionProbe()

        let start = Task {
            await scheduler.start()
        }
        try await eventually { await clock.hasSuspendedNow() }

        let first = Task {
            await scheduler.quiesceForTerminalRuntime()
            await firstCompletion.markCompleted()
        }
        try await eventually { !(await scheduler.snapshot().isStarted) }
        first.cancel()
        let second = Task {
            await scheduler.quiesceForTerminalRuntime()
            await secondCompletion.markCompleted()
        }
        await Task.yield()
        let firstCompletedBeforeRelease = await firstCompletion.isCompleted()
        let secondCompletedBeforeRelease = await secondCompletion.isCompleted()
        XCTAssertFalse(firstCompletedBeforeRelease)
        XCTAssertFalse(secondCompletedBeforeRelease)

        await clock.resumeNow()
        await start.value
        await first.value
        await second.value

        let firstCompleted = await firstCompletion.isCompleted()
        let secondCompleted = await secondCompletion.isCompleted()
        let sampleCount = await sampler.sampleCount
        let cancelCount = await sampler.cancelCount
        XCTAssertTrue(firstCompleted)
        XCTAssertTrue(secondCompleted)
        XCTAssertEqual(sampleCount, 0)
        XCTAssertEqual(cancelCount, 1)
    }

    func testTerminalQuiescenceJoinsSignalSuspendedBeforeDriverAdmission() async throws {
        let clock = ManualCapacityClock()
        let sampler = await StubCapacitySampler()
        let scheduler = makeScheduler(sampler: sampler, clock: clock)
        let completion = CapacityTerminalCompletionProbe()

        await scheduler.start()
        try await eventually {
            await scheduler.snapshot().nextDeadline?.milliseconds == 300
        }
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
        let completedBeforeRelease = await completion.isCompleted()
        XCTAssertFalse(completedBeforeRelease)

        await clock.resumeNow()
        await signal.value
        await terminal.value

        let completed = await completion.isCompleted()
        let sampleCount = await sampler.sampleCount
        XCTAssertTrue(completed)
        XCTAssertEqual(sampleCount, 1)
    }

    func testReentrantSamplerGetsNoProofAndOuterTerminalDrainCompletes() async throws {
        let clock = ManualCapacityClock()
        let sampler = ReentrantCapacitySampler()
        let scheduler = DuxCapacitySamplingScheduler(
            sampler: sampler,
            clock: clock,
            timing: DuxCapacitySamplingTiming(cadenceMilliseconds: 300)
        )
        await sampler.attach(scheduler)

        await scheduler.start()
        try await eventually {
            await sampler.observation() == .reentrantNoProof
        }
        let outer = await scheduler.terminalQuiescenceObservation()
        let reentrant = await sampler.observation()
        let sampleCount = await sampler.sampleCount

        XCTAssertEqual(outer, .completed)
        XCTAssertEqual(reentrant, .reentrantNoProof)
        XCTAssertEqual(sampleCount, 1)
    }

    func testPolicyResampleRouterCoalescesPendingAttachmentAndInvalidates() async {
        let router = DuxCapacityResampleRouter()
        let scheduler = ResampleSchedulerSpy()

        await router.requestCapacityResample()
        await router.requestCapacityResample()
        await router.attach(scheduler)
        var signalCount = await scheduler.signalCount()
        XCTAssertEqual(signalCount, 1)

        await router.requestCapacityResample()
        signalCount = await scheduler.signalCount()
        XCTAssertEqual(signalCount, 2)

        await router.invalidate()
        await router.requestCapacityResample()
        await router.attach(scheduler)
        signalCount = await scheduler.signalCount()
        XCTAssertEqual(signalCount, 2)
    }

    private func makeScheduler(
        sampler: StubCapacitySampler,
        clock: any DuxMaintenanceSchedulingClock
    ) -> DuxCapacitySamplingScheduler {
        DuxCapacitySamplingScheduler(
            sampler: sampler,
            clock: clock,
            timing: DuxCapacitySamplingTiming(cadenceMilliseconds: 300)
        )
    }

    private func eventually(
        attempts: Int = 200,
        _ condition: @escaping @Sendable () async -> Bool
    ) async throws {
        for _ in 0 ..< attempts {
            if await condition() {
                return
            }
            await Task.yield()
        }
        XCTFail("Timed out waiting for asynchronous state")
    }
}

@MainActor
private final class ReentrantCapacitySampler: DuxCapacitySampling {
    private weak var scheduler: DuxCapacitySamplingScheduler?
    private(set) var sampleCount = 0
    private var terminalObservation: DuxCapacityTerminalQuiescenceObservation?

    func attach(_ scheduler: DuxCapacitySamplingScheduler) {
        self.scheduler = scheduler
    }

    func refreshVolumeCapacity() async {
        sampleCount += 1
        terminalObservation = await scheduler?.terminalQuiescenceObservation()
    }

    func cancelVolumeRefresh() {}

    func observation() -> DuxCapacityTerminalQuiescenceObservation? {
        terminalObservation
    }
}

private actor CapacityTerminalCompletionProbe {
    private var completed = false

    func markCompleted() {
        completed = true
    }

    func isCompleted() -> Bool {
        completed
    }
}

private actor ResampleSchedulerSpy: DuxCapacityScheduling {
    private var signals = 0

    func start() {}

    func signal(_ trigger: DuxCapacitySamplingTrigger) {
        XCTAssertEqual(trigger, .manual)
        signals += 1
    }

    func stop() {}

    func quiesceForTerminalRuntime() async { await stop() }

    func signalCount() -> Int { signals }
}

@MainActor
private final class StubCapacitySampler: DuxCapacitySampling {
    private(set) var sampleCount = 0
    private(set) var cancelCount = 0
    private var blockFirstSample: Bool
    private var firstSampleContinuation: CheckedContinuation<Void, Never>?

    init(blockFirstSample: Bool = false) {
        self.blockFirstSample = blockFirstSample
    }

    func refreshVolumeCapacity() async {
        sampleCount += 1
        if blockFirstSample, sampleCount == 1 {
            await withCheckedContinuation { continuation in
                firstSampleContinuation = continuation
            }
        }
    }

    func cancelVolumeRefresh() {
        cancelCount += 1
    }

    func releaseFirstSample() {
        blockFirstSample = false
        firstSampleContinuation?.resume()
        firstSampleContinuation = nil
    }
}

private actor ManualCapacityClock: DuxMaintenanceSchedulingClock {
    private struct Sleeper {
        let deadline: DuxMaintenanceInstant
        let continuation: CheckedContinuation<Void, Error>
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
        if deadline <= current {
            return
        }
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                sleepers[id] = Sleeper(deadline: deadline, continuation: continuation)
            }
        } onCancel: {
            Task { await self.cancel(id) }
        }
    }

    func advance(by milliseconds: Int64) {
        current = current.advanced(by: milliseconds)
        let due = sleepers.filter { $0.value.deadline <= current }
        for (id, sleeper) in due {
            sleepers.removeValue(forKey: id)
            sleeper.continuation.resume()
        }
    }

    private func cancel(_ id: UUID) {
        sleepers.removeValue(forKey: id)?.continuation.resume(throwing: CancellationError())
    }
}

private actor SuspendedCapacityNowClock: DuxMaintenanceSchedulingClock {
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
