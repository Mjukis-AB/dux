@testable import DUX
import XCTest

final class AutomationDecisionSchedulerTests: XCTestCase {
    func testProductionSourceIsSealedInactiveAndEffectDormant() async throws {
        let instant = DuxAutomationWallInstant(milliseconds: 42)

        let observation = try await NoEnabledSchedulesDuxAutomationDecisionSource()
            .observeAutomationDecision(at: instant, reasons: [.launch, .wake])

        XCTAssertEqual(
            observation,
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: nil
            )
        )
    }

    func testLaunchChecksImmediatelyAndExactFutureDeadlineChecksOnce() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 100)
        let source = StubAutomationDecisionSource { instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: instant.advanced(by: 50)
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock)

        await scheduler.start()
        try await eventually {
            await scheduler.snapshot().nextDeadline?.milliseconds == 150
        }
        var calls = await source.calls()
        XCTAssertEqual(calls[0].reasons, .launch)
        let snapshot = await scheduler.snapshot()
        XCTAssertEqual(snapshot.nextDeadline?.milliseconds, 150)

        await clock.advance(by: 49)
        let callCount = await source.callCount()
        XCTAssertEqual(callCount, 1)
        await clock.advance(by: 1)
        try await eventually { await source.callCount() == 2 }
        calls = await source.calls()
        XCTAssertEqual(calls[1].reasons, .deadline)
        await scheduler.stop()
    }

    func testWakeForcesEarlyReevaluationAndReplacesIdleDeadline() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 10)
        let source = StubAutomationDecisionSource { instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: instant.advanced(by: 100)
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock)

        await scheduler.start()
        try await eventually { await source.callCount() == 1 }
        await clock.advance(by: 20)
        await scheduler.signal(.wake)
        try await eventually {
            let snapshot = await scheduler.snapshot()
            return await source.callCount() == 2
                && snapshot.nextDeadline?.milliseconds == 130
        }

        let calls = await source.calls()
        XCTAssertEqual(calls[1].instant.milliseconds, 30)
        XCTAssertEqual(calls[1].reasons, .wake)
        let snapshot = await scheduler.snapshot()
        XCTAssertEqual(snapshot.nextDeadline?.milliseconds, 130)
        await scheduler.stop()
    }

    func testSignificantBackwardTimeChangeCancelsAndRearmsWallDeadline() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 100)
        let source = StubAutomationDecisionSource { instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: instant.advanced(by: 100)
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock)

        await scheduler.start()
        try await eventually { await source.callCount() == 1 }
        await clock.jump(to: 50, releaseDueSleepers: false)
        await scheduler.signal(.significantTimeChange)
        try await eventually {
            let snapshot = await scheduler.snapshot()
            return await source.callCount() == 2
                && snapshot.nextDeadline?.milliseconds == 150
        }

        let calls = await source.calls()
        XCTAssertEqual(calls[1].instant.milliseconds, 50)
        XCTAssertEqual(calls[1].reasons, .significantTimeChange)
        let snapshot = await scheduler.snapshot()
        XCTAssertEqual(snapshot.nextDeadline?.milliseconds, 150)
        await clock.advance(by: 99)
        let callCount = await source.callCount()
        XCTAssertEqual(callCount, 2)
        await clock.advance(by: 1)
        try await eventually { await source.callCount() == 3 }
        await scheduler.stop()
    }

    func testWakeAndTimeChangeDuringDecisionUnionIntoOneFollowUp() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 0)
        let source = StubAutomationDecisionSource(blockFirstCall: true) {
            instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: nil
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock)

        await scheduler.start()
        try await eventually { await source.callCount() == 1 }
        await scheduler.signal(.wake)
        await scheduler.signal(.significantTimeChange)
        let during = await scheduler.snapshot()
        XCTAssertEqual(during.pendingReasons, [.wake, .significantTimeChange])

        await source.releaseFirstCall()
        try await eventually { await source.callCount() == 2 }
        await Task.yield()
        let calls = await source.calls()
        XCTAssertEqual(calls.count, 2)
        XCTAssertEqual(calls[1].reasons, [.wake, .significantTimeChange])
        let after = await scheduler.snapshot()
        XCTAssertTrue(after.pendingReasons.isEmpty)
        await scheduler.stop()
    }

    func testOneOptionalMissedObservationIsPublishedWithoutFollowUpLoop() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 1000)
        let source = StubAutomationDecisionSource { instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: DuxAutomationDueObservation(
                    scheduleID: "schedule:oldest",
                    scheduleRevision: 7,
                    scheduledAt: DuxAutomationWallInstant(milliseconds: 10),
                    classification: .missed
                ),
                nextCheckAt: nil
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock)

        await scheduler.start()
        try await eventually {
            await scheduler.snapshot().lastObservation?.due?.scheduleID
                == "schedule:oldest"
        }

        let snapshot = await scheduler.snapshot()
        XCTAssertEqual(snapshot.lastObservation?.due?.scheduleID, "schedule:oldest")
        XCTAssertEqual(snapshot.lastObservation?.due?.classification, .missed)
        XCTAssertNil(snapshot.nextDeadline)
        let callCount = await source.callCount()
        XCTAssertEqual(callCount, 1)
        await scheduler.stop()
    }

    func testInvalidObservationFailsClosedAndUsesBoundedRetry() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 100)
        let source = StubAutomationDecisionSource { instant, _, call in
            if call == 0 {
                return DuxAutomationDecisionObservation(
                    checkedAt: instant.advanced(by: 1),
                    due: nil,
                    nextCheckAt: instant
                )
            }
            return DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: nil
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock, failureRetry: 10)

        await scheduler.start()
        try await eventually {
            let snapshot = await scheduler.snapshot()
            return snapshot.lastFailure == .invalidObservation
                && snapshot.nextDeadline?.milliseconds == 110
        }
        var snapshot = await scheduler.snapshot()
        XCTAssertNil(snapshot.lastObservation)
        XCTAssertEqual(snapshot.nextDeadline?.milliseconds, 110)

        await clock.advance(by: 9)
        let callCount = await source.callCount()
        XCTAssertEqual(callCount, 1)
        await clock.advance(by: 1)
        try await eventually {
            let snapshot = await scheduler.snapshot()
            return await source.callCount() == 2
                && snapshot.lastFailure == nil
                && snapshot.lastObservation != nil
        }
        snapshot = await scheduler.snapshot()
        XCTAssertNil(snapshot.lastFailure)
        XCTAssertNotNil(snapshot.lastObservation)
        await scheduler.stop()
    }

    func testContradictoryDueClassificationFailsClosed() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 100)
        let source = StubAutomationDecisionSource { instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: DuxAutomationDueObservation(
                    scheduleID: "schedule:contradiction",
                    scheduleRevision: 1,
                    scheduledAt: instant.advanced(by: -1),
                    classification: .onTime
                ),
                nextCheckAt: nil
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock, failureRetry: 10)

        await scheduler.start()
        try await eventually {
            let snapshot = await scheduler.snapshot()
            return snapshot.lastFailure == .invalidObservation
                && snapshot.lastObservation == nil
                && snapshot.nextDeadline?.milliseconds == 110
        }
        await scheduler.stop()
    }

    func testDeadlineThatPassesDuringReadFailsClosedWithoutHotLoop() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 20)
        let source = StubAutomationDecisionSource(blockFirstCall: true) {
            instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: instant.advanced(by: 1)
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock, failureRetry: 10)

        await scheduler.start()
        try await eventually { await source.callCount() == 1 }
        await clock.advance(by: 5)
        await source.releaseFirstCall()
        try await eventually {
            let snapshot = await scheduler.snapshot()
            return snapshot.lastFailure == .invalidObservation
                && snapshot.nextDeadline?.milliseconds == 35
        }

        let snapshot = await scheduler.snapshot()
        XCTAssertEqual(snapshot.nextDeadline?.milliseconds, 35)
        let callCount = await source.callCount()
        XCTAssertEqual(callCount, 1)
        await scheduler.stop()
    }

    func testSourceFailureUsesBoundedRetryAndPreservesConfirmedObservation() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 0)
        let source = StubAutomationDecisionSource { instant, _, call in
            if call == 1 {
                throw AutomationDecisionTestError.unavailable
            }
            return DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: instant.advanced(by: 20)
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock, failureRetry: 10)

        await scheduler.start()
        try await eventually {
            await scheduler.snapshot().lastObservation != nil
        }
        let confirmed = await scheduler.snapshot().lastObservation
        await scheduler.signal(.wake)
        try await eventually {
            let snapshot = await scheduler.snapshot()
            return snapshot.lastFailure == .sourceUnavailable
                && snapshot.nextDeadline?.milliseconds == 10
        }

        let failed = await scheduler.snapshot()
        XCTAssertEqual(failed.lastObservation, confirmed)
        XCTAssertEqual(failed.nextDeadline?.milliseconds, 10)
        await scheduler.stop()
    }

    func testStopJoinsSuspendedReadAndRejectsLatePublication() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 0)
        let source = StubAutomationDecisionSource(blockFirstCall: true) {
            instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: instant.advanced(by: 100)
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock)
        let completion = AutomationDecisionCompletionProbe()

        await scheduler.start()
        try await eventually { await source.callCount() == 1 }
        let stop = Task {
            await scheduler.stop()
            await completion.markCompleted()
        }
        await Task.yield()
        let completed = await completion.isCompleted()
        XCTAssertFalse(completed)

        await source.releaseFirstCall()
        await stop.value
        let snapshot = await scheduler.snapshot()
        XCTAssertFalse(snapshot.isStarted)
        XCTAssertNil(snapshot.lastObservation)
        XCTAssertNil(snapshot.nextDeadline)
    }

    func testTerminalQuiescenceJoinsReadSurvivesCallerCancellationAndPermanentlyFences() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 0)
        let source = StubAutomationDecisionSource(blockFirstCall: true) {
            instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: nil
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock)
        let firstCompletion = AutomationDecisionCompletionProbe()
        let secondCompletion = AutomationDecisionCompletionProbe()

        await scheduler.start()
        try await eventually { await source.callCount() == 1 }
        let first = Task {
            await scheduler.quiesceForTerminalRuntime()
            await firstCompletion.markCompleted()
        }
        first.cancel()
        let second = Task {
            await scheduler.quiesceForTerminalRuntime()
            await secondCompletion.markCompleted()
        }
        await Task.yield()
        var firstCompleted = await firstCompletion.isCompleted()
        var secondCompleted = await secondCompletion.isCompleted()
        XCTAssertFalse(firstCompleted)
        XCTAssertFalse(secondCompleted)

        await source.releaseFirstCall()
        await first.value
        await second.value
        await scheduler.start()
        await scheduler.signal(.wake)
        await clock.advance(by: 1000)
        await Task.yield()

        firstCompleted = await firstCompletion.isCompleted()
        secondCompleted = await secondCompletion.isCompleted()
        let callCount = await source.callCount()
        let snapshot = await scheduler.snapshot()
        XCTAssertTrue(firstCompleted)
        XCTAssertTrue(secondCompleted)
        XCTAssertEqual(callCount, 1)
        XCTAssertFalse(snapshot.isStarted)
    }

    func testTerminalQuiescenceJoinsSignalSuspendedBeforeDriverAdmission() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 0)
        let source = StubAutomationDecisionSource { instant, _, _ in
            DuxAutomationDecisionObservation(
                checkedAt: instant,
                due: nil,
                nextCheckAt: instant.advanced(by: 100)
            )
        }
        let scheduler = makeScheduler(source: source, clock: clock)
        let completion = AutomationDecisionCompletionProbe()

        await scheduler.start()
        try await eventually { await source.callCount() == 1 }
        await clock.suspendNextNowCall()
        let signal = Task { await scheduler.signal(.wake) }
        try await eventually { await clock.hasSuspendedNow() }
        let terminal = Task {
            await scheduler.quiesceForTerminalRuntime()
            await completion.markCompleted()
        }
        await Task.yield()
        var completed = await completion.isCompleted()
        XCTAssertFalse(completed)

        await clock.resumeNow()
        await signal.value
        await terminal.value
        completed = await completion.isCompleted()
        let callCount = await source.callCount()
        XCTAssertTrue(completed)
        XCTAssertEqual(callCount, 1)
    }

    func testReentrantSourceGetsNoProofAndOuterTerminalDrainCompletes() async throws {
        let clock = ManualAutomationDecisionClock(milliseconds: 0)
        let source = ReentrantAutomationDecisionSource()
        let scheduler = makeScheduler(source: source, clock: clock)
        await source.attach(scheduler)

        await scheduler.start()
        try await eventually {
            await source.observation() == .reentrantNoProof
        }
        let outer = await scheduler.terminalQuiescenceObservation()

        XCTAssertEqual(outer, .completed)
        let reentrant = await source.observation()
        let callCount = await source.callCount()
        XCTAssertEqual(reentrant, .reentrantNoProof)
        XCTAssertEqual(callCount, 1)
    }

    private func makeScheduler(
        source: any DuxAutomationDecisionServing,
        clock: any DuxAutomationDecisionClock,
        failureRetry: Int64 = 10
    ) -> DuxAutomationDecisionScheduler {
        DuxAutomationDecisionScheduler(
            source: source,
            clock: clock,
            timing: DuxAutomationDecisionSchedulerTiming(
                failureRetryMilliseconds: failureRetry
            )
        )
    }

    private func eventually(
        _ predicate: @escaping @Sendable () async -> Bool,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws {
        for _ in 0 ..< 2000 {
            if await predicate() {
                return
            }
            await Task.yield()
        }
        XCTFail("Condition was not eventually true", file: file, line: line)
    }
}

