import Foundation
import XCTest

@testable import DUX

final class DuxOwnedStorageFootprintAdapterTests: XCTestCase {
  func testAdapterAcceptsExactAdditivePathFreeObservation() throws {
    let mapped = try EngineService.ownedStorageFootprint(
      validRawOwnedStorageFootprint()
    )

    XCTAssertEqual(
      mapped.observedAt,
      Date(timeIntervalSince1970: 1.234)
    )
    XCTAssertEqual(mapped.database.chargedBytes, 100)
    XCTAssertEqual(mapped.snapshots.total.chargedBytes, 69)
    XCTAssertEqual(mapped.managedScanCache.total.chargedBytes, 11)
    XCTAssertEqual(mapped.managedScanCache.entryCount, 2)
    XCTAssertEqual(mapped.managedScanCache.temporaryCount, 1)
    XCTAssertEqual(mapped.managedScanCache.clearableCount, 3)
    XCTAssertEqual(mapped.managedScanCache.clearable?.chargedBytes, 10)
    XCTAssertEqual(mapped.physicalTotal.chargedBytes, 180)
    XCTAssertEqual(mapped.snapshots.availableCount, 2)
    XCTAssertEqual(mapped.snapshots.maintenanceDebt?.chargedBytes, 9)
    XCTAssertEqual(mapped.snapshots.maintenanceDebtCount, 3)
    XCTAssertEqual(mapped.embeddedAiCache.logicalContentBytes, 36)
  }

  func testAdapterAcceptsMoreThan4096ValidAiRows() throws {
    let recordCount = UInt32(4_097)
    let aiBytes =
      UInt64(recordCount)
      * DuxEmbeddedAiCacheFootprintModel.minimumContentBytesPerRecord
    let database = rawOwnedStorageUsage(aiBytes)
    let mapped = try EngineService.ownedStorageFootprint(
      validRawOwnedStorageFootprint(
        database: database,
        embeddedAiCache: EmbeddedAiCacheFootprint(
          recordVersion: 1,
          recordCount: recordCount,
          logicalContentBytes: aiBytes,
          expiredRecordCount: 0,
          expiredLogicalContentBytes: 0
        ),
        physicalTotal: rawOwnedStorageUsage(aiBytes + 69 + 11)
      )
    )

    XCTAssertEqual(mapped.embeddedAiCache.recordCount, 4_097)
    XCTAssertEqual(mapped.embeddedAiCache.logicalContentBytes, aiBytes)
  }

