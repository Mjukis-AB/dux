import XCTest
@testable import DUX

@MainActor
final class DuxAppDelegateTests: XCTestCase {
    func testApplicationLifetimeOptsOutOfAutomaticTerminationUntilShutdown() {
        let controller = AutomaticTerminationControllerSpy()
        let lease = DuxAutomaticTerminationLease(controller: controller)
        let delegate = DuxAppDelegate(
            runtime: RuntimeSpy(),
            automaticTerminationLease: lease
        )

        delegate.applicationDidFinishLaunching(
            Notification(name: NSApplication.didFinishLaunchingNotification)
        )
        XCTAssertFalse(controller.automaticTerminationSupportEnabled)
        XCTAssertEqual(controller.events, ["support:false", "disable"])

        delegate.applicationWillTerminate(
            Notification(name: NSApplication.willTerminateNotification)
        )
        XCTAssertFalse(controller.automaticTerminationSupportEnabled)
        XCTAssertEqual(controller.events, ["support:false", "disable", "enable"])
    }

    func testAutomaticTerminationLeaseCanBeReassertedAfterSceneRestoration() {
        let controller = AutomaticTerminationControllerSpy()
        let lease = DuxAutomaticTerminationLease(controller: controller)

        lease.reassert()
        XCTAssertTrue(controller.events.isEmpty)

        lease.acquire()
        controller.automaticTerminationSupportEnabled = true
        lease.reassert()

        XCTAssertEqual(
            controller.events,
            ["support:false", "disable", "support:true", "support:false", "disable"]
        )
        XCTAssertFalse(controller.automaticTerminationSupportEnabled)
    }

    func testRepeatedSceneReassertionsBalanceEveryTerminationOptOut() {
        let controller = AutomaticTerminationControllerSpy()
        let lease = DuxAutomaticTerminationLease(controller: controller)

        lease.acquire()
        lease.reassert()
        lease.reassert()
        lease.release()

        XCTAssertEqual(
            controller.events,
            [
                "support:false", "disable",
                "support:false", "disable",
                "support:false", "disable",
                "enable", "enable", "enable",
            ]
        )
    }

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

    func testExplicitReopenRevealsMenuBarItemForSession() {
        let runtime = RuntimeSpy()
        let delegate = DuxAppDelegate(runtime: runtime)

        XCTAssertTrue(
            delegate.applicationShouldHandleReopen(
                NSApplication.shared,
                hasVisibleWindows: false
            )
        )
        XCTAssertEqual(runtime.revealCount, 1)
    }

    func testClosingLastWindowNeverTerminatesMenuBarApp() {
        let controller = AutomaticTerminationControllerSpy()
        let lease = DuxAutomaticTerminationLease(controller: controller)
        lease.acquire()
        let delegate = DuxAppDelegate(
            runtime: RuntimeSpy(),
            automaticTerminationLease: lease
        )

        XCTAssertFalse(
            delegate.applicationShouldTerminateAfterLastWindowClosed(
                NSApplication.shared
            )
        )
        XCTAssertEqual(
            controller.events,
            ["support:false", "disable", "support:false", "disable"]
        )
    }

    func testIncidentalTerminationRequestFromTransientMenuWindowIsCancelled() {
        let controller = AutomaticTerminationControllerSpy()
        let lease = DuxAutomaticTerminationLease(controller: controller)
        lease.acquire()
        let delegate = DuxAppDelegate(
            runtime: RuntimeSpy(),
            automaticTerminationLease: lease
        )

        XCTAssertEqual(
            delegate.applicationShouldTerminate(NSApplication.shared),
            .terminateCancel
        )
        XCTAssertEqual(
            controller.events,
            ["support:false", "disable", "support:false", "disable"]
        )
    }

