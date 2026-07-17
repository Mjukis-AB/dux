import XCTest
@testable import DUX

final class EngineServiceTests: XCTestCase {
    @MainActor
    func testDefaultInitializationDefersEngineOpenOffMainActor() async throws {
        let baseline = liveEngineInstanceCount()
        let service = EngineService()
        XCTAssertEqual(liveEngineInstanceCount(), baseline)

        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    @MainActor
    func testFirstRealEngineOpenIsLazyAndRunsOffMainActor() async throws {
        let baseline = liveEngineInstanceCount()

        do {
            let fixture = try TestStorageRootsFixture()
            let service = EngineService(storageRoots: fixture.storageRoots)
            XCTAssertEqual(liveEngineInstanceCount(), baseline)

            let result = try await service.loadStatus()
            XCTAssertTrue(result.executedOffMainThread)
            XCTAssertEqual(liveEngineInstanceCount(), baseline + 1)
            let closed = await service.close()
            XCTAssertTrue(closed)
        }

        XCTAssertEqual(liveEngineInstanceCount(), baseline)
    }

    func testTransientEngineOpenFailureIsNotCached() async throws {
        let fixture = try TestStorageRootsFixture()
        let dataRoot = URL(fileURLWithPath: fixture.storageRoots.dataRoot, isDirectory: true)
        XCTAssertTrue(FileManager.default.createFile(atPath: dataRoot.path, contents: Data()))
        let service = EngineService(storageRoots: fixture.storageRoots)

        do {
            _ = try await service.loadStatus()
            XCTFail("Expected the obstructed storage root to fail")
        } catch {
            XCTAssertTrue(error is EngineServiceError)
        }

        // DUX-DESTRUCTIVE: allow=test-swift-retry-obstruction-remove -- remove only this test fixture's deliberate file obstruction
        try FileManager.default.removeItem(at: dataRoot)
        let status = try await service.loadStatus()
        XCTAssertEqual(status.ffiContractVersion, 5)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testLoadsTypedRustValuesOffTheMainThread() async throws {
        let fixture = try TestEngineFixture()
        let result = try await EngineService(engine: fixture.engine).loadStatus()

        XCTAssertEqual(result.libraryVersion, "0.5.0")
        XCTAssertEqual(result.ffiContractVersion, 5)
        XCTAssertTrue(result.executedOffMainThread)
    }

    func testRealEngineClassifiesAndPersistsStartupVolumeOffMainThread() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let gib: UInt64 = 1_024 * 1_024 * 1_024
        let snapshot = VolumeCapacitySnapshot(
            stableVolumeID: "01234567-89AB-CDEF-0123-456789ABCDEF",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 1_024 * gib,
            filesystemAvailableBytes: 100 * gib,
            importantAvailableBytes: 20 * gib,
            effectiveAvailableBytes: 20 * gib,
            availabilityBasis: .importantUsage,
            pressure: .unknown,
            criticalBoundaryBytes: nil,
            warningBoundaryBytes: nil,
            historyDisposition: nil,
            sampledAt: Date(timeIntervalSince1970: 3_600)
        )

        let status = try await service.observeVolumeCapacity(snapshot)

        XCTAssertEqual(status.pressure, .warning)
        XCTAssertEqual(status.criticalBoundaryBytes, 10 * gib)
        XCTAssertEqual(status.warningBoundaryBytes, 30 * gib)
        XCTAssertEqual(status.historyDisposition, .stored)
        XCTAssertEqual(
            status.stableVolumeID,
            "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        )
    }

    func testRealEngineRejectsMalformedVolumeIdentityWithTypedError() async throws {
        let fixture = try TestEngineFixture()
        let service = EngineService(engine: fixture.engine)
        let snapshot = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)

        do {
            _ = try await service.observeVolumeCapacity(snapshot)
            XCTFail("Expected malformed stable identity to be rejected")
        } catch let error as EngineServiceError {
            XCTAssertEqual(error, .invalidCapacityObservation)
        }
    }

    func testLinkedEngineRejectsUnknownStartupVolumeRecordVersion() throws {
        let fixture = try TestEngineFixture()
        let gib: UInt64 = 1_024 * 1_024 * 1_024

        XCTAssertThrowsError(
            try fixture.engine.observeStartupVolume(
                observation: StartupVolumeObservation(
                    recordVersion: 2,
                    stableVolumeId: "01234567-89AB-CDEF-0123-456789ABCDEF",
                    displayName: "Macintosh HD",
                    filesystem: "APFS",
                    isInternal: true,
                    isRemovable: false,
                    sampledAtUnixMs: 1,
                    totalBytes: 1_024 * gib,
                    ordinaryAvailableBytes: 100 * gib,
                    importantAvailableBytes: 100 * gib
                )
            )
        ) { error in
            XCTAssertEqual(error as? EngineError, .InvalidCapacityObservation)
        }
    }

    func testVolumeStatusValidationRejectsContradictoryHistoryDisposition() {
        let gib: UInt64 = 1_024 * 1_024 * 1_024
        let snapshot = VolumeCapacitySnapshot(
            stableVolumeID: "01234567-89AB-CDEF-0123-456789ABCDEF",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 1_024 * gib,
            filesystemAvailableBytes: 100 * gib,
            importantAvailableBytes: 20 * gib,
            effectiveAvailableBytes: 20 * gib,
            availabilityBasis: .importantUsage,
            pressure: .unknown,
            criticalBoundaryBytes: nil,
            warningBoundaryBytes: nil,
            historyDisposition: nil,
            sampledAt: Date(timeIntervalSince1970: 1)
        )
        let stableID = "volume:macos:01234567-89ab-cdef-0123-456789abcdef"
        let valid = StartupVolumeStatus(
            recordVersion: 1,
            stableVolumeId: stableID,
            sampledAtUnixMs: 1_000,
            totalBytes: snapshot.totalBytes,
            ordinaryAvailableBytes: snapshot.filesystemAvailableBytes,
            importantAvailableBytes: snapshot.importantAvailableBytes,
            headlineAvailableBytes: snapshot.effectiveAvailableBytes,
            headlineSource: .importantUsage,
            pressure: .warning,
            previousDurablePressure: nil,
            criticalBoundaryBytes: 10 * gib,
            warningBoundaryBytes: 30 * gib,
            historyDisposition: .stored
        )
        let contradictory = StartupVolumeStatus(
            recordVersion: valid.recordVersion,
            stableVolumeId: valid.stableVolumeId,
            sampledAtUnixMs: valid.sampledAtUnixMs,
            totalBytes: valid.totalBytes,
            ordinaryAvailableBytes: valid.ordinaryAvailableBytes,
            importantAvailableBytes: valid.importantAvailableBytes,
            headlineAvailableBytes: valid.headlineAvailableBytes,
            headlineSource: valid.headlineSource,
            pressure: valid.pressure,
            previousDurablePressure: valid.previousDurablePressure,
            criticalBoundaryBytes: valid.criticalBoundaryBytes,
            warningBoundaryBytes: valid.warningBoundaryBytes,
            historyDisposition: .notStoredMissingOrdinaryAvailability
        )

        XCTAssertTrue(EngineService.hasConsistentHistoryEvidence(valid, observation: snapshot))
        XCTAssertFalse(
            EngineService.hasConsistentHistoryEvidence(contradictory, observation: snapshot)
        )
    }

    @MainActor
    func testAppModelPublishesLoadedStateOnTheMainActor() async {
        guard let fixture = try? TestEngineFixture() else {
            return XCTFail("Expected a temporary engine")
        }
        let model = AppModel(engineService: EngineService(engine: fixture.engine))

        await model.loadEngineStatus()

        guard case let .loaded(result) = model.engineState else {
            return XCTFail("Expected the app model to publish a loaded result")
        }
        XCTAssertTrue(result.executedOffMainThread)
    }

    func testEngineCloseIsIdempotentAndRejectsUseAfterClose() throws {
        let baseline = liveEngineInstanceCount()
        weak var weakEngine: DuxEngine?

        do {
            let fixture = try TestEngineFixture()
            let engine = fixture.engine
            weakEngine = engine

            XCTAssertEqual(liveEngineInstanceCount(), baseline + 1)
            XCTAssertEqual(try engine.libraryVersion().ffiContractVersion, 5)
            XCTAssertTrue(engine.close())
            XCTAssertTrue(engine.close())
            XCTAssertThrowsError(try engine.formatSize(bytes: 1_536)) { error in
                XCTAssertEqual(error as? EngineError, .Closed)
            }
        }

        XCTAssertNil(weakEngine)
        XCTAssertEqual(liveEngineInstanceCount(), baseline)
    }

    func testEngineServiceMapsClosedError() async {
        guard let fixture = try? TestEngineFixture() else {
            return XCTFail("Expected a temporary engine")
        }
        let engine = fixture.engine
        let service = EngineService(engine: engine)

        let firstClose = await service.close()
        let secondClose = await service.close()
        XCTAssertTrue(firstClose)
        XCTAssertTrue(secondClose)

        do {
            _ = try await service.loadStatus()
            XCTFail("Expected a closed engine error")
        } catch let error as EngineServiceError {
            XCTAssertEqual(error, .closed)
        } catch {
            XCTFail("Expected EngineServiceError.closed, got \(error)")
        }
    }

    @MainActor
    func testConcurrentSceneLoadsUseOneEngineAndVolumeRequest() async {
        let engineService = CountingEngineService()
        let volumeMonitor = CountingVolumeMonitor()
        let model = AppModel(
            engineService: engineService,
            volumeMonitor: volumeMonitor
        )

        async let menuLoad: Void = model.loadInitialState()
        async let explorerLoad: Void = model.loadInitialState()
        _ = await (menuLoad, explorerLoad)
        await model.loadInitialState()

        let engineLoadCount = await engineService.currentLoadCount()
        let volumeObservationCount = await engineService.currentVolumeObservationCount()
        let volumeLoadCount = await volumeMonitor.currentLoadCount()
        XCTAssertEqual(engineLoadCount, 1)
        XCTAssertEqual(volumeObservationCount, 1)
        XCTAssertEqual(volumeLoadCount, 1)
        guard case .loaded = model.engineState else {
            return XCTFail("Expected one shared loaded engine state")
        }
        guard case .loaded = model.volumeState else {
            return XCTFail("Expected one shared loaded volume state")
        }
    }

    @MainActor
    func testConcurrentVolumeRefreshesCoalesceAndPublishOneResult() async {
        let monitor = ControllableVolumeMonitor()
        let model = AppModel(
            engineService: CountingEngineService(),
            volumeMonitor: monitor
        )
        let expected = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)

        let first = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(1)
        XCTAssertEqual(model.volumeState, .loading)

        let second = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await Task.yield()
        let requestCountWhileCoalesced = await monitor.currentRequestCount()
        XCTAssertEqual(requestCountWhileCoalesced, 1)

        await monitor.succeed(request: 1, with: expected)
        await first.value
        await second.value

        XCTAssertEqual(model.volumeState, .loaded(expected))
        let finalRequestCount = await monitor.currentRequestCount()
        XCTAssertEqual(finalRequestCount, 1)
    }

