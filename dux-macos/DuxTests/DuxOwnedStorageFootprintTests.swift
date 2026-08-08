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
    XCTAssertEqual(
      mapped.legacyExternalSnapshotStages.inspectedParentEntryCount,
      12
    )
    XCTAssertEqual(
      mapped.legacyExternalSnapshotStages.stageShapedEntryCount,
      2
    )
    XCTAssertTrue(mapped.legacyExternalSnapshotStages.inspectionComplete)
    XCTAssertEqual(mapped.physicalTotal.chargedBytes, 180)
    XCTAssertEqual(mapped.snapshots.availableCount, 2)
    XCTAssertEqual(mapped.snapshots.maintenanceDebt?.chargedBytes, 9)
    XCTAssertEqual(mapped.snapshots.maintenanceDebtCount, 3)
    XCTAssertEqual(mapped.embeddedAiCache.logicalContentBytes, 36)
  }

  func testAdapterAcceptsMoreThan4096ValidAiRows() throws {
    let recordCount = UInt32(4097)
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

    XCTAssertEqual(mapped.embeddedAiCache.recordCount, 4097)
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
        legacyExternalSnapshotStages:
          LegacyExternalSnapshotStageCensus(
            recordVersion: 2,
            inspectedParentEntryCount: 12,
            stageShapedEntryCount: 2,
            inspectionComplete: true
          )
      ),
      validRawOwnedStorageFootprint(
        legacyExternalSnapshotStages:
          LegacyExternalSnapshotStageCensus(
            recordVersion: 1,
            inspectedParentEntryCount:
              DuxLegacyExternalSnapshotStageCensusModel
              .maximumInspectedParentEntryCount + 1,
            stageShapedEntryCount: 2,
            inspectionComplete: false
          )
      ),
      validRawOwnedStorageFootprint(
        legacyExternalSnapshotStages:
          LegacyExternalSnapshotStageCensus(
            recordVersion: 1,
            inspectedParentEntryCount: 1,
            stageShapedEntryCount: 2,
            inspectionComplete: true
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

  func testSnapshotStorageClearPreviewMapsEveryExactExclusion() throws {
    let mapped = try EngineService.snapshotStorageClearPreview(
      validRawSnapshotStorageClearPreview()
    )

    XCTAssertEqual(mapped.eligibleSnapshotCount, 1)
    XCTAssertEqual(mapped.tombstonedResidualCount, 1)
    XCTAssertEqual(mapped.clearableCount, 2)
    XCTAssertEqual(mapped.clearable.chargedBytes, 30)
    XCTAssertEqual(mapped.protectedSnapshotCount, 1)
    XCTAssertEqual(mapped.protected.chargedBytes, 20)
    XCTAssertEqual(mapped.activeReviewCount, 1)
    XCTAssertEqual(mapped.excludedMaintenanceObjectCount, 2)
    XCTAssertEqual(mapped.excludedMaintenance.chargedBytes, 7)
  }

  func testSnapshotStorageClearPreviewRejectsMismatchedClearableCount() {
    XCTAssertThrowsError(
      try EngineService.snapshotStorageClearPreview(
        validRawSnapshotStorageClearPreview(clearableCount: 3)
      )
    ) { error in
      XCTAssertEqual(
        error as? DuxSnapshotStorageClearServiceError,
        .invalidResponse
      )
    }
  }

  func testSnapshotStorageClearPreviewRejectsContradictoryExclusions() {
    let invalid = [
      validRawSnapshotStorageClearPreview(
        protectedSnapshotCount: 0
      ),
      validRawSnapshotStorageClearPreview(
        excludedMaintenanceObjectCount: 0
      ),
      validRawSnapshotStorageClearPreview(
        protectedSnapshotCount: 0,
        protected: .init(
          recordVersion: 1,
          logicalBytes: 0,
          allocatedBytes: 0,
          chargedBytes: 0
        ),
        activeReviewCount: 1
      ),
    ]

    for value in invalid {
      XCTAssertThrowsError(
        try EngineService.snapshotStorageClearPreview(value)
      ) { error in
        XCTAssertEqual(
          error as? DuxSnapshotStorageClearServiceError,
          .invalidResponse
        )
      }
    }
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

  func testShutdownJoinsRetiredManagedCacheReleaseAfterCancelDropsSlots() async {
    let service = ManagedScanCacheClearTestService(suspendRelease: true)
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.prepareManagedScanCacheClear()
    guard let confirmation = model.managedScanCacheClearConfirmation else {
      return XCTFail("Expected exact managed scan-cache confirmation")
    }

    let cancellation = Task { @MainActor in
      await model.cancelManagedScanCacheClear(confirmation)
    }
    await service.waitForRelease()
    XCTAssertNil(model.managedScanCacheClearConfirmation)

    let completion = OwnedStorageTerminalCompletionProbe()
    let firstShutdown = Task { @MainActor in
      await model.shutdown()
      await completion.finish()
    }
    let secondShutdown = Task { @MainActor in
      await model.shutdown()
      await completion.finish()
    }
    await Task.yield()
    let completedBeforeRelease = await completion.count()
    XCTAssertEqual(completedBeforeRelease, 0)

    await service.completeRelease()
    await cancellation.value
    await firstShutdown.value
    await secondShutdown.value
    let completedAfterRelease = await completion.count()
    XCTAssertEqual(completedAfterRelease, 2)
  }

  func testShutdownJoinsExpiredManagedPreparationRelease() async {
    let service = ManagedScanCacheClearTestService(
      preview: managedScanCacheClearPreviewModel(expiresIn: -1),
      suspendRelease: true
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    let preparation = Task { @MainActor in
      await model.prepareManagedScanCacheClear()
    }
    await service.waitForRelease()

    let completion = OwnedStorageTerminalCompletionProbe()
    let shutdown = Task { @MainActor in
      await model.shutdown()
      await completion.finish()
    }
    await Task.yield()
    let completedBeforeRelease = await completion.count()
    XCTAssertEqual(completedBeforeRelease, 0)

    await service.completeRelease()
    await preparation.value
    await shutdown.value
    let completedAfterRelease = await completion.count()
    XCTAssertEqual(completedAfterRelease, 1)
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

  func testConfirmedSnapshotClearInvalidatesAndRemeasuresExactlyOnce() async {
    let before = ownedStorageFootprintModel(observedAt: 10)
    let after = ownedStorageFootprintModel(observedAt: 20)
    let service = SnapshotStorageClearTestService(
      footprintResults: [.success(before), .success(after)],
      suspendClear: true
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.load()
    await model.prepareSnapshotStorageClear()
    guard let confirmation = model.snapshotStorageClearConfirmation else {
      return XCTFail("Expected exact snapshot confirmation")
    }

    let clearing = Task { @MainActor in
      await model.confirmSnapshotStorageClear(confirmation)
    }
    await service.waitForClear()
    XCTAssertNil(model.observation)
    XCTAssertEqual(
      model.snapshotStorageClearState,
      .clearing(confirmation.preview)
    )

    await service.completeClear()
    await clearing.value

    let footprintCount = await service.footprintCount()
    let clearCount = await service.clearCount()
    let releaseCount = await service.releaseCount()
    XCTAssertEqual(model.observation, after)
    XCTAssertEqual(footprintCount, 2)
    XCTAssertEqual(clearCount, 1)
    XCTAssertEqual(releaseCount, 1)
    await model.confirmSnapshotStorageClear(confirmation)
    let clearCountAfterReplay = await service.clearCount()
    XCTAssertEqual(clearCountAfterReplay, 1)
  }

  func testSnapshotOutcomeUnknownRemeasuresOnceAndNeverRetries() async {
    let after = ownedStorageFootprintModel(observedAt: 20)
    let service = SnapshotStorageClearTestService(
      footprintResults: [
        .success(ownedStorageFootprintModel(observedAt: 10)),
        .success(after),
      ],
      clearFailure: .outcomeUnknown
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.load()
    await model.prepareSnapshotStorageClear()
    guard let confirmation = model.snapshotStorageClearConfirmation else {
      return XCTFail("Expected exact snapshot confirmation")
    }

    await model.confirmSnapshotStorageClear(confirmation)
    await model.confirmSnapshotStorageClear(confirmation)

    let footprintCount = await service.footprintCount()
    let clearCount = await service.clearCount()
    XCTAssertEqual(model.observation, after)
    XCTAssertEqual(model.snapshotStorageClearState, .outcomeUnknown)
    XCTAssertEqual(footprintCount, 2)
    XCTAssertEqual(clearCount, 1)
  }

  func testStorageClearConfirmationsAreMutuallyExclusive() async {
    let service = SnapshotStorageClearTestService()
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)

    await model.prepareSnapshotStorageClear()
    guard let confirmation = model.snapshotStorageClearConfirmation else {
      return XCTFail("Expected exact snapshot confirmation")
    }
    await model.prepareManagedScanCacheClear()

    let cachePreparationCount = await service.cachePreparationCount()
    XCTAssertEqual(cachePreparationCount, 0)
    XCTAssertEqual(
      model.snapshotStorageClearState,
      .awaitingConfirmation(confirmation)
    )
    await model.cancelSnapshotStorageClear(confirmation)
    let releaseCount = await service.releaseCount()
    XCTAssertEqual(releaseCount, 1)
  }

  func testShutdownJoinsRetiredSnapshotReleaseAfterCancelDropsSlots() async {
    let service = SnapshotStorageClearTestService(suspendRelease: true)
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.prepareSnapshotStorageClear()
    guard let confirmation = model.snapshotStorageClearConfirmation else {
      return XCTFail("Expected exact snapshot confirmation")
    }

    let cancellation = Task { @MainActor in
      await model.cancelSnapshotStorageClear(confirmation)
    }
    await service.waitForRelease()
    XCTAssertNil(model.snapshotStorageClearConfirmation)

    let completion = OwnedStorageTerminalCompletionProbe()
    let firstShutdown = Task { @MainActor in
      await model.shutdown()
      await completion.finish()
    }
    let secondShutdown = Task { @MainActor in
      await model.shutdown()
      await completion.finish()
    }
    await Task.yield()
    let completedBeforeRelease = await completion.count()
    XCTAssertEqual(completedBeforeRelease, 0)

    await service.completeRelease()
    await cancellation.value
    await firstShutdown.value
    await secondShutdown.value
    let completedAfterRelease = await completion.count()
    XCTAssertEqual(completedAfterRelease, 2)
  }

  func testShutdownJoinsExpiredSnapshotPreparationRelease() async {
    let service = SnapshotStorageClearTestService(
      preview: snapshotStorageClearPreviewModel(expiresIn: -1),
      suspendRelease: true
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    let preparation = Task { @MainActor in
      await model.prepareSnapshotStorageClear()
    }
    await service.waitForRelease()

    let completion = OwnedStorageTerminalCompletionProbe()
    let shutdown = Task { @MainActor in
      await model.shutdown()
      await completion.finish()
    }
    await Task.yield()
    let completedBeforeRelease = await completion.count()
    XCTAssertEqual(completedBeforeRelease, 0)

    await service.completeRelease()
    await preparation.value
    await shutdown.value
    let completedAfterRelease = await completion.count()
    XCTAssertEqual(completedAfterRelease, 1)
  }

  func testSnapshotPreparationRejectsUnstableAccounting() async {
    let service = SnapshotStorageClearTestService(
      footprintResults: [
        .success(
          ownedStorageFootprintModel(
            observedAt: 10,
            snapshotAccountingUnstable: true
          )
        )
      ]
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.load()

    await model.prepareSnapshotStorageClear()

    let preparationCount = await service.preparationCount()
    XCTAssertEqual(model.snapshotStorageClearState, .failed(.retryable))
    XCTAssertEqual(preparationCount, 0)
  }

  func testShutdownWaitsConsumedSnapshotClearAndSuppressesLatePresentation() async {
    let service = SnapshotStorageClearTestService(
      footprintResults: [
        .success(ownedStorageFootprintModel(observedAt: 1)),
        .success(ownedStorageFootprintModel(observedAt: 2)),
      ],
      suspendClear: true
    )
    let model = DuxOwnedStorageFootprintSettingsModel(service: service)
    await model.load()
    await model.prepareSnapshotStorageClear()
    guard let confirmation = model.snapshotStorageClearConfirmation else {
      return XCTFail("Expected exact snapshot confirmation")
    }
    let clearing = Task { @MainActor in
      await model.confirmSnapshotStorageClear(confirmation)
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
    XCTAssertEqual(model.snapshotStorageClearState, .idle)
  }

  func testSnapshotCopyNamesClearableProtectedAndExcludedConsequences() {
    let message =
      DuxOwnedStorageFootprintSettingsView
      .snapshotConfirmationMessage(for: snapshotStorageClearPreviewModel())
      .lowercased()
    for required in [
      "1 retention-eligible snapshot",
      "1 retired residual",
      "1 protected snapshot",
      "1 active review",
      "2 excluded maintenance objects",
      "orphaned snapshots",
      "snapshot-store controls",
      "database and cleanup history",
      "scan and candidate history",
      "scan cache",
      "ai content",
      "settings",
      "exclusions",
      "user files",
      "no longer be opened or compared in explorer",
      "history records remain",
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

  func testLegacyExternalStageCopyIsExplicitlyUnattributedAndNonActionable() {
    let zero = DuxLegacyExternalSnapshotStageCensusModel(
      inspectedParentEntryCount: 9,
      stageShapedEntryCount: 0,
      inspectionComplete: true
    )
    let exact = DuxLegacyExternalSnapshotStageCensusModel(
      inspectedParentEntryCount: 12,
      stageShapedEntryCount: 2,
      inspectionComplete: true
    )
    let lowerBound = DuxLegacyExternalSnapshotStageCensusModel(
      inspectedParentEntryCount: 4_096,
      stageShapedEntryCount: 3,
      inspectionComplete: false
    )

    XCTAssertEqual(
      DuxOwnedStorageFootprintSettingsView
        .legacyExternalStageStatus(zero),
      "No stage-shaped older setup entries observed."
    )
    XCTAssertTrue(
      DuxOwnedStorageFootprintSettingsView
        .legacyExternalStageStatus(exact)
        .contains("2 possible older setup remnants")
    )
    let incomplete = DuxOwnedStorageFootprintSettingsView
      .legacyExternalStageStatus(lowerBound)
      .lowercased()
    XCTAssertTrue(incomplete.contains("at least 3"))
    XCTAssertTrue(incomplete.contains("incomplete"))
    XCTAssertTrue(incomplete.contains("4096 entries"))

    let accessibility = DuxOwnedStorageFootprintSettingsView
      .legacyExternalStageAccessibilityValue(lowerBound)
      .lowercased()
    for required in [
      "ownership is unknown",
      "byte size is unknown",
      "excluded from totals",
      "not cleanup eligible",
      "no removal action",
    ] {
      XCTAssertTrue(
        accessibility.contains(required),
        "Missing accessibility copy: \(required)"
      )
    }
  }
}

private actor OwnedStorageTerminalCompletionProbe {
  private var completions = 0

  func finish() {
    completions += 1
  }

  func count() -> Int {
    completions
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
  private let releaseTracker: ManagedScanCacheReleaseTracker

  init(
    footprintResults: [Result<
      DuxOwnedStorageFootprintModel,
      DuxOwnedStorageFootprintServiceError
    >] = [],
    preview: DuxManagedScanCacheClearPreviewModel =
      managedScanCacheClearPreviewModel(),
    clearFailure: DuxManagedScanCacheClearServiceError? = nil,
    suspendPrepare: Bool = false,
    suspendClear: Bool = false,
    suspendRelease: Bool = false
  ) {
    self.preview = preview
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
    releaseTracker = ManagedScanCacheReleaseTracker(
      suspendRelease: suspendRelease
    )
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
  func waitForRelease() async { await releaseTracker.waitForRelease() }
  func completeRelease() async { await releaseTracker.completeRelease() }
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
  private var shouldSuspendRelease: Bool
  private var releaseContinuation: CheckedContinuation<Void, Never>?
  private var releaseWaiters: [CheckedContinuation<Void, Never>] = []

  init(suspendRelease: Bool = false) {
    shouldSuspendRelease = suspendRelease
  }

  func record() async {
    releases += 1
    releaseWaiters.forEach { $0.resume() }
    releaseWaiters.removeAll()
    guard shouldSuspendRelease else {
      return
    }
    shouldSuspendRelease = false
    await withCheckedContinuation { continuation in
      releaseContinuation = continuation
    }
  }

  func count() -> Int {
    releases
  }

  func waitForRelease() async {
    guard releases == 0 else {
      return
    }
    await withCheckedContinuation { continuation in
      releaseWaiters.append(continuation)
    }
  }

  func completeRelease() {
    releaseContinuation?.resume()
    releaseContinuation = nil
  }
}

private actor SnapshotStorageClearTestService:
  DuxOwnedStorageFootprintServing
{
  private var footprintResults:
    [Result<
      DuxOwnedStorageFootprintModel,
      DuxOwnedStorageFootprintServiceError
    >]
  private let preview: DuxSnapshotStorageClearPreviewModel
  private let result: DuxSnapshotStorageClearResultModel
  private let clearFailure: DuxSnapshotStorageClearServiceError?
  private var shouldSuspendClear: Bool
  private var clearContinuation: CheckedContinuation<Void, Never>?
  private var clearWaiters: [CheckedContinuation<Void, Never>] = []
  private var footprintRequests = 0
  private var preparationRequests = 0
  private var cachePreparationRequests = 0
  private var clearRequests = 0
  private let releaseTracker: SnapshotStorageClearReleaseTracker

  init(
    footprintResults: [Result<
      DuxOwnedStorageFootprintModel,
      DuxOwnedStorageFootprintServiceError
    >] = [],
    preview: DuxSnapshotStorageClearPreviewModel =
      snapshotStorageClearPreviewModel(),
    clearFailure: DuxSnapshotStorageClearServiceError? = nil,
    suspendClear: Bool = false,
    suspendRelease: Bool = false
  ) {
    self.preview = preview
    result = DuxSnapshotStorageClearResultModel(
      clearedEligibleSnapshotCount: preview.eligibleSnapshotCount,
      clearedTombstonedResidualCount: preview.tombstonedResidualCount,
      clearedCount: preview.clearableCount,
      clearedUsage: preview.clearable
    )
    self.footprintResults = footprintResults
    self.clearFailure = clearFailure
    shouldSuspendClear = suspendClear
    releaseTracker = SnapshotStorageClearReleaseTracker(
      suspendRelease: suspendRelease
    )
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
    cachePreparationRequests += 1
    throw DuxManagedScanCacheClearServiceError.unavailable
  }

  func prepareSnapshotStorageClear() async throws
    -> any DuxSnapshotStorageClearPreviewLease
  {
    preparationRequests += 1
    return SnapshotStorageClearTestLease(
      preview: preview,
      tracker: releaseTracker
    )
  }

  func clearSnapshotStorage(
    _: any DuxSnapshotStorageClearPreviewLease
  ) async throws -> DuxSnapshotStorageClearResultModel {
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
  func preparationCount() -> Int { preparationRequests }
  func cachePreparationCount() -> Int { cachePreparationRequests }
  func clearCount() -> Int { clearRequests }
  func releaseCount() async -> Int { await releaseTracker.count() }
  func waitForRelease() async { await releaseTracker.waitForRelease() }
  func completeRelease() async { await releaseTracker.completeRelease() }
}

private final class SnapshotStorageClearTestLease:
  DuxSnapshotStorageClearPreviewLease, @unchecked Sendable
{
  let preview: DuxSnapshotStorageClearPreviewModel
  private let tracker: SnapshotStorageClearReleaseTracker

  init(
    preview: DuxSnapshotStorageClearPreviewModel,
    tracker: SnapshotStorageClearReleaseTracker
  ) {
    self.preview = preview
    self.tracker = tracker
  }

  func release() async {
    await tracker.record()
  }
}

private actor SnapshotStorageClearReleaseTracker {
  private var releases = 0
  private var shouldSuspendRelease: Bool
  private var releaseContinuation: CheckedContinuation<Void, Never>?
  private var releaseWaiters: [CheckedContinuation<Void, Never>] = []

  init(suspendRelease: Bool = false) {
    shouldSuspendRelease = suspendRelease
  }

  func record() async {
    releases += 1
    releaseWaiters.forEach { $0.resume() }
    releaseWaiters.removeAll()
    guard shouldSuspendRelease else {
      return
    }
    shouldSuspendRelease = false
    await withCheckedContinuation { continuation in
      releaseContinuation = continuation
    }
  }

  func count() -> Int {
    releases
  }

  func waitForRelease() async {
    guard releases == 0 else {
      return
    }
    await withCheckedContinuation { continuation in
      releaseWaiters.append(continuation)
    }
  }

  func completeRelease() {
    releaseContinuation?.resume()
    releaseContinuation = nil
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

private func validRawSnapshotStorageClearPreview(
  recordVersion: UInt32 = 1,
  eligibleSnapshotCount: UInt32 = 1,
  tombstonedResidualCount: UInt32 = 1,
  clearableCount: UInt32 = 2,
  clearable: OwnedStorageUsage = rawOwnedStorageUsage(30),
  protectedSnapshotCount: UInt32 = 1,
  protected: OwnedStorageUsage = rawOwnedStorageUsage(20),
  activeReviewCount: UInt32 = 1,
  excludedMaintenanceObjectCount: UInt32 = 2,
  excludedMaintenance: OwnedStorageUsage = rawOwnedStorageUsage(7),
  preparedAtUnixMs: Int64 = 1000,
  expiresAtUnixMs: Int64 = 61000
) -> SnapshotStorageClearPreviewInfo {
  SnapshotStorageClearPreviewInfo(
    recordVersion: recordVersion,
    eligibleSnapshotCount: eligibleSnapshotCount,
    tombstonedResidualCount: tombstonedResidualCount,
    clearableCount: clearableCount,
    clearable: clearable,
    protectedSnapshotCount: protectedSnapshotCount,
    protected: protected,
    activeReviewCount: activeReviewCount,
    excludedMaintenanceObjectCount: excludedMaintenanceObjectCount,
    excludedMaintenance: excludedMaintenance,
    preparedAtUnixMs: preparedAtUnixMs,
    expiresAtUnixMs: expiresAtUnixMs
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
  observedAtUnixMs: Int64 = 1234,
  database: OwnedStorageUsage = rawOwnedStorageUsage(100),
  snapshots: SnapshotStorageFootprint =
    validRawSnapshotStorageFootprint(),
  managedScanCache: ManagedScanCacheFootprint =
    validRawManagedScanCacheFootprint(),
  legacyExternalSnapshotStages: LegacyExternalSnapshotStageCensus =
    LegacyExternalSnapshotStageCensus(
      recordVersion: 1,
      inspectedParentEntryCount: 12,
      stageShapedEntryCount: 2,
      inspectionComplete: true
    ),
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
    legacyExternalSnapshotStages: legacyExternalSnapshotStages,
    embeddedAiCache: embeddedAiCache,
    physicalTotal: physicalTotal
  )
}

private func ownedStorageFootprintModel(
  observedAt: TimeInterval,
  snapshotAccountingUnstable: Bool = false
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
      accountingUnstable: snapshotAccountingUnstable
    ),
    managedScanCache: DuxManagedScanCacheFootprintModel(
      controls: zero,
      entries: zero,
      temporary: zero,
      total: zero,
      entryCount: 0,
      temporaryCount: 0
    ),
    legacyExternalSnapshotStages:
      DuxLegacyExternalSnapshotStageCensusModel(
        inspectedParentEntryCount: 1,
        stageShapedEntryCount: 0,
        inspectionComplete: true
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

private func managedScanCacheClearPreviewModel(
  expiresIn: TimeInterval = 60
)
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
    expiresAt: Date().addingTimeInterval(expiresIn)
  )
}

private func snapshotStorageClearPreviewModel(
  expiresIn: TimeInterval = 60
)
  -> DuxSnapshotStorageClearPreviewModel
{
  let preparedAt = Date()
  return DuxSnapshotStorageClearPreviewModel(
    eligibleSnapshotCount: 1,
    tombstonedResidualCount: 1,
    clearableCount: 2,
    clearable: DuxOwnedStorageUsageModel(
      logicalBytes: 30,
      allocatedBytes: 30,
      chargedBytes: 30
    ),
    protectedSnapshotCount: 1,
    protected: DuxOwnedStorageUsageModel(
      logicalBytes: 20,
      allocatedBytes: 20,
      chargedBytes: 20
    ),
    activeReviewCount: 1,
    excludedMaintenanceObjectCount: 2,
    excludedMaintenance: DuxOwnedStorageUsageModel(
      logicalBytes: 7,
      allocatedBytes: 7,
      chargedBytes: 7
    ),
    preparedAt: preparedAt,
    expiresAt: preparedAt.addingTimeInterval(expiresIn)
  )
}
