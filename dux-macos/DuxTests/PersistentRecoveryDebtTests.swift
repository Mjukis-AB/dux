@testable import DUX
import XCTest

final class PersistentRecoveryDebtAdapterTests: XCTestCase {
    func testAdapterAcceptsExactBoundedCounts() throws {
        let mapped = try EngineService.persistentRecoveryDebt(
            RunningScanDebtCensus(
                recordVersion: 1,
                inspectedUnclaimedCount: 64,
                pristineUnclaimedCount: 60,
                unexplainedUnclaimedCount: 4,
                hasMore: true
            )
        )

        XCTAssertEqual(mapped.inspectedUnclaimedCount, 64)
        XCTAssertEqual(mapped.pristineUnclaimedCount, 60)
        XCTAssertEqual(mapped.unexplainedUnclaimedCount, 4)
        XCTAssertTrue(mapped.hasMore)
        XCTAssertEqual(mapped.displayedCount, "64+")
        XCTAssertEqual(
            mapped.accessibilityCount,
            "64 or more retained recovery records; incomplete bounded census"
        )
    }

    func testAdapterRejectsMalformedEnvelopeAndAccounting() {
        let malformed = [
            RunningScanDebtCensus(
                recordVersion: 2,
                inspectedUnclaimedCount: 0,
                pristineUnclaimedCount: 0,
                unexplainedUnclaimedCount: 0,
                hasMore: false
            ),
            RunningScanDebtCensus(
                recordVersion: 1,
                inspectedUnclaimedCount: 65,
                pristineUnclaimedCount: 65,
                unexplainedUnclaimedCount: 0,
                hasMore: true
            ),
            RunningScanDebtCensus(
                recordVersion: 1,
                inspectedUnclaimedCount: 2,
                pristineUnclaimedCount: 1,
                unexplainedUnclaimedCount: 0,
                hasMore: false
            ),
            RunningScanDebtCensus(
                recordVersion: 1,
                inspectedUnclaimedCount: 2,
                pristineUnclaimedCount: 1,
                unexplainedUnclaimedCount: 1,
                hasMore: true
            ),
        ]

        for value in malformed {
            XCTAssertThrowsError(try EngineService.persistentRecoveryDebt(value)) { error in
                XCTAssertEqual(
                    error as? PersistentRecoveryDebtServiceError,
                    .invalidResponse
                )
            }
        }
    }

    func testPresentationAndAccessibilityIdentifiersAreTruthfulAndUnique() {
        let none = PersistentRecoveryDebt(
            inspectedUnclaimedCount: 0,
            pristineUnclaimedCount: 0,
            unexplainedUnclaimedCount: 0,
            hasMore: false
        )
        let one = PersistentRecoveryDebt(
            inspectedUnclaimedCount: 1,
            pristineUnclaimedCount: 1,
            unexplainedUnclaimedCount: 0,
            hasMore: false
        )

        XCTAssertEqual(none.displayedCount, "0")
        XCTAssertEqual(none.accessibilityCount, "0 retained recovery records")
        XCTAssertEqual(one.displayedCount, "1")
        XCTAssertEqual(one.accessibilityCount, "1 retained recovery record")

        let identifiers = PersistentRecoveryDebtAccessibility.allControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
    }
}

final class ClaimedRunningScanProvenanceAdapterTests: XCTestCase {
    func testAdapterAcceptsFiveWayBoundedAccounting() throws {
        let mapped = try EngineService.claimedRunningScanProvenance(
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 64,
                sameHostCurrentBootCount: 40,
                sameHostPriorBootCount: 10,
                foreignHostCount: 5,
                storedUnprovenCount: 9,
                currentContextUnavailableCount: 0,
                hasMore: true
            )
        )

        XCTAssertEqual(mapped.inspectedClaimedCount, 64)
        XCTAssertEqual(mapped.sameHostCurrentBootCount, 40)
        XCTAssertEqual(mapped.sameHostPriorBootCount, 10)
        XCTAssertEqual(mapped.foreignHostCount, 5)
        XCTAssertEqual(mapped.storedUnprovenCount, 9)
        XCTAssertEqual(mapped.currentContextUnavailableCount, 0)
        XCTAssertTrue(mapped.hasMore)
        XCTAssertEqual(mapped.displayedCount, "64+")
        XCTAssertEqual(
            mapped.accessibilityCount,
            "64 or more claimed running scan records; incomplete bounded census"
        )
        XCTAssertEqual(
            mapped.accessibilityDistribution,
            "Current startup session 40; earlier startup session on this Mac 10; "
                + "different host 5; identity not stored 9; current identity unavailable 0"
        )

