@testable import DUX
import XCTest

@MainActor
final class DuxAppDelegateTests: XCTestCase {
    func testApplicationLifetimeDisablesAutomaticTerminationUntilShutdown() {
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
        // Model AppKit resetting the opt-out during transient-scene teardown.
        controller.automaticTerminationSupportEnabled = true
        lease.reassert()

        XCTAssertEqual(
            controller.events,
            ["support:false", "disable", "support:true", "support:false"]
        )
        XCTAssertFalse(controller.automaticTerminationSupportEnabled)
    }

    func testRepeatedSceneReassertionsDoNotAccumulateTerminationLeases() {
        let controller = AutomaticTerminationControllerSpy()
        let lease = DuxAutomaticTerminationLease(controller: controller)

        lease.acquire()
        lease.reassert()
        lease.reassert()
        lease.release()

        XCTAssertEqual(
            controller.events,
            ["support:false", "disable", "support:false", "support:false", "enable"]
        )
    }

    func testRepeatedAcquireDoesNotAccumulateTerminationLeases() {
        let controller = AutomaticTerminationControllerSpy()
        let lease = DuxAutomaticTerminationLease(controller: controller)

        lease.acquire()
        lease.acquire()
        lease.release()

        XCTAssertEqual(
            controller.events,
            ["support:false", "disable", "enable"]
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

    func testDelegateRoutesWakeAndVolumeEventsToRuntimePolicies() async throws {
        let runtime = RuntimeSpy()
        let delegate = DuxAppDelegate(runtime: runtime)

        await delegate.handleWake()
        await delegate.handleVolumesChanged()

        let events = runtime.events()
        XCTAssertEqual(
            Set(events),
            Set([
                "maintenance:wake",
                "automation:wake",
                "capacity:wake",
                "capacity:volumes",
            ])
        )
        XCTAssertLessThan(
            try XCTUnwrap(events.firstIndex(of: "capacity:wake")),
            try XCTUnwrap(events.firstIndex(of: "maintenance:wake"))
        )
        XCTAssertLessThan(
            try XCTUnwrap(events.firstIndex(of: "capacity:wake")),
            try XCTUnwrap(events.firstIndex(of: "automation:wake"))
        )
    }

    func testWakeCapacitySignalIsNotBlockedBySuspendedMaintenance() async {
        let runtime = DelayedMaintenanceRuntimeSpy()
        let delegate = DuxAppDelegate(runtime: runtime)

        let wake = Task { @MainActor in
            await delegate.handleWake()
        }
        while !runtime.maintenanceStarted || !runtime.automationSignaled {
            await Task.yield()
        }

        XCTAssertTrue(runtime.capacitySignaled)
        XCTAssertTrue(runtime.automationSignaled)
        runtime.releaseMaintenance()
        await wake.value
    }

    func testWakeMaintenanceSignalIsNotBlockedBySuspendedAutomation() async {
        let runtime = DelayedAutomationRuntimeSpy()
        let delegate = DuxAppDelegate(runtime: runtime)

        let wake = Task { @MainActor in
            await delegate.handleWake()
        }
        while !runtime.automationStarted || !runtime.maintenanceSignaled {
            await Task.yield()
        }

        XCTAssertTrue(runtime.capacitySignaled)
        XCTAssertTrue(runtime.maintenanceSignaled)
        runtime.releaseAutomation()
        await wake.value
    }

    func testSignificantTimeChangeRoutesAutomationAndMaintenanceConcurrently() async {
        let runtime = RuntimeSpy()
        let delegate = DuxAppDelegate(runtime: runtime)

        await delegate.handleSignificantTimeChange()

        XCTAssertEqual(
            Set(runtime.events()),
            Set(["automation:time", "maintenance:other"])
        )
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
            ["support:false", "disable", "support:false"]
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
            ["support:false", "disable", "support:false"]
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
            ["support:false", "disable", "support:false"]
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

    func testRuntimeStartsAllSchedulersOnceAndStopsThemBeforeEngineClose() async throws {
        let recorder = RuntimeEventRecorder()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let maintenance = RuntimeMaintenanceSpy(recorder: recorder)
        let capacity = RuntimeCapacitySpy(recorder: recorder)
        let automation = RuntimeAutomationSpy(recorder: recorder)
        let reviews = RuntimeReviewSpy(recorder: recorder)
        let scans = RuntimeScanSpy(recorder: recorder)
        let model = AppModel(engineService: engine)
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: maintenance,
            capacityScheduler: capacity,
            automationScheduler: automation,
            reviews: reviews,
            scans: scans
        )

        await runtime.start()
        await runtime.start()
        await runtime.signalMaintenance(.wake)
        await runtime.signalCapacity(.volumesChanged)
        await runtime.signalAutomation(.wake)
        await runtime.shutdown()
        await runtime.shutdown()

        let events = await recorder.values()
        XCTAssertEqual(events.count(where: { $0 == "maintenance:start" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "capacity:start" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "automation:start" }), 1)
        XCTAssertTrue(events.contains("reviews:renew"))
        XCTAssertTrue(events.contains("capacity:volumes"))
        XCTAssertTrue(events.contains("automation:wake"))
        XCTAssertEqual(events.count(where: { $0 == "engine:close" }), 1)
        let scanShutdown = try XCTUnwrap(events.firstIndex(of: "scans:shutdown"))
        let capacityStop = try XCTUnwrap(events.firstIndex(of: "capacity:stop"))
        let automationStop = try XCTUnwrap(events.firstIndex(of: "automation:stop"))
        let maintenanceStop = try XCTUnwrap(events.firstIndex(of: "maintenance:stop"))
        let reviewsShutdown = try XCTUnwrap(events.firstIndex(of: "reviews:shutdown"))
        let engineClose = try XCTUnwrap(events.firstIndex(of: "engine:close"))
        XCTAssertLessThan(scanShutdown, reviewsShutdown)
        XCTAssertLessThan(automationStop, reviewsShutdown)
        XCTAssertLessThan(reviewsShutdown, capacityStop)
        XCTAssertLessThan(capacityStop, maintenanceStop)
        XCTAssertLessThan(maintenanceStop, engineClose)
    }

    func testRuntimeJoinsConfirmedCLIMutationAndInstallerCloseBeforeEngineClose() async throws {
        let recorder = RuntimeEventRecorder()
        let gate = RuntimeAsyncGate()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let cliInstaller = RuntimeCLIInstallerSpy(
            recorder: recorder,
            performGate: gate
        )
        let maintenance = RuntimeMaintenanceSpy(recorder: recorder)
        let capacity = RuntimeCapacitySpy(recorder: recorder)
        let automation = RuntimeAutomationSpy(recorder: recorder)
        let reviews = RuntimeReviewSpy(recorder: recorder)
        let scans = RuntimeScanSpy(recorder: recorder)
        let model = AppModel(
            engineService: engine,
            cliInstallerService: cliInstaller
        )
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: maintenance,
            capacityScheduler: capacity,
            automationScheduler: automation,
            reviews: reviews,
            scans: scans
        )
        await model.cliInstallation.loadStatus()
        await model.cliInstallation.prepare(.install)
        let mutation = Task { @MainActor in
            await model.cliInstallation.confirmPreparedAction()
        }
        await gate.waitUntilStarted()

        let shutdown = Task { @MainActor in
            await runtime.shutdown()
        }
        await Task.yield()
        var events = await recorder.values()
        XCTAssertTrue(events.contains("cli:perform:start"))
        XCTAssertFalse(events.contains("cli:perform:end"))
        XCTAssertFalse(events.contains("cli:close"))
        XCTAssertFalse(events.contains("engine:close"))

        await gate.release()
        await mutation.value
        await shutdown.value

        events = await recorder.values()
        let performEnd = try XCTUnwrap(events.firstIndex(of: "cli:perform:end"))
        let installerClose = try XCTUnwrap(events.firstIndex(of: "cli:close"))
        let engineClose = try XCTUnwrap(events.firstIndex(of: "engine:close"))
        XCTAssertLessThan(performEnd, installerClose)
        XCTAssertLessThan(installerClose, engineClose)
    }

    func testResetWinsPermanentlyWithoutOrdinaryEngineCloseOrLateAdmission() async throws {
        let recorder = RuntimeEventRecorder()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let cliInstaller = RuntimeCLIInstallerSpy(
            recorder: recorder,
            performGate: RuntimeAsyncGate()
        )
        let maintenance = RuntimeMaintenanceSpy(recorder: recorder)
        let capacity = RuntimeCapacitySpy(recorder: recorder)
        let automation = RuntimeAutomationSpy(recorder: recorder)
        let reviews = RuntimeReviewSpy(recorder: recorder)
        let scans = RuntimeScanSpy(recorder: recorder)
        let model = AppModel(
            engineService: engine,
            cliInstallerService: cliInstaller
        )
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: maintenance,
            capacityScheduler: capacity,
            automationScheduler: automation,
            reviews: reviews,
            scans: scans
        )
        var explorerOpenCount = 0
        runtime.installExplorerOpener { _ in explorerOpenCount += 1 }

        let first = await runtime.quiesceForAppDataReset()
        await runtime.shutdown()
        let second = await runtime.quiesceForAppDataReset()
        await runtime.start()
        await runtime.signalMaintenance(.wake)
        await runtime.signalCapacity(.manual)
        await runtime.signalAutomation(.wake)
        runtime.installExplorerOpener { _ in explorerOpenCount += 1 }
        let payload = try XCTUnwrap(
            DiskPressureNotificationPayload(
                stableVolumeID: "runtime:terminal:test",
                urgency: .critical
            )
        )
        await runtime.handleUrgentRecommendations(payload)

        XCTAssertEqual(terminalIntent(first), .appDataReset)
        XCTAssertEqual(terminalIntent(second), .appDataReset)
        let events = await recorder.values()
        XCTAssertEqual(events.count(where: { $0 == "engine:close" }), 0)
        XCTAssertEqual(events.count(where: { $0 == "maintenance:stop" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "capacity:stop" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "automation:stop" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "reviews:shutdown" }), 1)
        XCTAssertFalse(events.contains("maintenance:start"))
        XCTAssertFalse(events.contains("capacity:start"))
        XCTAssertFalse(events.contains("automation:start"))
        XCTAssertFalse(events.contains("maintenance:wake"))
        XCTAssertFalse(events.contains("capacity:manual"))
        XCTAssertFalse(events.contains("automation:wake"))
        XCTAssertEqual(explorerOpenCount, 0)
    }

