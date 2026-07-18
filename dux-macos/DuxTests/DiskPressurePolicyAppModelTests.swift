import XCTest
@testable import DUX

@MainActor
final class DiskPressurePolicyAppModelTests: XCTestCase {
    func testChangedSaveRequestsExactlyOneResampleAndUnchangedSaveRequestsNone() async {
        let engine = PolicyEngineSpy()
        let resamples = PolicyResampleSpy()
        let model = AppModel(
            engineService: engine,
            capacityResampleRequester: resamples
        )
        await model.loadDiskPressurePolicy()
        model.diskPressurePolicyDraft.criticalGiB = "9"
        await engine.enqueueSetResult(changed: true)

        await model.saveDiskPressurePolicy()

        var resampleCount = await resamples.count()
        XCTAssertEqual(resampleCount, 1)
        XCTAssertEqual(model.diskPressurePolicy?.source, .stored)
        XCTAssertEqual(model.diskPressurePolicy?.configuration.criticalAvailableBytes, 9 << 30)

        await engine.enqueueSetResult(changed: false)
        await model.saveDiskPressurePolicy()
        resampleCount = await resamples.count()
        XCTAssertEqual(resampleCount, 1)
    }

    func testChangedResetRequestsOneResampleAndUnchangedResetRequestsNone() async {
        let engine = PolicyEngineSpy()
        let resamples = PolicyResampleSpy()
        let model = AppModel(
            engineService: engine,
            capacityResampleRequester: resamples
        )
        await model.loadDiskPressurePolicy()
        await engine.enqueueResetResult(changed: true)
        await model.resetDiskPressurePolicy()
        var resampleCount = await resamples.count()
        XCTAssertEqual(resampleCount, 1)
        XCTAssertEqual(model.diskPressurePolicy?.source, .default)

        await engine.enqueueResetResult(changed: false)
        await model.resetDiskPressurePolicy()
        resampleCount = await resamples.count()
        XCTAssertEqual(resampleCount, 1)
    }

    func testFailedSavePreservesLastGoodPolicyAndAttemptedDraft() async {
        let engine = PolicyEngineSpy()
        let resamples = PolicyResampleSpy()
        let model = AppModel(
            engineService: engine,
            capacityResampleRequester: resamples
        )
        await model.loadDiskPressurePolicy()
        let lastGood = model.diskPressurePolicy
        model.diskPressurePolicyDraft.warningGiB = "12.5"
        await engine.enqueueSetFailure(.retryable)

        await model.saveDiskPressurePolicy()

        XCTAssertEqual(model.diskPressurePolicy, lastGood)
        XCTAssertEqual(model.diskPressurePolicyDraft.warningGiB, "12.5")
        XCTAssertEqual(model.diskPressurePolicyState, .failed(.service(.retryable)))
        let resampleCount = await resamples.count()
        XCTAssertEqual(resampleCount, 0)
    }

    func testInvalidDraftNeverCallsEngineOrResamples() async {
        let engine = PolicyEngineSpy()
        let resamples = PolicyResampleSpy()
        let model = AppModel(
            engineService: engine,
            capacityResampleRequester: resamples
        )
        await model.loadDiskPressurePolicy()
        model.diskPressurePolicyDraft.recoveryGiB = "0.1"

        await model.saveDiskPressurePolicy()

        let setRequestCount = await engine.setRequestCount()
        let resampleCount = await resamples.count()
        XCTAssertEqual(setRequestCount, 0)
        XCTAssertEqual(resampleCount, 0)
        XCTAssertEqual(
            model.diskPressurePolicyState,
            .failed(.draft(.invalidNumber(.recoveryGiB)))
        )
    }

    func testCancellationRejectsLateWriteAndResample() async {
        let engine = PolicyEngineSpy()
        let resamples = PolicyResampleSpy()
        let model = AppModel(
            engineService: engine,
            capacityResampleRequester: resamples
        )
        await model.loadDiskPressurePolicy()
        let lastGood = model.diskPressurePolicy
        await engine.suspendNextSet()
        let save = Task { @MainActor in
            await model.saveDiskPressurePolicy()
        }
        await engine.waitForSetRequest(count: 1)

        model.invalidatePressurePolicyOperations()
        await engine.completeSuspendedSet(changed: true)
        await save.value

        XCTAssertEqual(model.diskPressurePolicy, lastGood)
        XCTAssertEqual(model.diskPressurePolicyState, .ready)
        let resampleCount = await resamples.count()
        XCTAssertEqual(resampleCount, 0)
        await model.resetDiskPressurePolicy()
        let resetRequestCount = await engine.resetRequestCount()
        XCTAssertEqual(resetRequestCount, 0)
    }