        let unavailable = try EngineService.claimedRunningScanProvenance(
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 5,
                sameHostCurrentBootCount: 0,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 2,
                currentContextUnavailableCount: 3,
                hasMore: false
            )
        )
        XCTAssertEqual(unavailable.storedUnprovenCount, 2)
        XCTAssertEqual(unavailable.currentContextUnavailableCount, 3)
    }

    func testAdapterRejectsMalformedEnvelopeAccountingAndTruncation() {
        let malformed = [
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 2,
                inspectedClaimedCount: 0,
                sameHostCurrentBootCount: 0,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: false
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 65,
                sameHostCurrentBootCount: 65,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: true
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 2,
                sameHostCurrentBootCount: 1,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: false
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 2,
                sameHostCurrentBootCount: 1,
                sameHostPriorBootCount: 1,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: true
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 2,
                sameHostCurrentBootCount: 1,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 1,
                hasMore: false
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 64,
                sameHostCurrentBootCount: .max,
                sameHostPriorBootCount: .max,
                foreignHostCount: .max,
                storedUnprovenCount: .max,
                currentContextUnavailableCount: .max,
                hasMore: true
            ),
        ]

        for value in malformed {
            XCTAssertThrowsError(
                try EngineService.claimedRunningScanProvenance(value)
            ) { error in
                XCTAssertEqual(
                    error as? ClaimedRunningScanProvenanceServiceError,
                    .invalidResponse
                )
            }
        }
    }

    func testPresentationAndAccessibilityIdentifiersAreTruthfulAndUnique() {
        let none = ClaimedRunningScanProvenance(
            inspectedClaimedCount: 0,
            sameHostCurrentBootCount: 0,
            sameHostPriorBootCount: 0,
            foreignHostCount: 0,
            storedUnprovenCount: 0,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        let one = ClaimedRunningScanProvenance(
            inspectedClaimedCount: 1,
            sameHostCurrentBootCount: 1,
            sameHostPriorBootCount: 0,
            foreignHostCount: 0,
            storedUnprovenCount: 0,
            currentContextUnavailableCount: 0,
            hasMore: false
        )

        XCTAssertEqual(none.displayedCount, "0")
        XCTAssertEqual(none.accessibilityCount, "0 claimed running scan records")
        XCTAssertEqual(one.displayedCount, "1")
        XCTAssertEqual(one.accessibilityCount, "1 claimed running scan record")

        let identifiers =
            ClaimedRunningScanProvenanceAccessibility.allControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
        XCTAssertTrue(
            Set(identifiers).isDisjoint(
                with: Set(PersistentRecoveryDebtAccessibility.allControlIdentifiers)
            )
        )
    }
}

@MainActor
final class PersistentRecoveryDebtAppModelTests: XCTestCase {
    func testLoadIsLazyCoalescedAndFailedRefreshKeepsEarlierResult() async {
        let service = PersistentRecoveryDebtEngineSpy()
        let model = AppModel(engineService: service)

        var requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 0)
        XCTAssertNil(model.persistentRecoveryDebt)
        XCTAssertEqual(model.persistentRecoveryDebtState, .idle)

        let first = Task { @MainActor in
            await model.loadPersistentRecoveryDebt()
        }
        await service.waitForRequestCount(1)
        let second = Task { @MainActor in
            await model.loadPersistentRecoveryDebt()
        }
        await Task.yield()
        requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
        XCTAssertEqual(model.persistentRecoveryDebtState, .loading)

        let initial = PersistentRecoveryDebt(
            inspectedUnclaimedCount: 2,
            pristineUnclaimedCount: 1,
            unexplainedUnclaimedCount: 1,
            hasMore: false
        )
        await service.resolve(at: 0, with: .success(initial))
        await first.value
        await second.value

        XCTAssertEqual(model.persistentRecoveryDebt, initial)
        XCTAssertEqual(model.persistentRecoveryDebtState, .loaded)
        let readAt = model.persistentRecoveryDebtReadAt
        XCTAssertNotNil(readAt)

        let refresh = Task { @MainActor in
            await model.refreshPersistentRecoveryDebt()
        }
        await service.waitForRequestCount(2)
        await service.resolve(at: 1, with: .failure(.retryable))
        await refresh.value

        XCTAssertEqual(model.persistentRecoveryDebt, initial)
        XCTAssertEqual(model.persistentRecoveryDebtReadAt, readAt)
        XCTAssertEqual(model.persistentRecoveryDebtState, .failed(.retryable))
    }

    func testInvalidationFencesLateReplyAndFutureLoads() async {
        let service = PersistentRecoveryDebtEngineSpy()
        let model = AppModel(engineService: service)

        let load = Task { @MainActor in
            await model.loadPersistentRecoveryDebt()
        }
        await service.waitForRequestCount(1)
        model.invalidatePersistentRecoveryDebtOperations()
        await service.resolve(
            at: 0,
            with: .success(
                PersistentRecoveryDebt(
                    inspectedUnclaimedCount: 1,
                    pristineUnclaimedCount: 1,
                    unexplainedUnclaimedCount: 0,
                    hasMore: false
                )
            )
        )
        await load.value

        XCTAssertNil(model.persistentRecoveryDebt)
        XCTAssertNil(model.persistentRecoveryDebtReadAt)
        XCTAssertEqual(model.persistentRecoveryDebtState, .idle)

        await model.loadPersistentRecoveryDebt()
        let requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
    }
}