  func testAdapterRejectsIndependentVersionAccountingAndSchemaFailures() {
    let invalid: [OwnedStorageFootprint] = [
      validRawOwnedStorageFootprint(recordVersion: 2),
      validRawOwnedStorageFootprint(
        database: rawOwnedStorageUsage(
          logical: 100,
          allocated: 100,
          charged: 99
        )
      ),
      validRawOwnedStorageFootprint(
        database: rawOwnedStorageUsage(
          100,
          recordVersion: 2
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          recordVersion: 2
        )
      ),
      validRawOwnedStorageFootprint(
        managedScanCache: validRawManagedScanCacheFootprint(
          recordVersion: 2
        )
      ),
      validRawOwnedStorageFootprint(
        managedScanCache: validRawManagedScanCacheFootprint(
          total: rawOwnedStorageUsage(10)
        ),
        physicalTotal: rawOwnedStorageUsage(179)
      ),
      validRawOwnedStorageFootprint(
        managedScanCache: validRawManagedScanCacheFootprint(
          entryCount:
            DuxManagedScanCacheFootprintModel.maximumObjectCount,
          temporaryCount: 1
        )
      ),
      validRawOwnedStorageFootprint(
        managedScanCache: validRawManagedScanCacheFootprint(
          temporaryCount:
            DuxManagedScanCacheFootprintModel
            .maximumTemporaryObjectCount + 1
        )
      ),
      validRawOwnedStorageFootprint(
        managedScanCache: validRawManagedScanCacheFootprint(
          entryCount: 0
        )
      ),
      validRawOwnedStorageFootprint(
        managedScanCache: validRawManagedScanCacheFootprint(
          controls: rawOwnedStorageUsage(0)
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          available: rawOwnedStorageUsage(49)
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          availableCount: 3
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          capExcessBytes: 8
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          accountingUnstable: true
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          nonEvictableOverCap: true
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          tombstonedResidualCount:
            DuxSnapshotStorageFootprintModel.maximumObjectCount
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          residualTemporaryLeaseCount:
            DuxSnapshotStorageFootprintModel
            .maximumResidualTemporaryLeaseCount + 1
        )
      ),
      validRawOwnedStorageFootprint(
        snapshots: validRawSnapshotStorageFootprint(
          activePinRows:
            DuxSnapshotStorageFootprintModel.maximumPinRowCount,
          expiredPinRows: 1
        )
      ),
      validRawOwnedStorageFootprint(
        embeddedAiCache: EmbeddedAiCacheFootprint(
          recordVersion: 2,
          recordCount: 1,
          logicalContentBytes: 36,
          expiredRecordCount: 0,
          expiredLogicalContentBytes: 0
        )
      ),
      validRawOwnedStorageFootprint(
        embeddedAiCache: EmbeddedAiCacheFootprint(
          recordVersion: 1,
          recordCount: 1,
          logicalContentBytes: 35,
          expiredRecordCount: 0,
          expiredLogicalContentBytes: 0
        )
      ),
      validRawOwnedStorageFootprint(
        embeddedAiCache: EmbeddedAiCacheFootprint(
          recordVersion: 1,
          recordCount: 1,
          logicalContentBytes: 101,
          expiredRecordCount: 0,
          expiredLogicalContentBytes: 0
        )
      ),
      validRawOwnedStorageFootprint(
        physicalTotal: rawOwnedStorageUsage(179)
      ),
      validRawOwnedStorageFootprint(
        observedAtUnixMs: -1
      ),
    ]

    for value in invalid {
      XCTAssertThrowsError(
        try EngineService.ownedStorageFootprint(value)
      ) { error in
        XCTAssertEqual(
          error as? DuxOwnedStorageFootprintServiceError,
          .invalidResponse
        )
      }
    }
  }

  func testEngineServiceReadsFootprintOffMainAndMapsIt() async throws {
    let engine = OwnedStorageFootprintEngineSpy(
      response: validRawOwnedStorageFootprint()
    )
    let service = EngineService(engine: engine)

    let mapped = try await service.loadOwnedStorageFootprint()

    XCTAssertEqual(mapped.physicalTotal.chargedBytes, 180)
    XCTAssertEqual(engine.executedOnMainThread, false)
    let closed = await service.close()
    XCTAssertTrue(closed)
  }
}

@MainActor
final class DuxOwnedStorageFootprintSettingsModelTests: XCTestCase {
  func testLoadCoalescesAndPublishesOneAuthoritativeObservation() async {
    let service = ControlledOwnedStorageFootprintService()
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)

    let first = Task { await model.load() }
    await service.waitForRequestCount(1)
    let second = Task { await model.load() }
    await Task.yield()
    let requestCountBeforeResolution = await service.requestCountValue()
    XCTAssertEqual(requestCountBeforeResolution, 1)

    let observation = ownedStorageFootprintModel(observedAt: 7)
    await service.resolveNext(.success(observation))
    await first.value
    await second.value

