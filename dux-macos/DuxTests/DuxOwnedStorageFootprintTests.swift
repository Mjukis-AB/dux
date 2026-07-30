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
    XCTAssertEqual(mapped.physicalTotal.chargedBytes, 169)
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
        physicalTotal: rawOwnedStorageUsage(aiBytes + 69)
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
        physicalTotal: rawOwnedStorageUsage(168)
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

    XCTAssertEqual(mapped.physicalTotal.chargedBytes, 169)
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

  func testChartFormatterAndAccessibilityCoverZeroAndUInt64Max() {
    XCTAssertEqual(
      DuxOwnedStorageChartMath.shares(database: 0, snapshots: 0),
      .init(database: 0, snapshots: 0)
    )
    XCTAssertEqual(
      DuxOwnedStorageChartMath.shares(
        database: .max,
        snapshots: .max
      ),
      .init(database: 0.5, snapshots: 0.5)
    )
    XCTAssertEqual(
      DuxOwnedStorageChartMath.shares(
        database: .max,
        snapshots: 0
      ),
      .init(database: 1, snapshots: 0)
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
      snapshotBytes: 0
    )
    XCTAssertTrue(chartValue.contains("Database and history"))
    XCTAssertTrue(chartValue.contains("snapshots"))

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

private func validRawOwnedStorageFootprint(
  recordVersion: UInt32 = 1,
  observedAtUnixMs: Int64 = 1_234,
  database: OwnedStorageUsage = rawOwnedStorageUsage(100),
  snapshots: SnapshotStorageFootprint =
    validRawSnapshotStorageFootprint(),
  embeddedAiCache: EmbeddedAiCacheFootprint =
    EmbeddedAiCacheFootprint(
      recordVersion: 1,
      recordCount: 1,
      logicalContentBytes: 36,
      expiredRecordCount: 0,
      expiredLogicalContentBytes: 0
    ),
  physicalTotal: OwnedStorageUsage = rawOwnedStorageUsage(169)
) -> OwnedStorageFootprint {
  OwnedStorageFootprint(
    recordVersion: recordVersion,
    observedAtUnixMs: observedAtUnixMs,
    database: database,
    snapshots: snapshots,
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
    embeddedAiCache: DuxEmbeddedAiCacheFootprintModel(
      recordCount: 0,
      logicalContentBytes: 0,
      expiredRecordCount: 0,
      expiredLogicalContentBytes: 0
    ),
    physicalTotal: zero
  )
}
