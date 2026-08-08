import XCTest
@testable import DUX

@MainActor
final class AppModelTerminalQuiescenceTests: XCTestCase {
    func testBeginFencesSynchronouslyCoalescesAndJoinsAcceptedLoad() async {
        let gate = AppModelTerminalGate()
        let engine = AppModelTerminalEngineSpy(loadGate: gate)
        let model = AppModel(engineService: engine)

        let acceptedLoad = Task { @MainActor in
            await model.loadEngineStatus()
        }
        await gate.waitUntilStarted()

        let firstDrain = model.beginTerminalRuntimeQuiescence()
        let secondDrain = model.beginTerminalRuntimeQuiescence()
        let firstCompletion = AppModelTerminalCompletionProbe()
        let secondCompletion = AppModelTerminalCompletionProbe()
        let firstWaiter = Task {
            await firstDrain.value
            await firstCompletion.finish()
        }
        let secondWaiter = Task {
            await secondDrain.value
            await secondCompletion.finish()
        }

        // The permanent fence is installed by begin(), not later when its
        // retained task first gets scheduled.
        await model.loadEngineStatus()
        await model.loadDiskPressurePolicy()
        model.requestExplorerDestination(.recommendations)
        await Task.yield()

        var statusLoads = await engine.statusLoadCount()
        var policyLoads = await engine.policyLoadCount()
        var firstFinished = await firstCompletion.isFinished()
        var secondFinished = await secondCompletion.isFinished()
        XCTAssertEqual(statusLoads, 1)
        XCTAssertEqual(policyLoads, 0)
        XCTAssertNil(model.pendingExplorerDestination)
        XCTAssertFalse(firstFinished)
        XCTAssertFalse(secondFinished)

        await gate.release()
        await acceptedLoad.value
        await firstWaiter.value
        await secondWaiter.value

        statusLoads = await engine.statusLoadCount()
        policyLoads = await engine.policyLoadCount()
        firstFinished = await firstCompletion.isFinished()
        secondFinished = await secondCompletion.isFinished()
        XCTAssertEqual(statusLoads, 1)
        XCTAssertEqual(policyLoads, 0)
        XCTAssertTrue(firstFinished)
        XCTAssertTrue(secondFinished)
        guard case .loading = model.engineState else {
            return XCTFail("A terminal-fenced load must not publish late state")
        }

        await model.loadEngineStatus()
        statusLoads = await engine.statusLoadCount()
        XCTAssertEqual(statusLoads, 1)
    }

    func testCancellingAwaiterDoesNotCancelRetainedDrain() async {
        let gate = AppModelTerminalGate()
        let engine = AppModelTerminalEngineSpy(loadGate: gate)
        let model = AppModel(engineService: engine)
        let acceptedLoad = Task { @MainActor in
            await model.loadEngineStatus()
        }
        await gate.waitUntilStarted()

        let completion = AppModelTerminalCompletionProbe()
        let waiter = Task { @MainActor in
            await model.quiesceForTerminalRuntime()
            await completion.finish()
        }
        waiter.cancel()
        await Task.yield()
        var finished = await completion.isFinished()
        XCTAssertFalse(finished)

        await gate.release()
        await acceptedLoad.value
        await waiter.value
        finished = await completion.isFinished()
        XCTAssertTrue(finished)
    }

    func testDrainJoinsInvalidatedOperationAfterPrimarySlotWasCleared() async {
        let loadGate = AppModelTerminalGate()
        let policyGate = AppModelTerminalGate()
        let engine = AppModelTerminalEngineSpy(
            loadGate: loadGate,
            policyGate: policyGate
        )
        let model = AppModel(engineService: engine)

        let acceptedPolicyLoad = Task { @MainActor in
            await model.loadDiskPressurePolicy()
        }
        await policyGate.waitUntilStarted()

        // Ordinary presentation invalidation cancels and clears the primary
        // slot. The service deliberately ignores task cancellation until its
        // own terminal result, so reset quiescence must retain it elsewhere.
        model.invalidatePressurePolicyOperations()
        let drain = model.beginTerminalRuntimeQuiescence()
        let completion = AppModelTerminalCompletionProbe()
        let waiter = Task {
            await drain.value
            await completion.finish()
        }
        await Task.yield()

        var finished = await completion.isFinished()
        XCTAssertFalse(finished)

        await policyGate.release()
        await acceptedPolicyLoad.value
        await waiter.value

        finished = await completion.isFinished()
        XCTAssertTrue(finished)
        let policyLoads = await engine.policyLoadCount()
        XCTAssertEqual(policyLoads, 1)
    }
}

private actor AppModelTerminalEngineSpy: EngineServing {
    private let loadGate: AppModelTerminalGate
    private let policyGate: AppModelTerminalGate?
    private var statusLoads = 0
    private var policyLoads = 0

    init(
        loadGate: AppModelTerminalGate,
        policyGate: AppModelTerminalGate? = nil
    ) {
        self.loadGate = loadGate
        self.policyGate = policyGate
    }

    func loadStatus() async throws -> EngineStatus {
        statusLoads += 1
        await loadGate.beginAndWaitForRelease()
        return EngineStatus(
            libraryVersion: "terminal-test",
            ffiContractVersion: 56,
            executedOffMainThread: true
        )
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        policyLoads += 1
        await policyGate?.beginAndWaitForRelease()
        return DiskPressurePolicy(
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

    func statusLoadCount() -> Int {
        statusLoads
    }

    func policyLoadCount() -> Int {
        policyLoads
    }
}

private actor AppModelTerminalGate {
    private var started = false
    private var released = false
    private var startWaiters: [CheckedContinuation<Void, Never>] = []
    private var releaseWaiters: [CheckedContinuation<Void, Never>] = []

    func beginAndWaitForRelease() async {
        started = true
        let waiters = startWaiters
        startWaiters.removeAll(keepingCapacity: false)
        for waiter in waiters {
            waiter.resume()
        }
        guard !released else {
            return
        }
        await withCheckedContinuation { continuation in
            releaseWaiters.append(continuation)
        }
    }

    func waitUntilStarted() async {
        guard !started else {
            return
        }
        await withCheckedContinuation { continuation in
            startWaiters.append(continuation)
        }
    }

    func release() {
        released = true
        let waiters = releaseWaiters
        releaseWaiters.removeAll(keepingCapacity: false)
        for waiter in waiters {
            waiter.resume()
        }
    }
}

private actor AppModelTerminalCompletionProbe {
    private var finished = false

    func finish() {
        finished = true
    }

    func isFinished() -> Bool {
        finished
    }
}
