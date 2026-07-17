import Observation

@MainActor
@Observable
final class AppModel: DuxCapacitySampling {
    private(set) var engineState = EngineConnectionState.idle
    private(set) var volumeState = VolumeCapacityState.idle

    private let engineService: any EngineServing
    private let volumeMonitor: any VolumeMonitoring

    @ObservationIgnored
    private var engineLoadTask: Task<Void, Never>?
    @ObservationIgnored
    private var volumeRefreshTask: Task<Void, Never>?
    @ObservationIgnored
    private var volumeStateBeforeRefresh = VolumeCapacityState.idle
    @ObservationIgnored
    private var volumeRefreshGeneration: UInt64 = 0

    init(
        engineService: any EngineServing = EngineService(),
        volumeMonitor: any VolumeMonitoring = VolumeMonitor()
    ) {
        self.engineService = engineService
        self.volumeMonitor = volumeMonitor
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