    func testOrdinaryQuitWinsPermanentlyAndClosesEngineExactlyOnce() async {
        let recorder = RuntimeEventRecorder()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let model = AppModel(engineService: engine)
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: RuntimeMaintenanceSpy(recorder: recorder),
            capacityScheduler: RuntimeCapacitySpy(recorder: recorder),
            reviews: RuntimeReviewSpy(recorder: recorder),
            scans: RuntimeScanSpy(recorder: recorder)
        )

        await runtime.shutdown()
        let losingReset = await runtime.quiesceForAppDataReset()
        await runtime.shutdown()

        XCTAssertEqual(terminalIntent(losingReset), .ordinaryQuit)
        let events = await recorder.values()
        XCTAssertEqual(events.count(where: { $0 == "engine:close" }), 1)
    }

    func testCancelledAndConcurrentResetCallersShareBlockedDrain() async throws {
        let recorder = RuntimeEventRecorder()
        let mutationGate = RuntimeAsyncGate()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let cliInstaller = RuntimeCLIInstallerSpy(
            recorder: recorder,
            performGate: mutationGate
        )
        let model = AppModel(
            engineService: engine,
            cliInstallerService: cliInstaller
        )
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: RuntimeMaintenanceSpy(recorder: recorder),
            capacityScheduler: RuntimeCapacitySpy(recorder: recorder),
            reviews: RuntimeReviewSpy(recorder: recorder),
            scans: RuntimeScanSpy(recorder: recorder)
        )
        await model.cliInstallation.loadStatus()
        await model.cliInstallation.prepare(.install)
        let mutation = Task { @MainActor in
            await model.cliInstallation.confirmPreparedAction()
        }
        await mutationGate.waitUntilStarted()

        let first = Task { @MainActor in
            await runtime.quiesceForAppDataReset()
        }
        first.cancel()
        let second = Task { @MainActor in
            await runtime.quiesceForAppDataReset()
        }
        await Task.yield()
        var events = await recorder.values()
        XCTAssertFalse(events.contains("cli:perform:end"))
        XCTAssertFalse(events.contains("capacity:stop"))
        XCTAssertFalse(events.contains("engine:close"))

        await mutationGate.release()
        await mutation.value
        let outcomes = await [first.value, second.value]

        XCTAssertEqual(outcomes.compactMap(terminalIntent), [.appDataReset, .appDataReset])
        events = await recorder.values()
        XCTAssertEqual(events.count(where: { $0 == "cli:perform:start" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "cli:perform:end" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "cli:close" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "engine:close" }), 0)
    }

    func testConcurrentQuitAndResetPublishOneImmutableWinner() async throws {
        let recorder = RuntimeEventRecorder()
        let mutationGate = RuntimeAsyncGate()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let model = AppModel(
            engineService: engine,
            cliInstallerService: RuntimeCLIInstallerSpy(
                recorder: recorder,
                performGate: mutationGate
            )
        )
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: RuntimeMaintenanceSpy(recorder: recorder),
            capacityScheduler: RuntimeCapacitySpy(recorder: recorder),
            reviews: RuntimeReviewSpy(recorder: recorder),
            scans: RuntimeScanSpy(recorder: recorder)
        )
        await model.cliInstallation.loadStatus()
        await model.cliInstallation.prepare(.install)
        let mutation = Task { @MainActor in
            await model.cliInstallation.confirmPreparedAction()
        }
        await mutationGate.waitUntilStarted()

        let quit = Task { @MainActor in
            await runtime.shutdown()
        }
        let reset = Task { @MainActor in
            await runtime.quiesceForAppDataReset()
        }
        await Task.yield()
        await mutationGate.release()
        await mutation.value
        await quit.value
        let firstObservation = await reset.value
        let secondObservation = await runtime.quiesceForAppDataReset()

        let winner = try XCTUnwrap(terminalIntent(firstObservation))
        XCTAssertEqual(terminalIntent(secondObservation), winner)
        let events = await recorder.values()
        XCTAssertEqual(events.count(where: { $0 == "cli:perform:start" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "cli:perform:end" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "cli:close" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "capacity:stop" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "maintenance:stop" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "reviews:shutdown" }), 1)
        XCTAssertEqual(
            events.count(where: { $0 == "engine:close" }),
            winner == .ordinaryQuit ? 1 : 0
        )
    }

    func testReentrantResetCallerObservesWinnerWithoutStartingSecondDrain() async {
        let recorder = RuntimeEventRecorder()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let reviews = RuntimeReentrantReviewSpy(recorder: recorder)
        let model = AppModel(engineService: engine)
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: RuntimeMaintenanceSpy(recorder: recorder),
            capacityScheduler: RuntimeCapacitySpy(recorder: recorder),
            reviews: reviews,
            scans: RuntimeScanSpy(recorder: recorder)
        )
        reviews.runtime = runtime

        let outcome = await runtime.quiesceForAppDataReset()

        XCTAssertEqual(terminalIntent(outcome), .appDataReset)
        XCTAssertEqual(reviews.reentrantIntent, .appDataReset)
        let events = await recorder.values()
        XCTAssertEqual(events.count(where: { $0 == "reviews:shutdown" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "engine:close" }), 0)
    }

    func testStartupReentrantResetDoesNotWaitOnItsOwnStartupTask() async {
        let recorder = RuntimeEventRecorder()
        let engine = RuntimeEngineSpy(recorder: recorder)
        let maintenance = RuntimeStartupReentrantMaintenanceSpy(recorder: recorder)
        let model = AppModel(engineService: engine)
        let runtime = AppRuntime(
            model: model,
            engineService: engine,
            scheduler: maintenance,
            capacityScheduler: RuntimeCapacitySpy(recorder: recorder),
            reviews: RuntimeReviewSpy(recorder: recorder),
            scans: RuntimeScanSpy(recorder: recorder)
        )
        maintenance.runtime = runtime

        await runtime.start()
        let completed = await runtime.quiesceForAppDataReset()

        XCTAssertEqual(maintenance.reentrantIntent, .appDataReset)
        XCTAssertEqual(terminalIntent(completed), .appDataReset)
        let events = await recorder.values()
        XCTAssertEqual(events.count(where: { $0 == "maintenance:start" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "maintenance:stop" }), 1)
        XCTAssertEqual(events.count(where: { $0 == "engine:close" }), 0)
    }

    private func terminalIntent(
        _ result: NativeRuntimeTerminalRequestResult
    ) -> NativeRuntimeTerminalIntent? {
        switch result {
        case let .completed(completion): completion.intent
        case .reentrant: nil
        }
    }
}