private enum AutomationDecisionTestError: Error {
    case unavailable
}

private actor StubAutomationDecisionSource: DuxAutomationDecisionServing {
    struct Call: Equatable, Sendable {
        let instant: DuxAutomationWallInstant
        let reasons: DuxAutomationDecisionReasons
    }

    typealias Handler = @Sendable (
        DuxAutomationWallInstant,
        DuxAutomationDecisionReasons,
        Int
    ) throws -> DuxAutomationDecisionObservation

    private let handler: Handler
    private var recordedCalls: [Call] = []
    private var blocksFirstCall: Bool
    private var firstCallContinuation: CheckedContinuation<Void, Never>?

    init(
        blockFirstCall: Bool = false,
        handler: @escaping Handler
    ) {
        blocksFirstCall = blockFirstCall
        self.handler = handler
    }

    func observeAutomationDecision(
        at instant: DuxAutomationWallInstant,
        reasons: DuxAutomationDecisionReasons
    ) async throws -> DuxAutomationDecisionObservation {
        let call = recordedCalls.count
        recordedCalls.append(Call(instant: instant, reasons: reasons))
        if blocksFirstCall, call == 0 {
            await withCheckedContinuation { continuation in
                firstCallContinuation = continuation
            }
        }
        return try handler(instant, reasons, call)
    }

    func calls() -> [Call] { recordedCalls }
    func callCount() -> Int { recordedCalls.count }

    func releaseFirstCall() {
        blocksFirstCall = false
        firstCallContinuation?.resume()
        firstCallContinuation = nil
    }
}

