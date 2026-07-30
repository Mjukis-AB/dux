import Foundation
import Observation

@MainActor
@Observable
final class AppModel: DuxCapacitySampling {
    private enum ScanRequestKey: Equatable {
        case home
        case subtree(sourceScanID: String, nodeID: UInt64)
    }

    private(set) var engineState = EngineConnectionState.idle
    private(set) var volumeState = VolumeCapacityState.idle {
        didSet {
            updateMenuBarVisibility()
        }
    }

    private(set) var capacityTrend: VolumeCapacityTrend?
    private(set) var pressureHistoryState = VolumePressureHistoryState.idle
    var menuBarLabelMode: MenuBarLabelMode {
        didSet {
            guard menuBarLabelMode != oldValue else {
                return
            }
            menuBarLabelPreferenceStore.save(menuBarLabelMode)
        }
    }

    private(set) var diskPressurePolicy: DiskPressurePolicy?
    private(set) var diskPressurePolicyState = DiskPressurePolicyState.idle
    var diskPressurePolicyDraft = DiskPressurePolicyDraft.defaults
    private(set) var permanentCleanupPolicy: PermanentCleanupPolicy?
    private(set) var permanentCleanupPolicyState = PermanentCleanupPolicyState.idle
    private(set) var cleanupExclusions: CleanupExclusionsPolicy?
    private(set) var cleanupExclusionsState = CleanupExclusionsState.idle
    private(set) var projectDiscoveryRoots: ProjectDiscoveryRoots?
    private(set) var projectDiscoveryRootsState = ProjectDiscoveryRootsState.idle
    private(set) var projectDiscoveryRootsRequiresAuthoritativeReload = false
    private(set) var targetedReclaimScanState = TargetedReclaimScanState.idle
    private(set) var directCargoEnrollmentStatus: DirectCargoEnrollmentStatusModel?
    private(set) var directCargoEnrollmentPreview: DirectCargoEnrollmentPreviewModel?
    private(set) var directCargoEnrollmentConfirmation:
        DirectCargoEnrollmentConfirmation?
    private(set) var directCargoEnrollmentState = DirectCargoEnrollmentViewState.idle
    var directCargoEnrollmentNeedsStatusReload: Bool {
        directCargoEnrollmentRequiresAuthoritativeReload
    }

    private(set) var cleanupHistoryRecords: [CleanupHistorySessionSummaryModel] = []
    private(set) var cleanupHistoryNextCursor: CleanupHistoryCursorModel?
    private(set) var cleanupHistoryState = CleanupHistoryLoadState.idle
    private(set) var selectedCleanupHistorySessionID: String?
    private(set) var cleanupHistoryDetailState = CleanupHistoryDetailLoadState.idle
    private(set) var cleanupHistoryRuleOutcomeState =
        CleanupHistoryRuleOutcomeLoadState.idle
    private(set) var cleanupHistoryRuleOutcomesReadAt: Date?
    private(set) var cleanupHistoryStorageThiefRanking:
        CleanupHistoryStorageThiefRankingModel?
    private(set) var cleanupHistoryStorageThiefState =
        CleanupHistoryStorageThiefLoadState.idle
    private(set) var cleanupHistoryStorageThiefReadAt: Date?
    private(set) var cleanupHistoryClearConfirmation:
        CleanupHistoryClearConfirmation?
    private(set) var cleanupHistoryClearState = CleanupHistoryClearState.idle
    private(set) var scanState = AppScanState.idle
    private(set) var loginItemState = LoginItemState.idle
    private(set) var notificationAuthorizationState = NotificationAuthorizationState.idle
    private(set) var menuBarVisibilityPreference: MenuBarVisibilityPreference
    private(set) var isMenuBarItemInserted = true
    private(set) var showsStorageAccessIntroduction: Bool
    private(set) var broaderStorageAnalysisRequested = false
    private(set) var storageAccessProbeState = StorageAccessProbeState.idle
    private(set) var latestHomeScanEvents: [HomeScanEvent] = []
    private(set) var pendingExplorerDestination: ExplorerDestination?

    private let engineService: any EngineServing
    private let volumeMonitor: any VolumeMonitoring
    private let capacityResampleRequester: any DuxCapacityResampleRequesting
    private let menuBarLabelPreferenceStore: any MenuBarLabelPreferenceStoring
    private let homeScanService: any HomeScanServing
    private let targetedReclaimScanService: any DuxTargetedReclaimScanServing
    private let homeScanClock: any HomeScanPollingClock
    private let loginItemService: any LoginItemServing
    private let notificationService: any NotificationServing
    private let diskPressureNotificationCooldownStore:
        any DiskPressureNotificationCooldownStoring
    private let menuBarVisibilityPreferenceStore: any MenuBarVisibilityPreferenceStoring
    private let storageAccessIntroductionPreferenceStore:
        any StorageAccessIntroductionPreferenceStoring
    private let storageAccessProbe: any StorageAccessProbing

    @ObservationIgnored
    private var engineLoadTask: Task<Void, Never>?
    @ObservationIgnored
    private var volumeRefreshTask: Task<Void, Never>?
    @ObservationIgnored
    private var volumeStateBeforeRefresh = VolumeCapacityState.idle
    @ObservationIgnored
    private var volumeRefreshGeneration: UInt64 = 0
    @ObservationIgnored
    private var capacityTrendTask: Task<Void, Never>?
    @ObservationIgnored
    private var capacityTrendGeneration: UInt64 = 0
    @ObservationIgnored
    private var capacityTrendRequestVolumeID: String?
    @ObservationIgnored
    private var capacityTrendRequestAnchorAt: Date?
    @ObservationIgnored
    private var capacityTrendLoadedForAnchorAt: Date?
    @ObservationIgnored
    private var pressureHistoryTask: Task<Void, Never>?
    @ObservationIgnored
    private var pressureHistoryGeneration: UInt64 = 0
    @ObservationIgnored
    private var pressureHistoryRequestVolumeID: String?
    @ObservationIgnored
    private var pressureHistoryRequestAnchorAt: Date?
    @ObservationIgnored
    private var pressurePolicyTask: Task<Void, Never>?
    @ObservationIgnored
    private var pressurePolicyGeneration: UInt64 = 0
    @ObservationIgnored
    private var pressurePolicyIsInvalidated = false
    @ObservationIgnored
    private var permanentCleanupPolicyTask: Task<Void, Never>?
    @ObservationIgnored
    private var permanentCleanupPolicyGeneration: UInt64 = 0
    @ObservationIgnored
    private var permanentCleanupPolicyIsInvalidated = false
    @ObservationIgnored
    private var cleanupExclusionsTask: Task<Void, Never>?
    @ObservationIgnored
    private var cleanupExclusionsGeneration: UInt64 = 0
    @ObservationIgnored
    private var cleanupExclusionsIsInvalidated = false
    @ObservationIgnored
    private var projectDiscoveryRootsTask: Task<Void, Never>?
    @ObservationIgnored
    private var projectDiscoveryRootsGeneration: UInt64 = 0
    @ObservationIgnored
    private var projectDiscoveryRootsIsInvalidated = false
    @ObservationIgnored
    private var targetedReclaimScanDriverTask: Task<Void, Never>?
    @ObservationIgnored
    private var activeTargetedReclaimScanTask: (any HomeScanTask)?
    @ObservationIgnored
    private var targetedReclaimScanGeneration: UInt64 = 0
    @ObservationIgnored
    private var targetedReclaimScanIsInvalidated = false
    @ObservationIgnored
    private var targetedReclaimScanShutdownInProgress = false
    @ObservationIgnored
    private var targetedReclaimScanCancellationRequested = false
    @ObservationIgnored
    private var targetedReclaimScanVolumeID: String?
    @ObservationIgnored
    private var directCargoEnrollmentTask: Task<Void, Never>?
    @ObservationIgnored
    private var directCargoEnrollmentGeneration: UInt64 = 0
    @ObservationIgnored
    private var directCargoEnrollmentIsInvalidated = false
    @ObservationIgnored
    private var directCargoEnrollmentRequiresAuthoritativeReload = false
    @ObservationIgnored
    private var pendingDirectCargoEnrollmentPreview:
        (any DuxDirectCargoEnrollmentPreviewLease)?
    @ObservationIgnored
    private var cleanupHistoryTask: Task<Void, Never>?
    @ObservationIgnored
    private var cleanupHistoryGeneration: UInt64 = 0
    @ObservationIgnored
    private var cleanupHistoryDetailTask: Task<Void, Never>?
    @ObservationIgnored
    private var cleanupHistoryDetailGeneration: UInt64 = 0
    @ObservationIgnored
    private var cleanupHistoryRuleOutcomeTask: Task<Void, Never>?
    @ObservationIgnored
    private var cleanupHistoryRuleOutcomeGeneration: UInt64 = 0
    @ObservationIgnored
    private var cleanupHistoryStorageThiefTask: Task<Void, Never>?
    @ObservationIgnored
    private var cleanupHistoryStorageThiefGeneration: UInt64 = 0
    @ObservationIgnored
    private var cleanupHistoryStorageThiefWasRequested = false
    @ObservationIgnored
    private var cleanupHistoryClearTask: Task<Void, Never>?
    @ObservationIgnored
    private var cleanupHistoryClearGeneration: UInt64 = 0
    @ObservationIgnored
    private var cleanupHistoryClearIsShuttingDown = false
    @ObservationIgnored
    private var pendingCleanupHistoryClearPreview:
        (any DuxCleanupHistoryClearPreviewLease)?
    @ObservationIgnored
    private var homeScanDriverTask: Task<AppScanRunOutcome, Never>?
    @ObservationIgnored
    private var activeHomeScanTask: (any HomeScanTask)?
    @ObservationIgnored
    private var activeScanRequest: ScanRequestKey?
    @ObservationIgnored
    private var homeScanGeneration: UInt64 = 0
    @ObservationIgnored
    private var homeScanCancellationRequested = false
    @ObservationIgnored
    private var latestHomeScanProgress: ScanProgressFacts?
    @ObservationIgnored
    private var homeScanIsInvalidated = false
    @ObservationIgnored
    private var loginItemTask: Task<Void, Never>?
    @ObservationIgnored
    private var loginItemGeneration: UInt64 = 0
    @ObservationIgnored
    private var notificationAuthorizationTask: Task<Void, Never>?
    @ObservationIgnored
    private var notificationAuthorizationGeneration: UInt64 = 0
    @ObservationIgnored
    private var diskPressureNotificationTask: Task<Void, Never>?
    @ObservationIgnored
    private var menuBarRevealOverride = false
    @ObservationIgnored
    private var storageAccessProbeTask: Task<Void, Never>?
    @ObservationIgnored
    private var storageAccessProbeGeneration: UInt64 = 0
    @ObservationIgnored
    private var storageAccessReturnProbeArmed = false
    @ObservationIgnored
    private var storageAccessProbeIsInvalidated = false

    init(
        engineService: any EngineServing = EngineService(),
        volumeMonitor: any VolumeMonitoring = VolumeMonitor(),
        capacityResampleRequester: any DuxCapacityResampleRequesting =
            NoopDuxCapacityResampleRequester(),
        menuBarLabelPreferenceStore: any MenuBarLabelPreferenceStoring =
            UserDefaultsMenuBarLabelPreferenceStore(),
        homeScanService: (any HomeScanServing)? = nil,
        targetedReclaimScanService: (any DuxTargetedReclaimScanServing)? = nil,
        homeScanClock: any HomeScanPollingClock = ContinuousHomeScanPollingClock(),
        loginItemService: any LoginItemServing = LoginItemService(),
        notificationService: any NotificationServing = NotificationService(),
        diskPressureNotificationCooldownStore:
        any DiskPressureNotificationCooldownStoring =
            UserDefaultsDiskPressureNotificationCooldownStore(),
        menuBarVisibilityPreferenceStore: any MenuBarVisibilityPreferenceStoring =
            UserDefaultsMenuBarVisibilityPreferenceStore(),
        storageAccessIntroductionPreferenceStore:
        any StorageAccessIntroductionPreferenceStoring =
            UserDefaultsStorageAccessIntroductionPreferenceStore(),
        storageAccessProbe: any StorageAccessProbing = StorageAccessProbeService()
    ) {
        self.engineService = engineService
        self.volumeMonitor = volumeMonitor
        self.capacityResampleRequester = capacityResampleRequester
        self.menuBarLabelPreferenceStore = menuBarLabelPreferenceStore
        self.homeScanService = resolvedHomeScanService(
            engineService: engineService,
            override: homeScanService
        )
        self.targetedReclaimScanService = targetedReclaimScanService ?? engineService
        self.homeScanClock = homeScanClock
        self.loginItemService = loginItemService
        self.notificationService = notificationService
        self.diskPressureNotificationCooldownStore = diskPressureNotificationCooldownStore
        self.menuBarVisibilityPreferenceStore = menuBarVisibilityPreferenceStore
        self.storageAccessIntroductionPreferenceStore =
            storageAccessIntroductionPreferenceStore
        self.storageAccessProbe = storageAccessProbe
        capacityTrend = nil
        permanentCleanupPolicy = nil
        cleanupExclusions = nil
        projectDiscoveryRoots = nil
        directCargoEnrollmentStatus = nil
        directCargoEnrollmentPreview = nil
        directCargoEnrollmentConfirmation = nil
        menuBarLabelMode = menuBarLabelPreferenceStore.load()
        menuBarVisibilityPreference = menuBarVisibilityPreferenceStore.load()
        showsStorageAccessIntroduction =
            !storageAccessIntroductionPreferenceStore.loadAcknowledged()
        updateMenuBarVisibility()
        pendingExplorerDestination = nil
    }

    func requestExplorerDestination(_ destination: ExplorerDestination) {
        pendingExplorerDestination = destination
    }

    func loadInitialState() async {
        async let engineLoad: Void = loadEngineStatus()
        async let volumeLoad: Void = loadVolumeCapacity()
        async let cleanupHistoryLoad: Void = loadCleanupHistory()
        async let projectRootsLoad: Void = loadProjectDiscoveryRoots()
        _ = await (engineLoad, volumeLoad, cleanupHistoryLoad, projectRootsLoad)
        async let trendLoad: Void = loadCapacityTrend()
        async let pressureHistoryLoad: Void = loadPressureHistory()
        _ = await (trendLoad, pressureHistoryLoad)
    }

