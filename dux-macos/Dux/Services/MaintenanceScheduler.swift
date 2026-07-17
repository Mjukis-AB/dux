import Foundation

enum DuxMaintenanceKind: CaseIterable, Sendable {
    case snapshotTerminalTemp
    case snapshotUnleasedTemp
    case snapshotProvisioningStage
    case snapshotOrphan
    case snapshotRetention
    case history
}

enum DuxMaintenanceDeferral: Sendable {
    case active
    case unstable
    case unproven
    case noEligibleWork
}

struct DuxMaintenanceBatchSignal: Sendable {
    let hasMore: Bool
    let deferral: DuxMaintenanceDeferral?

    init(hasMore: Bool, deferral: DuxMaintenanceDeferral? = nil) {
        self.hasMore = hasMore
        self.deferral = deferral
    }
}

enum DuxMaintenanceFailureDisposition: Sendable {
    case retryable
    case blockedUntilRestart
}

enum DuxMaintenanceTaskPoll: Sendable {
    case running
    case finished(DuxMaintenanceBatchSignal)
    case failed(DuxMaintenanceFailureDisposition)
    case cancelled
}

protocol DuxMaintenanceTask: AnyObject, Sendable {
    func poll() async -> DuxMaintenanceTaskPoll
    func requestCancellation() async
}

enum DuxMaintenanceStartDisposition: Sendable {
    case started(any DuxMaintenanceTask)
    case alreadyActive(any DuxMaintenanceTask)
    case deferredBusy
    case failed(DuxMaintenanceFailureDisposition)
}

protocol DuxMaintenanceServing: Sendable {
    func startMaintenance(_ kind: DuxMaintenanceKind) async -> DuxMaintenanceStartDisposition
}

struct DuxMaintenanceInstant: Comparable, Sendable {
    let milliseconds: Int64

    init(milliseconds: Int64) {
        self.milliseconds = milliseconds
    }

    static func < (lhs: Self, rhs: Self) -> Bool {
        lhs.milliseconds < rhs.milliseconds
    }

    func advanced(by delta: Int64) -> Self {
        let (value, overflow) = milliseconds.addingReportingOverflow(delta)
        return Self(milliseconds: overflow ? Int64.max : value)
    }
}

protocol DuxMaintenanceSchedulingClock: Sendable {
    func now() async -> DuxMaintenanceInstant
    func sleep(until deadline: DuxMaintenanceInstant) async throws
}

struct ContinuousDuxMaintenanceClock: DuxMaintenanceSchedulingClock {
    func now() async -> DuxMaintenanceInstant {
        let milliseconds = ProcessInfo.processInfo.systemUptime * 1_000
        return DuxMaintenanceInstant(
            milliseconds: Int64(min(milliseconds, Double(Int64.max)))
        )
    }

    func sleep(until deadline: DuxMaintenanceInstant) async throws {
        let current = await now()
        let delay = max(0, deadline.milliseconds - current.milliseconds)
        try await Task.sleep(for: .milliseconds(delay))
    }
}

protocol DuxMaintenanceEnergyPolicy: Sendable {
    func permitsMaintenance() async -> Bool
}

struct AlwaysPermittedDuxMaintenanceEnergyPolicy: DuxMaintenanceEnergyPolicy {
    func permitsMaintenance() async -> Bool {
        true
    }
}

enum DuxMaintenanceTrigger: Equatable, Sendable {
    case wake
    case applicationBecameActive
    case significantTimeChange
    case energyPolicyChanged
}

struct DuxMaintenanceSchedulerTiming: Sendable {
    let initialGraceMilliseconds: Int64
    let interBatchDelayMilliseconds: Int64
    let normalCadenceMilliseconds: Int64
    let hasMoreDelayMilliseconds: Int64
    let deferralDelayMilliseconds: Int64
    let energyRetryMilliseconds: Int64
    let pollIntervalMilliseconds: Int64
    let busyBackoffMilliseconds: [Int64]
    let failureBackoffMilliseconds: [Int64]

