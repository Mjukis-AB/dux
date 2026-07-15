import Observation

@MainActor
@Observable
final class AppModel {
    private(set) var engineState = EngineSmokeState.idle
    private(set) var volumeState = VolumeCapacityState.idle

    private let engineService: any EngineServing
    private let volumeMonitor: any VolumeMonitoring

    init(
        engineService: any EngineServing = EngineService(),
        volumeMonitor: any VolumeMonitoring = VolumeMonitor()
    ) {
        self.engineService = engineService
        self.volumeMonitor = volumeMonitor
    }

    func loadInitialState() async {
        async let engineLoad: Void = loadEngineSmokeResult()
        async let volumeLoad: Void = loadVolumeCapacity()
        _ = await (engineLoad, volumeLoad)
    }

    func loadEngineSmokeResult() async {
        guard engineState == .idle else {
            return
        }

        engineState = .loading
        do {
            let result = try await engineService.loadSmokeResult(bytes: 1_536)
            engineState = .loaded(result)
        } catch let error as EngineServiceError {
            engineState = .failed(error)
        } catch {
            engineState = .failed(.unexpected(String(describing: error)))
        }
    }

    func loadVolumeCapacity() async {
        guard volumeState == .idle else {
            return
        }

        volumeState = .loading
        do {
            volumeState = .loaded(try await volumeMonitor.sampleStartupVolume())
        } catch {
            volumeState = .failed
        }
    }
}
