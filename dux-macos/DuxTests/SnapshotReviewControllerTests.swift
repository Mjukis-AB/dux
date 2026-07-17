import XCTest
@testable import DUX

final class SnapshotReviewControllerTests: XCTestCase {
    func testAcquireDeduplicatesAndReleaseIsExplicit() async throws {
        let lease = StubSnapshotReviewLease(scanID: "scan:one")
        let service = StubSnapshotReviewService(leases: [lease])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )

        try await controller.acquire(scanID: "scan:one")
        try await controller.acquire(scanID: "scan:one")

        let acquisitions = await service.acquisitionCount()
        let activeBeforeRelease = await controller.activeLeaseCount()
        XCTAssertEqual(acquisitions, 1)
        XCTAssertEqual(activeBeforeRelease, 1)
        await controller.release(scanID: "scan:one")
        let releases = await lease.releaseCount()
        let activeAfterRelease = await controller.activeLeaseCount()
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(activeAfterRelease, 0)
    }

    func testConcurrentDuplicateAcquisitionReleasesStaleLease() async throws {
        let first = StubSnapshotReviewLease(scanID: "scan:one")
        let second = StubSnapshotReviewLease(scanID: "scan:one")
        let service = StubSnapshotReviewService(leases: [first, second], suspendedReturns: 2)
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )

        let left = Task { try await controller.acquire(scanID: "scan:one") }
        let right = Task { try await controller.acquire(scanID: "scan:one") }
        try await eventually { await service.suspendedAcquisitionCount() == 2 }
        await service.resumeAcquisitions()
        let results = await [left.result, right.result]

        let acquisitions = await service.acquisitionCount()
        let active = await controller.activeLeaseCount()
        let firstReleasesBeforeShutdown = await first.releaseCount()
        let secondReleasesBeforeShutdown = await second.releaseCount()
        let releasesBeforeShutdown = firstReleasesBeforeShutdown + secondReleasesBeforeShutdown
        XCTAssertEqual(acquisitions, 2)
        XCTAssertEqual(active, 1)
        XCTAssertEqual(releasesBeforeShutdown, 1)
        XCTAssertEqual(results.filter(\.isSuccess).count, 1)
        XCTAssertEqual(results.filter(\.isCancellation).count, 1)
        await controller.shutdown()
        let firstReleasesAfterShutdown = await first.releaseCount()
        let secondReleasesAfterShutdown = await second.releaseCount()
        let releasesAfterShutdown = firstReleasesAfterShutdown + secondReleasesAfterShutdown
        XCTAssertEqual(releasesAfterShutdown, 2)
    }

    func testRenewFailureDropsAndReleasesRetainedHandle() async throws {
        let lease = StubSnapshotReviewLease(scanID: "scan:one", renewFails: true)
        let service = StubSnapshotReviewService(leases: [lease])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        await controller.renewNow()

        let renewals = await lease.renewCount()
        let releases = await lease.releaseCount()
        let active = await controller.activeLeaseCount()
        XCTAssertEqual(renewals, 1)
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(active, 0)
    }

    func testReleaseInvalidatesPendingAcquisitionAndReleasesItsCompletion() async throws {
        let lease = StubSnapshotReviewLease(scanID: "scan:one")
        let service = StubSnapshotReviewService(
            leases: [lease],
            suspendedReturns: 1
        )
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )

        let acquisition = Task {
            try await controller.acquire(scanID: "scan:one")
        }
        try await eventually { await service.hasSuspendedAcquisition() }
        await controller.release(scanID: "scan:one")
        await service.resumeAcquisitions()

        do {
            try await acquisition.value
            XCTFail("Expected the released pending acquisition to be cancelled")
        } catch is CancellationError {
            // Expected: the selection closed before the lease arrived.
        }
        let releases = await lease.releaseCount()
        let active = await controller.activeLeaseCount()
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(active, 0)
    }

    func testStaleRenewFailureCannotRemoveReplacementLease() async throws {
        let old = StubSnapshotReviewLease(
            scanID: "scan:one",
            renewFails: true,
            suspendsFirstRenewal: true
        )
        let replacement = StubSnapshotReviewLease(scanID: "scan:one")
        let service = StubSnapshotReviewService(leases: [old, replacement])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        let renewal = Task { await controller.renewNow() }
        try await eventually { await old.hasSuspendedRenewal() }
        await controller.release(scanID: "scan:one")
        try await controller.acquire(scanID: "scan:one")
        await old.resumeRenewal()
        await renewal.value

        let active = await controller.activeLeaseCount()
        let replacementReleases = await replacement.releaseCount()
        XCTAssertEqual(active, 1)
        XCTAssertEqual(replacementReleases, 0)
        await controller.shutdown()
        let finalReplacementReleases = await replacement.releaseCount()
        XCTAssertEqual(finalReplacementReleases, 1)
    }

    func testOverlappingRenewalsAreSerializedAndCoalesced() async throws {
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            suspendsFirstRenewal: true
        )
        let service = StubSnapshotReviewService(leases: [lease])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        let first = Task { await controller.renewNow() }
        try await eventually { await lease.hasSuspendedRenewal() }
        await controller.renewNow()
        let renewalsWhileSuspended = await lease.renewCount()
        XCTAssertEqual(renewalsWhileSuspended, 1)
        await lease.resumeRenewal()
        await first.value

        let renewals = await lease.renewCount()
        XCTAssertEqual(renewals, 2)
        await controller.shutdown()
    }

    func testShutdownReleasesEveryLeaseExactlyOnce() async throws {
        let first = StubSnapshotReviewLease(scanID: "scan:one")
        let second = StubSnapshotReviewLease(scanID: "scan:two")
        let service = StubSnapshotReviewService(leases: [first, second])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        try await controller.acquire(scanID: "scan:two")

        await controller.shutdown()
        await controller.shutdown()

        let firstReleases = await first.releaseCount()
        let secondReleases = await second.releaseCount()
        let active = await controller.activeLeaseCount()
        XCTAssertEqual(firstReleases, 1)
        XCTAssertEqual(secondReleases, 1)
        XCTAssertEqual(active, 0)
    }

    private func eventually(
        _ condition: @escaping @Sendable () async -> Bool,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws {
        for _ in 0 ..< 2_000 {
            if await condition() {
                return
            }
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTFail("Condition did not become true", file: file, line: line)
    }
}

private extension Result where Success == Void, Failure == any Error {
    var isSuccess: Bool {
        if case .success = self { true } else { false }
    }

    var isCancellation: Bool {
        if case let .failure(error) = self { error is CancellationError } else { false }
    }
}

private struct SuspendedSnapshotReviewClock: DuxSnapshotReviewRenewalClock {
    func sleepForRenewalInterval() async throws {
        try await Task.sleep(for: .seconds(3_600))
    }
}

private actor StubSnapshotReviewService: DuxSnapshotReviewServing {
    private var leases: [StubSnapshotReviewLease]
    private var acquisitions = 0
    private var suspendedReturnsRemaining: Int
    private var acquisitionContinuations: [CheckedContinuation<Void, Never>] = []

    init(
        leases: [StubSnapshotReviewLease],
        suspendedReturns: Int = 0
    ) {
        self.leases = leases
        suspendedReturnsRemaining = suspendedReturns
    }

    func acquireExplorerReview(scanID: String) async throws -> any DuxSnapshotReviewLease {
        acquisitions += 1
        guard !leases.isEmpty else {
            throw EngineServiceError.unexpected("missing test lease")
        }
        let lease = leases.removeFirst()
        XCTAssertEqual(lease.scanID, scanID)
        if suspendedReturnsRemaining > 0 {
            suspendedReturnsRemaining -= 1
            await withCheckedContinuation { continuation in
                acquisitionContinuations.append(continuation)
            }
        }
        return lease
    }

    func acquisitionCount() -> Int {
        acquisitions
    }

    func hasSuspendedAcquisition() -> Bool {
        !acquisitionContinuations.isEmpty
    }

    func suspendedAcquisitionCount() -> Int {
        acquisitionContinuations.count
    }

    func resumeAcquisitions() {
        let continuations = acquisitionContinuations
        acquisitionContinuations.removeAll(keepingCapacity: false)
        for continuation in continuations {
            continuation.resume()
        }
    }
}

private actor StubSnapshotReviewLease: DuxSnapshotReviewLease {
    nonisolated let scanID: String

    private let renewFails: Bool
    private let suspendsFirstRenewal: Bool
    private var renewals = 0
    private var releases = 0
    private var renewalContinuation: CheckedContinuation<Void, Never>?

    init(
        scanID: String,
        renewFails: Bool = false,
        suspendsFirstRenewal: Bool = false
    ) {
        self.scanID = scanID
        self.renewFails = renewFails
        self.suspendsFirstRenewal = suspendsFirstRenewal
    }

    func renew() async throws -> Int64 {
        renewals += 1
        if suspendsFirstRenewal, renewals == 1 {
            await withCheckedContinuation { continuation in
                renewalContinuation = continuation
            }
        }
        if renewFails {
            throw EngineServiceError.unexpected("renew failed")
        }
        return 1
    }

    func release() {
        releases += 1
    }

    func renewCount() -> Int {
        renewals
    }

    func hasSuspendedRenewal() -> Bool {
        renewalContinuation != nil
    }

    func resumeRenewal() {
        renewalContinuation?.resume()
        renewalContinuation = nil
    }

    func releaseCount() -> Int {
        releases
    }
}