    static let production = Self(
        initialGraceMilliseconds: 60_000,
        interBatchDelayMilliseconds: 60_000,
        normalCadenceMilliseconds: 6 * 60 * 60 * 1_000,
        hasMoreDelayMilliseconds: 10 * 60 * 1_000,
        deferralDelayMilliseconds: 30 * 60 * 1_000,
        energyRetryMilliseconds: 30 * 60 * 1_000,
        pollIntervalMilliseconds: 1_000,
        busyBackoffMilliseconds: [60_000, 120_000, 240_000, 480_000, 960_000, 1_800_000],
        failureBackoffMilliseconds: [
            1_800_000,
            3_600_000,
            7_200_000,
            14_400_000,
            21_600_000,
        ]
    )

    init(
        initialGraceMilliseconds: Int64,
        interBatchDelayMilliseconds: Int64,
        normalCadenceMilliseconds: Int64,
        hasMoreDelayMilliseconds: Int64,
        deferralDelayMilliseconds: Int64,
        energyRetryMilliseconds: Int64,
        pollIntervalMilliseconds: Int64,
        busyBackoffMilliseconds: [Int64],
        failureBackoffMilliseconds: [Int64]
    ) {
        precondition(initialGraceMilliseconds >= 0)
        precondition(interBatchDelayMilliseconds > 0)
        precondition(normalCadenceMilliseconds > 0)
        precondition(hasMoreDelayMilliseconds > 0)
        precondition(deferralDelayMilliseconds > 0)
        precondition(energyRetryMilliseconds > 0)
        precondition(pollIntervalMilliseconds > 0)
        precondition(!busyBackoffMilliseconds.isEmpty)
        precondition(!failureBackoffMilliseconds.isEmpty)
        precondition(busyBackoffMilliseconds.allSatisfy { $0 > 0 })
        precondition(failureBackoffMilliseconds.allSatisfy { $0 > 0 })

        self.initialGraceMilliseconds = initialGraceMilliseconds
        self.interBatchDelayMilliseconds = interBatchDelayMilliseconds
        self.normalCadenceMilliseconds = normalCadenceMilliseconds
        self.hasMoreDelayMilliseconds = hasMoreDelayMilliseconds
        self.deferralDelayMilliseconds = deferralDelayMilliseconds
        self.energyRetryMilliseconds = energyRetryMilliseconds
        self.pollIntervalMilliseconds = pollIntervalMilliseconds
        self.busyBackoffMilliseconds = busyBackoffMilliseconds
        self.failureBackoffMilliseconds = failureBackoffMilliseconds
    }
}

struct DuxMaintenanceSchedulerSnapshot: Sendable {
    let isStarted: Bool
    let inFlightKind: DuxMaintenanceKind?
    let nextKind: DuxMaintenanceKind?
    let nextDeadline: DuxMaintenanceInstant?
    let hasCoalescedTrigger: Bool
}