@MainActor
private final class AutomaticTerminationControllerSpy: DuxAutomaticTerminationControlling {
    var automaticTerminationSupportEnabled = true {
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

    func signalAutomation(_ trigger: DuxAutomationDecisionTrigger) async {
        recorded.append(
            trigger == .wake ? "automation:wake" : "automation:time"
        )
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
    private(set) var automationSignaled = false

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

    func signalAutomation(_ trigger: DuxAutomationDecisionTrigger) async {
        _ = trigger
        automationSignaled = true
    }

    func releaseMaintenance() {
        maintenanceContinuation?.resume()
        maintenanceContinuation = nil
    }
}

@MainActor
private final class DelayedAutomationRuntimeSpy: DuxAppRuntimeServing {
    private var automationContinuation: CheckedContinuation<Void, Never>?
    private(set) var automationStarted = false
    private(set) var maintenanceSignaled = false
    private(set) var capacitySignaled = false

    func start() async {}
    func shutdown() async {}
    func revealMenuBarItemForSession() {}
    func refreshStorageAccessEvidenceAfterActivation() async {}

    func signalMaintenance(_ trigger: DuxMaintenanceTrigger) async {
        _ = trigger
        maintenanceSignaled = true
    }

    func signalCapacity(_ trigger: DuxCapacitySamplingTrigger) async {
        _ = trigger
        capacitySignaled = true
    }

    func signalAutomation(_ trigger: DuxAutomationDecisionTrigger) async {
        _ = trigger
        automationStarted = true
        await withCheckedContinuation { continuation in
            automationContinuation = continuation
        }
    }

    func releaseAutomation() {
        automationContinuation?.resume()
        automationContinuation = nil
    }
}

private actor RuntimeEventRecorder {
    private var events: [String] = []
    func append(_ event: String) { events.append(event) }
    func values() -> [String] { events }
}

private actor RuntimeAsyncGate {
    private var started = false
    private var released = false
    private var startWaiters: [CheckedContinuation<Void, Never>] = []
    private var releaseWaiters: [CheckedContinuation<Void, Never>] = []

    func waitUntilStarted() async {
        if started {
            return
        }
        await withCheckedContinuation { continuation in
            startWaiters.append(continuation)
        }
    }

    func beginAndWaitForRelease() async {
        started = true
        let waiters = startWaiters
        startWaiters.removeAll()
        for waiter in waiters {
            waiter.resume()
        }
        if released {
            return
        }
        await withCheckedContinuation { continuation in
            releaseWaiters.append(continuation)
        }
    }

    func release() {
        released = true
        let waiters = releaseWaiters
        releaseWaiters.removeAll()
        for waiter in waiters {
            waiter.resume()
        }
    }
}

private actor RuntimeCLIInstallerSpy: CLIInstallerServing {
    private let recorder: RuntimeEventRecorder
    private let performGate: RuntimeAsyncGate
    private let status = CLIInstallationStatus(
        bundled: CLIBundledMetadata(
            version: CLIVersion("0.5.0")!,
            databaseSchemaVersion: 16,
            snapshotFormatVersion: 1,
            sha256: Data(repeating: 0xAB, count: 32)
        ),
        disposition: .absent,
        pathEnvironment: .included
    )
    private var confirmation: CLIInstallationConfirmation?

    init(
        recorder: RuntimeEventRecorder,
        performGate: RuntimeAsyncGate
    ) {
        self.recorder = recorder
        self.performGate = performGate
    }

    func loadStatus() async throws -> CLIInstallationStatus {
        status
    }

    func prepare(
        _ action: CLIInstallationAction
    ) async throws -> CLIInstallationConfirmation {
        let confirmation = CLIInstallationConfirmation(
            token: UUID(),
            action: action,
            bundledVersion: status.bundled.version,
            installedVersion: nil,
            destinationDisplayText: CLIInstallationStatus.destinationDisplayText
        )
        self.confirmation = confirmation
        return confirmation
    }

    func perform(
        _ confirmation: CLIInstallationConfirmation
    ) async throws -> CLIInstallationUpdate {
        guard self.confirmation == confirmation else {
            throw CLIInstallerServiceError.confirmationUnavailable
        }
        await recorder.append("cli:perform:start")
        await performGate.beginAndWaitForRelease()
        await recorder.append("cli:perform:end")
        self.confirmation = nil
        return CLIInstallationUpdate(status: status, changed: false)
    }

    func discard(_ confirmation: CLIInstallationConfirmation) async {
        guard self.confirmation == confirmation else {
            return
        }
        self.confirmation = nil
    }

    func close() async {
        await recorder.append("cli:close")
        confirmation = nil
    }
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
    func quiesceForTerminalRuntime() async { await stop() }
}

@MainActor
private final class RuntimeStartupReentrantMaintenanceSpy: DuxMaintenanceScheduling {
    let recorder: RuntimeEventRecorder
    weak var runtime: AppRuntime?
    private(set) var reentrantIntent: NativeRuntimeTerminalIntent?

    init(recorder: RuntimeEventRecorder) {
        self.recorder = recorder
    }

    func start() async {
        await recorder.append("maintenance:start")
        guard let runtime else { return }
        switch await runtime.quiesceForAppDataReset() {
        case let .reentrant(intent): reentrantIntent = intent
        case let .completed(completion): reentrantIntent = completion.intent
        }
    }

    func signal(_: DuxMaintenanceTrigger) async {}

    func stop() async {
        await recorder.append("maintenance:stop")
    }

    func quiesceForTerminalRuntime() async {
        await stop()
    }
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
    func quiesceForTerminalRuntime() async { await stop() }
}

private actor RuntimeAutomationSpy: DuxAutomationDecisionScheduling {
    let recorder: RuntimeEventRecorder
    init(recorder: RuntimeEventRecorder) { self.recorder = recorder }
    func start() async { await recorder.append("automation:start") }
    func signal(_ trigger: DuxAutomationDecisionTrigger) async {
        await recorder.append(
            trigger == .wake ? "automation:wake" : "automation:other"
        )
    }

    func stop() async { await recorder.append("automation:stop") }
    func quiesceForTerminalRuntime() async { await stop() }
}

private actor RuntimeReviewSpy: DuxReviewManaging {
    let recorder: RuntimeEventRecorder
    init(recorder: RuntimeEventRecorder) { self.recorder = recorder }
    func renewNow() async { await recorder.append("reviews:renew") }
    func shutdown() async { await recorder.append("reviews:shutdown") }
}

@MainActor
private final class RuntimeReentrantReviewSpy: DuxReviewManaging {
    let recorder: RuntimeEventRecorder
    weak var runtime: AppRuntime?
    private(set) var reentrantIntent: NativeRuntimeTerminalIntent?

    init(recorder: RuntimeEventRecorder) {
        self.recorder = recorder
    }

    func renewNow() async {}

    func shutdown() async {
        await recorder.append("reviews:shutdown")
        guard let runtime else {
            return
        }
        switch await runtime.quiesceForAppDataReset() {
        case let .reentrant(intent): reentrantIntent = intent
        case let .completed(completion): reentrantIntent = completion.intent
        }
    }
}

@MainActor
private final class RuntimeScanSpy: DuxScanManaging {
    let recorder: RuntimeEventRecorder
    init(recorder: RuntimeEventRecorder) { self.recorder = recorder }

    func shutdownTargetedReclaimScan() async {}

    func shutdownHomeScan() async {
        await recorder.append("scans:shutdown")
    }
}
