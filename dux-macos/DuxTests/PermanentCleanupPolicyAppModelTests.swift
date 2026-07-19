import XCTest
@testable import DUX

@MainActor
final class PermanentCleanupPolicyAppModelTests: XCTestCase {
    func testSettingsSafetyAccessibilityAndMessagesStayStable() {
        let identifiers = PermanentCleanupPolicyAccessibility.allControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertFalse(
            DuxSettingsView.message(for: .confirmationRequired).isEmpty
        )
        XCTAssertFalse(
            DuxSettingsView.message(for: .service(.invalidResponse)).isEmpty
        )
    }

    func testReenableRequiresExactConfirmationAndSuccessfulChangesPublish() async {
        let engine = PermanentCleanupEngineSpy()
        let model = AppModel(engineService: engine)

        await model.loadPermanentCleanupPolicy()
        XCTAssertEqual(model.permanentCleanupPolicy?.enabled, true)

        await model.setPermanentCleanupEnabled(false)
        XCTAssertEqual(model.permanentCleanupPolicy?.enabled, false)
        var setCount = await engine.setRequestCount()
        XCTAssertEqual(setCount, 1)

        await model.setPermanentCleanupEnabled(true, confirmation: "enable permanent cleanup")
        XCTAssertEqual(model.permanentCleanupPolicy?.enabled, false)
        XCTAssertEqual(
            model.permanentCleanupPolicyState,
            .failed(.confirmationRequired)
        )
        setCount = await engine.setRequestCount()
        XCTAssertEqual(setCount, 1)

        await model.setPermanentCleanupEnabled(
            true,
            confirmation: AppModel.permanentCleanupReenableConfirmation
        )
        XCTAssertEqual(model.permanentCleanupPolicy?.enabled, true)
        setCount = await engine.setRequestCount()
        XCTAssertEqual(setCount, 2)
    }

    func testResetFromDisabledAlsoRequiresConfirmation() async {
        let engine = PermanentCleanupEngineSpy()
        let model = AppModel(engineService: engine)
        await model.loadPermanentCleanupPolicy()
        await model.setPermanentCleanupEnabled(false)

        await model.resetPermanentCleanup()
        XCTAssertEqual(model.permanentCleanupPolicy?.enabled, false)
        XCTAssertEqual(model.permanentCleanupPolicyState, .failed(.confirmationRequired))
        var resetCount = await engine.resetRequestCount()
        XCTAssertEqual(resetCount, 0)

        await model.resetPermanentCleanup(
            confirmation: AppModel.permanentCleanupReenableConfirmation
        )
        XCTAssertEqual(model.permanentCleanupPolicy?.enabled, true)
        resetCount = await engine.resetRequestCount()
        XCTAssertEqual(resetCount, 1)
    }

    func testInvalidationRejectsLateLoadPublication() async {
        let engine = PermanentCleanupEngineSpy()
        let model = AppModel(engineService: engine)
        await engine.suspendNextLoad()

        let load = Task { @MainActor in
            await model.loadPermanentCleanupPolicy()
        }
        await engine.waitForLoadRequest()

        model.invalidatePermanentCleanupPolicyOperations()
        await engine.completeSuspendedLoad()
        await load.value

        XCTAssertNil(model.permanentCleanupPolicy)
        XCTAssertEqual(model.permanentCleanupPolicyState, .idle)
    }
}

private actor PermanentCleanupEngineSpy: EngineServing {
    private var policy = PermanentCleanupPolicy(
        enabled: true,
        source: .default,
        revision: 0,
        updatedAtUnixMilliseconds: nil
    )
    private var setCount = 0
    private var resetCount = 0
    private var suspendNextLoad = false
    private var loadStarted = false
    private var loadWaiter: CheckedContinuation<Void, Never>?
    private var suspendedLoad: CheckedContinuation<PermanentCleanupPolicy, Never>?

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 22, executedOffMainThread: true)
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
                revision: 0,
                configuration: .defaults,
                updatedAtUnixMilliseconds: nil
            ),
            changed: true
        )
    }

    func loadPermanentCleanupPolicy() async throws -> PermanentCleanupPolicy {
        loadStarted = true
        loadWaiter?.resume()
        loadWaiter = nil
        if suspendNextLoad {
            suspendNextLoad = false
            return await withCheckedContinuation { continuation in
                suspendedLoad = continuation
            }
        }
        policy
    }

    func setPermanentCleanupEnabled(
        _ enabled: Bool
    ) async throws -> PermanentCleanupPolicyUpdateResult {
        setCount += 1
        policy = PermanentCleanupPolicy(
            enabled: enabled,
            source: .stored,
            revision: policy.revision + 1,
            updatedAtUnixMilliseconds: Int64(policy.revision + 1)
        )
        return PermanentCleanupPolicyUpdateResult(policy: policy, changed: true)
    }

    func resetPermanentCleanup() async throws -> PermanentCleanupPolicyUpdateResult {
        resetCount += 1
        policy = PermanentCleanupPolicy(
            enabled: true,
            source: .default,
            revision: policy.revision + 1,
            updatedAtUnixMilliseconds: Int64(policy.revision + 1)
        )
        return PermanentCleanupPolicyUpdateResult(policy: policy, changed: true)
    }

    func setRequestCount() -> Int { setCount }
    func resetRequestCount() -> Int { resetCount }

    func suspendNextLoad() {
        suspendNextLoad = true
    }

    func waitForLoadRequest() async {
        guard !loadStarted else {
            return
        }
        await withCheckedContinuation { continuation in
            loadWaiter = continuation
        }
    }

    func completeSuspendedLoad() {
        suspendedLoad?.resume(returning: policy)
        suspendedLoad = nil
    }
}