    func loadEngineStatus() async {
        if let engineLoadTask {
            await engineLoadTask.value
            return
        }
        if case .loaded = engineState {
            return
        }

        let engineService = self.engineService
        let task = Task { @MainActor [weak self] in
            let state: EngineConnectionState
            do {
                let result = try await engineService.loadStatus()
                state = .loaded(result)
            } catch let error as EngineServiceError {
                state = .failed(error)
            } catch {
                state = .failed(.unexpected(String(describing: error)))
            }
            self?.engineState = state
            self?.engineLoadTask = nil
        }
        engineState = .loading
        engineLoadTask = task
        await task.value
    }

    func loadVolumeCapacity() async {
        if case .loaded = volumeState {
            return
        }
        await refreshVolumeCapacity()
    }

    func refreshVolumeCapacity() async {
        if let volumeRefreshTask {
            await volumeRefreshTask.value
            return
        }

        volumeRefreshGeneration &+= 1
        let generation = volumeRefreshGeneration
        volumeStateBeforeRefresh = volumeState
        if let snapshot = volumeState.snapshot {
            volumeState = .refreshing(snapshot)
        } else {
            volumeState = .loading
        }

        let volumeMonitor = self.volumeMonitor
        let engineService = self.engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<VolumeCapacitySnapshot, Error>
            do {
                let observed = try await volumeMonitor.sampleStartupVolume()
                result = try .success(await engineService.observeVolumeCapacity(observed))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled else {
                return
            }
            self?.publishVolumeRefresh(result, generation: generation)
        }
        volumeRefreshTask = task
        await task.value
    }

    func loadCapacityTrend() async {
        guard
            let snapshot = volumeState.snapshot,
            let stableVolumeID = snapshot.stableVolumeID
        else {
            capacityTrend = nil
            capacityTrendLoadedForAnchorAt = nil
            return
        }
        if let capacityTrend,
           capacityTrend.stableVolumeID == stableVolumeID,
           capacityTrendLoadedForAnchorAt == snapshot.sampledAt
        {
            return
        }
        if let capacityTrendTask {
            if capacityTrendRequestVolumeID == stableVolumeID,
               capacityTrendRequestAnchorAt == snapshot.sampledAt
            {
                await capacityTrendTask.value
                return
            }
            capacityTrendGeneration &+= 1
            capacityTrendTask.cancel()
            self.capacityTrendTask = nil
        }
        capacityTrendGeneration &+= 1
        let generation = capacityTrendGeneration
        let anchorAt = snapshot.sampledAt
        if capacityTrend?.stableVolumeID != stableVolumeID
            || capacityTrendLoadedForAnchorAt != anchorAt
        {
            capacityTrend = nil
            capacityTrendLoadedForAnchorAt = nil
        }
        let service = engineService
        capacityTrendRequestVolumeID = stableVolumeID
        capacityTrendRequestAnchorAt = anchorAt
        let task = Task { @MainActor [weak self] in
            defer {
                if let self, generation == self.capacityTrendGeneration {
                    self.capacityTrendTask = nil
                    self.capacityTrendRequestVolumeID = nil
                    self.capacityTrendRequestAnchorAt = nil
                }
            }
            do {
                let trend = try await service.loadCapacityTrend(
                    stableVolumeID: stableVolumeID,
                    at: anchorAt
                )
                guard let self,
                      !Task.isCancelled,
                      generation == self.capacityTrendGeneration,
                      self.volumeState.snapshot?.stableVolumeID == stableVolumeID,
                      self.volumeState.snapshot?.sampledAt == anchorAt,
                      trend.stableVolumeID == stableVolumeID,
                      trend.sampledAt <= anchorAt
                else {
                    return
                }
                self.capacityTrend = trend
                self.capacityTrendLoadedForAnchorAt = anchorAt
            } catch is CancellationError {
                return
            } catch {
                // Trend history is optional presentation context. Capacity
                // status remains authoritative while this anchor has no chart.
            }
        }
        capacityTrendTask = task
        await task.value
    }

    func loadPressureHistory() async {
        guard
            let snapshot = volumeState.snapshot,
            let stableVolumeID = snapshot.stableVolumeID
        else {
            pressureHistoryState = .idle
            return
        }
        if case let .loaded(history) = pressureHistoryState,
           history.stableVolumeID == stableVolumeID,
           history.anchorAt == snapshot.sampledAt
        {
            return
        }
        if let pressureHistoryTask {
            if pressureHistoryRequestVolumeID == stableVolumeID,
               pressureHistoryRequestAnchorAt == snapshot.sampledAt
            {
                await pressureHistoryTask.value
                return
            }
            pressureHistoryGeneration &+= 1
            pressureHistoryTask.cancel()
            self.pressureHistoryTask = nil
        }
        pressureHistoryGeneration &+= 1
        let generation = pressureHistoryGeneration
        let anchorAt = snapshot.sampledAt
        let previous = pressureHistoryState.history
        pressureHistoryState = .loading
        let service = engineService
        pressureHistoryRequestVolumeID = stableVolumeID
        pressureHistoryRequestAnchorAt = anchorAt
        let task = Task { @MainActor [weak self] in
            defer {
                if let self, generation == self.pressureHistoryGeneration {
                    self.pressureHistoryTask = nil
                    self.pressureHistoryRequestVolumeID = nil
                    self.pressureHistoryRequestAnchorAt = nil
                }
            }
            do {
                let history = try await service.loadPressureEpisodeHistory(
                    stableVolumeID: stableVolumeID,
                    at: anchorAt,
                    limit: 64
                )
                guard let self,
                      !Task.isCancelled,
                      generation == self.pressureHistoryGeneration,
                      self.volumeState.snapshot?.stableVolumeID == stableVolumeID,
                      self.volumeState.snapshot?.sampledAt == anchorAt
                else {
                    return
                }
                self.pressureHistoryState = .loaded(history)
            } catch is CancellationError {
                return
            } catch {
                guard let self,
                      generation == self.pressureHistoryGeneration,
                      self.volumeState.snapshot?.stableVolumeID == stableVolumeID,
                      self.volumeState.snapshot?.sampledAt == anchorAt
                else {
                    return
                }
                self.pressureHistoryState = if let previous {
                    .stale(previous, .unavailable)
                } else {
                    .failed(.unavailable)
                }
            }
        }
        pressureHistoryTask = task
        await task.value
    }

    func invalidateCapacityHistoryOperations() {
        capacityTrendGeneration &+= 1
        pressureHistoryGeneration &+= 1
        capacityTrendTask?.cancel()
        pressureHistoryTask?.cancel()
        capacityTrendTask = nil
        capacityTrendRequestVolumeID = nil
        capacityTrendRequestAnchorAt = nil
        pressureHistoryTask = nil
        pressureHistoryRequestVolumeID = nil
        pressureHistoryRequestAnchorAt = nil
    }

    func cancelVolumeRefresh() {
        guard let volumeRefreshTask else {
            return
        }
        volumeRefreshGeneration &+= 1
        volumeRefreshTask.cancel()
        self.volumeRefreshTask = nil
        volumeState = volumeStateBeforeRefresh
    }

    func loadDiskPressurePolicy() async {
        guard !pressurePolicyIsInvalidated else {
            return
        }
        if let pressurePolicyTask {
            await pressurePolicyTask.value
            return
        }

        pressurePolicyGeneration &+= 1
        let generation = pressurePolicyGeneration
        diskPressurePolicyState = .loading
        let engineService = self.engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<DiskPressurePolicy, Error>
            do {
                result = try .success(await engineService.loadDiskPressurePolicy())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled else {
                return
            }
            self?.publishPressurePolicyLoad(result, generation: generation)
        }
        pressurePolicyTask = task
        await task.value
    }

    func saveDiskPressurePolicy() async {
        guard
            !pressurePolicyIsInvalidated,
            pressurePolicyTask == nil,
            !diskPressurePolicyState.isBusy
        else {
            return
        }
        let configuration: DiskPressurePolicyConfiguration
        do {
            configuration = try diskPressurePolicyDraft.configuration()
        } catch let error as DiskPressurePolicyDraftError {
            diskPressurePolicyState = .failed(.draft(error))
            return
        } catch {
            diskPressurePolicyState = .failed(.unexpected)
            return
        }
        await mutateDiskPressurePolicy(state: .saving) { service in
            try await service.setDiskPressurePolicy(configuration)
        }
    }

    func resetDiskPressurePolicy() async {
        guard
            !pressurePolicyIsInvalidated,
            pressurePolicyTask == nil,
            !diskPressurePolicyState.isBusy
        else {
            return
        }
        await mutateDiskPressurePolicy(state: .resetting) { service in
            try await service.resetDiskPressurePolicy()
        }
    }

    func invalidatePressurePolicyOperations() {
        pressurePolicyIsInvalidated = true
        pressurePolicyGeneration &+= 1
        pressurePolicyTask?.cancel()
        pressurePolicyTask = nil
        diskPressurePolicyState = diskPressurePolicy == nil ? .idle : .ready
    }

    static let permanentCleanupEnableConfirmation = "ENABLE PERMANENT CLEANUP"

    func loadPermanentCleanupPolicy() async {
        guard !permanentCleanupPolicyIsInvalidated else {
            return
        }
        if let permanentCleanupPolicyTask {
            await permanentCleanupPolicyTask.value
            return
        }

        permanentCleanupPolicyGeneration &+= 1
        let generation = permanentCleanupPolicyGeneration
        permanentCleanupPolicyState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<PermanentCleanupPolicy, Error>
            do {
                result = try .success(await service.loadPermanentCleanupPolicy())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled else {
                return
            }
            self?.publishPermanentCleanupPolicyLoad(result, generation: generation)
        }
        permanentCleanupPolicyTask = task
        await task.value
    }

    /// Changes the global opt-in. Enabling requires the exact phrase so the UI
    /// cannot accidentally permit permanent cleanup.
    func setPermanentCleanupEnabled(
        _ enabled: Bool,
        confirmation: String? = nil
    ) async {
        guard !permanentCleanupPolicyIsInvalidated,
              permanentCleanupPolicyTask == nil,
              !permanentCleanupPolicyState.isBusy
        else {
            return
        }
        if enabled {
            guard let permanentCleanupPolicy, !permanentCleanupPolicy.enabled,
                  confirmation == Self.permanentCleanupEnableConfirmation
            else {
                permanentCleanupPolicyState = .failed(.confirmationRequired)
                return
            }
        }
        await mutatePermanentCleanupPolicy(state: enabled ? .enabling : .disabling) { service in
            try await service.setPermanentCleanupEnabled(enabled)
        }
    }

    func resetPermanentCleanup() async {
        guard !permanentCleanupPolicyIsInvalidated,
              permanentCleanupPolicyTask == nil,
              !permanentCleanupPolicyState.isBusy
        else {
            return
        }
        // Reset is always protection-strengthening: the core default denies
        // permanent-cleanup effects, so it never needs enable confirmation.
        await mutatePermanentCleanupPolicy(state: .resetting) { service in
            try await service.resetPermanentCleanup()
        }
    }

    func invalidatePermanentCleanupPolicyOperations() {
        permanentCleanupPolicyIsInvalidated = true
        permanentCleanupPolicyGeneration &+= 1
        permanentCleanupPolicyTask?.cancel()
        permanentCleanupPolicyTask = nil
        permanentCleanupPolicyState = permanentCleanupPolicy == nil ? .idle : .ready
    }

    func loadCleanupExclusions() async {
        guard !cleanupExclusionsIsInvalidated else {
            return
        }
        if let cleanupExclusionsTask {
            await cleanupExclusionsTask.value
            return
        }

        cleanupExclusionsGeneration &+= 1
        let generation = cleanupExclusionsGeneration
        cleanupExclusionsState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupExclusionsPolicy, Error>
            do {
                result = try .success(await service.loadCleanupExclusions())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled else {
                return
            }
            self?.publishCleanupExclusionsLoad(result, generation: generation)
        }
        cleanupExclusionsTask = task
        await task.value
    }

    func addCleanupExclusion(_ path: CleanupExclusionPathObservation) async {
        guard
            !cleanupExclusionsIsInvalidated,
            cleanupExclusionsTask == nil,
            !cleanupExclusionsState.isBusy,
            let current = cleanupExclusions
        else {
            return
        }
        guard path.encoding == .unixBytes, !path.encodedBytes.isEmpty else {
            cleanupExclusionsState = .failed(.invalidSelection)
            return
        }
        guard !current.paths.contains(path) else {
            cleanupExclusionsState = .ready
            return
        }
        await mutateCleanupExclusions(state: .adding) { service in
            try await service.setCleanupExclusions(current.paths + [path])
        }
    }

    /// Removing a deny-only prefix weakens protection, so every removal must
    /// arrive from an explicit confirmation action for the exact observed row.
    func removeCleanupExclusion(
        _ path: CleanupExclusionPathObservation,
        confirmed: Bool = false
    ) async {
        guard
            !cleanupExclusionsIsInvalidated,
            cleanupExclusionsTask == nil,
            !cleanupExclusionsState.isBusy,
            let current = cleanupExclusions,
            current.paths.contains(path)
        else {
            return
        }
        guard confirmed else {
            cleanupExclusionsState = .failed(.confirmationRequired)
            return
        }
        await mutateCleanupExclusions(state: .removing) { service in
            try await service.setCleanupExclusions(current.paths.filter { $0 != path })
        }
    }

    func resetCleanupExclusions(confirmed: Bool = false) async {
        guard
            !cleanupExclusionsIsInvalidated,
            cleanupExclusionsTask == nil,
            !cleanupExclusionsState.isBusy,
            let current = cleanupExclusions
        else {
            return
        }
        guard !current.paths.isEmpty else {
            cleanupExclusionsState = .ready
            return
        }
        guard confirmed else {
            cleanupExclusionsState = .failed(.confirmationRequired)
            return
        }
        await mutateCleanupExclusions(state: .resetting) { service in
            try await service.resetCleanupExclusions()
        }
    }

    func invalidateCleanupExclusionsOperations() {
        cleanupExclusionsIsInvalidated = true
        cleanupExclusionsGeneration &+= 1
        cleanupExclusionsTask?.cancel()
        cleanupExclusionsTask = nil
        cleanupExclusionsState = cleanupExclusions == nil ? .idle : .ready
    }