    XCTAssertEqual(model.observation, observation)
    XCTAssertEqual(model.state, .ready)
    let requestCountAfterResolution = await service.requestCountValue()
    XCTAssertEqual(requestCountAfterResolution, 1)
  }

  func testRefreshFailurePreservesExactLastGoodObservationAndTime() async {
    let original = ownedStorageFootprintModel(observedAt: 123.456)
    let service = SequencedOwnedStorageFootprintService(
      results: [
        .success(original),
        .failure(.corruptData),
      ]
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)

    await model.load()
    await model.refresh()

    XCTAssertEqual(model.observation, original)
    XCTAssertEqual(model.observation?.observedAt, original.observedAt)
    XCTAssertEqual(model.state, .failed(.corruptData))
    let requestCount = await service.requestCountValue()
    XCTAssertEqual(requestCount, 2)
  }

  func testShutdownCancelsJoinsAndFencesLateReply() async {
    let service = ControlledOwnedStorageFootprintService()
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    let load = Task { await model.load() }
    await service.waitForRequestCount(1)

    let shutdown = Task { await model.shutdown() }
    await Task.yield()
    await service.resolveNext(
      .success(ownedStorageFootprintModel(observedAt: 99))
    )
    await shutdown.value
    await load.value

    XCTAssertNil(model.observation)
    XCTAssertEqual(model.state, .idle)
    await model.refresh()
    let requestCount = await service.requestCountValue()
    XCTAssertEqual(requestCount, 1)
  }

  func testManagedScanCacheCancellationReleasesExactPreview() async {
    let service = ManagedScanCacheClearTestService()
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)

    await model.prepareManagedScanCacheClear()
    guard let confirmation = model.managedScanCacheClearConfirmation else {
      return XCTFail("Expected exact managed scan-cache confirmation")
    }
    XCTAssertEqual(confirmation.preview.clearableCount, 3)

    await model.cancelManagedScanCacheClear(confirmation)

    XCTAssertNil(model.managedScanCacheClearConfirmation)
    XCTAssertEqual(model.managedScanCacheClearState, .idle)
    let releaseCount = await service.releaseCount()
    let clearCount = await service.clearCount()
    XCTAssertEqual(releaseCount, 1)
    XCTAssertEqual(clearCount, 0)
  }

  func testStaleManagedScanCacheConfirmationCannotClearOrCancelCurrentLease() async {
    let service = ManagedScanCacheClearTestService()
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.prepareManagedScanCacheClear()
    guard let confirmation = model.managedScanCacheClearConfirmation else {
      return XCTFail("Expected exact managed scan-cache confirmation")
    }
    let stale = DuxManagedScanCacheClearConfirmation(
      generation: confirmation.generation &+ 1,
      preview: confirmation.preview
    )

    await model.confirmManagedScanCacheClear(stale)
    await model.cancelManagedScanCacheClear(stale)

    XCTAssertEqual(
      model.managedScanCacheClearState,
      .awaitingConfirmation(confirmation)
    )
    let clearCount = await service.clearCount()
    let releaseCount = await service.releaseCount()
    XCTAssertEqual(clearCount, 0)
    XCTAssertEqual(releaseCount, 0)
    await model.cancelManagedScanCacheClear(confirmation)
  }

  func testConfirmedManagedScanCacheClearInvalidatesThenRefreshesExactlyOnce() async {
    let before = ownedStorageFootprintModel(observedAt: 10)
    let after = ownedStorageFootprintModel(observedAt: 20)
    let service = ManagedScanCacheClearTestService(
      footprintResults: [.success(before), .success(after)],
      suspendClear: true
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.load()
    await model.prepareManagedScanCacheClear()
    guard let confirmation = model.managedScanCacheClearConfirmation else {
      return XCTFail("Expected exact managed scan-cache confirmation")
    }

    let clearing = Task { @MainActor in
      await model.confirmManagedScanCacheClear(confirmation)
    }
    await service.waitForClear()
    XCTAssertNil(model.observation)
    XCTAssertEqual(
      model.managedScanCacheClearState,
      .clearing(confirmation.preview)
    )

    await service.completeClear()
    await clearing.value

    XCTAssertEqual(model.observation, after)
    let footprintCount = await service.footprintCount()
    var clearCount = await service.clearCount()
    XCTAssertEqual(footprintCount, 2)
    XCTAssertEqual(clearCount, 1)
    XCTAssertEqual(
      model.managedScanCacheClearState,
      .completed(
        DuxManagedScanCacheClearResultModel(
          clearedEntryCount: 2,
          clearedTemporaryCount: 1,
          clearedCount: 3,
          clearedUsage: confirmation.preview.clearable
        )
      )
    )

    await model.confirmManagedScanCacheClear(confirmation)
    clearCount = await service.clearCount()
    XCTAssertEqual(clearCount, 1)
  }

  func testOutcomeUnknownNeverRetriesAndFailedRefreshCannotRestoreStaleObservation() async {
    let before = ownedStorageFootprintModel(observedAt: 10)
    let service = ManagedScanCacheClearTestService(
      footprintResults: [
        .success(before),
        .failure(.unavailable),
      ],
      clearFailure: .outcomeUnknown
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.load()
    await model.prepareManagedScanCacheClear()
    guard let confirmation = model.managedScanCacheClearConfirmation else {
      return XCTFail("Expected exact managed scan-cache confirmation")
    }

    await model.confirmManagedScanCacheClear(confirmation)
    await model.confirmManagedScanCacheClear(confirmation)

    XCTAssertNil(model.observation)
    XCTAssertEqual(model.state, .failed(.unavailable))
    XCTAssertEqual(model.managedScanCacheClearState, .outcomeUnknown)
    let footprintCount = await service.footprintCount()
    let clearCount = await service.clearCount()
    XCTAssertEqual(footprintCount, 2)
    XCTAssertEqual(clearCount, 1)
  }

  func testChangedSincePreviewRefreshesDriftWithoutRetryingClear() async {
    let afterDrift = ownedStorageFootprintModel(observedAt: 20)
    let service = ManagedScanCacheClearTestService(
      footprintResults: [
        .success(ownedStorageFootprintModel(observedAt: 10)),
        .success(afterDrift),
      ],
      clearFailure: .changedSincePreview
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.load()
    await model.prepareManagedScanCacheClear()
    guard let confirmation = model.managedScanCacheClearConfirmation else {
      return XCTFail("Expected exact managed scan-cache confirmation")
    }

    await model.confirmManagedScanCacheClear(confirmation)

    XCTAssertEqual(model.observation, afterDrift)
    XCTAssertEqual(
      model.managedScanCacheClearState,
      .failed(.changedSincePreview)
    )
    let footprintCount = await service.footprintCount()
    let clearCount = await service.clearCount()
    XCTAssertEqual(footprintCount, 2)
    XCTAssertEqual(clearCount, 1)
  }

  func testDismissCancelsPreparationAndReleasesItsLatePreview() async {
    let service = ManagedScanCacheClearTestService(suspendPrepare: true)
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    let preparation = Task { @MainActor in
      await model.prepareManagedScanCacheClear()
    }
    await service.waitForPreparation()

    let dismissal = Task { @MainActor in
      await model.dismissManagedScanCacheClear()
    }
    await Task.yield()
    await service.completePreparation()
    await dismissal.value
    await preparation.value

    XCTAssertNil(model.managedScanCacheClearConfirmation)
    XCTAssertEqual(model.managedScanCacheClearState, .idle)
    let releaseCount = await service.releaseCount()
    XCTAssertEqual(releaseCount, 1)
  }

  func testShutdownWaitsConsumedClearAndSuppressesLatePresentation() async {
    let service = ManagedScanCacheClearTestService(
      footprintResults: [
        .success(ownedStorageFootprintModel(observedAt: 1)),
        .success(ownedStorageFootprintModel(observedAt: 2)),
      ],
      suspendClear: true
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.load()
    await model.prepareManagedScanCacheClear()
    guard let confirmation = model.managedScanCacheClearConfirmation else {
      return XCTFail("Expected exact managed scan-cache confirmation")
    }
    let clearing = Task { @MainActor in
      await model.confirmManagedScanCacheClear(confirmation)
    }
    await service.waitForClear()

    let shutdown = Task { @MainActor in
      await model.shutdown()
    }
    await Task.yield()
    await service.completeClear()
    await shutdown.value
    await clearing.value

    let clearCount = await service.clearCount()
    let footprintCount = await service.footprintCount()
    XCTAssertEqual(clearCount, 1)
    XCTAssertEqual(footprintCount, 2)
    XCTAssertNil(model.observation)
    XCTAssertEqual(model.state, .idle)
    XCTAssertEqual(model.managedScanCacheClearState, .idle)
  }

  func testManagedScanCacheCopyNamesEveryExcludedStoreAndFreeSpaceLimit() {
    let preview = managedScanCacheClearPreviewModel()
    let message =
      DuxOwnedStorageFootprintSettingsView.confirmationMessage(for: preview)
      .lowercased()
    for required in [
      "3 objects",
      "2 published entries",
      "1 temporary remnants",
      "outer legacy cache",
      "embedded ai",
      "database and history",
      "snapshots",
      "settings",
      "user files",
      "next cli scan may be slower",
      "not a promise",
    ] {
      XCTAssertTrue(message.contains(required), "Missing copy: \(required)")
    }
  }

  func testChartFormatterAndAccessibilityCoverZeroAndUInt64Max() {
    XCTAssertEqual(
      DuxOwnedStorageChartMath.shares(
        database: 0,
        snapshots: 0,
        managedScanCache: 0
      ),
      .init(database: 0, snapshots: 0, managedScanCache: 0)
    )
    XCTAssertEqual(
      DuxOwnedStorageChartMath.shares(
        database: .max,
        snapshots: .max,
        managedScanCache: .max
      ),
      .init(
        database: 1.0 / 3.0,
        snapshots: 1.0 / 3.0,
        managedScanCache: 1.0 / 3.0
      )
    )
    XCTAssertEqual(
      DuxOwnedStorageChartMath.shares(
        database: .max,
        snapshots: 0,
        managedScanCache: 0
      ),
      .init(database: 1, snapshots: 0, managedScanCache: 0)
    )

    let maximum = DuxOwnedStorageByteFormatter.string(
      from: .max,
      locale: Locale(identifier: "en_US")
    )
    XCTAssertTrue(maximum.contains("EiB"))
    XCTAssertTrue(maximum.contains("18,446,744,073,709,551,615 B"))
    XCTAssertFalse(maximum.contains("9,223,372,036,854,775,807 B"))

    let chartValue = DuxOwnedStorageStackedBar.accessibilityValue(
      databaseBytes: .max,
      snapshotBytes: 0,
      managedScanCacheBytes: 1
    )
    XCTAssertTrue(chartValue.contains("Database and history"))
    XCTAssertTrue(chartValue.contains("snapshots"))
    XCTAssertTrue(chartValue.contains("DUX scan cache"))

    let identifiers =
      DuxOwnedStorageFootprintAccessibility.allControlIdentifiers
    XCTAssertEqual(Set(identifiers).count, identifiers.count)
    XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
    XCTAssertTrue(
      Set(identifiers).isDisjoint(
        with: CleanupHistoryClearAccessibility.allControlIdentifiers
      )
    )
    XCTAssertNotEqual(
      DuxOwnedStorageFootprintSettingsView.message(for: .unsafeStorage),
      DuxOwnedStorageFootprintSettingsView.message(
        for: .incompatibleSchema
      )
    )
  }
}

private final class OwnedStorageFootprintEngineSpy:
  DuxEngine, @unchecked Sendable
{
  private let lock = NSLock()
  private let response: OwnedStorageFootprint
  private var wasMainThread: Bool?

  var executedOnMainThread: Bool? {
    lock.withLock { wasMainThread }
  }

  required init(unsafeFromHandle handle: UInt64) {
    fatalError("Unsupported test initializer: \(handle)")
  }

  init(response: OwnedStorageFootprint) {
    self.response = response
    super.init(noHandle: NoHandle())
  }

  override func getOwnedStorageFootprint() throws
    -> OwnedStorageFootprint
  {
    lock.withLock {
      wasMainThread = Thread.isMainThread
    }
    return response
  }

  override func close() -> Bool {
    true
  }
}

private actor ControlledOwnedStorageFootprintService:
  DuxOwnedStorageFootprintServing
{
  private(set) var requestCount = 0
  private var requests: [CheckedContinuation<DuxOwnedStorageFootprintModel, Error>] = []
  private var countWaiters: [CheckedContinuation<Void, Never>] = []

  func loadOwnedStorageFootprint() async throws
    -> DuxOwnedStorageFootprintModel
  {
    requestCount += 1
    resumeCountWaiters()
    return try await withCheckedThrowingContinuation { continuation in
      requests.append(continuation)
    }
  }

  func waitForRequestCount(_ expected: Int) async {
    while requestCount < expected {
      await withCheckedContinuation { continuation in
        countWaiters.append(continuation)
      }
    }
  }

  func resolveNext(
    _ result: Result<
      DuxOwnedStorageFootprintModel,
      DuxOwnedStorageFootprintServiceError
    >
  ) {
    precondition(!requests.isEmpty)
    requests.removeFirst().resume(with: result.mapError { $0 as Error })
  }

  func requestCountValue() -> Int {
    requestCount
  }

  private func resumeCountWaiters() {
    let waiters = countWaiters
    countWaiters.removeAll()
    for waiter in waiters {
      waiter.resume()
    }
  }
}

private actor SequencedOwnedStorageFootprintService:
  DuxOwnedStorageFootprintServing
{
  private var results:
    [Result<
      DuxOwnedStorageFootprintModel,
      DuxOwnedStorageFootprintServiceError
    >]
  private(set) var requestCount = 0

  init(
    results: [Result<
      DuxOwnedStorageFootprintModel,
      DuxOwnedStorageFootprintServiceError
    >]
  ) {
    self.results = results
  }

  func loadOwnedStorageFootprint() async throws
    -> DuxOwnedStorageFootprintModel
  {
    requestCount += 1
    return try results.removeFirst().get()
  }

  func requestCountValue() -> Int {
    requestCount
  }
}

private actor ManagedScanCacheClearTestService:
  DuxOwnedStorageFootprintServing
{
  private var footprintResults:
    [Result<
      DuxOwnedStorageFootprintModel,
      DuxOwnedStorageFootprintServiceError
    >]
  private let preview: DuxManagedScanCacheClearPreviewModel
  private let result: DuxManagedScanCacheClearResultModel
  private let clearFailure: DuxManagedScanCacheClearServiceError?
  private var shouldSuspendPrepare: Bool
  private var shouldSuspendClear: Bool
  private var prepareContinuation:
    CheckedContinuation<any DuxManagedScanCacheClearPreviewLease, Never>?
  private var clearContinuation: CheckedContinuation<Void, Never>?
  private var prepareWaiters: [CheckedContinuation<Void, Never>] = []
  private var clearWaiters: [CheckedContinuation<Void, Never>] = []
  private var footprintRequests = 0
  private var clearRequests = 0
  private let releaseTracker = ManagedScanCacheReleaseTracker()

  init(
    footprintResults: [Result<
      DuxOwnedStorageFootprintModel,
      DuxOwnedStorageFootprintServiceError
    >] = [],
    clearFailure: DuxManagedScanCacheClearServiceError? = nil,
    suspendPrepare: Bool = false,
    suspendClear: Bool = false
  ) {
    preview = managedScanCacheClearPreviewModel()
    result = DuxManagedScanCacheClearResultModel(
      clearedEntryCount: preview.entryCount,
      clearedTemporaryCount: preview.temporaryCount,
      clearedCount: preview.clearableCount,
      clearedUsage: preview.clearable
    )
    self.footprintResults = footprintResults
    self.clearFailure = clearFailure
    shouldSuspendPrepare = suspendPrepare
    shouldSuspendClear = suspendClear
  }

  func loadOwnedStorageFootprint() async throws
    -> DuxOwnedStorageFootprintModel
  {
    footprintRequests += 1
    guard !footprintResults.isEmpty else {
      return ownedStorageFootprintModel(
        observedAt: TimeInterval(footprintRequests)
      )
    }
    return try footprintResults.removeFirst().get()
  }

  func prepareManagedScanCacheClear() async throws
    -> any DuxManagedScanCacheClearPreviewLease
  {
    prepareWaiters.forEach { $0.resume() }
    prepareWaiters.removeAll()
    let lease = ManagedScanCacheTestLease(
      preview: preview,
      tracker: releaseTracker
    )
    guard shouldSuspendPrepare else {
      return lease
    }
    shouldSuspendPrepare = false
    return await withCheckedContinuation { continuation in
      prepareContinuation = continuation
    }
  }

  func clearManagedScanCache(
    _: any DuxManagedScanCacheClearPreviewLease
  ) async throws -> DuxManagedScanCacheClearResultModel {
    clearRequests += 1
    clearWaiters.forEach { $0.resume() }
    clearWaiters.removeAll()
    if shouldSuspendClear {
      shouldSuspendClear = false
      await withCheckedContinuation { continuation in
        clearContinuation = continuation
      }
    }
    if let clearFailure {
      throw clearFailure
    }
    return result
  }

  func waitForPreparation() async {
    if prepareContinuation != nil {
      return
    }
    await withCheckedContinuation { continuation in
      prepareWaiters.append(continuation)
    }
  }

  func completePreparation() {
    let lease = ManagedScanCacheTestLease(
      preview: preview,
      tracker: releaseTracker
    )
    prepareContinuation?.resume(returning: lease)
    prepareContinuation = nil
  }

  func waitForClear() async {
    if clearRequests > 0 {
      return
    }
    await withCheckedContinuation { continuation in
      clearWaiters.append(continuation)
    }
  }

  func completeClear() {
    clearContinuation?.resume()
    clearContinuation = nil
  }

  func footprintCount() -> Int { footprintRequests }
  func clearCount() -> Int { clearRequests }
  func releaseCount() async -> Int { await releaseTracker.count() }
}

private final class ManagedScanCacheTestLease:
  DuxManagedScanCacheClearPreviewLease, @unchecked Sendable
{
  let preview: DuxManagedScanCacheClearPreviewModel
  private let tracker: ManagedScanCacheReleaseTracker

  init(
    preview: DuxManagedScanCacheClearPreviewModel,
    tracker: ManagedScanCacheReleaseTracker
  ) {
    self.preview = preview
    self.tracker = tracker
  }

  func release() async {
    await tracker.record()
  }
}

private actor ManagedScanCacheReleaseTracker {
  private var releases = 0

  func record() {
    releases += 1
  }

  func count() -> Int {
    releases
  }
}

private func rawOwnedStorageUsage(
  _ value: UInt64,
  recordVersion: UInt32 = 1
) -> OwnedStorageUsage {
  rawOwnedStorageUsage(
    logical: value,
    allocated: value,
    charged: value,
    recordVersion: recordVersion
  )
}

private func rawOwnedStorageUsage(
  logical: UInt64,
  allocated: UInt64,
  charged: UInt64,
  recordVersion: UInt32 = 1
) -> OwnedStorageUsage {
  OwnedStorageUsage(
    recordVersion: recordVersion,
    logicalBytes: logical,
    allocatedBytes: allocated,
    chargedBytes: charged
  )
}

private func validRawSnapshotStorageFootprint(
  recordVersion: UInt32 = 1,
  available: OwnedStorageUsage = rawOwnedStorageUsage(50),
  availableCount: UInt32 = 2,
  capExcessBytes: UInt64 = 9,
  tombstonedResidualCount: UInt32 = 1,
  residualTemporaryLeaseCount: UInt32 = 0,
  activePinRows: UInt32 = 0,
  expiredPinRows: UInt32 = 0,
  nonEvictableOverCap: Bool = false,
  accountingUnstable: Bool = false
) -> SnapshotStorageFootprint {
  SnapshotStorageFootprint(
    recordVersion: recordVersion,
    capBytes: 60,
    capExcessBytes: capExcessBytes,
    controls: rawOwnedStorageUsage(10),
    available: available,
    protected: rawOwnedStorageUsage(20),
    retentionEligible: rawOwnedStorageUsage(30),
    tombstonedResidual: rawOwnedStorageUsage(2),
    orphan: rawOwnedStorageUsage(3),
    temporaryActive: rawOwnedStorageUsage(0),
    temporaryQuiescent: rawOwnedStorageUsage(4),
    temporaryUnleased: rawOwnedStorageUsage(0),
    total: rawOwnedStorageUsage(69),
    availableCount: availableCount,
    protectedCount: 1,
    retentionEligibleCount: 1,
    tombstonedResidualCount: tombstonedResidualCount,
    orphanCount: 1,
    activeTemporaryCount: 0,
    quiescentTemporaryCount: 1,
    unleasedTemporaryCount: 0,
    residualTemporaryLeaseCount: residualTemporaryLeaseCount,
    activePinRows: activePinRows,
    expiredPinRows: expiredPinRows,
    nonEvictableOverCap: nonEvictableOverCap,
    accountingUnstable: accountingUnstable
  )
}

private func validRawManagedScanCacheFootprint(
  recordVersion: UInt32 = 1,
  controls: OwnedStorageUsage = rawOwnedStorageUsage(1),
  entries: OwnedStorageUsage = rawOwnedStorageUsage(7),
  temporary: OwnedStorageUsage = rawOwnedStorageUsage(3),
  total: OwnedStorageUsage = rawOwnedStorageUsage(11),
  entryCount: UInt32 = 2,
  temporaryCount: UInt32 = 1
) -> ManagedScanCacheFootprint {
  ManagedScanCacheFootprint(
    recordVersion: recordVersion,
    controls: controls,
    entries: entries,
    temporary: temporary,
    total: total,
    entryCount: entryCount,
    temporaryCount: temporaryCount
  )
}

private func validRawOwnedStorageFootprint(
  recordVersion: UInt32 = 1,
  observedAtUnixMs: Int64 = 1_234,
  database: OwnedStorageUsage = rawOwnedStorageUsage(100),
  snapshots: SnapshotStorageFootprint =
    validRawSnapshotStorageFootprint(),
  managedScanCache: ManagedScanCacheFootprint =
    validRawManagedScanCacheFootprint(),
  embeddedAiCache: EmbeddedAiCacheFootprint =
    EmbeddedAiCacheFootprint(
      recordVersion: 1,
      recordCount: 1,
      logicalContentBytes: 36,
      expiredRecordCount: 0,
      expiredLogicalContentBytes: 0
    ),
  physicalTotal: OwnedStorageUsage = rawOwnedStorageUsage(180)
) -> OwnedStorageFootprint {
  OwnedStorageFootprint(
    recordVersion: recordVersion,
    observedAtUnixMs: observedAtUnixMs,
    database: database,
    snapshots: snapshots,
    managedScanCache: managedScanCache,
    embeddedAiCache: embeddedAiCache,
    physicalTotal: physicalTotal
  )
}

private func ownedStorageFootprintModel(
  observedAt: TimeInterval
) -> DuxOwnedStorageFootprintModel {
  let zero = DuxOwnedStorageUsageModel(
    logicalBytes: 0,
    allocatedBytes: 0,
    chargedBytes: 0
  )
  return DuxOwnedStorageFootprintModel(
    observedAt: Date(timeIntervalSince1970: observedAt),
    database: zero,
    snapshots: DuxSnapshotStorageFootprintModel(
      capBytes: 0,
      capExcessBytes: 0,
      controls: zero,
      available: zero,
      protected: zero,
      retentionEligible: zero,
      tombstonedResidual: zero,
      orphan: zero,
      temporaryActive: zero,
      temporaryQuiescent: zero,
      temporaryUnleased: zero,
      total: zero,
      availableCount: 0,
      protectedCount: 0,
      retentionEligibleCount: 0,
      tombstonedResidualCount: 0,
      orphanCount: 0,
      activeTemporaryCount: 0,
      quiescentTemporaryCount: 0,
      unleasedTemporaryCount: 0,
      residualTemporaryLeaseCount: 0,
      activePinRows: 0,
      expiredPinRows: 0,
      nonEvictableOverCap: false,
      accountingUnstable: false
    ),
    managedScanCache: DuxManagedScanCacheFootprintModel(
      controls: zero,
      entries: zero,
      temporary: zero,
      total: zero,
      entryCount: 0,
      temporaryCount: 0
    ),
    embeddedAiCache: DuxEmbeddedAiCacheFootprintModel(
      recordCount: 0,
      logicalContentBytes: 0,
      expiredRecordCount: 0,
      expiredLogicalContentBytes: 0
    ),
    physicalTotal: zero
  )
}

private func managedScanCacheClearPreviewModel()
  -> DuxManagedScanCacheClearPreviewModel
{
  DuxManagedScanCacheClearPreviewModel(
    entryCount: 2,
    temporaryCount: 1,
    clearableCount: 3,
    clearable: DuxOwnedStorageUsageModel(
      logicalBytes: 10,
      allocatedBytes: 10,
      chargedBytes: 10
    ),
    preparedAt: Date(),
    expiresAt: Date().addingTimeInterval(60)
  )
}
