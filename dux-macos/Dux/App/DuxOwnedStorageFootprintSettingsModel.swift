import Foundation
import Observation

@MainActor
@Observable
final class DuxOwnedStorageFootprintSettingsModel {
  private enum OperationKind {
    case footprintRead
    case managedScanCacheClearPreparation
    case snapshotClearPreparation
    case aiInsightCacheClearPreparation
  }

  private(set) var observation: DuxOwnedStorageFootprintModel?
  private(set) var state = DuxOwnedStorageFootprintLoadState.idle
  private(set) var managedScanCacheClearState =
    DuxManagedScanCacheClearState.idle
  private(set) var managedScanCacheClearConfirmation: DuxManagedScanCacheClearConfirmation?
  private(set) var snapshotStorageClearState =
    DuxSnapshotStorageClearState.idle
  private(set) var snapshotStorageClearConfirmation: DuxSnapshotStorageClearConfirmation?
  private(set) var aiInsightCacheClearState =
    DuxAIInsightCacheClearState.idle
  private(set) var aiInsightCacheClearConfirmation: DuxAIInsightCacheClearConfirmation?

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
  private var confirmedAIInsightCacheClearTask: Task<Void, Never>?
  @ObservationIgnored
  private var previewExpiryTask: Task<Void, Never>?
  @ObservationIgnored
  private var snapshotPreviewExpiryTask: Task<Void, Never>?
  @ObservationIgnored
  private var aiInsightCachePreviewExpiryTask: Task<Void, Never>?
  @ObservationIgnored
  private var terminalChildWaiters: [UUID: Task<Void, Never>] = [:]
  @ObservationIgnored
  private var managedScanCacheClearLease: (any DuxManagedScanCacheClearPreviewLease)?
  @ObservationIgnored
  private var snapshotStorageClearLease: (any DuxSnapshotStorageClearPreviewLease)?
  @ObservationIgnored
  private var aiInsightCacheClearLease: (any DuxAIInsightCacheClearPreviewLease)?
  @ObservationIgnored
  private var aiInsightCacheClearBarrier: (any DuxAIInsightCacheClearBarrier)?
  @ObservationIgnored
  private var generation: UInt64 = 0
  @ObservationIgnored
  private var shuttingDown = false
  @ObservationIgnored
  private var terminalShutdownTask: Task<Void, Never>?

  init(service: any DuxOwnedStorageFootprintServing) {
    self.service = service
  }

  /// Installs the runtime-owned AI drain gate without exposing it back through
  /// observable settings state.
  func installAIInsightCacheClearBarrier(
    _ barrier: any DuxAIInsightCacheClearBarrier
  ) {
    guard !shuttingDown else {
      return
    }
    aiInsightCacheClearBarrier = barrier
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
    if let confirmedAIInsightCacheClearTask {
      await confirmedAIInsightCacheClearTask.value
      return
    }
    if let operationTask {
      await operationTask.value
      return
    }
    guard
      managedScanCacheClearConfirmation == nil,
      snapshotStorageClearConfirmation == nil,
      aiInsightCacheClearConfirmation == nil
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
      confirmedAIInsightCacheClearTask == nil,
      managedScanCacheClearConfirmation == nil,
      snapshotStorageClearConfirmation == nil,
      aiInsightCacheClearConfirmation == nil,
      !snapshotStorageClearState.isBusy,
      !aiInsightCacheClearState.isBusy
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
      defer {
        self.operationTask = nil
        self.operationKind = nil
      }
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
    operationKind = .managedScanCacheClearPreparation
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
      confirmedAIInsightCacheClearTask == nil,
      snapshotStorageClearConfirmation == nil,
      aiInsightCacheClearConfirmation == nil,
      !snapshotStorageClearState.isBusy,
      !aiInsightCacheClearState.isBusy,
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
    if let lease {
      let release = Task { await lease.release() }
      retainForTerminal(release)
      await release.value
    }
  }

