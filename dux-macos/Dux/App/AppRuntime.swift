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

@MainActor
protocol DuxScanManaging: AnyObject {
    func shutdownTargetedReclaimScan() async
    func shutdownHomeScan() async
}

extension DuxScanManaging {
    func shutdownTargetedReclaimScan() async {}
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
    let explorerSnapshotBrowser: ExplorerSnapshotBrowserModel

    private let engineService: any DuxEngineClosing
    private let scheduler: any DuxMaintenanceScheduling
    private let capacityScheduler: any DuxCapacityScheduling
    private let capacityResampleRouter: DuxCapacityResampleRouter?
    private let reviews: any DuxReviewManaging
    private let scans: any DuxScanManaging
    private var started = false
    private var shuttingDown = false
    private var shutdownTask: Task<Void, Never>?
    private var explorerOpener: ((ExplorerDestination) -> Void)?

    private init() {
        let engineService = EngineService()
        self.engineService = engineService
        scheduler = DuxMaintenanceScheduler(
            service: engineService,
            energyPolicy: SystemDuxMaintenanceEnergyPolicy()
        )
        let reviewController = DuxSnapshotReviewController(service: engineService)
        reviews = reviewController
        let capacityResampleRouter = DuxCapacityResampleRouter()
        self.capacityResampleRouter = capacityResampleRouter
        let model = AppModel(
            engineService: engineService,
            capacityResampleRequester: capacityResampleRouter
        )
        self.model = model
        let liveActions = SystemExplorerLiveFileActionPresenter()
        explorerSnapshotBrowser = ExplorerSnapshotBrowserModel(
            reviews: reviewController,
            history: engineService,
            coverage: engineService,
            liveActions: liveActions,
            subtreeScans: reviewController,
            scanDriver: model,
            rustTargetCleanupTerminalObserver: {
                await model.refreshCleanupHistory()
            },
            rustTargetDryRunTerminalObserver: {
                await model.refreshCleanupHistory()
            }
        )
        scans = model
        capacityScheduler = DuxCapacitySamplingScheduler(sampler: model)
    }

    init(
        model: AppModel,
        engineService: any DuxEngineClosing,
        scheduler: any DuxMaintenanceScheduling,
        capacityScheduler: any DuxCapacityScheduling,
        reviews: any DuxReviewManaging,
        scans: (any DuxScanManaging)? = nil,
        explorerSnapshotBrowser: ExplorerSnapshotBrowserModel? = nil
    ) {
        self.model = model
        self.engineService = engineService
        self.scheduler = scheduler
        self.capacityScheduler = capacityScheduler
        capacityResampleRouter = nil
        self.reviews = reviews
        self.scans = scans ?? model
        self.explorerSnapshotBrowser = explorerSnapshotBrowser
            ?? ExplorerSnapshotBrowserModel(reviews: UnavailableDuxSnapshotReviewBrowser())
    }

    func start() async {
        guard !started, !shuttingDown else {
            return
        }
        started = true
        // The menu-bar popover is a transient scene. Keep startup I/O owned by
        // the app runtime rather than by a view task that is cancelled whenever
        // the user dismisses the popover. This also prevents repeated opens
        // from starting overlapping initial engine/volume loads.
        async let initialState: Void = model.loadInitialState()
        async let maintenance: Void = scheduler.start()
        async let capacity: Void = capacityScheduler.start()
        _ = await (initialState, maintenance, capacity)
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

    func revealMenuBarItemForSession() {
        model.revealMenuBarItemForSession()
    }

    func installExplorerOpener(_ opener: @escaping (ExplorerDestination) -> Void) {
        explorerOpener = opener
    }

    func handleUrgentRecommendations(_ payload: DiskPressureNotificationPayload) async {
        _ = payload
        model.requestExplorerDestination(.recommendations)
        explorerOpener?(.recommendations)
    }

    func refreshStorageAccessEvidenceAfterActivation() async {
        await model.refreshStorageAccessEvidenceAfterActivation()
    }

    func shutdown() async {
        if let shutdownTask {
            await shutdownTask.value
            return
        }
        shuttingDown = true
        let model = model
        let capacityScheduler = capacityScheduler
        let capacityResampleRouter = capacityResampleRouter
        let scheduler = scheduler
        let reviews = reviews
        let scans = scans
        let explorerSnapshotBrowser = explorerSnapshotBrowser
        let engineService = engineService
        let task = Task { @MainActor in
            model.invalidateCapacityHistoryOperations()
            model.invalidatePressurePolicyOperations()
            model.invalidatePermanentCleanupPolicyOperations()
            model.invalidateCleanupExclusionsOperations()
            model.invalidateProjectDiscoveryRootsOperations()
            model.invalidateStorageAccessProbeOperations()
            await model.shutdownDirectCargoEnrollment()
            await model.shutdownCleanupHistoryClear()
            model.invalidateCleanupHistoryOperations()
            model.invalidatePersistentRecoveryDebtOperations()
            model.invalidateClaimedRunningScanProvenanceOperations()
            await model.cliInstallation.shutdown()
            await scans.shutdownTargetedReclaimScan()
            await scans.shutdownHomeScan()
            await explorerSnapshotBrowser.shutdownRustTargetCleanup()
            await explorerSnapshotBrowser.shutdownRustTargetDryRun()
            await explorerSnapshotBrowser.close()
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
extension AppModel: DuxScanManaging {}