@MainActor
final class ClaimedRunningScanProvenanceAppModelTests: XCTestCase {
    func testLoadIsLazyCoalescedAndFailedRefreshKeepsEarlierResult() async {
        let service = ClaimedRunningScanProvenanceEngineSpy()
        let model = AppModel(engineService: service)

        var requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 0)
        XCTAssertNil(model.claimedRunningScanProvenance)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .idle)

        let first = Task { @MainActor in
            await model.loadClaimedRunningScanProvenance()
        }
        await service.waitForRequestCount(1)
        let second = Task { @MainActor in
            await model.loadClaimedRunningScanProvenance()
        }
        await Task.yield()
        requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .loading)

        let initial = ClaimedRunningScanProvenance(
            inspectedClaimedCount: 3,
            sameHostCurrentBootCount: 1,
            sameHostPriorBootCount: 1,
            foreignHostCount: 0,
            storedUnprovenCount: 1,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        await service.resolve(at: 0, with: .success(initial))
        await first.value
        await second.value

        XCTAssertEqual(model.claimedRunningScanProvenance, initial)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .loaded)
        let readAt = model.claimedRunningScanProvenanceReadAt
        XCTAssertNotNil(readAt)

        let refresh = Task { @MainActor in
            await model.refreshClaimedRunningScanProvenance()
        }
        await service.waitForRequestCount(2)
        await service.resolve(at: 1, with: .failure(.retryable))
        await refresh.value

        XCTAssertEqual(model.claimedRunningScanProvenance, initial)
        XCTAssertEqual(model.claimedRunningScanProvenanceReadAt, readAt)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .failed(.retryable))
    }

    func testInvalidationFencesLateReplyAndFutureLoads() async {
        let service = ClaimedRunningScanProvenanceEngineSpy()
        let model = AppModel(engineService: service)

        let load = Task { @MainActor in
            await model.loadClaimedRunningScanProvenance()
        }
        await service.waitForRequestCount(1)
        model.invalidateClaimedRunningScanProvenanceOperations()
        await service.resolve(
            at: 0,
            with: .success(
                ClaimedRunningScanProvenance(
                    inspectedClaimedCount: 1,
                    sameHostCurrentBootCount: 1,
                    sameHostPriorBootCount: 0,
                    foreignHostCount: 0,
                    storedUnprovenCount: 0,
                    currentContextUnavailableCount: 0,
                    hasMore: false
                )
            )
        )
        await load.value

        XCTAssertNil(model.claimedRunningScanProvenance)
        XCTAssertNil(model.claimedRunningScanProvenanceReadAt)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .idle)

        await model.loadClaimedRunningScanProvenance()
        let requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
    }
}

private actor PersistentRecoveryDebtEngineSpy: EngineServing {
    private struct PendingReply {
        var continuation:
            CheckedContinuation<PersistentRecoveryDebt, any Error>?
    }

    private var requests = 0
    private var replies: [PendingReply] = []

    func loadStatus() async throws -> EngineStatus {
        throw EngineServiceError.unavailable
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
            policy: try await loadDiskPressurePolicy(),
            changed: true
        )
    }

    func loadPersistentRecoveryDebt() async throws -> PersistentRecoveryDebt {
        requests += 1
        return try await withCheckedThrowingContinuation { continuation in
            replies.append(PendingReply(continuation: continuation))
        }
    }

    func requestCount() -> Int {
        requests
    }

    func waitForRequestCount(_ expected: Int) async {
        while requests < expected {
            await Task.yield()
        }
    }

    func resolve(
        at index: Int,
        with result:
        Result<PersistentRecoveryDebt, PersistentRecoveryDebtServiceError>
    ) {
        guard replies.indices.contains(index),
              let continuation = replies[index].continuation
        else {
            XCTFail("Missing persistent recovery debt reply \(index)")
            return
        }
        replies[index].continuation = nil
        switch result {
        case let .success(observation):
            continuation.resume(returning: observation)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }
}

private actor ClaimedRunningScanProvenanceEngineSpy: EngineServing {
    private struct PendingReply {
        var continuation:
            CheckedContinuation<ClaimedRunningScanProvenance, any Error>?
    }

    private var requests = 0
    private var replies: [PendingReply] = []

    func loadStatus() async throws -> EngineStatus {
        throw EngineServiceError.unavailable
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
            policy: try await loadDiskPressurePolicy(),
            changed: true
        )
    }

    func loadClaimedRunningScanProvenance() async throws
        -> ClaimedRunningScanProvenance
    {
        requests += 1
        return try await withCheckedThrowingContinuation { continuation in
            replies.append(PendingReply(continuation: continuation))
        }
    }

    func requestCount() -> Int {
        requests
    }

    func waitForRequestCount(_ expected: Int) async {
        while requests < expected {
            await Task.yield()
        }
    }

    func resolve(
        at index: Int,
        with result:
        Result<
            ClaimedRunningScanProvenance,
            ClaimedRunningScanProvenanceServiceError
        >
    ) {
        guard replies.indices.contains(index),
              let continuation = replies[index].continuation
        else {
            XCTFail("Missing claimed running scan provenance reply \(index)")
            return
        }
        replies[index].continuation = nil
        switch result {
        case let .success(observation):
            continuation.resume(returning: observation)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }
}
