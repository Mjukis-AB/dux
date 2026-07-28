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
        _ = await (engineLoad, volumeLoad, cleanupHistoryLoad)
        await loadCapacityTrend()
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
                result = .success(try await engineService.observeVolumeCapacity(observed))
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
            return
        }
        if let capacityTrendTask {
            await capacityTrendTask.value
            return
        }
        capacityTrendGeneration &+= 1
        let generation = capacityTrendGeneration
        let service = engineService
        let task = Task { @MainActor [weak self] in
            defer {
                self?.capacityTrendTask = nil
            }
            do {
                let trend = try await service.loadCapacityTrend(
                    stableVolumeID: stableVolumeID,
                    at: snapshot.sampledAt
                )
                guard let self,
                      generation == self.capacityTrendGeneration,
                      self.volumeState.snapshot?.stableVolumeID == stableVolumeID else {
                    return
                }
                self.capacityTrend = trend
            } catch is CancellationError {
                return
            } catch {
                // Trend history is optional presentation context. Keep the
                // last good chart while capacity status remains authoritative.
            }
        }
        capacityTrendTask = task
        await task.value
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
                result = .success(try await engineService.loadDiskPressurePolicy())
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

    static let permanentCleanupReenableConfirmation = "ENABLE PERMANENT CLEANUP"

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
                result = .success(try await service.loadPermanentCleanupPolicy())
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

    /// Changes the global kill switch. Re-enabling requires the exact phrase so
    /// the UI cannot accidentally turn permanent cleanup back on.
    func setPermanentCleanupEnabled(
        _ enabled: Bool,
        confirmation: String? = nil
    ) async {
        guard !permanentCleanupPolicyIsInvalidated,
              permanentCleanupPolicyTask == nil,
              !permanentCleanupPolicyState.isBusy else {
            return
        }
        guard !enabled || confirmation == Self.permanentCleanupReenableConfirmation else {
            permanentCleanupPolicyState = .failed(.confirmationRequired)
            return
        }
        await mutatePermanentCleanupPolicy(state: enabled ? .enabling : .disabling) { service in
            try await service.setPermanentCleanupEnabled(enabled)
        }
    }

    func resetPermanentCleanup(confirmation: String? = nil) async {
        guard !permanentCleanupPolicyIsInvalidated,
              permanentCleanupPolicyTask == nil,
              !permanentCleanupPolicyState.isBusy else {
            return
        }
        // The core default is enabled. Treat reset as a re-enable whenever the
        // current authoritative state is disabled.
        if permanentCleanupPolicy?.enabled == false,
           confirmation != Self.permanentCleanupReenableConfirmation {
            permanentCleanupPolicyState = .failed(.confirmationRequired)
            return
        }
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
                result = .success(try await service.loadCleanupExclusions())
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
                result = .success(try await service.loadDirectCargoEnrollmentStatus())
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
                result = .success(try await service.inspectDirectCargoExecutable(selection))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.directCargoEnrollmentGeneration,
                  !self.directCargoEnrollmentIsInvalidated else {
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
                result = .success(try await service.enrollDirectCargo(preview))
            } catch {
                result = .failure(error)
            }
            await preview.release()
            guard !Task.isCancelled, let self,
                  generation == self.directCargoEnrollmentGeneration,
                  !self.directCargoEnrollmentIsInvalidated else {
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
                result = .success(try await service.revokeDirectCargoEnrollment())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.directCargoEnrollmentGeneration,
                  !self.directCargoEnrollmentIsInvalidated else {
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
                result = .success(
                    try await service.loadRecentCleanupHistory(cursor: nil, limit: 64)
                )
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupHistoryGeneration else {
                return
            }
            self.publishCleanupHistory(result, appending: false, generation: generation)
        }
        cleanupHistoryTask = task
        await task.value
        await resumeSelectedCleanupHistoryDetailIfNeeded()
    }

    func loadMoreCleanupHistory() async {
        guard cleanupHistoryTask == nil,
              let cursor = cleanupHistoryNextCursor else {
            return
        }

        cleanupHistoryGeneration &+= 1
        let generation = cleanupHistoryGeneration
        cleanupHistoryState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupHistoryPageModel, Error>
            do {
                result = .success(
                    try await service.loadRecentCleanupHistory(cursor: cursor, limit: 64)
                )
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupHistoryGeneration else {
                return
            }
            self.publishCleanupHistory(result, appending: true, generation: generation)
        }
        cleanupHistoryTask = task
        await task.value
    }

    func refreshCleanupHistory() async {
        cleanupHistoryGeneration &+= 1
        cleanupHistoryTask?.cancel()
        cleanupHistoryTask = nil
        cleanupHistoryState = cleanupHistoryRecords.isEmpty ? .idle : .loading
        await loadCleanupHistory()
    }

    func invalidateCleanupHistoryOperations() {
        cleanupHistoryGeneration &+= 1
        cleanupHistoryTask?.cancel()
        cleanupHistoryTask = nil
        cleanupHistoryState = cleanupHistoryRecords.isEmpty ? .idle : .loaded
        closeCleanupHistorySession()
    }

    /// Selects one exact path-free history record. The ID must still be
    /// present in the current summary feed and is only an observation key.
    func selectCleanupHistorySession(_ sessionID: String) async {
        guard cleanupHistoryRecords.contains(where: { $0.sessionID == sessionID }) else {
            closeCleanupHistorySession()
            return
        }
        if selectedCleanupHistorySessionID == sessionID {
            if case .loaded = cleanupHistoryDetailState {
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
              cleanupHistoryRecords.contains(where: { $0.sessionID == sessionID }) else {
            closeCleanupHistorySession()
            return
        }
        await loadCleanupHistorySession(sessionID)
    }

    func closeCleanupHistorySession() {
        cleanupHistoryDetailGeneration &+= 1
        cleanupHistoryDetailTask?.cancel()
        cleanupHistoryDetailTask = nil
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
                  generation == self.loginItemGeneration else {
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
              status.registrationRequested != registrationRequested else {
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
                  generation == self.loginItemGeneration else {
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
                  generation == self.notificationAuthorizationGeneration else {
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
                  generation == self.notificationAuthorizationGeneration else {
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
                result = .success(try await probe.probe())
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.storageAccessProbeGeneration else {
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

    func startHomeScan() async {
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
        await startScan(
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
                capacityTrend = nil
            }
            volumeState = .loaded(snapshot)
            Task { @MainActor [weak self] in
                await self?.deliverPressureNotificationIfNeeded(snapshot)
            }
            Task { @MainActor [weak self] in
                await self?.loadCapacityTrend()
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
                  ) else {
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
                result = .success(try await operation(service))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.pressurePolicyGeneration else {
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
                      generation == self.pressurePolicyGeneration else {
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
                result = .success(try await operation(service))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.permanentCleanupPolicyGeneration else {
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
               !cleanupHistoryRecords.contains(where: { $0.sessionID == sessionID }) {
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
        selectedCleanupHistorySessionID = sessionID
        cleanupHistoryDetailState = .loading
        let service = engineService
        let task = Task { @MainActor [weak self] in
            let result: Result<CleanupHistorySessionDetailModel, Error>
            do {
                result = .success(
                    try await service.loadCleanupHistorySession(sessionID: sessionID)
                )
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupHistoryDetailGeneration,
                  self.selectedCleanupHistorySessionID == sessionID else {
                return
            }
            self.cleanupHistoryDetailTask = nil
            switch result {
            case let .success(detail):
                guard detail.summary.sessionID == sessionID,
                      self.cleanupHistoryRecords.contains(
                          where: { $0.sessionID == sessionID }
                      ) else {
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
    }

    private func fenceCleanupHistoryDetailForSummaryRefresh() {
        cleanupHistoryDetailGeneration &+= 1
        cleanupHistoryDetailTask?.cancel()
        cleanupHistoryDetailTask = nil
        if selectedCleanupHistorySessionID != nil {
            cleanupHistoryDetailState = .idle
        }
    }

    private func resumeSelectedCleanupHistoryDetailIfNeeded() async {
        guard cleanupHistoryTask == nil,
              let sessionID = selectedCleanupHistorySessionID else {
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
                result = .success(try await operation(service))
            } catch {
                result = .failure(error)
            }
            guard !Task.isCancelled, let self,
                  generation == self.cleanupExclusionsGeneration else {
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
