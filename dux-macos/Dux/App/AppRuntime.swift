import Foundation

protocol DuxEngineClosing: Sendable {
    func close() async -> Bool
}

protocol DuxMaintenanceScheduling: Sendable {
    func start() async
    func signal(_ trigger: DuxMaintenanceTrigger) async
    func stop() async
    func quiesceForTerminalRuntime() async
}

protocol DuxCapacityScheduling: AnyObject, Sendable {
    func start() async
    func signal(_ trigger: DuxCapacitySamplingTrigger) async
    func stop() async
    func quiesceForTerminalRuntime() async
}

protocol DuxReviewManaging: Sendable {
    func renewNow() async
    func shutdown() async
}

@MainActor
protocol DuxScanManaging: AnyObject {
    func shutdownTargetedReclaimScan() async
    func shutdownHomeScan() async
    func quiesceForTerminalRuntime() async
}

extension DuxScanManaging {
    func quiesceForTerminalRuntime() async {
        await shutdownTargetedReclaimScan()
        await shutdownHomeScan()
    }
}

enum NativeRuntimeTerminalIntent: Sendable, Equatable {
    case ordinaryQuit
    case appDataReset
}

/// Proves that every native owner accepted before the terminal claim has been
/// fenced and joined. FFI child and core quiescence remain separate proofs.
struct NativeRuntimeResetQuiescence: Sendable {
    private let confirmedCLIMutation: ConfirmedCLIMutationQuiescence

    fileprivate init(
        confirmedCLIMutation: ConfirmedCLIMutationQuiescence
    ) {
        self.confirmedCLIMutation = confirmedCLIMutation
    }
}

enum NativeRuntimeTerminalCompletion: Sendable {
    case ordinaryQuit(NativeRuntimeResetQuiescence)
    case appDataReset(NativeRuntimeResetQuiescence)

    var intent: NativeRuntimeTerminalIntent {
        switch self {
        case .ordinaryQuit: .ordinaryQuit
        case .appDataReset: .appDataReset
        }
    }
}

enum NativeRuntimeTerminalRequestResult: Sendable {
    case completed(NativeRuntimeTerminalCompletion)
    case reentrant(NativeRuntimeTerminalIntent)
}

private enum NativeRuntimeTerminalTaskContext {
    @TaskLocal static var isDraining = false
}

private enum NativeRuntimeStartupTaskContext {
    @TaskLocal static var isStarting = false
}

