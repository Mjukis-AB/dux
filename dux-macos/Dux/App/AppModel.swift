import Observation

@MainActor
@Observable
final class AppModel: DuxCapacitySampling {
    private(set) var engineState = EngineConnectionState.idle
    private(set) var volumeState = VolumeCapacityState.idle
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

    private let engineService: any EngineServing
    private let volumeMonitor: any VolumeMonitoring
    private let capacityResampleRequester: any DuxCapacityResampleRequesting
    private let menuBarLabelPreferenceStore: any MenuBarLabelPreferenceStoring

    @ObservationIgnored
    private var engineLoadTask: Task<Void, Never>?
    @ObservationIgnored
    private var volumeRefreshTask: Task<Void, Never>?
    @ObservationIgnored
    private var volumeStateBeforeRefresh = VolumeCapacityState.idle
    @ObservationIgnored
    private var volumeRefreshGeneration: UInt64 = 0
    @ObservationIgnored
    private var pressurePolicyTask: Task<Void, Never>?
    @ObservationIgnored
    private var pressurePolicyGeneration: UInt64 = 0
    @ObservationIgnored
    private var pressurePolicyIsInvalidated = false

    init(
        engineService: any EngineServing = EngineService(),
        volumeMonitor: any VolumeMonitoring = VolumeMonitor(),
        capacityResampleRequester: any DuxCapacityResampleRequesting =
            NoopDuxCapacityResampleRequester(),
        menuBarLabelPreferenceStore: any MenuBarLabelPreferenceStoring =
            UserDefaultsMenuBarLabelPreferenceStore()
    ) {
        self.engineService = engineService
        self.volumeMonitor = volumeMonitor
        self.capacityResampleRequester = capacityResampleRequester
        self.menuBarLabelPreferenceStore = menuBarLabelPreferenceStore
        menuBarLabelMode = menuBarLabelPreferenceStore.load()
    }

    func loadInitialState() async {
        async let engineLoad: Void = loadEngineStatus()
        async let volumeLoad: Void = loadVolumeCapacity()
        _ = await (engineLoad, volumeLoad)
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
            volumeState = .loaded(snapshot)
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

    private static func pressurePolicyFailure(for error: Error) -> DiskPressurePolicyFailure {
        if let error = error as? DiskPressurePolicyServiceError {
            return .service(error)
        }
        if let error = error as? DiskPressurePolicyDraftError {
            return .draft(error)
        }
        return .unexpected
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