    func testTransientSceneResignationReassertsAutomaticTerminationLease() {
        let controller = AutomaticTerminationControllerSpy()
        let lease = DuxAutomaticTerminationLease(controller: controller)
        lease.acquire()
        let delegate = DuxAppDelegate(
            runtime: RuntimeSpy(),
            automaticTerminationLease: lease
        )

        delegate.applicationDidResignActive(
            Notification(name: NSApplication.didResignActiveNotification)
        )

        XCTAssertEqual(
            controller.events,
            ["support:false", "disable", "support:false", "disable"]
        )
    }

    func testExplicitQuitIntentIsOneShot() {
        DuxTerminationIntent.requestExplicitQuit()

        XCTAssertTrue(DuxTerminationIntent.consumeExplicitQuitRequest())
        XCTAssertFalse(DuxTerminationIntent.consumeExplicitQuitRequest())
    }

    func testApplicationActivationRunsMaintenanceThenArmedAccessReprobe() async {
        let runtime = RuntimeSpy()
        let delegate = DuxAppDelegate(runtime: runtime)

        await delegate.handleApplicationBecameActive()

        XCTAssertEqual(runtime.events(), ["maintenance:other", "access:activation"])
    }

    func testRuntimeStartsBothSchedulersOnceAndStopsCapacityBeforeEngineClose() async throws {
        let recorder = RuntimeEventRecorder()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let maintenance = RuntimeMaintenanceSpy(recorder: recorder)
        let capacity = RuntimeCapacitySpy(recorder: recorder)
        let reviews = RuntimeReviewSpy(recorder: recorder)
        let scans = RuntimeScanSpy(recorder: recorder)
        let model = AppModel(engineService: engine)
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: maintenance,
            capacityScheduler: capacity,
            reviews: reviews,
            scans: scans
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
        let scanShutdown = try XCTUnwrap(events.firstIndex(of: "scans:shutdown"))
        let capacityStop = try XCTUnwrap(events.firstIndex(of: "capacity:stop"))
        let maintenanceStop = try XCTUnwrap(events.firstIndex(of: "maintenance:stop"))
        let reviewsShutdown = try XCTUnwrap(events.firstIndex(of: "reviews:shutdown"))
        let engineClose = try XCTUnwrap(events.firstIndex(of: "engine:close"))
        XCTAssertLessThan(scanShutdown, capacityStop)
        XCTAssertLessThan(capacityStop, maintenanceStop)
        XCTAssertLessThan(maintenanceStop, reviewsShutdown)
        XCTAssertLessThan(reviewsShutdown, engineClose)
    }
}

@MainActor
private final class AutomaticTerminationControllerSpy: DuxAutomaticTerminationControlling {
    var automaticTerminationSupportEnabled = false {
        didSet {
            events.append("support:\(automaticTerminationSupportEnabled)")
        }
    }
    private(set) var events: [String] = []

    func disableAutomaticTermination(_ reason: String) {
        _ = reason
        events.append("disable")
    }

    func enableAutomaticTermination(_ reason: String) {
        _ = reason
        events.append("enable")
    }
}

@MainActor
private final class RuntimeSpy: DuxAppRuntimeServing {
    private var recorded: [String] = []
    private(set) var revealCount = 0

    func start() async {}
    func shutdown() async {}
    func revealMenuBarItemForSession() { revealCount += 1 }
    func refreshStorageAccessEvidenceAfterActivation() async {
        recorded.append("access:activation")
    }

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
    func revealMenuBarItemForSession() {}
    func refreshStorageAccessEvidenceAfterActivation() async {}

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
        EngineStatus(libraryVersion: "test", ffiContractVersion: 12, executedOffMainThread: true)
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        DiskPressurePolicy(
            source: .default,
            revision: 0,
            configuration: .defaults,
            updatedAtUnixMilliseconds: nil
        )
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .stored,
                revision: 1,
                configuration: configuration,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .default,
                revision: 1,
                configuration: .defaults,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
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

@MainActor
private final class RuntimeScanSpy: DuxScanManaging {
    let recorder: RuntimeEventRecorder
    init(recorder: RuntimeEventRecorder) { self.recorder = recorder }

    func shutdownHomeScan() async {
        await recorder.append("scans:shutdown")
    }
}