actor DuxMaintenanceScheduler {
    private static let rotation: [DuxMaintenanceKind] = [
        .snapshotTerminalTemp,
        .snapshotUnleasedTemp,
        .snapshotProvisioningStage,
        .snapshotOrphan,
        .snapshotRetention,
        .history,
    ]

    private let service: any DuxMaintenanceServing
    private let clock: any DuxMaintenanceSchedulingClock
    private let energyPolicy: any DuxMaintenanceEnergyPolicy
    private let timing: DuxMaintenanceSchedulerTiming

    private var isStarted = false
    private var generation: UInt64 = 0
    private var rotationIndex = 0
    private var busyFailureCount = 0
    private var taskFailureCount = 0
    private var blockedKinds: Set<DuxMaintenanceKind> = []
    private var completedInCycle: Set<DuxMaintenanceKind> = []
    private var driverTask: Task<Void, Never>?
    private var currentTask: (any DuxMaintenanceTask)?
    private var inFlightKind: DuxMaintenanceKind?
    private var nextDeadline: DuxMaintenanceInstant?
    private var energyPolicyMayAdvanceDeadline = false
    private var hasCoalescedTrigger = false

    init(
        service: any DuxMaintenanceServing,
        clock: any DuxMaintenanceSchedulingClock = ContinuousDuxMaintenanceClock(),
        energyPolicy: any DuxMaintenanceEnergyPolicy = AlwaysPermittedDuxMaintenanceEnergyPolicy(),
        timing: DuxMaintenanceSchedulerTiming = .production
    ) {
        self.service = service
        self.clock = clock
        self.energyPolicy = energyPolicy
        self.timing = timing
    }

    func start() async {
        guard !isStarted else {
            return
        }
        isStarted = true
        generation &+= 1
        let current = await clock.now()
        schedule(at: current.advanced(by: timing.initialGraceMilliseconds))
    }

    func signal(_ trigger: DuxMaintenanceTrigger) async {
        _ = trigger
        guard isStarted else {
            return
        }
        if inFlightKind != nil {
            hasCoalescedTrigger = true
            return
        }

        let current = await clock.now()
        let energyPolicyRecovered = trigger == .energyPolicyChanged
            && energyPolicyMayAdvanceDeadline
        if energyPolicyRecovered || nextDeadline.map({ current >= $0 }) ?? true {
            schedule(at: current)
        }
    }

    func stop() async {
        guard isStarted else {
            return
        }
        isStarted = false
        generation &+= 1
        hasCoalescedTrigger = false
        nextDeadline = nil
        energyPolicyMayAdvanceDeadline = false
        let driver = driverTask
        driver?.cancel()
        driverTask = nil

        let task = currentTask
        currentTask = nil
        inFlightKind = nil
        if let task {
            await task.requestCancellation()
        }
        if let driver {
            await driver.value
        }
    }

    func snapshot() -> DuxMaintenanceSchedulerSnapshot {
        DuxMaintenanceSchedulerSnapshot(
            isStarted: isStarted,
            inFlightKind: inFlightKind,
            nextKind: nextEligibleKind(),
            nextDeadline: nextDeadline,
            hasCoalescedTrigger: hasCoalescedTrigger
        )
    }

    private func schedule(
        at deadline: DuxMaintenanceInstant,
        energyPolicyMayAdvance: Bool = false
    ) {
        guard isStarted else {
            return
        }
        nextDeadline = deadline
        energyPolicyMayAdvanceDeadline = energyPolicyMayAdvance
        driverTask?.cancel()
        generation &+= 1
        let scheduledGeneration = generation
        let clock = self.clock
        driverTask = Task { [weak self] in
            do {
                try await clock.sleep(until: deadline)
            } catch {
                return
            }
            await self?.runDueBatch(generation: scheduledGeneration)
        }
    }

    private func runDueBatch(generation scheduledGeneration: UInt64) async {
        guard
            isStarted,
            generation == scheduledGeneration,
            inFlightKind == nil,
            let kind = nextEligibleKind()
        else {
            return
        }

        nextDeadline = nil
        energyPolicyMayAdvanceDeadline = false
        inFlightKind = kind

        let energyPermitted = await energyPolicy.permitsMaintenance()
        guard
            isStarted,
            generation == scheduledGeneration,
            inFlightKind == kind
        else {
            return
        }
        guard energyPermitted else {
            await finishAttempt(
                kind: kind,
                delayMilliseconds: timing.energyRetryMilliseconds,
                advanceRotation: false,
                energyPolicyMayAdvance: true
            )
            return
        }
        guard isStarted, generation == scheduledGeneration else {
            inFlightKind = nil
            return
        }

        let admission = await service.startMaintenance(kind)
        guard isStarted, generation == scheduledGeneration else {
            if case let .started(task) = admission {
                await task.requestCancellation()
            } else if case let .alreadyActive(task) = admission {
                await task.requestCancellation()
            }
            inFlightKind = nil
            return
        }

        switch admission {
        case let .started(task), let .alreadyActive(task):
            currentTask = task
            await observe(task, kind: kind, generation: scheduledGeneration)
        case .deferredBusy:
            let delay = backoff(
                timing.busyBackoffMilliseconds,
                attempt: busyFailureCount
            )
            busyFailureCount = min(busyFailureCount + 1, Int.max - 1)
            await finishAttempt(
                kind: kind,
                delayMilliseconds: delay,
                advanceRotation: false
            )
        case let .failed(disposition):
            await handleFailure(disposition, kind: kind)
        }
    }

    private func observe(
        _ task: any DuxMaintenanceTask,
        kind: DuxMaintenanceKind,
        generation scheduledGeneration: UInt64
    ) async {
        while isStarted, generation == scheduledGeneration {
            let poll = await task.poll()
            guard
                isStarted,
                generation == scheduledGeneration,
                inFlightKind == kind
            else {
                return
            }
            switch poll {
            case .running:
                let current = await clock.now()
                do {
                    try await clock.sleep(
                        until: current.advanced(by: timing.pollIntervalMilliseconds)
                    )
                } catch {
                    return
                }
            case let .finished(signal):
                busyFailureCount = 0
                taskFailureCount = 0
                currentTask = nil
                await finishAttempt(
                    kind: kind,
                    delayMilliseconds: signal.deferral != nil
                        ? timing.deferralDelayMilliseconds
                        : signal.hasMore
                            ? timing.hasMoreDelayMilliseconds
                            : nil,
                    advanceRotation: true
                )
                return
            case let .failed(disposition):
                currentTask = nil
                await handleFailure(disposition, kind: kind)
                return
            case .cancelled:
                currentTask = nil
                await handleFailure(.retryable, kind: kind)
                return
            }
        }
    }

    private func handleFailure(
        _ disposition: DuxMaintenanceFailureDisposition,
        kind: DuxMaintenanceKind
    ) async {
        switch disposition {
        case .retryable:
            let delay = backoff(
                timing.failureBackoffMilliseconds,
                attempt: taskFailureCount
            )
            taskFailureCount = min(taskFailureCount + 1, Int.max - 1)
            await finishAttempt(
                kind: kind,
                delayMilliseconds: delay,
                advanceRotation: false
            )
        case .blockedUntilRestart:
            blockedKinds.insert(kind)
            busyFailureCount = 0
            taskFailureCount = 0
            await finishAttempt(
                kind: kind,
                delayMilliseconds: nil,
                advanceRotation: true
            )
        }
    }

    private func finishAttempt(
        kind: DuxMaintenanceKind,
        delayMilliseconds: Int64?,
        advanceRotation: Bool,
        energyPolicyMayAdvance: Bool = false
    ) async {
        precondition(inFlightKind == kind)
        currentTask = nil
        inFlightKind = nil
        let completedCycle = advanceRotation ? advancePast(kind) : false

        guard isStarted else {
            return
        }
        let current = await clock.now()
        hasCoalescedTrigger = false
        let delay = delayMilliseconds
            ?? (completedCycle
                ? timing.normalCadenceMilliseconds
                : timing.interBatchDelayMilliseconds)
        schedule(
            at: current.advanced(by: delay),
            energyPolicyMayAdvance: energyPolicyMayAdvance
        )
    }

    private func nextEligibleKind() -> DuxMaintenanceKind? {
        for offset in Self.rotation.indices {
            let index = (rotationIndex + offset) % Self.rotation.count
            let kind = Self.rotation[index]
            if !blockedKinds.contains(kind), !completedInCycle.contains(kind) {
                return kind
            }
        }
        return nil
    }

    private func advancePast(_ kind: DuxMaintenanceKind) -> Bool {
        guard let index = Self.rotation.firstIndex(of: kind) else {
            return false
        }
        completedInCycle.insert(kind)
        rotationIndex = (index + 1) % Self.rotation.count
        if nextEligibleKind() == nil {
            completedInCycle.removeAll(keepingCapacity: true)
            return true
        }
        return false
    }

    private func backoff(_ values: [Int64], attempt: Int) -> Int64 {
        values[min(attempt, values.count - 1)]
    }
}
