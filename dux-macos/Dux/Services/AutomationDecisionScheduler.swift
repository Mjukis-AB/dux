import Foundation

struct DuxAutomationWallInstant: Comparable, Equatable, Sendable {
    let milliseconds: Int64

    init(milliseconds: Int64) {
        self.milliseconds = milliseconds
    }

    static func < (lhs: Self, rhs: Self) -> Bool {
        lhs.milliseconds < rhs.milliseconds
    }

    func advanced(by delta: Int64) -> Self {
        let (value, overflow) = milliseconds.addingReportingOverflow(delta)
        return Self(milliseconds: overflow ? (delta >= 0 ? Int64.max : Int64.min) : value)
    }
}

protocol DuxAutomationDecisionClock: Sendable {
    func now() async -> DuxAutomationWallInstant
    func sleep(until deadline: DuxAutomationWallInstant) async throws
}

struct SystemDuxAutomationDecisionClock: DuxAutomationDecisionClock {
    func now() async -> DuxAutomationWallInstant {
        let milliseconds = Date().timeIntervalSince1970 * 1000
        guard milliseconds.isFinite, milliseconds >= 0 else {
            return DuxAutomationWallInstant(milliseconds: 0)
        }
        guard milliseconds < Double(Int64.max) else {
            return DuxAutomationWallInstant(milliseconds: Int64.max)
        }
        return DuxAutomationWallInstant(milliseconds: Int64(milliseconds))
    }

    func sleep(until deadline: DuxAutomationWallInstant) async throws {
        let current = await now()
        guard deadline > current else {
            return
        }
        let (difference, overflow) = deadline.milliseconds.subtractingReportingOverflow(
            current.milliseconds
        )
        let delay = overflow ? Int64.max : difference
        try await Task.sleep(for: .milliseconds(delay))
    }
}

enum DuxAutomationDecisionTrigger: Equatable, Sendable {
    case launch
    case deadline
    case wake
    case significantTimeChange
}

struct DuxAutomationDecisionReasons: OptionSet, Equatable, Sendable {
    let rawValue: UInt8

    static let launch = Self(rawValue: 1 << 0)
    static let deadline = Self(rawValue: 1 << 1)
    static let wake = Self(rawValue: 1 << 2)
    static let significantTimeChange = Self(rawValue: 1 << 3)

    init(rawValue: UInt8) {
        self.rawValue = rawValue
    }

    init(_ trigger: DuxAutomationDecisionTrigger) {
        self = switch trigger {
        case .launch: .launch
        case .deadline: .deadline
        case .wake: .wake
        case .significantTimeChange: .significantTimeChange
        }
    }
}

enum DuxAutomationDueClassification: Equatable, Sendable {
    case onTime
    case missed
}

/// Path-free observation only. No cleanup API accepts this value.
struct DuxAutomationDueObservation: Equatable, Sendable {
    let scheduleID: String
    let scheduleRevision: UInt64
    let scheduledAt: DuxAutomationWallInstant
    let classification: DuxAutomationDueClassification
}

/// One optional due observation makes the catch-up bound structural: a single
/// decision can never represent a queue of missed work.
struct DuxAutomationDecisionObservation: Equatable, Sendable {
    let checkedAt: DuxAutomationWallInstant
    let due: DuxAutomationDueObservation?
    let nextCheckAt: DuxAutomationWallInstant?
}

protocol DuxAutomationDecisionServing: Sendable {
    func observeAutomationDecision(
        at instant: DuxAutomationWallInstant,
        reasons: DuxAutomationDecisionReasons
    ) async throws -> DuxAutomationDecisionObservation
}

/// Production remains effect-dormant until a separately reviewed core contract
/// can prove an enabled schedule and a persisted next-run instant.
struct NoEnabledSchedulesDuxAutomationDecisionSource: DuxAutomationDecisionServing {
    func observeAutomationDecision(
        at instant: DuxAutomationWallInstant,
        reasons _: DuxAutomationDecisionReasons
    ) async throws -> DuxAutomationDecisionObservation {
        DuxAutomationDecisionObservation(
            checkedAt: instant,
            due: nil,
            nextCheckAt: nil
        )
    }
}

