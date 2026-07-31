import Foundation
import Observation

@MainActor
@Observable
final class DuxOwnedStorageFootprintSettingsModel {
  private enum OperationKind {
    case footprintRead
    case clearPreparation
    case snapshotClearPreparation
  }

  private(set) var observation: DuxOwnedStorageFootprintModel?
  private(set) var state = DuxOwnedStorageFootprintLoadState.idle
  private(set) var managedScanCacheClearState =
    DuxManagedScanCacheClearState.idle
  private(set) var managedScanCacheClearConfirmation: DuxManagedScanCacheClearConfirmation?
  private(set) var snapshotStorageClearState =
    DuxSnapshotStorageClearState.idle
  private(set) var snapshotStorageClearConfirmation: DuxSnapshotStorageClearConfirmation?

  private let service: any DuxOwnedStorageFootprintServing

  @ObservationIgnored
  private var operationTask: Task<Void, Never>?
  @ObservationIgnored
  private var operationKind: OperationKind?
  @ObservationIgnored
  private var confirmedClearTask: Task<Void, Never>?
  @ObservationIgnored
  private var confirmedSnapshotClearTask: Task<Void, Never>?
  @ObservationIgnored
  private var previewExpiryTask: Task<Void, Never>?
  @ObservationIgnored
  private var snapshotPreviewExpiryTask: Task<Void, Never>?
  @ObservationIgnored
  private var managedScanCacheClearLease: (any DuxManagedScanCacheClearPreviewLease)?
  @ObservationIgnored
  private var snapshotStorageClearLease: (any DuxSnapshotStorageClearPreviewLease)?
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
    if let confirmedClearTask {
      await confirmedClearTask.value
      return
    }
    if let confirmedSnapshotClearTask {
      await confirmedSnapshotClearTask.value
      return
    }
    if let operationTask {
      await operationTask.value
      return
    }
    guard
      managedScanCacheClearConfirmation == nil,
      snapshotStorageClearConfirmation == nil
    else {
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
      self.operationKind = nil
      self.applyFootprintResult(result)
    }
    operationKind = .footprintRead
    operationTask = task
    await task.value
  }

  func refresh() async {
    await load(force: true)
  }

  func prepareManagedScanCacheClear() async {
    guard
      !shuttingDown,
      operationTask == nil,
      confirmedClearTask == nil,
      confirmedSnapshotClearTask == nil,
      managedScanCacheClearConfirmation == nil,
      snapshotStorageClearConfirmation == nil,
      !snapshotStorageClearState.isBusy
    else {
      return
    }

    generation &+= 1
    let requestGeneration = generation
    managedScanCacheClearState = .preparing
    let service = service
    let task = Task { @MainActor [weak self] in
      let result: Result<any DuxManagedScanCacheClearPreviewLease, Error>
      do {
        result = try .success(
          await service.prepareManagedScanCacheClear()
        )
      } catch {
        result = .failure(error)
      }
      guard
        let self,
        !self.shuttingDown,
        self.generation == requestGeneration
      else {
        if case .success(let lease) = result {
          await lease.release()
        }
        return
      }
      self.operationTask = nil
      self.operationKind = nil
      switch result {
      case .success(let lease):
        guard lease.preview.expiresAt > Date() else {
          self.managedScanCacheClearState = .failed(.previewExpired)
          await lease.release()
          return
        }
        let confirmation = DuxManagedScanCacheClearConfirmation(
          generation: requestGeneration,
          preview: lease.preview
        )
        self.managedScanCacheClearLease = lease
        self.managedScanCacheClearConfirmation = confirmation
        self.managedScanCacheClearState =
          .awaitingConfirmation(confirmation)
        self.schedulePreviewExpiration(confirmation)
      case .failure(let error):
        self.managedScanCacheClearState =
          .failed(Self.clearFailure(for: error))
      }
    }
    operationKind = .clearPreparation
    operationTask = task
    await task.value
  }

  func confirmManagedScanCacheClear(
    _ confirmation: DuxManagedScanCacheClearConfirmation
  ) async {
    guard
      !shuttingDown,
      operationTask == nil,
      confirmedClearTask == nil,
      confirmedSnapshotClearTask == nil,
      snapshotStorageClearConfirmation == nil,
      !snapshotStorageClearState.isBusy,
      managedScanCacheClearConfirmation == confirmation,
      let lease = managedScanCacheClearLease
    else {
      return
    }
    guard confirmation.preview.expiresAt > Date() else {
      await cancelManagedScanCacheClear(confirmation)
      managedScanCacheClearState = .failed(.previewExpired)
      return
    }

    generation &+= 1
    let clearGeneration = generation
    previewExpiryTask?.cancel()
    previewExpiryTask = nil
    managedScanCacheClearConfirmation = nil
    managedScanCacheClearLease = nil
    // The pre-confirmation observation no longer describes authoritative
    // state once consume-once mutation begins.
    observation = nil
    state = .loading
    managedScanCacheClearState = .clearing(confirmation.preview)
    let service = service
    let task = Task { @MainActor [weak self] in
      let clearResult: Result<DuxManagedScanCacheClearResultModel, Error>
      do {
        clearResult = try .success(
          await service.clearManagedScanCache(lease)
        )
      } catch {
        clearResult = .failure(error)
      }
      await lease.release()
      guard let self else {
        return
      }

      let refreshIsRequired: Bool
      switch clearResult {
      case .success:
        refreshIsRequired = true
      case .failure(let error):
        let failure = Self.clearFailure(for: error)
        refreshIsRequired =
          failure == .outcomeUnknown || failure == .changedSincePreview
      }
      let refreshResult: Result<DuxOwnedStorageFootprintModel, Error>?
      if refreshIsRequired {
        do {
          refreshResult = try .success(
            await service.loadOwnedStorageFootprint()
          )
        } catch {
          refreshResult = .failure(error)
        }
      } else {
        refreshResult = nil
      }

      guard
        !self.shuttingDown,
        self.generation == clearGeneration
      else {
        self.confirmedClearTask = nil
        return
      }
      if let refreshResult {
        self.applyFootprintResult(refreshResult)
      } else {
        self.state = .idle
      }
      switch clearResult {
      case .success(let result):
        self.managedScanCacheClearState = .completed(result)
      case .failure(let error):
        let failure = Self.clearFailure(for: error)
        if failure == .outcomeUnknown {
          self.managedScanCacheClearState = .outcomeUnknown
        } else {
          self.managedScanCacheClearState = .failed(failure)
        }
      }
      self.confirmedClearTask = nil
    }
    confirmedClearTask = task
    await task.value
  }

  func cancelManagedScanCacheClear(
    _ confirmation: DuxManagedScanCacheClearConfirmation? = nil
  ) async {
    guard
      let current = managedScanCacheClearConfirmation,
      confirmation == nil || confirmation == current
    else {
      return
    }
    generation &+= 1
    previewExpiryTask?.cancel()
    previewExpiryTask = nil
    let lease = managedScanCacheClearLease
    managedScanCacheClearLease = nil
    managedScanCacheClearConfirmation = nil
    managedScanCacheClearState = .idle
    await lease?.release()
  }

  func dismissManagedScanCacheClear(
    _ confirmation: DuxManagedScanCacheClearConfirmation? = nil
  ) async {
    if confirmation == nil,
      operationKind == .clearPreparation,
      let preparation = operationTask
    {
      generation &+= 1
      preparation.cancel()
      managedScanCacheClearState = .idle
      await preparation.value
      operationKind = nil
      operationTask = nil
      return
    }
    await cancelManagedScanCacheClear(confirmation)
  }

  func dismissManagedScanCacheClearStatus() {
    switch managedScanCacheClearState {
    case .completed, .failed, .outcomeUnknown:
      generation &+= 1
      managedScanCacheClearState = .idle
    case .idle, .preparing, .awaitingConfirmation, .clearing:
      break
    }
  }

  func prepareSnapshotStorageClear() async {
    guard
      !shuttingDown,
      operationTask == nil,
      confirmedClearTask == nil,
      confirmedSnapshotClearTask == nil,
      managedScanCacheClearConfirmation == nil,
      snapshotStorageClearConfirmation == nil,
      !managedScanCacheClearState.isBusy,
      !snapshotStorageClearState.isBusy
    else {
      return
    }
    guard observation?.snapshots.accountingUnstable != true else {
      snapshotStorageClearState = .failed(.retryable)
      return
    }

    generation &+= 1
    let requestGeneration = generation
    snapshotStorageClearState = .preparing
    let service = service
    let task = Task { @MainActor [weak self] in
      let result: Result<any DuxSnapshotStorageClearPreviewLease, Error>
      do {
        result = try .success(
          await service.prepareSnapshotStorageClear()
        )
      } catch {
        result = .failure(error)
      }
      guard
        let self,
        !self.shuttingDown,
        self.generation == requestGeneration
      else {
        if case .success(let lease) = result {
          await lease.release()
        }
        return
      }
      self.operationTask = nil
      self.operationKind = nil
      switch result {
      case .success(let lease):
        guard lease.preview.expiresAt > Date() else {
          self.snapshotStorageClearState = .failed(.previewExpired)
          await lease.release()
          return
        }
        let confirmation = DuxSnapshotStorageClearConfirmation(
          generation: requestGeneration,
          preview: lease.preview
        )
        self.snapshotStorageClearLease = lease
        self.snapshotStorageClearConfirmation = confirmation
        self.snapshotStorageClearState = .awaitingConfirmation(confirmation)
        self.scheduleSnapshotPreviewExpiration(confirmation)
      case .failure(let error):
        self.snapshotStorageClearState =
          .failed(Self.snapshotClearFailure(for: error))
      }
    }
    operationKind = .snapshotClearPreparation
    operationTask = task
    await task.value
  }

  func confirmSnapshotStorageClear(
    _ confirmation: DuxSnapshotStorageClearConfirmation
  ) async {
    guard
      !shuttingDown,
      operationTask == nil,
      confirmedClearTask == nil,
      confirmedSnapshotClearTask == nil,
      managedScanCacheClearConfirmation == nil,
      !managedScanCacheClearState.isBusy,
      snapshotStorageClearConfirmation == confirmation,
      let lease = snapshotStorageClearLease
    else {
      return
    }
    guard confirmation.preview.expiresAt > Date() else {
      await cancelSnapshotStorageClear(confirmation)
      snapshotStorageClearState = .failed(.previewExpired)
      return
    }

    generation &+= 1
    let clearGeneration = generation
    snapshotPreviewExpiryTask?.cancel()
    snapshotPreviewExpiryTask = nil
    snapshotStorageClearConfirmation = nil
    snapshotStorageClearLease = nil
    observation = nil
    state = .loading
    snapshotStorageClearState = .clearing(confirmation.preview)
    let service = service
    let task = Task { @MainActor [weak self] in
      let clearResult: Result<DuxSnapshotStorageClearResultModel, Error>
      do {
        clearResult = try .success(
          await service.clearSnapshotStorage(lease)
        )
      } catch {
        clearResult = .failure(error)
      }
      await lease.release()
      guard let self else {
        return
      }

      // Once the consume-once effect call begins, every terminal response gets
      // exactly one observation-only refresh. The effect is never retried.
      let refreshResult: Result<DuxOwnedStorageFootprintModel, Error>
      do {
        refreshResult = try .success(
          await service.loadOwnedStorageFootprint()
        )
      } catch {
        refreshResult = .failure(error)
      }

      guard
        !self.shuttingDown,
        self.generation == clearGeneration
      else {
        self.confirmedSnapshotClearTask = nil
        return
      }
      self.applyFootprintResult(refreshResult)
      switch clearResult {
      case .success(let result):
        self.snapshotStorageClearState = .completed(result)
      case .failure(let error):
        let failure = Self.snapshotClearFailure(for: error)
        if failure == .outcomeUnknown {
          self.snapshotStorageClearState = .outcomeUnknown
        } else {
          self.snapshotStorageClearState = .failed(failure)
        }
      }
      self.confirmedSnapshotClearTask = nil
    }
    confirmedSnapshotClearTask = task
    await task.value
  }

  func cancelSnapshotStorageClear(
    _ confirmation: DuxSnapshotStorageClearConfirmation? = nil
  ) async {
    guard
      let current = snapshotStorageClearConfirmation,
      confirmation == nil || confirmation == current
    else {
      return
    }
    generation &+= 1
    snapshotPreviewExpiryTask?.cancel()
    snapshotPreviewExpiryTask = nil
    let lease = snapshotStorageClearLease
    snapshotStorageClearLease = nil
    snapshotStorageClearConfirmation = nil
    snapshotStorageClearState = .idle
    await lease?.release()
  }

  func dismissSnapshotStorageClear(
    _ confirmation: DuxSnapshotStorageClearConfirmation? = nil
  ) async {
    if confirmation == nil,
      operationKind == .snapshotClearPreparation,
      let preparation = operationTask
    {
      generation &+= 1
      preparation.cancel()
      snapshotStorageClearState = .idle
      await preparation.value
      operationKind = nil
      operationTask = nil
      return
    }
    await cancelSnapshotStorageClear(confirmation)
  }

  func dismissSnapshotStorageClearStatus() {
    switch snapshotStorageClearState {
    case .completed, .failed, .outcomeUnknown:
      generation &+= 1
      snapshotStorageClearState = .idle
    case .idle, .preparing, .awaitingConfirmation, .clearing:
      break
    }
  }

  func shutdown() async {
    guard !shuttingDown else {
      await operationTask?.value
      await confirmedClearTask?.value
      await confirmedSnapshotClearTask?.value
      return
    }
    shuttingDown = true
    generation &+= 1
    previewExpiryTask?.cancel()
    previewExpiryTask = nil
    snapshotPreviewExpiryTask?.cancel()
    snapshotPreviewExpiryTask = nil

    let operation = operationTask
    operation?.cancel()
    let pendingLease = managedScanCacheClearLease
    let pendingSnapshotLease = snapshotStorageClearLease
    managedScanCacheClearLease = nil
    snapshotStorageClearLease = nil
    managedScanCacheClearConfirmation = nil
    snapshotStorageClearConfirmation = nil
    if case .awaitingConfirmation = managedScanCacheClearState {
      managedScanCacheClearState = .idle
    }
    await pendingLease?.release()
    if case .awaitingConfirmation = snapshotStorageClearState {
      snapshotStorageClearState = .idle
    }
    await pendingSnapshotLease?.release()
    await operation?.value
    operationTask = nil
    operationKind = nil

    // A confirmed clear is consume-once and may already have committed.
    // Never cancel or retry it; wait through its authoritative refresh.
    let confirmedClear = confirmedClearTask
    await confirmedClear?.value
    confirmedClearTask = nil
    let confirmedSnapshotClear = confirmedSnapshotClearTask
    await confirmedSnapshotClear?.value
    confirmedSnapshotClearTask = nil
    state = observation == nil ? .idle : .ready
    switch managedScanCacheClearState {
    case .preparing, .awaitingConfirmation, .clearing:
      managedScanCacheClearState = .idle
    case .idle, .completed, .failed, .outcomeUnknown:
      break
    }
    switch snapshotStorageClearState {
    case .preparing, .awaitingConfirmation, .clearing:
      snapshotStorageClearState = .idle
    case .idle, .completed, .failed, .outcomeUnknown:
      break
    }
  }

  private func schedulePreviewExpiration(
    _ confirmation: DuxManagedScanCacheClearConfirmation
  ) {
    previewExpiryTask?.cancel()
    let delay = max(
      0,
      confirmation.preview.expiresAt.timeIntervalSinceNow
    )
    previewExpiryTask = Task { @MainActor [weak self] in
      do {
        try await Task.sleep(for: .seconds(delay))
      } catch {
        return
      }
      guard
        let self,
        !self.shuttingDown,
        self.managedScanCacheClearConfirmation == confirmation
      else {
        return
      }
      let lease = self.managedScanCacheClearLease
      self.generation &+= 1
      self.managedScanCacheClearLease = nil
      self.managedScanCacheClearConfirmation = nil
      self.managedScanCacheClearState = .failed(.previewExpired)
      await lease?.release()
    }
  }

  private func scheduleSnapshotPreviewExpiration(
    _ confirmation: DuxSnapshotStorageClearConfirmation
  ) {
    snapshotPreviewExpiryTask?.cancel()
    let delay = max(
      0,
      confirmation.preview.expiresAt.timeIntervalSinceNow
    )
    snapshotPreviewExpiryTask = Task { @MainActor [weak self] in
      do {
        try await Task.sleep(for: .seconds(delay))
      } catch {
        return
      }
      guard
        let self,
        !self.shuttingDown,
        self.snapshotStorageClearConfirmation == confirmation
      else {
        return
      }
      let lease = self.snapshotStorageClearLease
      self.generation &+= 1
      self.snapshotStorageClearLease = nil
      self.snapshotStorageClearConfirmation = nil
      self.snapshotStorageClearState = .failed(.previewExpired)
      await lease?.release()
    }
  }

  private func applyFootprintResult(
    _ result: Result<DuxOwnedStorageFootprintModel, Error>
  ) {
    switch result {
    case .success(let observation):
      self.observation = observation
      state = .ready
    case .failure(let error):
      state = .failed(Self.failure(for: error))
    }
  }

  private static func failure(
    for error: Error
  ) -> DuxOwnedStorageFootprintServiceError {
    error as? DuxOwnedStorageFootprintServiceError ?? .internalState
  }

  private static func clearFailure(
    for error: Error
  ) -> DuxManagedScanCacheClearServiceError {
    error as? DuxManagedScanCacheClearServiceError ?? .internalState
  }

  private static func snapshotClearFailure(
    for error: Error
  ) -> DuxSnapshotStorageClearServiceError {
    error as? DuxSnapshotStorageClearServiceError ?? .internalState
  }
}