    @MainActor
    func testCachedRefreshFailureBecomesStaleAndCanRetry() async {
        let monitor = ControllableVolumeMonitor()
        let model = AppModel(
            engineService: CountingEngineService(),
            volumeMonitor: monitor
        )
        let cached = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)
        let refreshed = makeVolumeSnapshot(availableBytes: 40, sampledAt: 2)

        let initial = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(1)
        await monitor.succeed(request: 1, with: cached)
        await initial.value

        let failedRefresh = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(2)
        XCTAssertEqual(model.volumeState, .refreshing(cached))
        await monitor.fail(request: 2, with: VolumeMonitorError.invalidCapacity)
        await failedRefresh.value
        XCTAssertEqual(model.volumeState, .stale(cached, .invalidObservation))

        let retry = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(3)
        XCTAssertEqual(model.volumeState, .refreshing(cached))
        await monitor.succeed(request: 3, with: refreshed)
        await retry.value

        XCTAssertEqual(model.volumeState, .loaded(refreshed))
    }

    @MainActor
    func testUncachedFailureIsTypedAndCanRetry() async {
        let monitor = ControllableVolumeMonitor()
        let model = AppModel(
            engineService: CountingEngineService(),
            volumeMonitor: monitor
        )
        let expected = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)

        let failedRefresh = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(1)
        await monitor.fail(request: 1, with: VolumeMonitorError.missingAvailableCapacity)
        await failedRefresh.value
        XCTAssertEqual(model.volumeState, .failed(.unavailable))

        let retry = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(2)
        XCTAssertEqual(model.volumeState, .loading)
        await monitor.succeed(request: 2, with: expected)
        await retry.value
        XCTAssertEqual(model.volumeState, .loaded(expected))
    }

    @MainActor
    func testCancellationRestoresCacheAndRejectsLatePublication() async {
        let monitor = ControllableVolumeMonitor()
        let model = AppModel(
            engineService: CountingEngineService(),
            volumeMonitor: monitor
        )
        let cached = makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)
        let cancelledResult = makeVolumeSnapshot(availableBytes: 5, sampledAt: 2)
        let current = makeVolumeSnapshot(availableBytes: 40, sampledAt: 3)

        let initial = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(1)
        await monitor.succeed(request: 1, with: cached)
        await initial.value

        let cancelledRefresh = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(2)
        XCTAssertEqual(model.volumeState, .refreshing(cached))
        model.cancelVolumeRefresh()
        XCTAssertEqual(model.volumeState, .loaded(cached))

        let replacementRefresh = Task { @MainActor in
            await model.refreshVolumeCapacity()
        }
        await monitor.waitForRequest(3)
        await monitor.succeed(request: 3, with: current)
        await replacementRefresh.value
        XCTAssertEqual(model.volumeState, .loaded(current))

        await monitor.succeed(request: 2, with: cancelledResult)
        await cancelledRefresh.value
        XCTAssertEqual(model.volumeState, .loaded(current))
    }

    @MainActor
    func testEngineFailureCanRetryWithoutChangingVolumeState() async {
        let engineService = FlakyEngineService()
        let model = AppModel(
            engineService: engineService,
            volumeMonitor: CountingVolumeMonitor()
        )

        await model.loadEngineStatus()
        guard case .failed = model.engineState else {
            return XCTFail("Expected the first engine request to fail")
        }
        XCTAssertEqual(model.volumeState, .idle)

        await model.loadEngineStatus()
        guard case .loaded = model.engineState else {
            return XCTFail("Expected the engine request to retry")
        }
        let engineLoadCount = await engineService.currentLoadCount()
        XCTAssertEqual(engineLoadCount, 2)
        XCTAssertEqual(model.volumeState, .idle)
    }

    func testApplicationRunsAsMenuBarAgent() {
        XCTAssertEqual(Bundle.main.object(forInfoDictionaryKey: "LSUIElement") as? Bool, true)
    }
}

