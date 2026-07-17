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

            let result = try await service.loadSmokeResult(bytes: 1_536)
            XCTAssertTrue(result.executedOffMainThread)
            XCTAssertEqual(liveEngineInstanceCount(), baseline + 1)
            let closed = await service.close()
            XCTAssertTrue(closed)
        }

        XCTAssertEqual(liveEngineInstanceCount(), baseline)
    }

    func testLoadsTypedRustValuesOffTheMainThread() async throws {
        let fixture = try TestEngineFixture()
        let result = try await EngineService(engine: fixture.engine).loadSmokeResult(bytes: 1_536)

        XCTAssertEqual(result.libraryVersion, "0.5.0")
        XCTAssertEqual(result.ffiContractVersion, 3)
        XCTAssertEqual(result.bytes, 1_536)
        XCTAssertEqual(result.displaySize, "1.5 KB")
        XCTAssertTrue(result.executedOffMainThread)
    }

    @MainActor
    func testAppModelPublishesLoadedStateOnTheMainActor() async {
        guard let fixture = try? TestEngineFixture() else {
            return XCTFail("Expected a temporary engine")
        }
        let model = AppModel(engineService: EngineService(engine: fixture.engine))

        await model.loadEngineSmokeResult()

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
            XCTAssertEqual(try engine.libraryVersion().ffiContractVersion, 3)
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
            _ = try await service.loadSmokeResult(bytes: 1_536)
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

        let engineLoadCount = await engineService.currentLoadCount()
        let volumeLoadCount = await volumeMonitor.currentLoadCount()
        XCTAssertEqual(engineLoadCount, 1)
        XCTAssertEqual(volumeLoadCount, 1)
        guard case .loaded = model.engineState else {
            return XCTFail("Expected one shared loaded engine state")
        }
        guard case .loaded = model.volumeState else {
            return XCTFail("Expected one shared loaded volume state")
        }
    }

    func testApplicationRunsAsMenuBarAgent() {
        XCTAssertEqual(Bundle.main.object(forInfoDictionaryKey: "LSUIElement") as? Bool, true)
    }
}

private actor CountingEngineService: EngineServing {
    private var loadCount = 0

    func loadSmokeResult(bytes: UInt64) async throws -> EngineSmokeResult {
        loadCount += 1
        await Task.yield()
        return EngineSmokeResult(
            libraryVersion: "test",
            ffiContractVersion: 3,
            bytes: bytes,
            displaySize: "test",
            executedOffMainThread: true
        )
    }

    func currentLoadCount() -> Int {
        loadCount
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
        return VolumeCapacitySnapshot(
            displayName: "Test Volume",
            totalBytes: 100,
            filesystemAvailableBytes: 20,
            importantAvailableBytes: 30,
            effectiveAvailableBytes: 30,
            availabilityBasis: .importantUsage,
            sampledAt: Date(timeIntervalSince1970: 1)
        )
    }

    func currentLoadCount() -> Int {
        loadCount
    }
}