    func loadProjectDiscoveryRoots() async {
        guard !projectDiscoveryRootsIsInvalidated else {
            return
        }
        if let projectDiscoveryRootsTask {
            await projectDiscoveryRootsTask.value
            return
        }

        projectDiscoveryRootsGeneration &+= 1
        let generation = projectDiscoveryRootsGeneration
        projectDiscoveryRootsState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<ProjectDiscoveryRoots, Error>
            do {
                result = try .success(await service.loadProjectDiscoveryRoots())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled else {
                return
            }
            self?.publishProjectDiscoveryRootsLoad(result, generation: generation)
        }
        projectDiscoveryRootsTask = task
        await task.value
    }

    func addProjectDiscoveryRoot(_ root: ProjectDiscoveryRoot) async {
        guard
            !projectDiscoveryRootsIsInvalidated,
            projectDiscoveryRootsTask == nil,
            !projectDiscoveryRootsState.isBusy,
            !projectDiscoveryRootsRequiresAuthoritativeReload,
            let current = projectDiscoveryRoots
        else {
            return
        }
        guard
            root.encoding == .unixBytes,
            ProjectDiscoveryRoot.hasValidUnixShape(root.encodedBytes)
        else {
            projectDiscoveryRootsState = .failed(.invalidSelection)
            return
        }
        guard !current.roots.contains(root) else {
            projectDiscoveryRootsState = .ready
            return
        }
        guard current.roots.count < ProjectDiscoveryRoot.maximumCount else {
            projectDiscoveryRootsState = .failed(.service(.tooManyPaths))
            return
        }
        guard !current.roots.contains(where: { $0.overlaps(root) }) else {
            projectDiscoveryRootsState = .failed(.overlappingSelection)
            return
        }
        await mutateProjectDiscoveryRoots(state: .adding) { service in
            try await service.setProjectDiscoveryRoots(current.roots + [root])
        }
    }

    func rejectProjectDiscoveryRootSelection() {
        guard !projectDiscoveryRootsState.isBusy else {
            return
        }
        projectDiscoveryRootsState = .failed(.invalidSelection)
    }

    func removeProjectDiscoveryRoot(_ root: ProjectDiscoveryRoot) async {
        guard
            !projectDiscoveryRootsIsInvalidated,
            projectDiscoveryRootsTask == nil,
            !projectDiscoveryRootsState.isBusy,
            !projectDiscoveryRootsRequiresAuthoritativeReload,
            let current = projectDiscoveryRoots,
            current.roots.contains(root)
        else {
            return
        }
        await mutateProjectDiscoveryRoots(state: .removing) { service in
            try await service.setProjectDiscoveryRoots(current.roots.filter { $0 != root })
        }
    }

    func resetProjectDiscoveryRoots() async {
        guard
            !projectDiscoveryRootsIsInvalidated,
            projectDiscoveryRootsTask == nil,
            !projectDiscoveryRootsState.isBusy,
            !projectDiscoveryRootsRequiresAuthoritativeReload,
            let current = projectDiscoveryRoots
        else {
            return
        }
        guard current.source != .default else {
            projectDiscoveryRootsState = .ready
            return
        }
        await mutateProjectDiscoveryRoots(state: .resetting) { service in
            try await service.resetProjectDiscoveryRoots()
        }
    }

    func invalidateProjectDiscoveryRootsOperations() {
        projectDiscoveryRootsIsInvalidated = true
        projectDiscoveryRootsGeneration &+= 1
        projectDiscoveryRootsTask?.cancel()
        projectDiscoveryRootsTask = nil
        projectDiscoveryRootsState = projectDiscoveryRoots == nil ? .idle : .ready
    }

    func loadDirectCargoEnrollmentStatus() async {
        guard !directCargoEnrollmentIsInvalidated else {
            return
        }
        if pendingDirectCargoEnrollmentPreview != nil {
            directCargoEnrollmentState = .awaitingEnrollmentConfirmation
            return
        }
        if let directCargoEnrollmentTask {
            await directCargoEnrollmentTask.value
            return
        }

        directCargoEnrollmentGeneration &+= 1
        let generation = directCargoEnrollmentGeneration
        directCargoEnrollmentState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<DirectCargoEnrollmentStatusModel, Error>
            do {
                result = try .success(await service.loadDirectCargoEnrollmentStatus())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled else {
                return
            }
            self?.publishDirectCargoEnrollmentLoad(result, generation: generation)
        }
        directCargoEnrollmentTask = task
        await task.value
    }

    func inspectDirectCargoExecutable(_ selection: DirectCargoExecutableSelection) async {
        guard
            !directCargoEnrollmentIsInvalidated,
            !directCargoEnrollmentRequiresAuthoritativeReload,
            directCargoEnrollmentTask == nil,
            !directCargoEnrollmentState.isBusy
        else {
            return
        }
        guard DirectCargoExecutableSelection.isValidUnixCargoPath(
            selection.encodedPathBytes
        ) else {
            directCargoEnrollmentState = .failed(.invalidSelection)
            return
        }

        directCargoEnrollmentGeneration &+= 1
        let generation = directCargoEnrollmentGeneration
        let superseded = pendingDirectCargoEnrollmentPreview
        pendingDirectCargoEnrollmentPreview = nil
        directCargoEnrollmentPreview = nil
        directCargoEnrollmentConfirmation = nil
        await superseded?.release()
        guard
            generation == directCargoEnrollmentGeneration,
            !directCargoEnrollmentIsInvalidated
        else {
            return
        }

        directCargoEnrollmentState = .inspecting
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<any DuxDirectCargoEnrollmentPreviewLease, Error>
            do {
                result = try .success(await service.inspectDirectCargoExecutable(selection))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.directCargoEnrollmentGeneration,
                  !self.directCargoEnrollmentIsInvalidated
            else {
                if case let .success(preview) = result {
                    await preview.release()
                }
                return
            }
            self.directCargoEnrollmentTask = nil
            switch result {
            case let .success(preview):
                self.pendingDirectCargoEnrollmentPreview = preview
                self.directCargoEnrollmentPreview = preview.preview
                self.directCargoEnrollmentConfirmation =
                    DirectCargoEnrollmentConfirmation(
                        generation: generation,
                        preview: preview.preview
                    )
                self.directCargoEnrollmentState = .awaitingEnrollmentConfirmation
            case let .failure(error):
                self.directCargoEnrollmentState = .failed(
                    Self.directCargoEnrollmentFailure(for: error)
                )
            }
        }
        directCargoEnrollmentTask = task
        await task.value
    }

    func rejectDirectCargoExecutableSelection() {
        guard
            !directCargoEnrollmentIsInvalidated,
            !directCargoEnrollmentRequiresAuthoritativeReload,
            directCargoEnrollmentTask == nil,
            !directCargoEnrollmentState.isBusy
        else {
            return
        }
        directCargoEnrollmentState = .failed(.invalidSelection)
    }

