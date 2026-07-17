import Foundation

struct SystemDuxMaintenanceEnergyPolicy: DuxMaintenanceEnergyPolicy {
    func permitsMaintenance() async -> Bool {
        let process = ProcessInfo.processInfo
        guard !process.isLowPowerModeEnabled else {
            return false
        }
        switch process.thermalState {
        case .nominal, .fair:
            return true
        case .serious, .critical:
            return false
        @unknown default:
            return false
        }
    }
}

@MainActor
final class AppRuntime {
    static let shared = AppRuntime()

    let model: AppModel

    private let engineService: EngineService
    private let scheduler: DuxMaintenanceScheduler
    private let reviews: DuxSnapshotReviewController
    private var started = false
    private var shuttingDown = false
    private var shutdownTask: Task<Void, Never>?

    private init() {
        let engineService = EngineService()
        self.engineService = engineService
        scheduler = DuxMaintenanceScheduler(
            service: engineService,
            energyPolicy: SystemDuxMaintenanceEnergyPolicy()
        )
        reviews = DuxSnapshotReviewController(service: engineService)
        model = AppModel(engineService: engineService)
    }

    func start() async {
        guard !started, !shuttingDown else {
            return
        }
        started = true
        await scheduler.start()
    }

    func signalMaintenance(_ trigger: DuxMaintenanceTrigger) async {
        guard started, !shuttingDown else {
            return
        }
        if trigger == .wake || trigger == .significantTimeChange {
            await reviews.renewNow()
        }
        await scheduler.signal(trigger)
    }

    func shutdown() async {
        if let shutdownTask {
            await shutdownTask.value
            return
        }
        shuttingDown = true
        let scheduler = scheduler
        let reviews = reviews
        let engineService = engineService
        let task = Task {
            await scheduler.stop()
            await reviews.shutdown()
            _ = await engineService.close()
        }
        shutdownTask = task
        await task.value
    }
}
