import Foundation

protocol DuxEngineClosing: Sendable {
    func close() async -> Bool
}

protocol DuxMaintenanceScheduling: Sendable {
    func start() async
    func signal(_ trigger: DuxMaintenanceTrigger) async
    func stop() async
}

protocol DuxCapacityScheduling: AnyObject, Sendable {
    func start() async
    func signal(_ trigger: DuxCapacitySamplingTrigger) async
    func stop() async
}

protocol DuxReviewManaging: Sendable {
    func renewNow() async
    func shutdown() async
}

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

    private let engineService: any DuxEngineClosing
    private let scheduler: any DuxMaintenanceScheduling
    private let capacityScheduler: any DuxCapacityScheduling
    private let capacityResampleRouter: DuxCapacityResampleRouter?
    private let reviews: any DuxReviewManaging
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
        let capacityResampleRouter = DuxCapacityResampleRouter()
        self.capacityResampleRouter = capacityResampleRouter
        let model = AppModel(
            engineService: engineService,
            capacityResampleRequester: capacityResampleRouter
        )
        self.model = model
        capacityScheduler = DuxCapacitySamplingScheduler(sampler: model)
    }

    init(
        model: AppModel,
        engineService: any DuxEngineClosing,
        scheduler: any DuxMaintenanceScheduling,
        capacityScheduler: any DuxCapacityScheduling,
        reviews: any DuxReviewManaging
    ) {
        self.model = model
        self.engineService = engineService
        self.scheduler = scheduler
        self.capacityScheduler = capacityScheduler
        capacityResampleRouter = nil
        self.reviews = reviews
    }

    func start() async {
        guard !started, !shuttingDown else {
            return
        }
        started = true
        async let maintenance: Void = scheduler.start()
        async let capacity: Void = capacityScheduler.start()
        _ = await (maintenance, capacity)
        await capacityResampleRouter?.attach(capacityScheduler)
    }

    func signalCapacity(_ trigger: DuxCapacitySamplingTrigger) async {
        guard started, !shuttingDown else {
            return
        }
        await capacityScheduler.signal(trigger)
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
        model.invalidatePressurePolicyOperations()
        let capacityScheduler = capacityScheduler
        let capacityResampleRouter = capacityResampleRouter
        let scheduler = scheduler
        let reviews = reviews
        let engineService = engineService
        let task = Task {
            await capacityResampleRouter?.invalidate()
            await capacityScheduler.stop()
            await scheduler.stop()
            await reviews.shutdown()
            _ = await engineService.close()
        }
        shutdownTask = task
        await task.value
    }
}

extension EngineService: DuxEngineClosing {}
extension DuxMaintenanceScheduler: DuxMaintenanceScheduling {}
extension DuxCapacitySamplingScheduler: DuxCapacityScheduling {}
extension DuxSnapshotReviewController: DuxReviewManaging {}