    func enrollInspectedDirectCargo(
        confirmation: DirectCargoEnrollmentConfirmation? = nil
    ) async {
        guard
            !directCargoEnrollmentIsInvalidated,
            !directCargoEnrollmentRequiresAuthoritativeReload,
            directCargoEnrollmentTask == nil,
            !directCargoEnrollmentState.isBusy,
            let preview = pendingDirectCargoEnrollmentPreview,
            let currentConfirmation = directCargoEnrollmentConfirmation
        else {
            return
        }
        guard let confirmation else {
            directCargoEnrollmentState = .failed(.enrollmentConfirmationRequired)
            return
        }
        guard
            confirmation == currentConfirmation,
            confirmation.preview == preview.preview,
            confirmation.generation == directCargoEnrollmentGeneration
        else {
            directCargoEnrollmentState = .failed(.enrollmentPreviewChanged)
            return
        }

        directCargoEnrollmentGeneration &+= 1
        let generation = directCargoEnrollmentGeneration
        pendingDirectCargoEnrollmentPreview = nil
        directCargoEnrollmentConfirmation = nil
        directCargoEnrollmentState = .enrolling
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<DirectCargoEnrollmentUpdateModel, Error>
            do {
                result = try .success(await service.enrollDirectCargo(preview))
            } catch {
                result = .failure(error)
            }
            await preview.release()
            guard !Task.isCancelled, let self,
                  generation == self.directCargoEnrollmentGeneration,
                  !self.directCargoEnrollmentIsInvalidated
            else {
                return
            }
            self.directCargoEnrollmentPreview = nil
            switch result {
            case let .success(update):
                self.directCargoEnrollmentStatus = update.status
                self.directCargoEnrollmentRequiresAuthoritativeReload = false
                self.directCargoEnrollmentState = .ready
            case let .failure(error):
                let failure = Self.directCargoEnrollmentFailure(for: error)
                if failure == .service(.outcomeUnknown) {
                    // The commit preview is consumed and must never be retried.
                    self.directCargoEnrollmentRequiresAuthoritativeReload = true
                    await self.reconcileUncertainDirectCargoMutation(
                        service: service,
                        generation: generation
                    )
                }
                guard
                    !Task.isCancelled,
                    generation == self.directCargoEnrollmentGeneration,
                    !self.directCargoEnrollmentIsInvalidated
                else {
                    return
                }
                self.directCargoEnrollmentState = .failed(failure)
            }
            self.directCargoEnrollmentTask = nil
        }
        directCargoEnrollmentTask = task
        await task.value
    }

    func discardDirectCargoEnrollmentPreview(
        matching confirmation: DirectCargoEnrollmentConfirmation? = nil
    ) async {
        guard
            directCargoEnrollmentTask == nil,
            !directCargoEnrollmentState.isBusy,
            let preview = pendingDirectCargoEnrollmentPreview
        else {
            return
        }
        if let confirmation {
            guard confirmation == directCargoEnrollmentConfirmation else {
                return
            }
        }
        directCargoEnrollmentGeneration &+= 1
        pendingDirectCargoEnrollmentPreview = nil
        directCargoEnrollmentPreview = nil
        directCargoEnrollmentConfirmation = nil
        directCargoEnrollmentState = directCargoEnrollmentStatus == nil ? .idle : .ready
        await preview.release()
    }

    func revokeDirectCargoEnrollment(confirmed: Bool = false) async {
        guard
            !directCargoEnrollmentIsInvalidated,
            !directCargoEnrollmentRequiresAuthoritativeReload,
            directCargoEnrollmentTask == nil,
            !directCargoEnrollmentState.isBusy
        else {
            return
        }
        guard confirmed else {
            directCargoEnrollmentState = .failed(.revocationConfirmationRequired)
            return
        }

        directCargoEnrollmentGeneration &+= 1
        let generation = directCargoEnrollmentGeneration
        let superseded = pendingDirectCargoEnrollmentPreview
        pendingDirectCargoEnrollmentPreview = nil
        directCargoEnrollmentPreview = nil
        directCargoEnrollmentConfirmation = nil
        directCargoEnrollmentState = .revoking
        await superseded?.release()
        guard
            generation == directCargoEnrollmentGeneration,
            !directCargoEnrollmentIsInvalidated
        else {
            return
        }

        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<DirectCargoEnrollmentUpdateModel, Error>
            do {
                result = try .success(await service.revokeDirectCargoEnrollment())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.directCargoEnrollmentGeneration,
                  !self.directCargoEnrollmentIsInvalidated
            else {
                return
            }
            self.directCargoEnrollmentTask = nil
            switch result {
            case let .success(update):
                self.directCargoEnrollmentStatus = update.status
                self.directCargoEnrollmentRequiresAuthoritativeReload = false
                self.directCargoEnrollmentState = .ready
            case let .failure(error):
                let failure = Self.directCargoEnrollmentFailure(for: error)
                if failure == .service(.outcomeUnknown) {
                    self.directCargoEnrollmentRequiresAuthoritativeReload = true
                    await self.reconcileUncertainDirectCargoMutation(
                        service: service,
                        generation: generation
                    )
                }
                guard
                    !Task.isCancelled,
                    generation == self.directCargoEnrollmentGeneration,
                    !self.directCargoEnrollmentIsInvalidated
                else {
                    return
                }
                self.directCargoEnrollmentState = .failed(failure)
            }
        }
        directCargoEnrollmentTask = task
        await task.value
    }

    func invalidateDirectCargoEnrollmentOperations() {
        directCargoEnrollmentIsInvalidated = true
        directCargoEnrollmentGeneration &+= 1
        directCargoEnrollmentTask?.cancel()
        directCargoEnrollmentTask = nil
        let preview = pendingDirectCargoEnrollmentPreview
        pendingDirectCargoEnrollmentPreview = nil
        directCargoEnrollmentPreview = nil
        directCargoEnrollmentConfirmation = nil
        directCargoEnrollmentState = directCargoEnrollmentStatus == nil ? .idle : .ready
        if let preview {
            Task { await preview.release() }
        }
    }

    /// Cancels only unconfirmed presentation work. A confirmed enrollment or
    /// revocation is never interrupted merely because Settings disappeared.
    func dismissDirectCargoEnrollmentPresentation() async {
        guard
            directCargoEnrollmentState != .enrolling,
            directCargoEnrollmentState != .revoking
        else {
            return
        }
        directCargoEnrollmentGeneration &+= 1
        let generation = directCargoEnrollmentGeneration
        let operation = directCargoEnrollmentTask
        let preview = pendingDirectCargoEnrollmentPreview
        pendingDirectCargoEnrollmentPreview = nil
        directCargoEnrollmentPreview = nil
        directCargoEnrollmentConfirmation = nil
        operation?.cancel()
        await operation?.value
        await preview?.release()
        guard generation == directCargoEnrollmentGeneration else {
            return
        }
        directCargoEnrollmentTask = nil
        directCargoEnrollmentState =
            directCargoEnrollmentRequiresAuthoritativeReload
                ? .failed(.service(.outcomeUnknown))
                : (directCargoEnrollmentStatus == nil ? .idle : .ready)
    }

    /// Suppresses late presentation but waits for any already-confirmed,
    /// synchronous core mutation to finish before the engine is closed.
    func shutdownDirectCargoEnrollment() async {
        directCargoEnrollmentIsInvalidated = true
        directCargoEnrollmentGeneration &+= 1
        let operation = directCargoEnrollmentTask
        let preview = pendingDirectCargoEnrollmentPreview
        pendingDirectCargoEnrollmentPreview = nil
        directCargoEnrollmentPreview = nil
        directCargoEnrollmentConfirmation = nil
        operation?.cancel()
        await operation?.value
        await preview?.release()
        directCargoEnrollmentTask = nil
        directCargoEnrollmentState = directCargoEnrollmentStatus == nil ? .idle : .ready
    }

    /// Loads only path-free durable cleanup outcome metadata. This operation
    /// never creates a plan, approval, journal claim, or effect capability.
    func loadCleanupHistory() async {
        guard !cleanupHistoryClearState.isClearing else {
            return
        }
        if case .loaded = cleanupHistoryState {
            return
        }
        if let cleanupHistoryTask {
            await cleanupHistoryTask.value
            return
        }

        cleanupHistoryGeneration &+= 1
        let generation = cleanupHistoryGeneration
        fenceCleanupHistoryDetailForSummaryRefresh()
        cleanupHistoryState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupHistoryPageModel, Error>
            do {
                result = try .success(
                    await service.loadRecentCleanupHistory(cursor: nil, limit: 64)
                )
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupHistoryGeneration
            else {
                return
            }
            self.publishCleanupHistory(result, appending: false, generation: generation)
        }
        cleanupHistoryTask = task
        await task.value
        await resumeSelectedCleanupHistoryDetailIfNeeded()
    }

    func loadMoreCleanupHistory() async {
        guard
            !cleanupHistoryClearState.isClearing,
            cleanupHistoryTask == nil,
            let cursor = cleanupHistoryNextCursor
        else {
            return
        }

        cleanupHistoryGeneration &+= 1
        let generation = cleanupHistoryGeneration
        cleanupHistoryState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupHistoryPageModel, Error>
            do {
                result = try .success(
                    await service.loadRecentCleanupHistory(cursor: cursor, limit: 64)
                )
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupHistoryGeneration
            else {
                return
            }
            self.publishCleanupHistory(result, appending: true, generation: generation)
        }
        cleanupHistoryTask = task
        await task.value
    }

    func refreshCleanupHistory() async {
        guard !cleanupHistoryClearState.isClearing else {
            return
        }
        cleanupHistoryGeneration &+= 1
        cleanupHistoryTask?.cancel()
        cleanupHistoryTask = nil
        cleanupHistoryState = cleanupHistoryRecords.isEmpty ? .idle : .loading
        await loadCleanupHistory()
        await refreshRecurringStorageThievesIfRequested()
    }

    /// Lazily derives a bounded recurring-growth ranking. This is independent
    /// from the cleanup-session feed and never scans, schedules, or cleans.
    func loadRecurringStorageThieves() async {
        cleanupHistoryStorageThiefWasRequested = true
        if cleanupHistoryStorageThiefState == .loaded {
            return
        }
        if let cleanupHistoryStorageThiefTask {
            await cleanupHistoryStorageThiefTask.value
            return
        }
        await startRecurringStorageThiefLoad()
    }

    func refreshRecurringStorageThieves() async {
        cleanupHistoryStorageThiefWasRequested = true
        cleanupHistoryStorageThiefGeneration &+= 1
        cleanupHistoryStorageThiefTask?.cancel()
        cleanupHistoryStorageThiefTask = nil
        await startRecurringStorageThiefLoad()
    }

    func invalidateCleanupHistoryOperations() {
        cleanupHistoryGeneration &+= 1
        cleanupHistoryTask?.cancel()
        cleanupHistoryTask = nil
        cleanupHistoryState = cleanupHistoryRecords.isEmpty ? .idle : .loaded
        fenceRecurringStorageThieves(discardSnapshot: true)
        closeCleanupHistorySession()
    }

    func prepareCleanupHistoryClear() async {
        guard
            !cleanupHistoryClearIsShuttingDown,
            cleanupHistoryClearTask == nil,
            !cleanupHistoryClearState.isBusy
        else {
            return
        }

        cleanupHistoryClearGeneration &+= 1
        let generation = cleanupHistoryClearGeneration
        let superseded = pendingCleanupHistoryClearPreview
        pendingCleanupHistoryClearPreview = nil
        cleanupHistoryClearConfirmation = nil
        await superseded?.release()
        guard
            generation == cleanupHistoryClearGeneration,
            !cleanupHistoryClearIsShuttingDown
        else {
            return
        }

        cleanupHistoryClearState = .preparing
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<any DuxCleanupHistoryClearPreviewLease, Error>
            do {
                result = try .success(await service.prepareCleanupHistoryClear())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupHistoryClearGeneration,
                  !self.cleanupHistoryClearIsShuttingDown
            else {
                if case let .success(preview) = result {
                    await preview.release()
                }
                return
            }
            self.cleanupHistoryClearTask = nil
            switch result {
            case let .success(preview):
                let confirmation = CleanupHistoryClearConfirmation(
                    generation: generation,
                    preview: preview.preview
                )
                self.pendingCleanupHistoryClearPreview = preview
                self.cleanupHistoryClearConfirmation = confirmation
                self.cleanupHistoryClearState = .awaitingConfirmation(
                    confirmation
                )
            case let .failure(error):
                let failure =
                    (error as? CleanupHistoryClearServiceError)
                        ?? .invalidResponse
                self.cleanupHistoryClearState =
                    failure == .outcomeUnknown
                        ? .outcomeUnknown
                        : .failed(failure)
            }
        }
        cleanupHistoryClearTask = task
        await task.value
    }

    func confirmCleanupHistoryClear(
        _ confirmation: CleanupHistoryClearConfirmation
    ) async {
        guard
            !cleanupHistoryClearIsShuttingDown,
            cleanupHistoryClearTask == nil,
            let currentConfirmation = cleanupHistoryClearConfirmation,
            let preview = pendingCleanupHistoryClearPreview,
            case .awaitingConfirmation = cleanupHistoryClearState,
            confirmation == currentConfirmation,
            confirmation.preview == preview.preview,
            confirmation.generation == cleanupHistoryClearGeneration
        else {
            return
        }

        cleanupHistoryClearGeneration &+= 1
        let generation = cleanupHistoryClearGeneration
        pendingCleanupHistoryClearPreview = nil
        cleanupHistoryClearConfirmation = nil
        cleanupHistoryClearState = .clearing(confirmation.preview)

        // Fence every pre-clear observation before the mutation is queued.
        // A late read may finish, but it cannot republish deleted rows.
        cleanupHistoryGeneration &+= 1
        let historyGeneration = cleanupHistoryGeneration
        cleanupHistoryTask?.cancel()
        cleanupHistoryTask = nil
        cleanupHistoryRecords = []
        cleanupHistoryNextCursor = nil
        cleanupHistoryState = .loading
        let reloadStorageThieves = cleanupHistoryStorageThiefWasRequested
        fenceRecurringStorageThieves(discardSnapshot: true)
        if reloadStorageThieves {
            cleanupHistoryStorageThiefWasRequested = true
            cleanupHistoryStorageThiefState = .loading
        }
        let storageThiefGeneration = cleanupHistoryStorageThiefGeneration
        closeCleanupHistorySession()

        let service = engineService
        let task = Task { @MainActor [weak self] in
            let clearResult: Result<CleanupHistoryClearResultModel, Error>
            do {
                clearResult = try .success(
                    await service.clearCleanupHistory(preview)
                )
            } catch {
                clearResult = .failure(error)
            }
            await preview.release()

            // Every terminal response performs exactly one observation-only
            // refresh. It never repeats the clearing operation.
            let historyResult: Result<CleanupHistoryPageModel, Error>
            do {
                historyResult = try .success(
                    await service.loadRecentCleanupHistory(
                        cursor: nil,
                        limit: 64
                    )
                )
            } catch {
                historyResult = .failure(error)
            }
            let storageThiefResult:
                Result<CleanupHistoryStorageThiefRankingModel, Error>?
            if reloadStorageThieves {
                do {
                    storageThiefResult = try .success(
                        await service.loadRecurringStorageThieves()
                    )
                } catch {
                    storageThiefResult = .failure(error)
                }
            } else {
                storageThiefResult = nil
            }

            guard !Task.isCancelled, let self,
                  generation == self.cleanupHistoryClearGeneration,
                  historyGeneration == self.cleanupHistoryGeneration,
                  storageThiefGeneration
                      == self.cleanupHistoryStorageThiefGeneration,
                  !self.cleanupHistoryClearIsShuttingDown
            else {
                return
            }
            self.cleanupHistoryClearTask = nil
            switch historyResult {
            case let .success(page):
                self.cleanupHistoryRecords = page.records
                self.cleanupHistoryNextCursor = page.nextCursor
                self.cleanupHistoryState = .loaded
            case let .failure(error):
                self.cleanupHistoryState = .failed(
                    (error as? CleanupHistoryServiceError) ?? .invalidResponse
                )
            }
            switch storageThiefResult {
            case let .success(ranking):
                self.cleanupHistoryStorageThiefRanking = ranking
                self.cleanupHistoryStorageThiefReadAt = Date()
                self.cleanupHistoryStorageThiefState = .loaded
            case let .failure(error):
                self.cleanupHistoryStorageThiefState = .failed(
                    (error as? CleanupHistoryServiceError) ?? .invalidResponse
                )
            case nil:
                break
            }
            switch clearResult {
            case let .success(result):
                self.cleanupHistoryClearState = .completed(result)
            case let .failure(error):
                let failure =
                    (error as? CleanupHistoryClearServiceError)
                        ?? .invalidResponse
                self.cleanupHistoryClearState =
                    failure == .outcomeUnknown
                        ? .outcomeUnknown
                        : .failed(failure)
            }
        }
        cleanupHistoryClearTask = task
        await task.value
    }

    func cancelCleanupHistoryClear(
        _ confirmation: CleanupHistoryClearConfirmation
    ) async {
        guard
            cleanupHistoryClearTask == nil,
            confirmation == cleanupHistoryClearConfirmation,
            let preview = pendingCleanupHistoryClearPreview,
            case .awaitingConfirmation = cleanupHistoryClearState
        else {
            return
        }
        cleanupHistoryClearGeneration &+= 1
        pendingCleanupHistoryClearPreview = nil
        cleanupHistoryClearConfirmation = nil
        cleanupHistoryClearState = .idle
        await preview.release()
    }

    func dismissCleanupHistoryClearNotice() {
        guard
            cleanupHistoryClearTask == nil,
            pendingCleanupHistoryClearPreview == nil,
            !cleanupHistoryClearState.isBusy
        else {
            return
        }
        cleanupHistoryClearState = .idle
    }

    /// Settings disappearance cancels only preparation/confirmation work.
    /// Once clearing begins, it is allowed to finish and remains one-shot.
    func dismissCleanupHistoryClearPresentation() async {
        if case .clearing = cleanupHistoryClearState {
            return
        }
        cleanupHistoryClearGeneration &+= 1
        let generation = cleanupHistoryClearGeneration
        let operation = cleanupHistoryClearTask
        let preview = pendingCleanupHistoryClearPreview
        pendingCleanupHistoryClearPreview = nil
        cleanupHistoryClearConfirmation = nil
        operation?.cancel()
        await operation?.value
        await preview?.release()
        guard generation == cleanupHistoryClearGeneration else {
            return
        }
        cleanupHistoryClearTask = nil
        cleanupHistoryClearState = .idle
    }

    /// Suppress late presentation but await a confirmed clear before engine
    /// close. Task cancellation never implies transaction cancellation.
    func shutdownCleanupHistoryClear() async {
        cleanupHistoryClearIsShuttingDown = true
        cleanupHistoryClearGeneration &+= 1
        let operation = cleanupHistoryClearTask
        let preview = pendingCleanupHistoryClearPreview
        pendingCleanupHistoryClearPreview = nil
        cleanupHistoryClearConfirmation = nil
        operation?.cancel()
        await operation?.value
        await preview?.release()
        cleanupHistoryClearTask = nil
        cleanupHistoryClearState = .idle
    }

    /// Selects one exact path-free history record. The ID must still be
    /// present in the current summary feed and is only an observation key.
    func selectCleanupHistorySession(_ sessionID: String) async {
        guard cleanupHistoryRecords.contains(where: { $0.sessionID == sessionID }) else {
            closeCleanupHistorySession()
            return
        }
        if selectedCleanupHistorySessionID == sessionID {
            if case let .loaded(detail) = cleanupHistoryDetailState {
                await loadCleanupHistoryRuleOutcomes(
                    sessionID: sessionID,
                    detail: detail
                )
                return
            }
            if let cleanupHistoryDetailTask {
                await cleanupHistoryDetailTask.value
                return
            }
        }
        await loadCleanupHistorySession(sessionID)
    }

    func retryCleanupHistorySession() async {
        guard let sessionID = selectedCleanupHistorySessionID,
              cleanupHistoryRecords.contains(where: { $0.sessionID == sessionID })
        else {
            closeCleanupHistorySession()
            return
        }
        await loadCleanupHistorySession(sessionID)
    }

    func retryCleanupHistoryRuleOutcomes() async {
        guard
            let sessionID = selectedCleanupHistorySessionID,
            case let .loaded(detail) = cleanupHistoryDetailState,
            detail.summary.sessionID == sessionID
        else {
            return
        }
        await loadCleanupHistoryRuleOutcomes(sessionID: sessionID, detail: detail)
    }

    func closeCleanupHistorySession() {
        cleanupHistoryDetailGeneration &+= 1
        cleanupHistoryDetailTask?.cancel()
        cleanupHistoryDetailTask = nil
        fenceCleanupHistoryRuleOutcomes()
        selectedCleanupHistorySessionID = nil
        cleanupHistoryDetailState = .idle
    }

    func refreshLoginItemState() async {
        if let loginItemTask {
            await loginItemTask.value
            return
        }

        loginItemGeneration &+= 1
        let generation = loginItemGeneration
        loginItemState = LoginItemState(
            status: loginItemState.status,
            activity: .loading,
            failure: nil
        )
        let service = loginItemService
        let task = Task { @MainActor [weak self] in
            let status = await service.status()
            guard !Task.isCancelled, let self,
                  generation == self.loginItemGeneration
            else {
                return
            }
            self.loginItemState = LoginItemState(
                status: status,
                activity: nil,
                failure: nil
            )
            self.loginItemTask = nil
        }
        loginItemTask = task
        await task.value
    }

    func setLaunchAtLogin(_ registrationRequested: Bool) async {
        if let loginItemTask {
            await loginItemTask.value
            return
        }
        guard let status = loginItemState.status,
              status != .notFound,
              status != .unknown,
              status.registrationRequested != registrationRequested
        else {
            return
        }

        loginItemGeneration &+= 1
        let generation = loginItemGeneration
        loginItemState = LoginItemState(
            status: status,
            activity: registrationRequested ? .registering : .unregistering,
            failure: nil
        )
        let service = loginItemService
        let task = Task { @MainActor [weak self] in
            let operationFailure: LoginItemFailureReason?
            do {
                if registrationRequested {
                    try await service.register()
                } else {
                    try await service.unregister()
                }
                operationFailure = nil
            } catch {
                operationFailure = Self.loginItemFailureReason(for: error)
            }

            let confirmedStatus = await service.status()
            let reachedRequestedState = registrationRequested
                ? confirmedStatus.registrationRequested
                : confirmedStatus == .notRegistered
            let failure: LoginItemFailure?
            if reachedRequestedState {
                failure = nil
            } else {
                let reason = operationFailure ?? .outcomeUnknown
                failure = registrationRequested
                    ? .registration(reason)
                    : .unregistration(reason)
            }

            guard !Task.isCancelled, let self,
                  generation == self.loginItemGeneration
            else {
                return
            }
            self.loginItemState = LoginItemState(
                status: confirmedStatus,
                activity: nil,
                failure: failure
            )
            self.loginItemTask = nil
        }
        loginItemTask = task
        await task.value
    }

    func refreshNotificationAuthorizationState() async {
        if let notificationAuthorizationTask {
            await notificationAuthorizationTask.value
            return
        }

        notificationAuthorizationGeneration &+= 1
        let generation = notificationAuthorizationGeneration
        notificationAuthorizationState = NotificationAuthorizationState(
            status: notificationAuthorizationState.status,
            activity: .loading,
            failure: nil
        )
        let service = notificationService
        let task = Task { @MainActor [weak self] in
            let status = await service.authorizationStatus()
            guard !Task.isCancelled, let self,
                  generation == self.notificationAuthorizationGeneration
            else {
                return
            }
            self.notificationAuthorizationState = NotificationAuthorizationState(
                status: status,
                activity: nil,
                failure: nil
            )
            self.notificationAuthorizationTask = nil
        }
        notificationAuthorizationTask = task
        await task.value
    }

    func requestNotificationAuthorization() async {
        if let notificationAuthorizationTask {
            await notificationAuthorizationTask.value
            return
        }
        guard notificationAuthorizationState.status == .notDetermined else {
            return
        }

        notificationAuthorizationGeneration &+= 1
        let generation = notificationAuthorizationGeneration
        notificationAuthorizationState = NotificationAuthorizationState(
            status: .notDetermined,
            activity: .requesting,
            failure: nil
        )
        let service = notificationService
        let task = Task { @MainActor [weak self] in
            let operationFailure: NotificationAuthorizationFailureReason?
            do {
                try await service.requestAuthorization()
                operationFailure = nil
            } catch {
                operationFailure = Self.notificationAuthorizationFailure(for: error)
            }

            let confirmedStatus = await service.authorizationStatus()
            let failure = confirmedStatus.isSettled
                ? nil
                : operationFailure ?? .outcomeUnknown
            guard !Task.isCancelled, let self,
                  generation == self.notificationAuthorizationGeneration
            else {
                return
            }
            self.notificationAuthorizationState = NotificationAuthorizationState(
                status: confirmedStatus,
                activity: nil,
                failure: failure
            )
            self.notificationAuthorizationTask = nil
        }
        notificationAuthorizationTask = task
        await task.value
    }

    func setMenuBarVisibilityMode(_ mode: MenuBarVisibilityMode) {
        let preference = menuBarVisibilityPreference.changing(mode: mode)
        guard preference != menuBarVisibilityPreference else {
            return
        }
        menuBarVisibilityPreference = preference
        menuBarVisibilityPreferenceStore.save(preference)
        updateMenuBarVisibility()
    }

    func setMenuBarVisibilityThresholdPercent(_ thresholdPercent: Int) {
        guard let preference = menuBarVisibilityPreference.changing(
            thresholdPercent: thresholdPercent
        ), preference != menuBarVisibilityPreference else {
            return
        }
        menuBarVisibilityPreference = preference
        menuBarVisibilityPreferenceStore.save(preference)
        updateMenuBarVisibility()
    }

    func revealMenuBarItemForSession() {
        menuBarRevealOverride = true
        isMenuBarItemInserted = true
    }

    func acknowledgeStorageAccessIntroduction() {
        guard showsStorageAccessIntroduction else {
            return
        }
        showsStorageAccessIntroduction = false
        storageAccessIntroductionPreferenceStore.saveAcknowledged()
    }

    func requestBroaderStorageAnalysis() async {
        guard !storageAccessProbeIsInvalidated else {
            return
        }
        broaderStorageAnalysisRequested = true
        await refreshStorageAccessEvidence()
    }

    func refreshStorageAccessEvidence() async {
        guard broaderStorageAnalysisRequested, !storageAccessProbeIsInvalidated else {
            return
        }
        if let storageAccessProbeTask {
            await storageAccessProbeTask.value
            return
        }

        storageAccessProbeGeneration &+= 1
        let generation = storageAccessProbeGeneration
        let previous = storageAccessProbeState.evidence
        storageAccessProbeState = .checking(previous: previous)
        let probe = storageAccessProbe
        let task = Task { @MainActor [weak self] in
            let result: Result<StorageAccessEvidence, Error>
            do {
                result = try .success(await probe.probe())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.storageAccessProbeGeneration
            else {
                return
            }
            switch result {
            case let .success(evidence):
                self.storageAccessProbeState = .observed(evidence)
            case .failure:
                self.storageAccessProbeState = .failed(previous: previous)
            }
            self.storageAccessProbeTask = nil
        }
        storageAccessProbeTask = task
        await task.value
    }

    func armStorageAccessSettingsReturnProbe() {
        guard broaderStorageAnalysisRequested, !storageAccessProbeIsInvalidated else {
            return
        }
        storageAccessReturnProbeArmed = true
    }

    func refreshStorageAccessEvidenceAfterActivation() async {
        guard storageAccessReturnProbeArmed else {
            return
        }
        storageAccessReturnProbeArmed = false
        await refreshStorageAccessEvidence()
    }

    func invalidateStorageAccessProbeOperations() {
        storageAccessProbeIsInvalidated = true
        storageAccessProbeGeneration &+= 1
        storageAccessProbeTask?.cancel()
        storageAccessProbeTask = nil
        storageAccessReturnProbeArmed = false
        storageAccessProbeState = storageAccessProbeState.evidence.map {
            .observed($0)
        } ?? .idle
    }

    func reconcileTargetedReclaimScan(for snapshot: VolumeCapacitySnapshot) async {
        guard
            !targetedReclaimScanIsInvalidated,
            !targetedReclaimScanShutdownInProgress
        else {
            return
        }
        guard
            let stableVolumeID = snapshot.stableVolumeID,
            snapshot.pressure == .warning || snapshot.pressure == .critical
        else {
            if targetedReclaimScanState.isActive {
                await cancelTargetedReclaimScan(preservingCompleted: true)
            }
            targetedReclaimScanState = .idle
            return
        }
        let expectedPressure: TargetedReclaimPressure =
            snapshot.pressure == .critical ? .critical : .warning
        if scanState.phase.isActive {
            if targetedReclaimScanState.isActive {
                await cancelTargetedReclaimScan(preservingCompleted: true)
            }
            targetedReclaimScanState = .deferred(
                .userScanActive,
                previous: targetedReclaimScanState.batch
            )
            return
        }
        if let targetedReclaimScanDriverTask {
            if targetedReclaimScanVolumeID == stableVolumeID {
                await targetedReclaimScanDriverTask.value
            } else {
                await cancelTargetedReclaimScan(preservingCompleted: true)
            }
            guard
                !targetedReclaimScanIsInvalidated,
                !scanState.phase.isActive
            else {
                return
            }
        }

        targetedReclaimScanGeneration &+= 1
        let generation = targetedReclaimScanGeneration
        targetedReclaimScanCancellationRequested = false
        targetedReclaimScanVolumeID = stableVolumeID
        let previous = targetedReclaimScanState.batch
        targetedReclaimScanState = .checking(previous: previous)
        let service = targetedReclaimScanService
        let clock = homeScanClock
        let anchorAt = snapshot.sampledAt
        let driver = Task { @MainActor [weak self] in
            guard let self else {
                return
            }
            await self.runTargetedReclaimScan(
                service: service,
                clock: clock,
                stableVolumeID: stableVolumeID,
                anchorAt: anchorAt,
                expectedPressure: expectedPressure,
                generation: generation,
                previous: previous
            )
        }
        targetedReclaimScanDriverTask = driver
        await driver.value
    }

    func cancelTargetedReclaimScan() async {
        await cancelTargetedReclaimScan(preservingCompleted: true)
    }

    func shutdownTargetedReclaimScan() async {
        guard
            !targetedReclaimScanIsInvalidated,
            !targetedReclaimScanShutdownInProgress
        else {
            if let targetedReclaimScanDriverTask {
                await targetedReclaimScanDriverTask.value
            }
            return
        }
        targetedReclaimScanShutdownInProgress = true
        await cancelTargetedReclaimScan(preservingCompleted: false)
        targetedReclaimScanIsInvalidated = true
        targetedReclaimScanShutdownInProgress = false
        targetedReclaimScanState = .idle
    }

    func startHomeScan() async {
        await cancelTargetedReclaimScan(preservingCompleted: true)
        let service = homeScanService
        _ = await startScan(request: .home, scope: .home) {
            try await service.startHomeScan()
        }
    }

    func startSubtreeScan(
        sourceScanID: String,
        nodeID: UInt64,
        displayName: String,
        using service: any DuxSnapshotSubtreeScanServing
    ) async -> AppScanRunOutcome {
        await cancelTargetedReclaimScan(preservingCompleted: true)
        return await startScan(
            request: .subtree(sourceScanID: sourceScanID, nodeID: nodeID),
            scope: .subtree(displayName: displayName)
        ) {
            try await service.startSubtreeScan(
                sourceScanID: sourceScanID,
                nodeID: nodeID
            )
        }
    }

    private func startScan(
        request: ScanRequestKey,
        scope: AppScanScope,
        start: @escaping @Sendable () async throws -> HomeScanStartDisposition
    ) async -> AppScanRunOutcome {
        guard !homeScanIsInvalidated else {
            return .superseded
        }
        if let homeScanDriverTask {
            guard activeScanRequest == request else {
                return .failed(.busy)
            }
            return await homeScanDriverTask.value
        }

        homeScanGeneration &+= 1
        let generation = homeScanGeneration
        homeScanCancellationRequested = false
        latestHomeScanProgress = nil
        latestHomeScanEvents = []
        activeScanRequest = request
        scanState.scope = scope
        scanState.phase = .queued

        let clock = homeScanClock
        let driver = Task { @MainActor [weak self] in
            guard let self else {
                return AppScanRunOutcome.superseded
            }
            do {
                let started = try await start()
                let task = started.task
                guard self.isCurrentHomeScan(generation) else {
                    _ = try? await task.requestCancellation()
                    return .superseded
                }
                self.activeHomeScanTask = task
                if self.homeScanCancellationRequested {
                    _ = try? await task.requestCancellation()
                }

                while self.isCurrentHomeScan(generation) {
                    let poll = try await task.poll()
                    guard self.isCurrentHomeScan(generation) else {
                        _ = try? await task.requestCancellation()
                        return .superseded
                    }
                    if let outcome = self.publishHomeScanPoll(poll, scope: scope) {
                        self.finishHomeScanDriver(generation: generation)
                        return outcome
                    }
                    try await clock.sleepUntilNextPoll()
                }
                _ = try? await task.requestCancellation()
                return .superseded
            } catch is CancellationError {
                return .superseded
            } catch {
                guard self.isCurrentHomeScan(generation) else {
                    return .superseded
                }
                if let task = self.activeHomeScanTask {
                    _ = try? await task.requestCancellation()
                }
                guard self.isCurrentHomeScan(generation) else {
                    return .superseded
                }
                let failure = Self.homeScanFailure(for: error)
                self.scanState.phase = .failed(failure)
                self.finishHomeScanDriver(generation: generation)
                return .failed(failure)
            }
        }
        homeScanDriverTask = driver
        return await driver.value
    }

    func cancelHomeScan() async {
        guard
            !homeScanIsInvalidated,
            scanState.phase.isActive,
            !homeScanCancellationRequested
        else {
            return
        }
        homeScanCancellationRequested = true
        scanState.phase = .cancellationRequested(latestHomeScanProgress)
        guard let activeHomeScanTask else {
            return
        }
        _ = try? await activeHomeScanTask.requestCancellation()
    }

    func shutdownHomeScan() async {
        guard !homeScanIsInvalidated else {
            if let homeScanDriverTask {
                _ = await homeScanDriverTask.value
            }
            return
        }
        homeScanIsInvalidated = true
        homeScanCancellationRequested = true
        homeScanGeneration &+= 1

        let task = activeHomeScanTask
        let driver = homeScanDriverTask
        activeHomeScanTask = nil
        homeScanDriverTask = nil
        activeScanRequest = nil
        if let task {
            _ = try? await task.requestCancellation()
        }
        driver?.cancel()
        _ = await driver?.value
    }

    private func publishVolumeRefresh(
        _ result: Result<VolumeCapacitySnapshot, Error>,
        generation: UInt64
    ) {
        guard generation == volumeRefreshGeneration else {
            return
        }
        volumeRefreshTask = nil
        switch result {
        case let .success(snapshot):
            if volumeState.snapshot?.stableVolumeID != snapshot.stableVolumeID {
                capacityTrendGeneration &+= 1
                capacityTrendTask?.cancel()
                capacityTrendTask = nil
                capacityTrendRequestVolumeID = nil
                capacityTrendRequestAnchorAt = nil
                capacityTrend = nil
                capacityTrendLoadedForAnchorAt = nil
                pressureHistoryGeneration &+= 1
                pressureHistoryTask?.cancel()
                pressureHistoryTask = nil
                pressureHistoryRequestVolumeID = nil
                pressureHistoryRequestAnchorAt = nil
                pressureHistoryState = .idle
            }
            volumeState = .loaded(snapshot)
            Task { @MainActor [weak self] in
                await self?.deliverPressureNotificationIfNeeded(snapshot)
            }
            Task { @MainActor [weak self] in
                await self?.loadCapacityTrend()
            }
            Task { @MainActor [weak self] in
                await self?.loadPressureHistory()
            }
            Task { @MainActor [weak self] in
                await self?.reconcileTargetedReclaimScan(for: snapshot)
            }
        case let .failure(error):
            if error is CancellationError {
                volumeState = volumeStateBeforeRefresh
            } else if let snapshot = volumeStateBeforeRefresh.snapshot {
                volumeState = .stale(snapshot, Self.volumeFailure(for: error))
            } else {
                volumeState = .failed(Self.volumeFailure(for: error))
            }
        }
    }

    private func runTargetedReclaimScan(
        service: any DuxTargetedReclaimScanServing,
        clock: any HomeScanPollingClock,
        stableVolumeID: String,
        anchorAt: Date,
        expectedPressure: TargetedReclaimPressure,
        generation: UInt64,
        previous: TargetedReclaimScanBatch?
    ) async {
        var ordinal: UInt16 = 0
        var expectedRootsRevision: UInt64?
        var expectedRootCatalogDigestSHA256: Data?
        var context: TargetedReclaimScanContext?
        var completed: [TargetedReclaimRootResult] = []
        var failed: [TargetedReclaimFailedRoot] = []

        defer {
            if generation == targetedReclaimScanGeneration {
                activeTargetedReclaimScanTask = nil
                targetedReclaimScanDriverTask = nil
                targetedReclaimScanVolumeID = nil
            }
        }

        while isCurrentTargetedReclaimScan(generation) {
            let admission: TargetedReclaimScanAdmission
            do {
                admission = try await service.startTargetedReclaimScan(
                    stableVolumeID: stableVolumeID,
                    anchorAt: anchorAt,
                    ordinal: ordinal,
                    expectedRootsRevision: expectedRootsRevision,
                    expectedRootCatalogDigestSHA256: expectedRootCatalogDigestSHA256
                )
            } catch {
                guard isCurrentTargetedReclaimScan(generation) else {
                    return
                }
                publishTargetedReclaimStartFailure(
                    error,
                    previous: batch(
                        context: context,
                        completed: completed,
                        failed: failed
                    ) ?? previous
                )
                return
            }
            guard isCurrentTargetedReclaimScan(generation) else {
                if let task = admission.disposition.ownedTask {
                    _ = try? await task.requestCancellation()
                }
                return
            }

            switch admission.disposition {
            case .pressureNotActive:
                guard admission.context == nil, admission.ordinal == nil, admission.root == nil else {
                    targetedReclaimScanState = .failed(.invalidResponse, previous: previous)
                    return
                }
                targetedReclaimScanState = previous.map(TargetedReclaimScanState.completed) ?? .idle
                return
            case .noEligibleRoots:
                guard admission.ordinal == nil, admission.root == nil else {
                    targetedReclaimScanState = .failed(.invalidResponse, previous: previous)
                    return
                }
                targetedReclaimScanState = .noEligibleRoots
                return
            case .unavailable, .current, .started, .observing:
                break
            }

            guard
                let admittedContext = admission.context,
                let admittedOrdinal = admission.ordinal,
                let root = admission.root,
                admittedOrdinal == ordinal,
                root.ordinal == admittedOrdinal,
                root.kind == (
                    admittedContext.knownUserLibraryCachesIncluded && admittedOrdinal == 0
                        ? .knownUserLibraryCaches
                        : .configuredProject
                ),
                admittedContext.stableVolumeID == stableVolumeID,
                admittedContext.capacityAnchorAt == anchorAt,
                admittedContext.pressure == expectedPressure,
                admittedContext.rootCount > 0,
                admittedOrdinal < admittedContext.rootCount,
                admittedContext.pressureEpisodeStartedAt <= anchorAt,
                admittedContext.lowPressureSequenceStartedAt
                <= admittedContext.pressureEpisodeStartedAt,
                expectedRootsRevision.map({ $0 == admittedContext.rootsRevision }) ?? true,
                context.map({ $0.identity == admittedContext.identity }) ?? true
            else {
                if let task = admission.disposition.ownedTask {
                    _ = try? await task.requestCancellation()
                }
                targetedReclaimScanState = .failed(
                    .invalidResponse,
                    previous: batch(context: context, completed: completed, failed: failed)
                        ?? previous
                )
                return
            }
            context = admittedContext
            expectedRootsRevision = admittedContext.rootsRevision
            expectedRootCatalogDigestSHA256 = admittedContext.rootCatalogDigestSHA256

            switch admission.disposition {
            case let .unavailable(failure):
                failed.append(
                    TargetedReclaimFailedRoot(
                        ordinal: ordinal,
                        root: root,
                        failure: failure
                    )
                )
            case let .current(result):
                guard validTargetedResult(result, context: admittedContext) else {
                    targetedReclaimScanState = .failed(
                        .invalidResponse,
                        previous: batch(
                            context: context,
                            completed: completed,
                            failed: failed
                        ) ?? previous
                    )
                    return
                }
                completed.append(
                    TargetedReclaimRootResult(
                        ordinal: ordinal,
                        root: root,
                        source: .currentDurable,
                        result: result
                    )
                )
                durableScanObservationDidChange()
            case let .started(task), let .observing(task):
                let ownsTask: Bool = switch admission.disposition {
                case .started: true
                case .observing: false
                case .pressureNotActive, .noEligibleRoots, .unavailable, .current:
                    preconditionFailure("unreachable targeted scan disposition")
                }
                if ownsTask {
                    activeTargetedReclaimScanTask = task
                }
                if targetedReclaimScanCancellationRequested, ownsTask {
                    _ = try? await task.requestCancellation()
                }
                let source: TargetedReclaimScanResultSource = switch admission.disposition {
                case .started: .focusedRun
                case .observing: .joinedActive
                case .pressureNotActive, .noEligibleRoots, .unavailable, .current:
                    preconditionFailure("unreachable targeted scan disposition")
                }
                let terminal = await observeTargetedReclaimTask(
                    task,
                    clock: clock,
                    context: admittedContext,
                    ordinal: ordinal,
                    completed: completed,
                    failed: failed,
                    generation: generation,
                    ownsTask: ownsTask
                )
                if ownsTask {
                    activeTargetedReclaimScanTask = nil
                }
                guard isCurrentTargetedReclaimScan(generation) else {
                    return
                }
                switch terminal {
                case let .succeeded(result):
                    completed.append(
                        TargetedReclaimRootResult(
                            ordinal: ordinal,
                            root: root,
                            source: source,
                            result: result
                        )
                    )
                    durableScanObservationDidChange()
                case let .failed(failure):
                    failed.append(
                        TargetedReclaimFailedRoot(
                            ordinal: ordinal,
                            root: root,
                            failure: failure
                        )
                    )
                case .cancelled:
                    targetedReclaimScanState = .cancelled(
                        previous: batch(
                            context: context,
                            completed: completed,
                            failed: failed
                        ) ?? previous
                    )
                    return
                case .superseded:
                    return
                }
            case .pressureNotActive, .noEligibleRoots:
                preconditionFailure("handled before targeted root validation")
            }

            if targetedReclaimScanCancellationRequested {
                targetedReclaimScanState = .cancelled(
                    previous: batch(
                        context: context,
                        completed: completed,
                        failed: failed
                    ) ?? previous
                )
                return
            }

            guard let context else {
                targetedReclaimScanState = .failed(.invalidResponse, previous: previous)
                return
            }
            if ordinal + 1 >= context.rootCount {
                let partial = TargetedReclaimScanBatch(
                    context: context,
                    completed: completed,
                    failed: failed
                )
                let emergencyRecovery: AppEmergencyRecoveryOrdering?
                do {
                    if expectedPressure == .critical {
                        let ordering = try await service.finalizeEmergencyRecovery(context)
                        let supportedPolicyRevision =
                            AppEmergencyRecoveryOrdering.supportedPolicyRevision
                        guard
                            ordering.policyRevision == supportedPolicyRevision,
                            ordering.context == context
                        else {
                            targetedReclaimScanState = .failed(
                                .invalidResponse,
                                previous: partial
                            )
                            return
                        }
                        emergencyRecovery = ordering
                    } else {
                        let checkpoint = try await service.validateTargetedReclaimScan(context)
                        guard checkpoint == context else {
                            targetedReclaimScanState = .failed(
                                .invalidResponse,
                                previous: partial
                            )
                            return
                        }
                        emergencyRecovery = nil
                    }
                    guard isCurrentTargetedReclaimScan(generation) else {
                        return
                    }
                } catch {
                    guard isCurrentTargetedReclaimScan(generation) else {
                        return
                    }
                    publishTargetedReclaimStartFailure(error, previous: partial)
                    return
                }
                targetedReclaimScanState = .completed(
                    TargetedReclaimScanBatch(
                        context: context,
                        completed: completed,
                        failed: failed,
                        emergencyRecovery: emergencyRecovery
                    )
                )
                return
            }
            ordinal += 1
            targetedReclaimScanState = .scanning(
                TargetedReclaimScanProgress(
                    context: context,
                    completed: completed,
                    failed: failed,
                    activeOrdinal: nil,
                    activeProgress: nil
                )
            )
        }
    }

    private enum TargetedReclaimTaskTerminal {
        case succeeded(HomeScanTaskResult)
        case failed(TargetedReclaimRootFailure)
        case cancelled
        case superseded
    }

    private func observeTargetedReclaimTask(
        _ task: any HomeScanTask,
        clock: any HomeScanPollingClock,
        context: TargetedReclaimScanContext,
        ordinal: UInt16,
        completed: [TargetedReclaimRootResult],
        failed: [TargetedReclaimFailedRoot],
        generation: UInt64,
        ownsTask: Bool
    ) async -> TargetedReclaimTaskTerminal {
        while isCurrentTargetedReclaimScan(generation) {
            do {
                let poll = try await task.poll()
                guard isCurrentTargetedReclaimScan(generation) else {
                    if ownsTask {
                        _ = try? await task.requestCancellation()
                    }
                    return .superseded
                }
                targetedReclaimScanState = .scanning(
                    TargetedReclaimScanProgress(
                        context: context,
                        completed: completed,
                        failed: failed,
                        activeOrdinal: ordinal,
                        activeProgress: poll.progress
                    )
                )
                switch poll.phase {
                case .queued, .running:
                    try await clock.sleepUntilNextPoll()
                case .succeeded:
                    guard let result = poll.result,
                          poll.failure == nil,
                          validTargetedResult(result, context: context)
                    else {
                        return .failed(.invalidResponse)
                    }
                    return .succeeded(result)
                case .failed:
                    guard let failure = poll.failure else {
                        return .failed(.invalidResponse)
                    }
                    return .failed(.scan(failure))
                case .cancelled:
                    return .cancelled
                }
            } catch is CancellationError {
                if ownsTask {
                    _ = try? await task.requestCancellation()
                }
                return .superseded
            } catch let error as HomeScanServiceError {
                return .failed(Self.targetedRootFailure(for: error))
            } catch {
                return .failed(.unexpected)
            }
        }
        if ownsTask {
            _ = try? await task.requestCancellation()
        }
        return .superseded
    }

    private func cancelTargetedReclaimScan(preservingCompleted: Bool) async {
        let previous = targetedReclaimScanState.batch
        guard let driver = targetedReclaimScanDriverTask else {
            if !preservingCompleted {
                targetedReclaimScanState = .idle
            }
            return
        }
        targetedReclaimScanCancellationRequested = true
        if let activeTargetedReclaimScanTask {
            _ = try? await activeTargetedReclaimScanTask.requestCancellation()
            // Keep polling the owned task until Rust publishes a terminal
            // outcome and releases its scope. Foreground scans must never
            // infer quiescence from cancellation intent alone.
            _ = await driver.value
        } else {
            // Checking and non-owning observation carry no task authority.
            // Detach locally without cancelling someone else's work.
            targetedReclaimScanGeneration &+= 1
            driver.cancel()
            _ = await driver.value
        }
        targetedReclaimScanGeneration &+= 1
        activeTargetedReclaimScanTask = nil
        targetedReclaimScanDriverTask = nil
        targetedReclaimScanVolumeID = nil
        targetedReclaimScanState = if preservingCompleted {
            .cancelled(previous: previous)
        } else {
            .idle
        }
    }

    private func isCurrentTargetedReclaimScan(_ generation: UInt64) -> Bool {
        !targetedReclaimScanIsInvalidated
            && generation == targetedReclaimScanGeneration
            && !Task.isCancelled
    }

    private func batch(
        context: TargetedReclaimScanContext?,
        completed: [TargetedReclaimRootResult],
        failed: [TargetedReclaimFailedRoot]
    ) -> TargetedReclaimScanBatch? {
        context.map {
            TargetedReclaimScanBatch(context: $0, completed: completed, failed: failed)
        }
    }

    private func validTargetedResult(
        _ result: HomeScanTaskResult,
        context: TargetedReclaimScanContext
    ) -> Bool {
        !result.scanID.isEmpty
            && result.startedAt >= context.pressureEpisodeStartedAt
            && result.completedAt >= result.startedAt
            && result.succeeded
            && result.snapshotAvailable
    }

    private func publishTargetedReclaimStartFailure(
        _ error: Error,
        previous: TargetedReclaimScanBatch?
    ) {
        if let error = error as? TargetedReclaimScanServiceError {
            switch error {
            case .busy:
                targetedReclaimScanState = .deferred(
                    .overlappingExternalScan,
                    previous: previous
                )
            case .queueFull:
                targetedReclaimScanState = .deferred(.queueFull, previous: previous)
            case .outcomeUnknown:
                targetedReclaimScanState = .deferred(.storageBusy, previous: previous)
            default:
                targetedReclaimScanState = .failed(
                    Self.targetedFailure(for: error),
                    previous: previous
                )
            }
        } else if error is CancellationError {
            targetedReclaimScanState = .cancelled(previous: previous)
        } else {
            targetedReclaimScanState = .failed(.unexpected, previous: previous)
        }
    }

    private func deliverPressureNotificationIfNeeded(
        _ snapshot: VolumeCapacitySnapshot
    ) async {
        if let diskPressureNotificationTask {
            await diskPressureNotificationTask.value
            return
        }
        let service = notificationService
        let cooldownStore = diskPressureNotificationCooldownStore
        let task = Task { @MainActor [weak self] in
            defer {
                self?.diskPressureNotificationTask = nil
            }
            guard let volumeID = snapshot.stableVolumeID,
                  let urgency = Self.notificationUrgency(for: snapshot),
                  let candidate = DiskPressureNotificationGate.candidate(
                      for: snapshot,
                      lastAcceptedAtForUrgency: cooldownStore.lastAcceptedAt(
                          volumeID: volumeID,
                          urgency: urgency
                      )
                  )
            else {
                return
            }
            do {
                try await service.deliverDiskPressure(candidate.delivery)
                cooldownStore.recordAccepted(
                    at: candidate.sampledAt,
                    volumeID: volumeID,
                    urgency: urgency
                )
            } catch {
                // Delivery failure must not alter authoritative capacity state
                // or consume the cooldown window.
            }
        }
        diskPressureNotificationTask = task
        await task.value
    }

    private static func notificationUrgency(
        for snapshot: VolumeCapacitySnapshot
    ) -> DiskPressureNotificationUrgency? {
        switch snapshot.pressure {
        case .warning: .warning
        case .critical: .critical
        case .healthy, .unknown: nil
        }
    }

    private func publishPressurePolicyLoad(
        _ result: Result<DiskPressurePolicy, Error>,
        generation: UInt64
    ) {
        guard generation == pressurePolicyGeneration else {
            return
        }
        pressurePolicyTask = nil
        switch result {
        case let .success(policy):
            diskPressurePolicy = policy
            diskPressurePolicyDraft = DiskPressurePolicyDraft(
                configuration: policy.configuration
            )
            diskPressurePolicyState = .ready
        case let .failure(error):
            if error is CancellationError {
                diskPressurePolicyState = diskPressurePolicy == nil ? .idle : .ready
            } else {
                diskPressurePolicyState = .failed(Self.pressurePolicyFailure(for: error))
            }
        }
    }

    private func mutateDiskPressurePolicy(
        state: DiskPressurePolicyState,
        _ operation: @escaping @Sendable (
            any DuxPressurePolicyServing
        ) async throws -> DiskPressurePolicyUpdateResult
    ) async {
        pressurePolicyGeneration &+= 1
        let generation = pressurePolicyGeneration
        precondition(state == .saving || state == .resetting)
        diskPressurePolicyState = state
        let service = engineService
        let requester = capacityResampleRequester
        let task = Task { @MainActor [weak self] in
            let result: Result<DiskPressurePolicyUpdateResult, Error>
            do {
                result = try .success(await operation(service))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.pressurePolicyGeneration
            else {
                return
            }
            switch result {
            case let .success(update):
                self.diskPressurePolicy = update.policy
                self.diskPressurePolicyDraft = DiskPressurePolicyDraft(
                    configuration: update.policy.configuration
                )
                if update.changed {
                    await requester.requestCapacityResample()
                }
                guard !Task.isCancelled,
                      generation == self.pressurePolicyGeneration
                else {
                    return
                }
                self.diskPressurePolicyState = .ready
            case let .failure(error):
                if error is CancellationError {
                    self.diskPressurePolicyState = self.diskPressurePolicy == nil ? .idle : .ready
                } else {
                    self.diskPressurePolicyState = .failed(
                        Self.pressurePolicyFailure(for: error)
                    )
                }
            }
            self.pressurePolicyTask = nil
        }
        pressurePolicyTask = task
        await task.value
    }

    private func publishPermanentCleanupPolicyLoad(
        _ result: Result<PermanentCleanupPolicy, Error>,
        generation: UInt64
    ) {
        guard generation == permanentCleanupPolicyGeneration else {
            return
        }
        permanentCleanupPolicyTask = nil
        switch result {
        case let .success(policy):
            permanentCleanupPolicy = policy
            permanentCleanupPolicyState = .ready
        case let .failure(error):
            if error is CancellationError {
                permanentCleanupPolicyState = permanentCleanupPolicy == nil ? .idle : .ready
            } else {
                permanentCleanupPolicyState = .failed(Self.permanentCleanupFailure(for: error))
            }
        }
    }

    private func mutatePermanentCleanupPolicy(
        state: PermanentCleanupPolicyState,
        _ operation: @escaping @Sendable (
            any DuxPermanentCleanupPolicyServing
        ) async throws -> PermanentCleanupPolicyUpdateResult
    ) async {
        permanentCleanupPolicyGeneration &+= 1
        let generation = permanentCleanupPolicyGeneration
        precondition(state == .enabling || state == .disabling || state == .resetting)
        permanentCleanupPolicyState = state
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<PermanentCleanupPolicyUpdateResult, Error>
            do {
                result = try .success(await operation(service))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.permanentCleanupPolicyGeneration
            else {
                return
            }
            switch result {
            case let .success(update):
                self.permanentCleanupPolicy = update.policy
                self.permanentCleanupPolicyState = .ready
            case let .failure(error):
                if error is CancellationError {
                    self.permanentCleanupPolicyState =
                        self.permanentCleanupPolicy == nil ? .idle : .ready
                } else {
                    self.permanentCleanupPolicyState = .failed(
                        Self.permanentCleanupFailure(for: error)
                    )
                }
            }
            self.permanentCleanupPolicyTask = nil
        }
        permanentCleanupPolicyTask = task
        await task.value
    }

    private static func permanentCleanupFailure(
        for error: Error
    ) -> PermanentCleanupPolicyFailure {
        if let error = error as? PermanentCleanupPolicyServiceError {
            return .service(error)
        }
        return .unexpected
    }

    private func publishCleanupExclusionsLoad(
        _ result: Result<CleanupExclusionsPolicy, Error>,
        generation: UInt64
    ) {
        guard generation == cleanupExclusionsGeneration else {
            return
        }
        cleanupExclusionsTask = nil
        switch result {
        case let .success(exclusions):
            cleanupExclusions = exclusions
            cleanupExclusionsState = .ready
        case let .failure(error):
            if error is CancellationError {
                cleanupExclusionsState = cleanupExclusions == nil ? .idle : .ready
            } else {
                cleanupExclusionsState = .failed(Self.cleanupExclusionsFailure(for: error))
            }
        }
    }

    private func publishProjectDiscoveryRootsLoad(
        _ result: Result<ProjectDiscoveryRoots, Error>,
        generation: UInt64
    ) {
        guard
            generation == projectDiscoveryRootsGeneration,
            !projectDiscoveryRootsIsInvalidated
        else {
            return
        }
        projectDiscoveryRootsTask = nil
        switch result {
        case let .success(roots):
            projectDiscoveryRoots = roots
            projectDiscoveryRootsRequiresAuthoritativeReload = false
            projectDiscoveryRootsState = .ready
        case let .failure(error):
            if error is CancellationError {
                projectDiscoveryRootsState =
                    projectDiscoveryRoots == nil ? .idle : .ready
            } else {
                projectDiscoveryRootsState = .failed(
                    Self.projectDiscoveryRootsFailure(for: error)
                )
            }
        }
    }

    private func publishDirectCargoEnrollmentLoad(
        _ result: Result<DirectCargoEnrollmentStatusModel, Error>,
        generation: UInt64
    ) {
        guard
            generation == directCargoEnrollmentGeneration,
            !directCargoEnrollmentIsInvalidated
        else {
            return
        }
        directCargoEnrollmentTask = nil
        switch result {
        case let .success(status):
            directCargoEnrollmentStatus = status
            directCargoEnrollmentRequiresAuthoritativeReload = false
            directCargoEnrollmentState = .ready
        case let .failure(error):
            if error is CancellationError {
                directCargoEnrollmentState =
                    directCargoEnrollmentStatus == nil ? .idle : .ready
            } else if directCargoEnrollmentRequiresAuthoritativeReload {
                directCargoEnrollmentState = .failed(.service(.outcomeUnknown))
            } else {
                directCargoEnrollmentState = .failed(
                    Self.directCargoEnrollmentFailure(for: error)
                )
            }
        }
    }

    /// A native mutation reported uncertainty after it may have reached
    /// durable storage. Observation is safe, but another mutation remains
    /// blocked until one authoritative status read succeeds.
    private func reconcileUncertainDirectCargoMutation(
        service: any EngineServing,
        generation: UInt64
    ) async {
        do {
            let status = try await service.loadDirectCargoEnrollmentStatus()
            guard
                !Task.isCancelled,
                generation == directCargoEnrollmentGeneration,
                !directCargoEnrollmentIsInvalidated
            else {
                return
            }
            directCargoEnrollmentStatus = status
            directCargoEnrollmentRequiresAuthoritativeReload = false
        } catch {
            // Keep the last proven status and the reconciliation gate. The
            // outcomeUnknown warning is published by the calling mutation.
        }
    }

    private func publishCleanupHistory(
        _ result: Result<CleanupHistoryPageModel, Error>,
        appending: Bool,
        generation: UInt64
    ) {
        guard generation == cleanupHistoryGeneration else {
            return
        }
        cleanupHistoryTask = nil
        switch result {
        case let .success(page):
            if appending {
                let existingIDs = Set(cleanupHistoryRecords.map(\.sessionID))
                cleanupHistoryRecords.append(
                    contentsOf: page.records.filter { !existingIDs.contains($0.sessionID) }
                )
            } else {
                cleanupHistoryRecords = page.records
            }
            cleanupHistoryNextCursor = page.nextCursor
            cleanupHistoryState = .loaded
            if !appending,
               let sessionID = selectedCleanupHistorySessionID,
               !cleanupHistoryRecords.contains(where: { $0.sessionID == sessionID })
            {
                closeCleanupHistorySession()
            }
        case let .failure(error):
            if error is CancellationError {
                cleanupHistoryState = cleanupHistoryRecords.isEmpty ? .idle : .loaded
            } else {
                cleanupHistoryState = .failed(
                    (error as? CleanupHistoryServiceError) ?? .invalidResponse
                )
            }
        }
    }

    private func loadCleanupHistorySession(_ sessionID: String) async {
        cleanupHistoryDetailGeneration &+= 1
        let generation = cleanupHistoryDetailGeneration
        cleanupHistoryDetailTask?.cancel()
        fenceCleanupHistoryRuleOutcomes()
        selectedCleanupHistorySessionID = sessionID
        cleanupHistoryDetailState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupHistorySessionDetailModel, Error>
            do {
                result = try .success(
                    await service.loadCleanupHistorySession(sessionID: sessionID)
                )
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupHistoryDetailGeneration,
                  self.selectedCleanupHistorySessionID == sessionID
            else {
                return
            }
            self.cleanupHistoryDetailTask = nil
            switch result {
            case let .success(detail):
                guard detail.summary.sessionID == sessionID,
                      self.cleanupHistoryRecords.contains(
                          where: { $0.sessionID == sessionID }
                      )
                else {
                    self.cleanupHistoryDetailState = .failed(.invalidResponse)
                    return
                }
                self.cleanupHistoryDetailState = .loaded(detail)
            case let .failure(error):
                if error is CancellationError {
                    self.cleanupHistoryDetailState = .idle
                } else {
                    self.cleanupHistoryDetailState = .failed(
                        (error as? CleanupHistoryServiceError) ?? .invalidResponse
                    )
                }
            }
        }
        cleanupHistoryDetailTask = task
        await task.value
        guard
            generation == cleanupHistoryDetailGeneration,
            selectedCleanupHistorySessionID == sessionID,
            case let .loaded(detail) = cleanupHistoryDetailState
        else {
            return
        }
        await loadCleanupHistoryRuleOutcomes(sessionID: sessionID, detail: detail)
    }

    private func loadCleanupHistoryRuleOutcomes(
        sessionID: String,
        detail: CleanupHistorySessionDetailModel
    ) async {
        fenceCleanupHistoryRuleOutcomes()
        guard detail.summary.format == .complete else {
            cleanupHistoryRuleOutcomeState = .unavailableForLegacyRecord
            return
        }

        let generation = cleanupHistoryRuleOutcomeGeneration
        cleanupHistoryRuleOutcomeState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupHistoryRuleOutcomeBatchModel, Error>
            do {
                result = try .success(
                    await service.loadCleanupHistoryRuleOutcomes(
                        sessionID: sessionID,
                        detail: detail
                    )
                )
            } catch {
                result = .failure(error)
            }
            guard
                !Task.isCancelled,
                let self,
                generation == self.cleanupHistoryRuleOutcomeGeneration,
                self.selectedCleanupHistorySessionID == sessionID,
                case let .loaded(currentDetail) = self.cleanupHistoryDetailState,
                currentDetail == detail,
                currentDetail.summary.sessionID == sessionID
            else {
                return
            }
            self.cleanupHistoryRuleOutcomeTask = nil
            switch result {
            case let .success(batch):
                guard
                    batch.sessionID == sessionID,
                    batch.outcomes.count == detail.items.count
                else {
                    self.cleanupHistoryRuleOutcomeState = .failed(.invalidResponse)
                    return
                }
                self.cleanupHistoryRuleOutcomeState = .loaded(batch)
                self.cleanupHistoryRuleOutcomesReadAt = Date()
            case let .failure(error):
                if error is CancellationError {
                    self.cleanupHistoryRuleOutcomeState = .idle
                } else {
                    self.cleanupHistoryRuleOutcomeState = .failed(
                        (error as? CleanupHistoryServiceError) ?? .invalidResponse
                    )
                }
            }
        }
        cleanupHistoryRuleOutcomeTask = task
        await task.value
    }

    private func fenceCleanupHistoryRuleOutcomes() {
        cleanupHistoryRuleOutcomeGeneration &+= 1
        cleanupHistoryRuleOutcomeTask?.cancel()
        cleanupHistoryRuleOutcomeTask = nil
        cleanupHistoryRuleOutcomeState = .idle
        cleanupHistoryRuleOutcomesReadAt = nil
    }

    private func startRecurringStorageThiefLoad() async {
        guard !cleanupHistoryClearState.isClearing,
              cleanupHistoryStorageThiefTask == nil
        else {
            return
        }
        cleanupHistoryStorageThiefGeneration &+= 1
        let generation = cleanupHistoryStorageThiefGeneration
        cleanupHistoryStorageThiefState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupHistoryStorageThiefRankingModel, Error>
            do {
                result = try .success(await service.loadRecurringStorageThieves())
            } catch {
                result = .failure(error)
            }
            guard
                !Task.isCancelled,
                let self,
                generation == self.cleanupHistoryStorageThiefGeneration
            else {
                return
            }
            self.cleanupHistoryStorageThiefTask = nil
            switch result {
            case let .success(ranking):
                self.cleanupHistoryStorageThiefRanking = ranking
                self.cleanupHistoryStorageThiefReadAt = Date()
                self.cleanupHistoryStorageThiefState = .loaded
            case let .failure(error):
                if error is CancellationError {
                    self.cleanupHistoryStorageThiefState =
                        self.cleanupHistoryStorageThiefRanking == nil ? .idle : .loaded
                } else {
                    self.cleanupHistoryStorageThiefState = .failed(
                        (error as? CleanupHistoryServiceError) ?? .invalidResponse
                    )
                }
            }
        }
        cleanupHistoryStorageThiefTask = task
        await task.value
    }

    private func refreshRecurringStorageThievesIfRequested() async {
        guard cleanupHistoryStorageThiefWasRequested else {
            return
        }
        await refreshRecurringStorageThieves()
    }

    private func fenceRecurringStorageThieves(discardSnapshot: Bool) {
        cleanupHistoryStorageThiefGeneration &+= 1
        cleanupHistoryStorageThiefTask?.cancel()
        cleanupHistoryStorageThiefTask = nil
        if discardSnapshot {
            cleanupHistoryStorageThiefRanking = nil
            cleanupHistoryStorageThiefReadAt = nil
            cleanupHistoryStorageThiefState = .idle
            cleanupHistoryStorageThiefWasRequested = false
        } else {
            cleanupHistoryStorageThiefState =
                cleanupHistoryStorageThiefRanking == nil ? .idle : .loaded
        }
    }

    /// Any newly observed durable successful scan may change the read-only
    /// outcome derivation for the exact history item currently on screen.
    /// Fence the old reply immediately and re-read; this grants no scan or
    /// cleanup authority and deliberately does not poll.
    private func durableScanObservationDidChange() {
        if cleanupHistoryStorageThiefWasRequested {
            Task { @MainActor [weak self] in
                await self?.refreshRecurringStorageThieves()
            }
        }
        guard
            selectedCleanupHistorySessionID != nil,
            case let .loaded(detail) = cleanupHistoryDetailState,
            detail.summary.format == .complete
        else {
            return
        }
        fenceCleanupHistoryRuleOutcomes()
        Task { @MainActor [weak self] in
            await self?.retryCleanupHistoryRuleOutcomes()
        }
    }

    private func fenceCleanupHistoryDetailForSummaryRefresh() {
        cleanupHistoryDetailGeneration &+= 1
        cleanupHistoryDetailTask?.cancel()
        cleanupHistoryDetailTask = nil
        fenceCleanupHistoryRuleOutcomes()
        if selectedCleanupHistorySessionID != nil {
            cleanupHistoryDetailState = .idle
        }
    }

    private func resumeSelectedCleanupHistoryDetailIfNeeded() async {
        guard cleanupHistoryTask == nil,
              let sessionID = selectedCleanupHistorySessionID
        else {
            return
        }
        guard cleanupHistoryRecords.contains(where: { $0.sessionID == sessionID }) else {
            closeCleanupHistorySession()
            return
        }
        guard case .idle = cleanupHistoryDetailState else {
            return
        }
        await loadCleanupHistorySession(sessionID)
    }

    private func mutateCleanupExclusions(
        state: CleanupExclusionsState,
        _ operation: @escaping @Sendable (
            any DuxCleanupExclusionsServing
        ) async throws -> CleanupExclusionsUpdateResult
    ) async {
        cleanupExclusionsGeneration &+= 1
        let generation = cleanupExclusionsGeneration
        precondition(state == .adding || state == .removing || state == .resetting)
        cleanupExclusionsState = state
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupExclusionsUpdateResult, Error>
            do {
                result = try .success(await operation(service))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupExclusionsGeneration
            else {
                return
            }
            switch result {
            case let .success(update):
                self.cleanupExclusions = update.exclusions
                self.cleanupExclusionsState = .ready
            case let .failure(error):
                if error is CancellationError {
                    self.cleanupExclusionsState =
                        self.cleanupExclusions == nil ? .idle : .ready
                } else {
                    self.cleanupExclusionsState = .failed(
                        Self.cleanupExclusionsFailure(for: error)
                    )
                }
            }
            self.cleanupExclusionsTask = nil
        }
        cleanupExclusionsTask = task
        await task.value
    }

    private static func cleanupExclusionsFailure(
        for error: Error
    ) -> CleanupExclusionsFailure {
        if let error = error as? CleanupExclusionsServiceError {
            return .service(error)
        }
        return .unexpected
    }

    private func mutateProjectDiscoveryRoots(
        state: ProjectDiscoveryRootsState,
        _ operation: @escaping @Sendable (
            any DuxProjectDiscoveryRootsServing
        ) async throws -> ProjectDiscoveryRootsUpdateResult
    ) async {
        projectDiscoveryRootsGeneration &+= 1
        let generation = projectDiscoveryRootsGeneration
        precondition(state == .adding || state == .removing || state == .resetting)
        projectDiscoveryRootsState = state
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<ProjectDiscoveryRootsUpdateResult, Error>
            do {
                result = try .success(await operation(service))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.projectDiscoveryRootsGeneration
            else {
                return
            }
            switch result {
            case let .success(update):
                self.projectDiscoveryRoots = update.roots
                self.projectDiscoveryRootsState = .ready
            case let .failure(mutationError):
                if mutationError is CancellationError {
                    self.projectDiscoveryRootsState =
                        self.projectDiscoveryRoots == nil ? .idle : .ready
                } else if Self.projectDiscoveryRootsRequiresReload(after: mutationError) {
                    self.projectDiscoveryRootsRequiresAuthoritativeReload = true
                    self.projectDiscoveryRootsState = .loading
                    do {
                        let authoritative = try await service.loadProjectDiscoveryRoots()
                        guard
                            !Task.isCancelled,
                            generation == self.projectDiscoveryRootsGeneration
                        else {
                            return
                        }
                        self.projectDiscoveryRoots = authoritative
                        self.projectDiscoveryRootsRequiresAuthoritativeReload = false
                        self.projectDiscoveryRootsState = .ready
                    } catch {
                        self.projectDiscoveryRootsState = .failed(
                            Self.projectDiscoveryRootsFailure(for: mutationError)
                        )
                    }
                } else {
                    self.projectDiscoveryRootsState = .failed(
                        Self.projectDiscoveryRootsFailure(for: mutationError)
                    )
                }
            }
            self.projectDiscoveryRootsTask = nil
        }
        projectDiscoveryRootsTask = task
        await task.value
    }

    private static func projectDiscoveryRootsFailure(
        for error: Error
    ) -> ProjectDiscoveryRootsFailure {
        if let error = error as? ProjectDiscoveryRootsServiceError {
            return .service(error)
        }
        return .unexpected
    }

    private static func projectDiscoveryRootsRequiresReload(after error: Error) -> Bool {
        guard let error = error as? ProjectDiscoveryRootsServiceError else {
            return true
        }
        return switch error {
        case .outcomeUnknown, .internalState, .invalidResponse:
            true
        case .closed, .invalidRecordVersion, .invalidPath, .tooManyPaths,
             .overlappingPaths, .revisionExhausted, .invalidClock,
             .incompatibleSchema, .retryable, .unsafeStorage, .budgetExceeded,
             .corruptData, .unavailable:
            false
        }
    }

    private static func directCargoEnrollmentFailure(
        for error: Error
    ) -> DirectCargoEnrollmentFailure {
        if let error = error as? DirectCargoEnrollmentServiceError {
            return .service(error)
        }
        return .unexpected
    }

    private static func pressurePolicyFailure(for error: Error) -> DiskPressurePolicyFailure {
        if let error = error as? DiskPressurePolicyServiceError {
            return .service(error)
        }
        if let error = error as? DiskPressurePolicyDraftError {
            return .draft(error)
        }
        return .unexpected
    }

    private static func loginItemFailureReason(
        for error: Error
    ) -> LoginItemFailureReason {
        guard let error = error as? LoginItemServiceError else {
            return .unexpected
        }
        return switch error {
        case .invalidSignature: .invalidSignature
        case .denied: .denied
        case .serviceUnavailable: .serviceUnavailable
        case .unexpected: .unexpected
        }
    }

    private static func notificationAuthorizationFailure(
        for error: Error
    ) -> NotificationAuthorizationFailureReason {
        guard let error = error as? NotificationServiceError else {
            return .unexpected
        }
        return switch error {
        case .denied: .denied
        case .unexpected: .unexpected
        }
    }

    private func updateMenuBarVisibility() {
        if menuBarRevealOverride {
            isMenuBarItemInserted = true
            return
        }
        isMenuBarItemInserted = MenuBarVisibilityEvaluator.shouldInsert(
            preference: menuBarVisibilityPreference,
            volumeState: volumeState,
            currentlyInserted: isMenuBarItemInserted
        )
    }

    private func isCurrentHomeScan(_ generation: UInt64) -> Bool {
        !homeScanIsInvalidated && generation == homeScanGeneration
    }

    private func publishHomeScanPoll(
        _ poll: HomeScanTaskPoll,
        scope: AppScanScope
    ) -> AppScanRunOutcome? {
        if !poll.events.isEmpty {
            latestHomeScanEvents.append(contentsOf: poll.events)
            if latestHomeScanEvents.count > 64 {
                latestHomeScanEvents.removeFirst(latestHomeScanEvents.count - 64)
            }
        }
        if let progress = poll.progress {
            latestHomeScanProgress = progress
        }

        switch poll.phase {
        case .queued:
            if homeScanCancellationRequested || poll.cancellationRequested {
                homeScanCancellationRequested = true
                scanState.phase = .cancellationRequested(latestHomeScanProgress)
            } else {
                scanState.phase = .queued
            }
            return nil
        case .running:
            if homeScanCancellationRequested || poll.cancellationRequested {
                homeScanCancellationRequested = true
                scanState.phase = .cancellationRequested(latestHomeScanProgress)
                return nil
            }
            scanState.phase = switch poll.stage {
            case .queued: .queued
            case .scanning: .scanning(latestHomeScanProgress)
            case .finalizing: .finalizing(latestHomeScanProgress)
            case .evaluating: .evaluating(latestHomeScanProgress)
            case .terminal: .failed(.invalidResponse)
            }
            return poll.stage == .terminal ? .failed(.invalidResponse) : nil
        case .succeeded:
            guard let summary = poll.result?.successfulSummary else {
                scanState.phase = .failed(.invalidResponse)
                return .failed(.invalidResponse)
            }
            if scope.isHome {
                scanState.lastSuccessful = summary
            }
            scanState.phase = .succeeded(summary)
            durableScanObservationDidChange()
            return .succeeded(summary)
        case .failed:
            let failure = Self.homeScanFailure(for: poll.failure)
            scanState.phase = .failed(failure)
            return .failed(failure)
        case .cancelled:
            scanState.phase = .cancelled
            return .cancelled
        }
    }

    private func finishHomeScanDriver(generation: UInt64) {
        guard generation == homeScanGeneration else {
            return
        }
        activeHomeScanTask = nil
        homeScanDriverTask = nil
        activeScanRequest = nil
        homeScanCancellationRequested = false
        if let snapshot = volumeState.snapshot,
           snapshot.pressure == .warning || snapshot.pressure == .critical
        {
            Task { @MainActor [weak self] in
                await self?.reconcileTargetedReclaimScan(for: snapshot)
            }
        }
    }

    private static func homeScanFailure(for failure: HomeScanTaskFailure?) -> AppScanFailure {
        switch failure {
        case .rootChanged: .rootUnavailable
        case .scanFailed: .scanFailed
        case .snapshotRejected: .snapshotRejected
        case .persistenceUnavailable: .storageUnavailable
        case .persistenceOutcomeUnknown: .outcomeUnknown
        case .internalFailure, .none: .unexpected
        }
    }

    private static func homeScanFailure(for error: Error) -> AppScanFailure {
        guard let error = error as? HomeScanServiceError else {
            return .unexpected
        }
        return switch error {
        case .closed: .closed
        case .queueFull, .busy: .busy
        case .inputTooLarge, .invalidRoot, .rootMissing, .rootAccessDenied, .rootNotDirectory,
             .rootSymlink, .rootChanged, .rootIdentityUnavailable, .unsupportedPlatform,
             .rootUnavailable:
            .rootUnavailable
        case .persistenceUnavailable, .budgetExceeded: .storageUnavailable
        case .readOnlyStore, .incompatibleSchema, .unsafeStorage, .corruptData:
            .incompatibleStorage
        case .taskExpired: .taskExpired
        case .outcomeUnknown: .outcomeUnknown
        case .wrongTaskKind, .internalState, .invalidResponse: .invalidResponse
        }
    }

    private static func targetedRootFailure(
        for error: HomeScanServiceError
    ) -> TargetedReclaimRootFailure {
        switch error {
        case .invalidRoot, .inputTooLarge, .rootNotDirectory, .rootSymlink,
             .rootIdentityUnavailable, .unsupportedPlatform:
            .invalidRoot
        case .rootMissing:
            .rootMissing
        case .rootAccessDenied:
            .accessDenied
        case .rootChanged, .rootUnavailable:
            .rootChanged
        case .queueFull:
            .queueFull
        case .busy:
            .busy
        case .closed, .persistenceUnavailable, .readOnlyStore, .incompatibleSchema,
             .unsafeStorage, .budgetExceeded, .corruptData, .taskExpired,
             .outcomeUnknown:
            .storageUnavailable
        case .wrongTaskKind, .internalState, .invalidResponse:
            .invalidResponse
        }
    }

    private static func targetedFailure(
        for error: TargetedReclaimScanServiceError
    ) -> TargetedReclaimScanFailure {
        switch error {
        case .invalidVolumeIdentity:
            .missingStableVolumeIdentity
        case .invalidAnchor, .pressureChanged:
            .invalidPressureEvidence
        case .configuredRootsChanged:
            .configuredRootsChanged
        case .incompatibleSchema, .readOnlyStore:
            .incompatibleSchema
        case .unsafeStorage:
            .unsafeStorage
        case .budgetExceeded:
            .budgetExceeded
        case .corruptData:
            .corruptData
        case .closed, .rootUnavailable, .storageUnavailable, .outcomeUnknown:
            .unavailable
        case .invalidRecordVersion, .invalidOrdinal, .rootMissing, .rootAccessDenied,
             .rootNotDirectory, .rootSymlink, .rootChanged, .queueFull, .busy,
             .internalState, .invalidResponse:
            .invalidResponse
        }
    }

    private static func volumeFailure(for error: Error) -> VolumeCapacityFailure {
        if let error = error as? EngineServiceError {
            switch error {
            case .invalidCapacityObservation:
                return .invalidObservation
            case .closed, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .retryable, .unavailable, .unexpected:
                return .engineUnavailable
            }
        }
        return switch error as? VolumeMonitorError {
        case .invalidCapacity:
            .invalidObservation
        case .missingTotalCapacity, .missingAvailableCapacity, .none:
            .unavailable
        }
    }
}