struct DuxAutomationDecisionSchedulerTiming: Sendable {
    let failureRetryMilliseconds: Int64

    static let production = Self(failureRetryMilliseconds: 15 * 60 * 1000)

    init(failureRetryMilliseconds: Int64) {
        precondition(failureRetryMilliseconds > 0)
        self.failureRetryMilliseconds = failureRetryMilliseconds
    }
}

enum DuxAutomationDecisionSchedulerFailure: Equatable, Sendable {
    case sourceUnavailable
    case invalidObservation
}

struct DuxAutomationDecisionSchedulerSnapshot: Equatable, Sendable {
    let isStarted: Bool
    let isEvaluating: Bool
    let nextDeadline: DuxAutomationWallInstant?
    let pendingReasons: DuxAutomationDecisionReasons
    let lastObservation: DuxAutomationDecisionObservation?
    let lastFailure: DuxAutomationDecisionSchedulerFailure?
}

enum DuxAutomationDecisionTerminalQuiescenceObservation: Equatable, Sendable {
    case completed
    case reentrantNoProof
}

protocol DuxAutomationDecisionScheduling: Sendable {
    func start() async
    func signal(_ trigger: DuxAutomationDecisionTrigger) async
    func stop() async
    func quiesceForTerminalRuntime() async
}

private enum DuxAutomationDecisionSchedulerTaskContext {
    @TaskLocal static var schedulerID: UUID?
}