private actor ReentrantAutomationDecisionSource: DuxAutomationDecisionServing {
    private weak var scheduler: DuxAutomationDecisionScheduler?
    private var terminalObservation: DuxAutomationDecisionTerminalQuiescenceObservation?
    private var calls = 0

    func attach(_ scheduler: DuxAutomationDecisionScheduler) {
        self.scheduler = scheduler
    }

    func observeAutomationDecision(
        at instant: DuxAutomationWallInstant,
        reasons _: DuxAutomationDecisionReasons
    ) async throws -> DuxAutomationDecisionObservation {
        calls += 1
        terminalObservation = await scheduler?.terminalQuiescenceObservation()
        return DuxAutomationDecisionObservation(
            checkedAt: instant,
            due: nil,
            nextCheckAt: nil
        )
    }

    func observation() -> DuxAutomationDecisionTerminalQuiescenceObservation? {
        terminalObservation
    }

    func callCount() -> Int { calls }
}

private actor AutomationDecisionCompletionProbe {
    private var completed = false
    func markCompleted() { completed = true }
    func isCompleted() -> Bool { completed }
}

private actor ManualAutomationDecisionClock: DuxAutomationDecisionClock {
    private struct Sleeper {
        let deadline: DuxAutomationWallInstant
        let continuation: CheckedContinuation<Void, Error>
    }

    private var current: DuxAutomationWallInstant
    private var sleepers: [UUID: Sleeper] = [:]
    private var suspendNextNow = false
    private var nowContinuation: CheckedContinuation<DuxAutomationWallInstant, Never>?

    init(milliseconds: Int64) {
        current = DuxAutomationWallInstant(milliseconds: milliseconds)
    }

    func now() async -> DuxAutomationWallInstant {
        if suspendNextNow {
            suspendNextNow = false
            return await withCheckedContinuation { continuation in
                precondition(nowContinuation == nil)
                nowContinuation = continuation
            }
        }
        return current
    }

    func sleep(until deadline: DuxAutomationWallInstant) async throws {
        if deadline <= current {
            return
        }
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                sleepers[id] = Sleeper(deadline: deadline, continuation: continuation)
            }
        } onCancel: {
            Task { await self.cancelSleeper(id) }
        }
    }

    func advance(by milliseconds: Int64) {
        current = current.advanced(by: milliseconds)
        releaseDueSleepers()
    }

    func jump(to milliseconds: Int64, releaseDueSleepers shouldRelease: Bool) {
        current = DuxAutomationWallInstant(milliseconds: milliseconds)
        if shouldRelease {
            releaseDueSleepers()
        }
    }

    func suspendNextNowCall() {
        precondition(nowContinuation == nil)
        suspendNextNow = true
    }

    func hasSuspendedNow() -> Bool { nowContinuation != nil }

    func resumeNow() {
        let pending = nowContinuation
        nowContinuation = nil
        pending?.resume(returning: current)
    }

    private func releaseDueSleepers() {
        let due = sleepers.filter { $0.value.deadline <= current }
        for (id, sleeper) in due {
            sleepers.removeValue(forKey: id)
            sleeper.continuation.resume()
        }
    }

    private func cancelSleeper(_ id: UUID) {
        sleepers.removeValue(forKey: id)?.continuation.resume(throwing: CancellationError())
    }
}
