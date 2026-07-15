import XCTest
@testable import DUX

final class EngineServiceTests: XCTestCase {
    func testLoadsTypedRustValuesOffTheMainThread() async throws {
        let result = try await EngineService().loadSmokeResult(bytes: 1_536)

        XCTAssertEqual(result.libraryVersion, "0.5.0")
        XCTAssertEqual(result.ffiContractVersion, 2)
        XCTAssertEqual(result.bytes, 1_536)
        XCTAssertEqual(result.displaySize, "1.5 KB")
        XCTAssertTrue(result.executedOffMainThread)
    }

    @MainActor
    func testAppModelPublishesLoadedStateOnTheMainActor() async {
        let model = AppModel()

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
            let engine = DuxEngine()
            weakEngine = engine

            XCTAssertEqual(liveEngineInstanceCount(), baseline + 1)
            XCTAssertEqual(try engine.libraryVersion().ffiContractVersion, 2)
            XCTAssertTrue(engine.close())
            XCTAssertFalse(engine.close())
            XCTAssertThrowsError(try engine.formatSize(bytes: 1_536)) { error in
                XCTAssertEqual(error as? EngineError, .Closed)
            }
        }

        XCTAssertNil(weakEngine)
        XCTAssertEqual(liveEngineInstanceCount(), baseline)
    }

    func testEngineServiceMapsClosedError() async {
        let engine = DuxEngine()
        let service = EngineService(engine: engine)

        let firstClose = await service.close()
        let secondClose = await service.close()
        XCTAssertTrue(firstClose)
        XCTAssertFalse(secondClose)

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
            ffiContractVersion: 2,
            bytes: bytes,
            displaySize: "test",
            executedOffMainThread: true
        )
    }

    func currentLoadCount() -> Int {
        loadCount
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
