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

    private var isStarted = false
    private var isSampling = false
    private var generation: UInt64 = 0
    private var driverTask: Task<Void, Never>?
    private var nextDeadline: DuxMaintenanceInstant?
    private var hasCoalescedTrigger = false

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
        guard !isStarted else {
            return
        }
        isStarted = true
        generation &+= 1
        schedule(at: await clock.now())
    }

    func signal(_ trigger: DuxCapacitySamplingTrigger) async {
        _ = trigger
        guard isStarted else {
            return
        }
        if isSampling {
            hasCoalescedTrigger = true
            return
        }
        schedule(at: await clock.now())
    }

    func stop() async {
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
        await sampler.cancelVolumeRefresh()
        // A local Foundation resource query cannot be interrupted safely. Do
        // not await it during app termination; generation checks and the
        // sampler's explicit invalidation prevent its completion publishing.
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
        guard isStarted else {
            return
        }
        nextDeadline = deadline
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
            await self?.runDueSample(generation: scheduledGeneration)
        }
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
}
