import XCTest
@testable import DUX

@MainActor
final class DuxAppDelegateTests: XCTestCase {
    func testTerminationGateStartsExactlyOneShutdown() {
        let gate = DuxTerminationGate()

        guard case .beginShutdown = gate.begin() else {
            return XCTFail("Expected the first request to start shutdown")
        }
        guard case .waitForExistingShutdown = gate.begin() else {
            return XCTFail("Expected a duplicate request to wait")
        }

        gate.approve()
        guard case .terminateNow = gate.begin() else {
            return XCTFail("Expected approval only after shutdown completed")
        }
    }

    func testDelegateRoutesWakeAndVolumeEventsToBothRuntimePolicies() async throws {
        let runtime = RuntimeSpy()
        let delegate = DuxAppDelegate(runtime: runtime)

        await delegate.handleWake()
        await delegate.handleVolumesChanged()

        let events = runtime.events()
        XCTAssertEqual(Set(events), Set(["maintenance:wake", "capacity:wake", "capacity:volumes"]))
        XCTAssertLessThan(
            try XCTUnwrap(events.firstIndex(of: "capacity:wake")),
            try XCTUnwrap(events.firstIndex(of: "maintenance:wake"))
        )
    }

    func testWakeCapacitySignalIsNotBlockedBySuspendedMaintenance() async {
        let runtime = DelayedMaintenanceRuntimeSpy()
        let delegate = DuxAppDelegate(runtime: runtime)

        let wake = Task { @MainActor in
            await delegate.handleWake()
        }
        while !runtime.maintenanceStarted {
            await Task.yield()
        }

        XCTAssertTrue(runtime.capacitySignaled)
        runtime.releaseMaintenance()
        await wake.value
    }

    func testRuntimeStartsBothSchedulersOnceAndStopsCapacityBeforeEngineClose() async throws {
        let recorder = RuntimeEventRecorder()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let maintenance = RuntimeMaintenanceSpy(recorder: recorder)
        let capacity = RuntimeCapacitySpy(recorder: recorder)
        let reviews = RuntimeReviewSpy(recorder: recorder)
        let model = AppModel(engineService: engine)
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: maintenance,
            capacityScheduler: capacity,
            reviews: reviews
        )

        await runtime.start()
        await runtime.start()
        await runtime.signalMaintenance(.wake)
        await runtime.signalCapacity(.volumesChanged)
        await runtime.shutdown()
        await runtime.shutdown()

        let events = await recorder.values()
        XCTAssertEqual(events.filter { $0 == "maintenance:start" }.count, 1)
        XCTAssertEqual(events.filter { $0 == "capacity:start" }.count, 1)
        XCTAssertTrue(events.contains("reviews:renew"))
        XCTAssertTrue(events.contains("capacity:volumes"))
        XCTAssertEqual(events.filter { $0 == "engine:close" }.count, 1)
        let capacityStop = try XCTUnwrap(events.firstIndex(of: "capacity:stop"))
        let maintenanceStop = try XCTUnwrap(events.firstIndex(of: "maintenance:stop"))
        let reviewsShutdown = try XCTUnwrap(events.firstIndex(of: "reviews:shutdown"))
        let engineClose = try XCTUnwrap(events.firstIndex(of: "engine:close"))
        XCTAssertLessThan(capacityStop, maintenanceStop)
        XCTAssertLessThan(maintenanceStop, reviewsShutdown)
        XCTAssertLessThan(reviewsShutdown, engineClose)
    }
}

@MainActor
private final class RuntimeSpy: DuxAppRuntimeServing {
    private var recorded: [String] = []

    func start() async {}
    func shutdown() async {}

    func signalMaintenance(_ trigger: DuxMaintenanceTrigger) async {
        recorded.append("maintenance:\(trigger == .wake ? "wake" : "other")")
    }

    func signalCapacity(_ trigger: DuxCapacitySamplingTrigger) async {
        switch trigger {
        case .wake: recorded.append("capacity:wake")
        case .volumesChanged: recorded.append("capacity:volumes")
        case .manual: recorded.append("capacity:manual")
        }
    }

    func events() -> [String] { recorded }
}

@MainActor
private final class DelayedMaintenanceRuntimeSpy: DuxAppRuntimeServing {
    private var maintenanceContinuation: CheckedContinuation<Void, Never>?
    private(set) var maintenanceStarted = false
    private(set) var capacitySignaled = false

    func start() async {}
    func shutdown() async {}

    func signalMaintenance(_ trigger: DuxMaintenanceTrigger) async {
        _ = trigger
        maintenanceStarted = true
        await withCheckedContinuation { continuation in
            maintenanceContinuation = continuation
        }
    }

    func signalCapacity(_ trigger: DuxCapacitySamplingTrigger) async {
        _ = trigger
        capacitySignaled = true
    }

    func releaseMaintenance() {
        maintenanceContinuation?.resume()
        maintenanceContinuation = nil
    }
}

private actor RuntimeEventRecorder {
    private var events: [String] = []
    func append(_ event: String) { events.append(event) }
    func values() -> [String] { events }
}

private actor RuntimeEngineSpy: EngineServing, DuxEngineClosing {
    let recorder: RuntimeEventRecorder
    init(recorder: RuntimeEventRecorder) { self.recorder = recorder }

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 5, executedOffMainThread: true)
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func close() async -> Bool {
        await recorder.append("engine:close")
        return true
    }
}

private actor RuntimeMaintenanceSpy: DuxMaintenanceScheduling {
    let recorder: RuntimeEventRecorder
    init(recorder: RuntimeEventRecorder) { self.recorder = recorder }
    func start() async { await recorder.append("maintenance:start") }
    func signal(_ trigger: DuxMaintenanceTrigger) async {
        await recorder.append(trigger == .wake ? "maintenance:wake" : "maintenance:other")
    }
    func stop() async { await recorder.append("maintenance:stop") }
}

private actor RuntimeCapacitySpy: DuxCapacityScheduling {
    let recorder: RuntimeEventRecorder
    init(recorder: RuntimeEventRecorder) { self.recorder = recorder }
    func start() async { await recorder.append("capacity:start") }
    func signal(_ trigger: DuxCapacitySamplingTrigger) async {
        switch trigger {
        case .wake: await recorder.append("capacity:wake")
        case .volumesChanged: await recorder.append("capacity:volumes")
        case .manual: await recorder.append("capacity:manual")
        }
    }
    func stop() async { await recorder.append("capacity:stop") }
}

private actor RuntimeReviewSpy: DuxReviewManaging {
    let recorder: RuntimeEventRecorder
    init(recorder: RuntimeEventRecorder) { self.recorder = recorder }
    func renewNow() async { await recorder.append("reviews:renew") }
    func shutdown() async { await recorder.append("reviews:shutdown") }
}