    func testOverlappingMutationIsRejected() async {
        let engine = PolicyEngineSpy()
        let model = AppModel(engineService: engine)
        await model.loadDiskPressurePolicy()
        await engine.suspendNextSet()
        let save = Task { @MainActor in
            await model.saveDiskPressurePolicy()
        }
        await engine.waitForSetRequest(count: 1)

        await model.resetDiskPressurePolicy()
        let resetRequestCount = await engine.resetRequestCount()
        XCTAssertEqual(resetRequestCount, 0)

        await engine.completeSuspendedSet(changed: false)
        await save.value
        XCTAssertEqual(model.diskPressurePolicyState, .ready)
    }
}

private actor PolicyResampleSpy: DuxCapacityResampleRequesting {
    private var requestCount = 0

    func requestCapacityResample() {
        requestCount += 1
    }

    func count() -> Int {
        requestCount
    }
}

private actor PolicyEngineSpy: EngineServing {
    private var policy = DiskPressurePolicy(
        source: .default,
        revision: 0,
        configuration: .defaults,
        updatedAtUnixMilliseconds: nil
    )
    private var setResults: [Result<Bool, DiskPressurePolicyServiceError>] = []
    private var resetResults: [Bool] = []
    private var shouldSuspendSet = false
    private var suspendedSet: CheckedContinuation<Bool, Never>?
    private var setRequests = 0
    private var resetRequests = 0
    private var setWaiters: [(Int, CheckedContinuation<Void, Never>)] = []

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 12, executedOffMainThread: true)
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        policy
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        setRequests += 1
        resumeSetWaiters()
        let changed: Bool
        if shouldSuspendSet {
            shouldSuspendSet = false
            changed = await withCheckedContinuation { continuation in
                suspendedSet = continuation
            }
        } else if !setResults.isEmpty {
            changed = try setResults.removeFirst().get()
        } else {
            changed = true
        }
        if changed {
            policy = DiskPressurePolicy(
                source: .stored,
                revision: policy.revision + 1,
                configuration: configuration,
                updatedAtUnixMilliseconds: Int64(policy.revision + 1)
            )
        }
        return DiskPressurePolicyUpdateResult(policy: policy, changed: changed)
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        resetRequests += 1
        let changed = resetResults.isEmpty ? true : resetResults.removeFirst()
        if changed {
            policy = DiskPressurePolicy(
                source: .default,
                revision: policy.revision + 1,
                configuration: .defaults,
                updatedAtUnixMilliseconds: Int64(policy.revision + 1)
            )
        }
        return DiskPressurePolicyUpdateResult(policy: policy, changed: changed)
    }

    func enqueueSetResult(changed: Bool) {
        setResults.append(.success(changed))
    }

    func enqueueSetFailure(_ error: DiskPressurePolicyServiceError) {
        setResults.append(.failure(error))
    }

    func enqueueResetResult(changed: Bool) {
        resetResults.append(changed)
    }

    func suspendNextSet() {
        shouldSuspendSet = true
    }

    func waitForSetRequest(count: Int) async {
        guard setRequests < count else {
            return
        }
        await withCheckedContinuation { continuation in
            setWaiters.append((count, continuation))
        }
    }

    func completeSuspendedSet(changed: Bool) {
        suspendedSet?.resume(returning: changed)
        suspendedSet = nil
    }

    func setRequestCount() -> Int { setRequests }
    func resetRequestCount() -> Int { resetRequests }

    private func resumeSetWaiters() {
        let ready = setWaiters.filter { $0.0 <= setRequests }
        setWaiters.removeAll { $0.0 <= setRequests }
        for waiter in ready {
            waiter.1.resume()
        }
    }
}
