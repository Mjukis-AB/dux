import Foundation
import Observation

@MainActor
@Observable
final class DuxOwnedStorageFootprintSettingsModel {
  private(set) var observation: DuxOwnedStorageFootprintModel?
  private(set) var state = DuxOwnedStorageFootprintLoadState.idle

  private let service: any DuxOwnedStorageFootprintServing

  @ObservationIgnored
  private var operationTask: Task<Void, Never>?
  @ObservationIgnored
  private var generation: UInt64 = 0
  @ObservationIgnored
  private var shuttingDown = false

  init(service: any DuxOwnedStorageFootprintServing) {
    self.service = service
  }

  func load(force: Bool = false) async {
    guard !shuttingDown else {
      return
    }
    if let operationTask {
      await operationTask.value
      return
    }
    guard force || observation == nil else {
      return
    }

    generation &+= 1
    let requestGeneration = generation
    state = .loading
    let service = service
    let task = Task { @MainActor [weak self] in
      let result: Result<DuxOwnedStorageFootprintModel, Error>
      do {
        result = try .success(
          await service.loadOwnedStorageFootprint()
        )
      } catch {
        result = .failure(error)
      }
      guard
        let self,
        !self.shuttingDown,
        self.generation == requestGeneration
      else {
        return
      }
      self.operationTask = nil
      switch result {
      case .success(let observation):
        self.observation = observation
        self.state = .ready
      case .failure(let error):
        self.state = .failed(Self.failure(for: error))
      }
    }
    operationTask = task
    await task.value
  }

  func refresh() async {
    await load(force: true)
  }

  func shutdown() async {
    guard !shuttingDown else {
      await operationTask?.value
      return
    }
    shuttingDown = true
    generation &+= 1
    let operation = operationTask
    operation?.cancel()
    await operation?.value
    operationTask = nil
    state = observation == nil ? .idle : .ready
  }

  private static func failure(
    for error: Error
  ) -> DuxOwnedStorageFootprintServiceError {
    error as? DuxOwnedStorageFootprintServiceError ?? .internalState
  }
}
