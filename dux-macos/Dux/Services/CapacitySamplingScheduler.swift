import Foundation

enum DuxCapacitySamplingTrigger: Equatable, Sendable {
    case wake
    case volumesChanged
    case manual
}

@MainActor
protocol DuxCapacitySampling: AnyObject, Sendable {
    func refreshVolumeCapacity() async
    func cancelVolumeRefresh()
}

struct DuxCapacitySamplingTiming: Sendable {
    let cadenceMilliseconds: Int64

    static let production = Self(cadenceMilliseconds: 5 * 60 * 1_000)

    init(cadenceMilliseconds: Int64) {
        precondition(cadenceMilliseconds > 0)
        self.cadenceMilliseconds = cadenceMilliseconds
    }
}

struct DuxCapacitySamplingSchedulerSnapshot: Sendable {
    let isStarted: Bool
    let isSampling: Bool
    let nextDeadline: DuxMaintenanceInstant?
    let hasCoalescedTrigger: Bool
}

enum DuxCapacityTerminalQuiescenceObservation: Equatable, Sendable {
    case completed
    case reentrantNoProof
}

private enum DuxCapacitySchedulerTaskContext {
    @TaskLocal static var schedulerID: UUID?
}

/// Owns cheap startup-volume sample cadence independently from UI scene life.
///
/// The scheduler starts immediately at application launch, permits one sample
/// at a time, coalesces events received during a sample into one follow-up,
/// and invalidates late completions on stop. Foundation and Rust work remain in
/// their dedicated service queues; this actor owns timing only.
actor DuxCapacitySamplingScheduler {
    private let sampler: any DuxCapacitySampling
    private let clock: any DuxMaintenanceSchedulingClock
    private let timing: DuxCapacitySamplingTiming
    private let schedulerID = UUID()

    private var isStarted = false
    private var isSampling = false
    private var generation: UInt64 = 0
    private var driverTask: Task<Void, Never>?
    private var currentDriverID: UUID?
    private var retainedDrivers: [UUID: Task<Void, Never>] = [:]
    private var nextDeadline: DuxMaintenanceInstant?
    private var hasCoalescedTrigger = false
    private var isTerminal = false
    private var admittedOperationCount = 0
    private var admittedOperationWaiters: [CheckedContinuation<Void, Never>] = []
    private var ordinaryStopTask: Task<Void, Never>?
    private var terminalQuiescenceTask: Task<Void, Never>?

    init(
        sampler: any DuxCapacitySampling,
        clock: any DuxMaintenanceSchedulingClock = ContinuousDuxMaintenanceClock(),
        timing: DuxCapacitySamplingTiming = .production
    ) {
        self.sampler = sampler
        self.clock = clock
        self.timing = timing
    }

    func start() async {
        if DuxCapacitySchedulerTaskContext.schedulerID == schedulerID,
           ordinaryStopTask != nil
        {
            return
        }
        guard beginAdmittedOperation() else {
            return
        }
        await DuxCapacitySchedulerTaskContext.$schedulerID.withValue(schedulerID) {
            if let ordinaryStopTask {
                await ordinaryStopTask.value
            }
            guard !isTerminal, !isStarted else {
                return
            }
            ordinaryStopTask = nil
            isStarted = true
            generation &+= 1
            schedule(at: await clock.now())
        }
        finishAdmittedOperation()
    }

    func signal(_ trigger: DuxCapacitySamplingTrigger) async {
        _ = trigger
        guard beginAdmittedOperation() else {
            return
        }
        await DuxCapacitySchedulerTaskContext.$schedulerID.withValue(schedulerID) {
            guard !isTerminal, isStarted else {
                return
            }
            if isSampling {
                hasCoalescedTrigger = true
                return
            }
            schedule(at: await clock.now())
        }
        finishAdmittedOperation()
    }

    func stop() async {
        if let ordinaryStopTask {
            if DuxCapacitySchedulerTaskContext.schedulerID == schedulerID {
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
        isSampling = false
        nextDeadline = nil
        hasCoalescedTrigger = false
        let driver = driverTask
        driverTask = nil
        driver?.cancel()
        let sampler = sampler
        let schedulerID = schedulerID
        let task = Task {
            await DuxCapacitySchedulerTaskContext.$schedulerID.withValue(schedulerID) {
                await sampler.cancelVolumeRefresh()
            }
        }
        ordinaryStopTask = task
        await task.value
        // A local Foundation resource query cannot be interrupted safely. Do
        // not await it during app termination; generation checks and the
        // sampler's explicit invalidation prevent its completion publishing.
    }

    /// Permanently fences start/signal admission, invalidates publication,
    /// requests cancellation, and joins every retained driver. Unlike stop(),
    /// this waits for a Foundation sample that cannot be interrupted safely.
    func quiesceForTerminalRuntime() async {
        let observation = await terminalQuiescenceObservation()
        precondition(
            observation == .completed,
            "reentrant capacity terminal quiescence cannot produce proof"
        )
    }

    /// Requests the same retained drain as `quiesceForTerminalRuntime`, but
    /// lets scheduler-owned callbacks observe that awaiting would self-deadlock.
    /// A reentrant observation is never terminal-quiescence proof.
    func terminalQuiescenceObservation() async
        -> DuxCapacityTerminalQuiescenceObservation
    {
        let isReentrant = DuxCapacitySchedulerTaskContext.schedulerID == schedulerID
        let task = beginTerminalQuiescence()
        guard !isReentrant else {
            return .reentrantNoProof
        }
        await task.value
        return .completed
    }

    private func beginTerminalQuiescence() -> Task<Void, Never> {
        if let terminalQuiescenceTask {
            return terminalQuiescenceTask
        }

        isTerminal = true
        isStarted = false
        generation &+= 1
        isSampling = false
        nextDeadline = nil
        hasCoalescedTrigger = false
        driverTask = nil
        currentDriverID = nil

        let drivers = Array(retainedDrivers.values)
        let acceptedStop = ordinaryStopTask
        for driver in drivers {
            driver.cancel()
        }
        let sampler = sampler
        let schedulerID = schedulerID
        let task = Task { [self] in
            await DuxCapacitySchedulerTaskContext.$schedulerID.withValue(schedulerID) {
                await sampler.cancelVolumeRefresh()
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

    func snapshot() -> DuxCapacitySamplingSchedulerSnapshot {
        DuxCapacitySamplingSchedulerSnapshot(
            isStarted: isStarted,
            isSampling: isSampling,
            nextDeadline: nextDeadline,
            hasCoalescedTrigger: hasCoalescedTrigger
        )
    }

    private func schedule(at deadline: DuxMaintenanceInstant) {
        guard !isTerminal, isStarted else {
            return
        }
        nextDeadline = deadline
        driverTask?.cancel()
        generation &+= 1
        let scheduledGeneration = generation
        let clock = self.clock
        let driverID = UUID()
        let schedulerID = schedulerID
        let driver = Task { [weak self] in
            await DuxCapacitySchedulerTaskContext.$schedulerID.withValue(schedulerID) {
                do {
                    try await clock.sleep(until: deadline)
                } catch {
                    await self?.driverDidFinish(driverID)
                    return
                }
                await self?.runDueSample(generation: scheduledGeneration)
                await self?.driverDidFinish(driverID)
            }
        }
        currentDriverID = driverID
        driverTask = driver
        retainedDrivers[driverID] = driver
    }

    private func runDueSample(generation scheduledGeneration: UInt64) async {
        guard
            isStarted,
            generation == scheduledGeneration,
            !isSampling
        else {
            return
        }
        nextDeadline = nil
        isSampling = true
        await sampler.refreshVolumeCapacity()
        guard isStarted, generation == scheduledGeneration else {
            isSampling = false
            return
        }
        isSampling = false
        let current = await clock.now()
        if hasCoalescedTrigger {
            hasCoalescedTrigger = false
            schedule(at: current)
        } else {
            schedule(at: current.advanced(by: timing.cadenceMilliseconds))
        }
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