private actor CountingEngineService: EngineServing {
    private var loadCount = 0
    private var volumeObservationCount = 0

    func loadStatus() async throws -> EngineStatus {
        loadCount += 1
        await Task.yield()
        return EngineStatus(
            libraryVersion: "test",
            ffiContractVersion: 5,
            executedOffMainThread: true
        )
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        volumeObservationCount += 1
        return snapshot
    }

    func currentLoadCount() -> Int {
        loadCount
    }

    func currentVolumeObservationCount() -> Int {
        volumeObservationCount
    }
}

private final class TestEngineFixture {
    let engine: DuxEngine

    private let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory
            .appending(path: "dux-swift-tests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        engine = try DuxEngine(
            storage: EngineStorageRoots(
                dataRoot: root.appending(path: "data", directoryHint: .isDirectory).path,
                cacheRoot: root.appending(path: "cache", directoryHint: .isDirectory).path
            )
        )
    }

    deinit {
        _ = engine.close()
        // DUX-DESTRUCTIVE: allow=test-swift-storage-roots-fixture-remove -- remove only this fixture's UUID-named temporary root
        try? FileManager.default.removeItem(at: root)
    }
}

private final class TestStorageRootsFixture {
    let storageRoots: EngineStorageRoots

    private let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory
            .appending(path: "dux-swift-lazy-tests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        storageRoots = EngineStorageRoots(
            dataRoot: root.appending(path: "data", directoryHint: .isDirectory).path,
            cacheRoot: root.appending(path: "cache", directoryHint: .isDirectory).path
        )
    }

    deinit {
        // DUX-DESTRUCTIVE: allow=test-swift-engine-fixture-remove -- remove only this fixture's UUID-named temporary root
        try? FileManager.default.removeItem(at: root)
    }
}