private enum NativeRuntimeTerminalPhase: Equatable {
    case open
    case closing(NativeRuntimeTerminalIntent)
    case closed(NativeRuntimeTerminalIntent)
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
    private var startupTask: Task<Void, Never>?
    private var terminalPhase = NativeRuntimeTerminalPhase.open
    private var terminalTask: Task<NativeRuntimeTerminalCompletion, Never>?
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
                await model.refreshCleanupRecoveryDiagnosticsIfLoaded()
            },
            rustTargetDryRunTerminalObserver: {
                await model.refreshCleanupHistory()
                await model.refreshCleanupRecoveryDiagnosticsIfLoaded()
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
        if let startupTask {
            await startupTask.value
            return
        }
        guard !started, terminalPhase == .open, !shuttingDown else {
            return
        }
        started = true
        // The menu-bar popover is a transient scene. Keep startup I/O owned by
        // the app runtime rather than by a view task that is cancelled whenever
        // the user dismisses the popover. This also prevents repeated opens
        // from starting overlapping initial engine/volume loads.
        let model = model
        let scheduler = scheduler
        let capacityScheduler = capacityScheduler
        let capacityResampleRouter = capacityResampleRouter
        let task = Task { @MainActor in
            await NativeRuntimeStartupTaskContext.$isStarting.withValue(true) {
                async let initialState: Void = model.loadInitialState()
                async let maintenance: Void = scheduler.start()
                async let capacity: Void = capacityScheduler.start()
                _ = await (initialState, maintenance, capacity)
                await capacityResampleRouter?.attach(capacityScheduler)
            }
        }
        startupTask = task
        await task.value
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
        guard terminalPhase == .open else {
            return
        }
        model.revealMenuBarItemForSession()
    }

    func installExplorerOpener(_ opener: @escaping (ExplorerDestination) -> Void) {
        guard terminalPhase == .open else {
            return
        }
        explorerOpener = opener
    }

    func handleUrgentRecommendations(_ payload: DiskPressureNotificationPayload) async {
        guard terminalPhase == .open else {
            return
        }
        _ = payload
        model.requestExplorerDestination(.recommendations)
        explorerOpener?(.recommendations)
    }

    func refreshStorageAccessEvidenceAfterActivation() async {
        guard terminalPhase == .open else {
            return
        }
        await model.refreshStorageAccessEvidenceAfterActivation()
    }

    func shutdown() async {
        _ = await requestTerminal(.ordinaryQuit)
    }

    /// Effect-dormant native reset handoff. This fences and joins native work
    /// but deliberately performs no ordinary engine close, FFI reset call,
    /// preference mutation, relaunch, journal transition, or filesystem effect.
    func quiesceForAppDataReset() async -> NativeRuntimeTerminalRequestResult {
        await requestTerminal(.appDataReset)
    }

    private func requestTerminal(
        _ intent: NativeRuntimeTerminalIntent
    ) async -> NativeRuntimeTerminalRequestResult {
        // Terminal ingress is top-level lifecycle/UI only. No model, service,
        // scheduler, review, Explorer, scan, or other operation joined below
        // may retain AppRuntime or a terminal-request closure. That layering
        // rule prevents an admitted child from awaiting the terminal task that
        // is itself awaiting that child. The explicit reentrant results cover
        // only the two allowed ancestors: startup and this retained drain.
        if NativeRuntimeTerminalTaskContext.isDraining,
           case let .closing(winner) = terminalPhase
        {
            return .reentrant(winner)
        }
        if NativeRuntimeStartupTaskContext.isStarting,
           case let .closing(winner) = terminalPhase
        {
            return .reentrant(winner)
        }
        if let terminalTask {
            return .completed(await terminalTask.value)
        }

        precondition(terminalPhase == .open)
        terminalPhase = .closing(intent)
        shuttingDown = true
        explorerOpener = nil

        // Every synchronous fence is installed before this method reaches its
        // first suspension. The returned unstructured tasks are retained by
        // their owners, so cancelling a caller cannot abandon the drain.
        let modelDrain = model.beginTerminalRuntimeQuiescence()
        let explorerDrain = explorerSnapshotBrowser.beginTerminalRuntimeQuiescence()
        let cliDrain = model.cliInstallation.beginTerminalRuntimeQuiescence()
        let acceptedStartup = startupTask
        let capacityScheduler = capacityScheduler
        let capacityResampleRouter = capacityResampleRouter
        let scheduler = scheduler
        let reviews = reviews
        let scans = scans
        let engineService = engineService
        let task = Task { @MainActor [weak self] in
            await NativeRuntimeTerminalTaskContext.$isDraining.withValue(true) {
                await acceptedStartup?.value
                await modelDrain.value
                let confirmedCLIMutation = await cliDrain.value
                await scans.quiesceForTerminalRuntime()
                await explorerDrain.value
                await reviews.shutdown()
                await capacityResampleRouter?.invalidate()
                await capacityScheduler.quiesceForTerminalRuntime()
                await scheduler.quiesceForTerminalRuntime()

                let proof = NativeRuntimeResetQuiescence(
                    confirmedCLIMutation: confirmedCLIMutation
                )
                let completion: NativeRuntimeTerminalCompletion
                switch intent {
                case .ordinaryQuit:
                    await Self.closeEngine(engineService, after: proof)
                    completion = .ordinaryQuit(proof)
                case .appDataReset:
                    completion = .appDataReset(proof)
                }
                self?.terminalPhase = .closed(intent)
                return completion
            }
        }
        terminalTask = task
        if NativeRuntimeStartupTaskContext.isStarting {
            return .reentrant(intent)
        }
        return .completed(await task.value)
    }

    private static func closeEngine(
        _ engineService: any DuxEngineClosing,
        after _: NativeRuntimeResetQuiescence
    ) async {
        _ = await engineService.close()
    }
}

extension EngineService: DuxEngineClosing {}
extension DuxMaintenanceScheduler: DuxMaintenanceScheduling {}
extension DuxCapacitySamplingScheduler: DuxCapacityScheduling {}
extension DuxSnapshotReviewController: DuxReviewManaging {}
extension AppModel: DuxScanManaging {}