  func dismissManagedScanCacheClear(
    _ confirmation: DuxManagedScanCacheClearConfirmation? = nil
  ) async {
    if confirmation == nil,
      operationKind == .managedScanCacheClearPreparation,
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
      confirmedAIInsightCacheClearTask == nil,
      managedScanCacheClearConfirmation == nil,
      snapshotStorageClearConfirmation == nil,
      aiInsightCacheClearConfirmation == nil,
      !managedScanCacheClearState.isBusy,
      !snapshotStorageClearState.isBusy,
      !aiInsightCacheClearState.isBusy
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
      defer {
        self.operationTask = nil
        self.operationKind = nil
      }
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
      confirmedAIInsightCacheClearTask == nil,
      managedScanCacheClearConfirmation == nil,
      aiInsightCacheClearConfirmation == nil,
      !managedScanCacheClearState.isBusy,
      !aiInsightCacheClearState.isBusy,
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
    if let lease {
      let release = Task { await lease.release() }
      retainForTerminal(release)
      await release.value
    }
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

  func prepareAIInsightCacheClear() async {
    guard
      !shuttingDown,
      operationTask == nil,
      confirmedClearTask == nil,
      confirmedSnapshotClearTask == nil,
      confirmedAIInsightCacheClearTask == nil,
      managedScanCacheClearConfirmation == nil,
      snapshotStorageClearConfirmation == nil,
      aiInsightCacheClearConfirmation == nil,
      !managedScanCacheClearState.isBusy,
      !snapshotStorageClearState.isBusy,
      !aiInsightCacheClearState.isBusy
    else {
      return
    }
    guard
      aiInsightCacheClearBarrier != nil,
      let clearService = service as? any DuxAIInsightCacheClearServing
    else {
      aiInsightCacheClearState = .failed(.unavailable)
      return
    }

    generation &+= 1
    let requestGeneration = generation
    aiInsightCacheClearState = .preparing
    let task = Task { @MainActor [weak self] in
      let result: Result<any DuxAIInsightCacheClearPreviewLease, Error>
      do {
        result = try .success(
          await clearService.prepareAIInsightCacheClear()
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
      defer {
        self.operationTask = nil
        self.operationKind = nil
      }
      switch result {
      case .success(let lease):
        guard Self.isValidAIInsightCachePreview(lease.preview) else {
          self.aiInsightCacheClearState = .failed(.invalidResponse)
          await lease.release()
          return
        }
        guard lease.preview.expiresAt > Date() else {
          self.aiInsightCacheClearState = .failed(.previewExpired)
          await lease.release()
          return
        }
        let confirmation = DuxAIInsightCacheClearConfirmation(
          generation: requestGeneration,
          preview: lease.preview
        )
        self.aiInsightCacheClearLease = lease
        self.aiInsightCacheClearConfirmation = confirmation
        self.aiInsightCacheClearState = .awaitingConfirmation(confirmation)
        self.scheduleAIInsightCachePreviewExpiration(confirmation)
      case .failure(let error):
        self.aiInsightCacheClearState =
          .failed(Self.aiInsightCacheClearFailure(for: error))
      }
    }
    operationKind = .aiInsightCacheClearPreparation
    operationTask = task
    await task.value
  }

  func confirmAIInsightCacheClear(
    _ confirmation: DuxAIInsightCacheClearConfirmation
  ) async {
    guard
      !shuttingDown,
      operationTask == nil,
      confirmedClearTask == nil,
      confirmedSnapshotClearTask == nil,
      confirmedAIInsightCacheClearTask == nil,
      managedScanCacheClearConfirmation == nil,
      snapshotStorageClearConfirmation == nil,
      !managedScanCacheClearState.isBusy,
      !snapshotStorageClearState.isBusy,
      aiInsightCacheClearConfirmation == confirmation,
      let lease = aiInsightCacheClearLease
    else {
      return
    }
    guard confirmation.preview.expiresAt > Date() else {
      await cancelAIInsightCacheClear(confirmation)
      aiInsightCacheClearState = .failed(.previewExpired)
      return
    }
    guard
      let barrier = aiInsightCacheClearBarrier,
      let clearService = service as? any DuxAIInsightCacheClearServing
    else {
      await cancelAIInsightCacheClear(confirmation)
      aiInsightCacheClearState = .failed(.unavailable)
      return
    }

    generation &+= 1
    let clearGeneration = generation
    aiInsightCachePreviewExpiryTask?.cancel()
    aiInsightCachePreviewExpiryTask = nil
    aiInsightCacheClearConfirmation = nil
    aiInsightCacheClearLease = nil
    aiInsightCacheClearState = .clearing(confirmation.preview)
    let footprintService = service
    let task = Task { @MainActor [weak self] in
      // Entering the barrier cancels and joins any active/ready AI
      // presentation and prevents a cache repopulation until release.
      let barrierLease: any DuxAIInsightCacheClearBarrierLease
      do {
        barrierLease = try await barrier.beginAIInsightCacheClear()
      } catch {
        await lease.release()
        guard let self else {
          return
        }
        if
          !self.shuttingDown,
          self.generation == clearGeneration
        {
          self.aiInsightCacheClearState = .failed(.unavailable)
        }
        self.confirmedAIInsightCacheClearTask = nil
        return
      }
      // The earlier footprint remains valid if barrier acquisition fails.
      // Invalidate it only once the final clear can actually be invoked.
      self?.observation = nil
      self?.state = .loading
      let clearResult: Result<DuxAIInsightCacheClearResultModel, Error>
      do {
        clearResult = try .success(
          await clearService.clearAIInsightCache(lease)
        )
      } catch {
        clearResult = .failure(error)
      }
      await lease.release()
      await barrierLease.releaseAndWait()
      guard let self else {
        return
      }

      let refreshIsRequired: Bool
      switch clearResult {
      case .success:
        refreshIsRequired = true
      case .failure(let error):
        let failure = Self.aiInsightCacheClearFailure(for: error)
        refreshIsRequired =
          failure == .outcomeUnknown || failure == .changedSincePreview
      }
      let refreshResult: Result<DuxOwnedStorageFootprintModel, Error>?
      if refreshIsRequired {
        do {
          refreshResult = try .success(
            await footprintService.loadOwnedStorageFootprint()
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
        self.confirmedAIInsightCacheClearTask = nil
        return
      }
      if let refreshResult {
        self.applyFootprintResult(refreshResult)
      } else {
        self.state = .idle
      }
      switch clearResult {
      case .success(let result):
        self.aiInsightCacheClearState = .completed(result)
      case .failure(let error):
        let failure = Self.aiInsightCacheClearFailure(for: error)
        if failure == .outcomeUnknown {
          self.aiInsightCacheClearState = .outcomeUnknown
        } else {
          self.aiInsightCacheClearState = .failed(failure)
        }
      }
      self.confirmedAIInsightCacheClearTask = nil
    }
    confirmedAIInsightCacheClearTask = task
    await task.value
  }

  func cancelAIInsightCacheClear(
    _ confirmation: DuxAIInsightCacheClearConfirmation? = nil
  ) async {
    guard
      let current = aiInsightCacheClearConfirmation,
      confirmation == nil || confirmation == current
    else {
      return
    }
    generation &+= 1
    aiInsightCachePreviewExpiryTask?.cancel()
    aiInsightCachePreviewExpiryTask = nil
    let lease = aiInsightCacheClearLease
    aiInsightCacheClearLease = nil
    aiInsightCacheClearConfirmation = nil
    aiInsightCacheClearState = .idle
    if let lease {
      let release = Task { await lease.release() }
      retainForTerminal(release)
      await release.value
    }
  }

  func dismissAIInsightCacheClear(
    _ confirmation: DuxAIInsightCacheClearConfirmation? = nil
  ) async {
    if confirmation == nil,
      operationKind == .aiInsightCacheClearPreparation,
      let preparation = operationTask
    {
      generation &+= 1
      preparation.cancel()
      aiInsightCacheClearState = .idle
      await preparation.value
      operationKind = nil
      operationTask = nil
      return
    }
    await cancelAIInsightCacheClear(confirmation)
  }

  func dismissAIInsightCacheClearStatus() {
    switch aiInsightCacheClearState {
    case .completed, .failed, .outcomeUnknown:
      generation &+= 1
      aiInsightCacheClearState = .idle
    case .idle, .preparing, .awaitingConfirmation, .clearing:
      break
    }
  }

  func shutdown() async {
    if let terminalShutdownTask {
      await terminalShutdownTask.value
      return
    }
    shuttingDown = true
    generation &+= 1
    let expiryTasks = [
      previewExpiryTask,
      snapshotPreviewExpiryTask,
      aiInsightCachePreviewExpiryTask,
    ].compactMap { $0 }
    expiryTasks.forEach { $0.cancel() }
    let childWaiters = Array(terminalChildWaiters.values)
    previewExpiryTask = nil
    snapshotPreviewExpiryTask = nil
    aiInsightCachePreviewExpiryTask = nil

    let operation = operationTask
    operation?.cancel()
    let pendingLease = managedScanCacheClearLease
    let pendingSnapshotLease = snapshotStorageClearLease
    let pendingAIInsightCacheLease = aiInsightCacheClearLease
    managedScanCacheClearLease = nil
    snapshotStorageClearLease = nil
    aiInsightCacheClearLease = nil
    managedScanCacheClearConfirmation = nil
    snapshotStorageClearConfirmation = nil
    aiInsightCacheClearConfirmation = nil
    if case .awaitingConfirmation = managedScanCacheClearState {
      managedScanCacheClearState = .idle
    }
    if case .awaitingConfirmation = snapshotStorageClearState {
      snapshotStorageClearState = .idle
    }
    if case .awaitingConfirmation = aiInsightCacheClearState {
      aiInsightCacheClearState = .idle
    }
    let confirmedClear = confirmedClearTask
    let confirmedSnapshotClear = confirmedSnapshotClearTask
    let confirmedAIInsightCacheClear = confirmedAIInsightCacheClearTask
    let task = Task { @MainActor [self] in
      await pendingLease?.release()
      await pendingSnapshotLease?.release()
      await pendingAIInsightCacheLease?.release()
      await operation?.value

      // An expiry task may already have consumed the model's lease slot and
      // be suspended in its own release call. Its independent registry keeps
      // that retired task joinable after confirm, cancel, or rescheduling.
      for childWaiter in childWaiters {
        await childWaiter.value
      }

      // A confirmed clear is consume-once and may already have committed.
      // Never cancel or retry it; wait through its authoritative refresh.
      await confirmedClear?.value
      await confirmedSnapshotClear?.value
      await confirmedAIInsightCacheClear?.value

      operationTask = nil
      operationKind = nil
      confirmedClearTask = nil
      confirmedSnapshotClearTask = nil
      confirmedAIInsightCacheClearTask = nil
      aiInsightCacheClearBarrier = nil
      terminalChildWaiters.removeAll(keepingCapacity: false)
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
      switch aiInsightCacheClearState {
      case .preparing, .awaitingConfirmation, .clearing:
        aiInsightCacheClearState = .idle
      case .idle, .completed, .failed, .outcomeUnknown:
        break
      }
    }
    terminalShutdownTask = task
    await task.value
  }

  private func schedulePreviewExpiration(
    _ confirmation: DuxManagedScanCacheClearConfirmation
  ) {
    previewExpiryTask?.cancel()
    let delay = max(
      0,
      confirmation.preview.expiresAt.timeIntervalSinceNow
    )
    let task = Task { @MainActor [weak self] in
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
    previewExpiryTask = task
    retainForTerminal(task)
  }

  private func scheduleSnapshotPreviewExpiration(
    _ confirmation: DuxSnapshotStorageClearConfirmation
  ) {
    snapshotPreviewExpiryTask?.cancel()
    let delay = max(
      0,
      confirmation.preview.expiresAt.timeIntervalSinceNow
    )
    let task = Task { @MainActor [weak self] in
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
    snapshotPreviewExpiryTask = task
    retainForTerminal(task)
  }

  private func scheduleAIInsightCachePreviewExpiration(
    _ confirmation: DuxAIInsightCacheClearConfirmation
  ) {
    aiInsightCachePreviewExpiryTask?.cancel()
    let delay = max(
      0,
      confirmation.preview.expiresAt.timeIntervalSinceNow
    )
    let task = Task { @MainActor [weak self] in
      do {
        try await Task.sleep(for: .seconds(delay))
      } catch {
        return
      }
      guard
        let self,
        !self.shuttingDown,
        self.aiInsightCacheClearConfirmation == confirmation
      else {
        return
      }
      let lease = self.aiInsightCacheClearLease
      self.generation &+= 1
      self.aiInsightCacheClearLease = nil
      self.aiInsightCacheClearConfirmation = nil
      self.aiInsightCacheClearState = .failed(.previewExpired)
      await lease?.release()
    }
    aiInsightCachePreviewExpiryTask = task
    retainForTerminal(task)
  }

  private func retainForTerminal(_ operation: Task<Void, Never>) {
    let id = UUID()
    let waiter = Task { @MainActor [weak self] in
      await operation.value
      self?.terminalChildWaiters.removeValue(forKey: id)
    }
    terminalChildWaiters[id] = waiter
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

  private static func aiInsightCacheClearFailure(
    for error: Error
  ) -> DuxAIInsightCacheClearServiceError {
    error as? DuxAIInsightCacheClearServiceError ?? .internalState
  }

  private static func isValidAIInsightCachePreview(
    _ preview: DuxAIInsightCacheClearPreviewModel
  ) -> Bool {
    preview.recordCount > 0
      && preview.expiredRecordCount <= preview.recordCount
      && preview.expiredLogicalContentBytes <= preview.logicalContentBytes
      && preview.expiresAt.timeIntervalSince(preview.preparedAt)
        == DuxAIInsightCacheClearPreviewModel.lifetime
  }
}