private actor CountingVolumeMonitor: VolumeMonitoring {
    private var loadCount = 0

    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot {
        loadCount += 1
        await Task.yield()
        return makeVolumeSnapshot(availableBytes: 30, sampledAt: 1)
    }

    func currentLoadCount() -> Int {
        loadCount
    }
}

private actor ControllableVolumeMonitor: VolumeMonitoring {
    private var requestCount = 0
    private var requests: [Int: CheckedContinuation<VolumeCapacitySnapshot, any Error>] = [:]
    private var requestWaiters: [
        (expected: Int, continuation: CheckedContinuation<Void, Never>)
    ] = []

    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot {
        requestCount += 1
        let request = requestCount
        resumeSatisfiedWaiters()
        return try await withCheckedThrowingContinuation { continuation in
            requests[request] = continuation
        }
    }

    func waitForRequest(_ expected: Int) async {
        guard requestCount < expected else {
            return
        }
        await withCheckedContinuation { continuation in
            requestWaiters.append((expected, continuation))
        }
    }

    func currentRequestCount() -> Int {
        requestCount
    }

    func succeed(request: Int, with snapshot: VolumeCapacitySnapshot) {
        guard let continuation = requests.removeValue(forKey: request) else {
            preconditionFailure("No pending volume request \(request)")
        }
        continuation.resume(returning: snapshot)
    }

    func fail(request: Int, with error: any Error) {
        guard let continuation = requests.removeValue(forKey: request) else {
            preconditionFailure("No pending volume request \(request)")
        }
        continuation.resume(throwing: error)
    }

    private func resumeSatisfiedWaiters() {
        let satisfied = requestWaiters.filter { $0.expected <= requestCount }
        requestWaiters.removeAll { $0.expected <= requestCount }
        for waiter in satisfied {
            waiter.continuation.resume()
        }
    }
}

private actor FlakyEngineService: EngineServing {
    private var loadCount = 0

    func loadStatus() async throws -> EngineStatus {
        loadCount += 1
        guard loadCount > 1 else {
            throw EngineServiceError.unexpected("transient test failure")
        }
        return EngineStatus(
            libraryVersion: "test",
            ffiContractVersion: 5,
            executedOffMainThread: true
        )
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func currentLoadCount() -> Int {
        loadCount
    }
}

private func makeVolumeSnapshot(
    availableBytes: UInt64,
    sampledAt: TimeInterval
) -> VolumeCapacitySnapshot {
    VolumeCapacitySnapshot(
        stableVolumeID: "test-volume",
        displayName: "Test Volume",
        filesystem: "APFS",
        isInternal: true,
        isRemovable: false,
        totalBytes: 100,
        filesystemAvailableBytes: 20,
        importantAvailableBytes: availableBytes,
        effectiveAvailableBytes: availableBytes,
        availabilityBasis: .importantUsage,
        pressure: .unknown,
        criticalBoundaryBytes: nil,
        warningBoundaryBytes: nil,
        historyDisposition: nil,
        sampledAt: Date(timeIntervalSince1970: sampledAt)
    )
}