/// Owns timing and lifecycle for effect-dormant automation decisions. It has no
/// engine, draft, suggestion, planning, scan, or cleanup capability.
actor DuxAutomationDecisionScheduler: DuxAutomationDecisionScheduling {
    private let source: any DuxAutomationDecisionServing
    private let clock: any DuxAutomationDecisionClock
    private let timing: DuxAutomationDecisionSchedulerTiming
    private let schedulerID = UUID()

    private var isStarted = false
    private var isEvaluating = false
    private var generation: UInt64 = 0
    private var driverTask: Task<Void, Never>?
    private var currentDriverID: UUID?
    private var retainedDrivers: [UUID: Task<Void, Never>] = [:]
    private var nextDeadline: DuxAutomationWallInstant?
    private var pendingReasons: DuxAutomationDecisionReasons = []
    private var lastObservation: DuxAutomationDecisionObservation?
    private var lastFailure: DuxAutomationDecisionSchedulerFailure?
    private var isTerminal = false
    private var admittedOperationCount = 0
    private var admittedOperationWaiters: [CheckedContinuation<Void, Never>] = []
    private var ordinaryStopTask: Task<Void, Never>?
    private var terminalQuiescenceTask: Task<Void, Never>?

    init(
        source: any DuxAutomationDecisionServing,
        clock: any DuxAutomationDecisionClock = SystemDuxAutomationDecisionClock(),
        timing: DuxAutomationDecisionSchedulerTiming = .production
    ) {
        self.source = source
        self.clock = clock
        self.timing = timing
    }

    func start() async {
        if DuxAutomationDecisionSchedulerTaskContext.schedulerID == schedulerID,
           ordinaryStopTask != nil
        {
            return
        }
        guard beginAdmittedOperation() else {
            return
        }
        await DuxAutomationDecisionSchedulerTaskContext.$schedulerID.withValue(schedulerID) {
            if let ordinaryStopTask {
                await ordinaryStopTask.value
            }
            guard !isTerminal, !isStarted else {
                return
            }
            ordinaryStopTask = nil
            isStarted = true
            generation &+= 1
            let current = await clock.now()
            guard !isTerminal, isStarted else {
                return
            }
            schedule(at: current, reasons: .launch)
        }
        finishAdmittedOperation()
    }

    func signal(_ trigger: DuxAutomationDecisionTrigger) async {
        guard beginAdmittedOperation() else {
            return
        }
        await DuxAutomationDecisionSchedulerTaskContext.$schedulerID.withValue(schedulerID) {
            guard !isTerminal, isStarted else {
                return
            }
            let reason = DuxAutomationDecisionReasons(trigger)
            if isEvaluating {
                pendingReasons.formUnion(reason)
                return
            }
            let current = await clock.now()
            guard !isTerminal, isStarted else {
                return
            }
            schedule(at: current, reasons: reason)
        }
        finishAdmittedOperation()
    }

    func stop() async {
        if let ordinaryStopTask {
            if DuxAutomationDecisionSchedulerTaskContext.schedulerID == schedulerID {
                return
            }
            await ordinaryStopTask.value
            return
        }
        guard isStarted else {
            return
        }
        isStarted = false
        generation &+= 1
        isEvaluating = false
        nextDeadline = nil
        pendingReasons = []
        driverTask = nil
        currentDriverID = nil

        let drivers = Array(retainedDrivers.values)
        for driver in drivers {
            driver.cancel()
        }
        let schedulerID = schedulerID
        let task = Task {
            await DuxAutomationDecisionSchedulerTaskContext.$schedulerID.withValue(schedulerID) {
                for driver in drivers {
                    await driver.value
                }
            }
        }
        ordinaryStopTask = task
        await task.value
    }

    func quiesceForTerminalRuntime() async {
        let observation = await terminalQuiescenceObservation()
        precondition(
            observation == .completed,
            "reentrant automation-decision terminal quiescence cannot produce proof"
        )
    }

    func terminalQuiescenceObservation() async
        -> DuxAutomationDecisionTerminalQuiescenceObservation
    {
        let isReentrant =
            DuxAutomationDecisionSchedulerTaskContext.schedulerID == schedulerID
        let task = beginTerminalQuiescence()
        guard !isReentrant else {
            return .reentrantNoProof
        }
        await task.value
        return .completed
    }

    func snapshot() -> DuxAutomationDecisionSchedulerSnapshot {
        DuxAutomationDecisionSchedulerSnapshot(
            isStarted: isStarted,
            isEvaluating: isEvaluating,
            nextDeadline: nextDeadline,
            pendingReasons: pendingReasons,
            lastObservation: lastObservation,
            lastFailure: lastFailure
        )
    }

    private func beginTerminalQuiescence() -> Task<Void, Never> {
        if let terminalQuiescenceTask {
            return terminalQuiescenceTask
        }

        isTerminal = true
        isStarted = false
        generation &+= 1
        isEvaluating = false
        nextDeadline = nil
        pendingReasons = []
        driverTask = nil
        currentDriverID = nil

        let drivers = Array(retainedDrivers.values)
        let acceptedStop = ordinaryStopTask
        for driver in drivers {
            driver.cancel()
        }
        let schedulerID = schedulerID
        let task = Task { [self] in
            await DuxAutomationDecisionSchedulerTaskContext.$schedulerID.withValue(schedulerID) {
                await acceptedStop?.value
                await waitForAdmittedOperations()
                for driver in drivers {
                    await driver.value
                }
            }
        }
        terminalQuiescenceTask = task
        return task
    }

    private func schedule(
        at deadline: DuxAutomationWallInstant,
        reasons: DuxAutomationDecisionReasons
    ) {
        guard !isTerminal, isStarted, !reasons.isEmpty else {
            return
        }
        nextDeadline = deadline
        driverTask?.cancel()
        generation &+= 1
        let scheduledGeneration = generation
        let clock = clock
        let driverID = UUID()
        let schedulerID = schedulerID
        let driver = Task { [weak self] in
            await DuxAutomationDecisionSchedulerTaskContext.$schedulerID.withValue(schedulerID) {
                do {
                    try await clock.sleep(until: deadline)
                } catch {
                    await self?.driverDidFinish(driverID)
                    return
                }
                await self?.runDueDecision(
                    generation: scheduledGeneration,
                    reasons: reasons
                )
                await self?.driverDidFinish(driverID)
            }
        }
        currentDriverID = driverID
        driverTask = driver
        retainedDrivers[driverID] = driver
    }

    private func runDueDecision(
        generation scheduledGeneration: UInt64,
        reasons: DuxAutomationDecisionReasons
    ) async {
        guard
            isStarted,
            generation == scheduledGeneration,
            !isEvaluating
        else {
            return
        }
        nextDeadline = nil
        isEvaluating = true

        let checkedAt = await clock.now()
        guard isStarted, generation == scheduledGeneration else {
            isEvaluating = false
            return
        }

        let result: Result<DuxAutomationDecisionObservation, Error>
        do {
            result = try await .success(
                source.observeAutomationDecision(at: checkedAt, reasons: reasons)
            )
        } catch {
            result = .failure(error)
        }
        guard isStarted, generation == scheduledGeneration else {
            isEvaluating = false
            return
        }
        isEvaluating = false

        switch result {
        case let .success(observation):
            guard Self.isValid(observation, checkedAt: checkedAt) else {
                await finishFailure(.invalidObservation)
                return
            }
            let current = await clock.now()
            guard isStarted, generation == scheduledGeneration else {
                return
            }
            guard observation.nextCheckAt.map({ $0 > current }) ?? true else {
                await finishFailure(.invalidObservation, current: current)
                return
            }
            lastObservation = observation
            lastFailure = nil
            await scheduleAfterDecision(observation.nextCheckAt, current: current)
        case .failure:
            await finishFailure(.sourceUnavailable)
        }
    }

    private func scheduleAfterDecision(
        _ nextCheckAt: DuxAutomationWallInstant?,
        current: DuxAutomationWallInstant
    ) async {
        if !pendingReasons.isEmpty {
            let reasons = pendingReasons
            pendingReasons = []
            schedule(at: current, reasons: reasons)
        } else if let nextCheckAt {
            schedule(at: nextCheckAt, reasons: .deadline)
        }
    }

    private func finishFailure(
        _ failure: DuxAutomationDecisionSchedulerFailure,
        current suppliedCurrent: DuxAutomationWallInstant? = nil
    ) async {
        lastFailure = failure
        let current = if let suppliedCurrent {
            suppliedCurrent
        } else {
            await clock.now()
        }
        guard isStarted else {
            return
        }
        if !pendingReasons.isEmpty {
            let reasons = pendingReasons
            pendingReasons = []
            schedule(at: current, reasons: reasons)
        } else {
            schedule(
                at: current.advanced(by: timing.failureRetryMilliseconds),
                reasons: .deadline
            )
        }
    }

    private static func isValid(
        _ observation: DuxAutomationDecisionObservation,
        checkedAt: DuxAutomationWallInstant
    ) -> Bool {
        guard
            checkedAt.milliseconds >= 0,
            observation.checkedAt == checkedAt,
            observation.nextCheckAt.map({ $0 > checkedAt }) ?? true
        else {
            return false
        }
        guard let due = observation.due else {
            return true
        }
        let classificationIsConsistent = switch due.classification {
        case .onTime:
            due.scheduledAt == checkedAt
        case .missed:
            due.scheduledAt < checkedAt
        }
        return classificationIsConsistent
            && !due.scheduleID.isEmpty
            && due.scheduleID.utf8.count <= 128
            && due.scheduleID.utf8.allSatisfy(isScheduleIDByte)
            && due.scheduleRevision > 0
            && due.scheduledAt.milliseconds >= 0
            && due.scheduledAt <= checkedAt
    }

    private static func isScheduleIDByte(_ byte: UInt8) -> Bool {
        (0x41 ... 0x5A).contains(byte)
            || (0x61 ... 0x7A).contains(byte)
            || (0x30 ... 0x39).contains(byte)
            || byte == 0x2E
            || byte == 0x5F
            || byte == 0x2D
            || byte == 0x3A
    }

    private func driverDidFinish(_ driverID: UUID) {
        retainedDrivers.removeValue(forKey: driverID)
        if currentDriverID == driverID {
            currentDriverID = nil
            driverTask = nil
        }
    }

    private func beginAdmittedOperation() -> Bool {
        guard !isTerminal else {
            return false
        }
        admittedOperationCount += 1
        return true
    }

    private func finishAdmittedOperation() {
        precondition(admittedOperationCount > 0)
        admittedOperationCount -= 1
        guard admittedOperationCount == 0 else {
            return
        }
        let waiters = admittedOperationWaiters
        admittedOperationWaiters.removeAll(keepingCapacity: true)
        for waiter in waiters {
            waiter.resume()
        }
    }

    private func waitForAdmittedOperations() async {
        guard admittedOperationCount > 0 else {
            return
        }
        await withCheckedContinuation { continuation in
            admittedOperationWaiters.append(continuation)
        }
    }
}
