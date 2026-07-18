import XCTest
@testable import DUX

final class SnapshotReviewControllerTests: XCTestCase {
    func testSubtreeScanStartsFromExactRetainedLease() async throws {
        let scanTask = ControllerScanTaskSpy()
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            subtreeStart: .success(.started(scanTask))
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        _ = try await controller.startSubtreeScan(sourceScanID: "scan:one", nodeID: 72)

        let nodeIDs = await lease.subtreeNodeIDs()
        let active = await controller.activeLeaseCount()
        XCTAssertEqual(nodeIDs, [72])
        XCTAssertEqual(active, 1)
        await controller.shutdown()
    }

    func testExpiredSubtreeStartDropsExactLease() async throws {
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            subtreeStart: .failure(.reviewExpired)
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        do {
            _ = try await controller.startSubtreeScan(sourceScanID: "scan:one", nodeID: 3)
            XCTFail("expected expired subtree review")
        } catch {
            XCTAssertEqual(error as? HomeScanServiceError, .rootUnavailable)
        }
        let active = await controller.activeLeaseCount()
        let releases = await lease.releaseCount()
        XCTAssertEqual(active, 0)
        XCTAssertEqual(releases, 1)
    }

    func testStaleSubtreeStartCancelsReturnedTaskBeforePublication() async throws {
        let scanTask = ControllerScanTaskSpy()
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            subtreeStart: .success(.started(scanTask)),
            suspendsSubtreeStart: true
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        let start = Task {
            try await controller.startSubtreeScan(sourceScanID: "scan:one", nodeID: 8)
        }
        try await eventually { await lease.hasSuspendedSubtreeStart() }

        await controller.release(scanID: "scan:one")
        await lease.resumeSubtreeStart()

        do {
            _ = try await start.value
            XCTFail("expected stale start cancellation")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
        let cancellations = await scanTask.cancelCount()
        XCTAssertEqual(cancellations, 1)
    }

    func testAcquireDeduplicatesUnderlyingLeaseAndReferenceCountsOwners() async throws {
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
        let ownersBeforeRelease = await controller.activeOwnerCount(scanID: "scan:one")
        XCTAssertEqual(acquisitions, 1)
        XCTAssertEqual(activeBeforeRelease, 1)
        XCTAssertEqual(ownersBeforeRelease, 2)
        await controller.release(scanID: "scan:one")
        let releasesAfterFirstOwner = await lease.releaseCount()
        let activeAfterFirstOwner = await controller.activeLeaseCount()
        let ownersAfterFirstOwner = await controller.activeOwnerCount(scanID: "scan:one")
        XCTAssertEqual(releasesAfterFirstOwner, 0)
        XCTAssertEqual(activeAfterFirstOwner, 1)
        XCTAssertEqual(ownersAfterFirstOwner, 1)
        await controller.release(scanID: "scan:one")
        let finalReleases = await lease.releaseCount()
        let activeAfterFinalOwner = await controller.activeLeaseCount()
        XCTAssertEqual(finalReleases, 1)
        XCTAssertEqual(activeAfterFinalOwner, 0)
    }

    func testAcquireLatestUsesReturnedScanIDAndRetainsLease() async throws {
        let lease = StubSnapshotReviewLease(scanID: "scan:latest")
        let service = StubSnapshotReviewService(leases: [lease])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )

        let scanID = try await controller.acquireLatest()

        let acquisitions = await service.acquisitionCount()
        let activeBeforeRelease = await controller.activeLeaseCount()
        XCTAssertEqual(scanID, "scan:latest")
        XCTAssertEqual(acquisitions, 1)
        XCTAssertEqual(activeBeforeRelease, 1)
        await controller.release(scanID: scanID)
        let releases = await lease.releaseCount()
        XCTAssertEqual(releases, 1)
    }

    func testLatestOwnersSharingScanCannotReleaseEachOthersLease() async throws {
        let retained = StubSnapshotReviewLease(scanID: "scan:latest")
        let duplicate = StubSnapshotReviewLease(scanID: "scan:latest")
        let service = StubSnapshotReviewService(leases: [retained, duplicate])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )

        let first = try await controller.acquireLatest()
        let second = try await controller.acquireLatest()
        XCTAssertEqual(first, second)
        let sharedOwners = await controller.activeOwnerCount(scanID: first)
        let duplicateReleases = await duplicate.releaseCount()
        XCTAssertEqual(sharedOwners, 2)
        XCTAssertEqual(duplicateReleases, 1)

        await controller.release(scanID: first)
        let leasesAfterFirst = await controller.activeLeaseCount()
        let ownersAfterFirst = await controller.activeOwnerCount(scanID: first)
        let retainedReleasesAfterFirst = await retained.releaseCount()
        XCTAssertEqual(leasesAfterFirst, 1)
        XCTAssertEqual(ownersAfterFirst, 1)
        XCTAssertEqual(retainedReleasesAfterFirst, 0)

        await controller.release(scanID: second)
        let finalLeases = await controller.activeLeaseCount()
        let finalReleases = await retained.releaseCount()
        XCTAssertEqual(finalLeases, 0)
        XCTAssertEqual(finalReleases, 1)
    }

    func testConcurrentLatestAcquisitionReleasesStaleLease() async throws {
        let first = StubSnapshotReviewLease(scanID: "scan:first")
        let second = StubSnapshotReviewLease(scanID: "scan:second")
        let service = StubSnapshotReviewService(leases: [first, second], suspendedReturns: 2)
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )

        let left = Task { try await controller.acquireLatest() }
        let right = Task { try await controller.acquireLatest() }
        try await eventually { await service.suspendedAcquisitionCount() == 2 }
        await service.resumeAcquisitions()
        let results = await [left.result, right.result]

        let active = await controller.activeLeaseCount()
        let firstReleases = await first.releaseCount()
        let secondReleases = await second.releaseCount()
        XCTAssertEqual(active, 1)
        XCTAssertEqual(firstReleases + secondReleases, 1)
        XCTAssertEqual(results.filter(\.isStringSuccess).count, 1)
        XCTAssertEqual(results.filter(\.isCancellation).count, 1)
        await controller.shutdown()
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

    func testExpiredNavigationDropsLeaseAndAllowsImmediateReacquisition() async throws {
        let expired = StubSnapshotReviewLease(scanID: "scan:one", navigationExpires: true)
        let replacement = StubSnapshotReviewLease(scanID: "scan:one")
        let service = StubSnapshotReviewService(leases: [expired, replacement])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        do {
            _ = try await controller.rootNode(scanID: "scan:one")
            XCTFail("expected expired review")
        } catch {
            XCTAssertEqual(error as? ExplorerSnapshotNodeError, .reviewExpired)
        }
        let activeAfterExpiry = await controller.activeLeaseCount()
        let expiredReleases = await expired.releaseCount()
        XCTAssertEqual(activeAfterExpiry, 0)
        XCTAssertEqual(expiredReleases, 1)

        try await controller.acquire(scanID: "scan:one")
        let acquisitions = await service.acquisitionCount()
        let activeAfterReacquire = await controller.activeLeaseCount()
        XCTAssertEqual(acquisitions, 2)
        XCTAssertEqual(activeAfterReacquire, 1)
        await controller.shutdown()
    }

    func testExpiredTreemapDropsAndReleasesExactLease() async throws {
        let expired = StubSnapshotReviewLease(scanID: "scan:one", navigationExpires: true)
        let service = StubSnapshotReviewService(leases: [expired])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        do {
            _ = try await controller.treemap(
                scanID: "scan:one",
                parentID: 0,
                maxCells: 48
            )
            XCTFail("expected expired review")
        } catch {
            XCTAssertEqual(error as? ExplorerSnapshotTreemapError, .reviewExpired)
        }

        let active = await controller.activeLeaseCount()
        let releases = await expired.releaseCount()
        XCTAssertEqual(active, 0)
        XCTAssertEqual(releases, 1)
    }

    func testExpiredLargeFilesDropsAndReleasesExactLease() async throws {
        let expired = StubSnapshotReviewLease(scanID: "scan:one", navigationExpires: true)
        let service = StubSnapshotReviewService(leases: [expired])
        let controller = DuxSnapshotReviewController(
            service: service,
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        do {
            _ = try await controller.largeFiles(
                scanID: "scan:one",
                minimumLogicalBytes: 1_073_741_824,
                modifiedBefore: nil,
                maxResults: 100
            )
            XCTFail("expected expired review")
        } catch {
            XCTAssertEqual(error as? ExplorerSnapshotLargeFilesError, .reviewExpired)
        }

        let active = await controller.activeLeaseCount()
        let releases = await expired.releaseCount()
        XCTAssertEqual(active, 0)
        XCTAssertEqual(releases, 1)
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

private extension Result where Success == String, Failure == any Error {
    var isStringSuccess: Bool {
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
        let lease = try await nextLease()
        XCTAssertEqual(lease.scanID, scanID)
        return lease
    }

    func acquireLatestExplorerReview() async throws -> any DuxSnapshotReviewLease {
        try await nextLease()
    }

    private func nextLease() async throws -> StubSnapshotReviewLease {
        acquisitions += 1
        guard !leases.isEmpty else {
            throw EngineServiceError.unexpected("missing test lease")
        }
        let lease = leases.removeFirst()
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
    private let navigationExpires: Bool
    private let subtreeStart: Result<HomeScanStartDisposition, ExplorerSnapshotSubtreeScanError>
    private let suspendsSubtreeStart: Bool
    private var renewals = 0
    private var releases = 0
    private var renewalContinuation: CheckedContinuation<Void, Never>?
    private var subtreeStartContinuation: CheckedContinuation<Void, Never>?
    private var requestedSubtreeNodeIDs: [UInt64] = []

    init(
        scanID: String,
        renewFails: Bool = false,
        suspendsFirstRenewal: Bool = false,
        navigationExpires: Bool = false,
        subtreeStart: Result<HomeScanStartDisposition, ExplorerSnapshotSubtreeScanError> =
            .failure(.unavailable),
        suspendsSubtreeStart: Bool = false
    ) {
        self.scanID = scanID
        self.renewFails = renewFails
        self.suspendsFirstRenewal = suspendsFirstRenewal
        self.navigationExpires = navigationExpires
        self.subtreeStart = subtreeStart
        self.suspendsSubtreeStart = suspendsSubtreeStart
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

    func rootNode() throws -> ExplorerSnapshotNode {
        if navigationExpires {
            throw ExplorerSnapshotNodeError.reviewExpired
        }
        throw EngineServiceError.unexpected("unused root node stub")
    }

    func childNodes(
        parentID _: UInt64,
        sort _: ExplorerSnapshotNodeSort,
        offset _: UInt64,
        limit _: UInt16
    ) throws -> ExplorerSnapshotNodePage {
        throw EngineServiceError.unexpected("unused child node stub")
    }

    func treemap(
        parentID _: UInt64,
        maxCells _: UInt16
    ) throws -> ExplorerSnapshotTreemap {
        if navigationExpires {
            throw ExplorerSnapshotTreemapError.reviewExpired
        }
        throw EngineServiceError.unexpected("unused treemap stub")
    }

    func largeFiles(
        minimumLogicalBytes _: UInt64,
        modifiedBefore _: ExplorerSnapshotTimestamp?,
        maxResults _: UInt16
    ) throws -> ExplorerSnapshotLargeFilesPage {
        if navigationExpires {
            throw ExplorerSnapshotLargeFilesError.reviewExpired
        }
        throw EngineServiceError.unexpected("unused large-files stub")
    }

    func startSubtreeScan(nodeID: UInt64) async throws -> HomeScanStartDisposition {
        requestedSubtreeNodeIDs.append(nodeID)
        if suspendsSubtreeStart {
            await withCheckedContinuation { continuation in
                subtreeStartContinuation = continuation
            }
        }
        return try subtreeStart.get()
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

    func subtreeNodeIDs() -> [UInt64] {
        requestedSubtreeNodeIDs
    }

    func hasSuspendedSubtreeStart() -> Bool {
        subtreeStartContinuation != nil
    }

    func resumeSubtreeStart() {
        subtreeStartContinuation?.resume()
        subtreeStartContinuation = nil
    }
}

private actor ControllerScanTaskSpy: HomeScanTask {
    private var cancellations = 0

    func poll() async throws -> HomeScanTaskPoll {
        throw HomeScanServiceError.invalidResponse
    }

    func requestCancellation() async throws -> HomeScanCancelOutcome {
        cancellations += 1
        return .requested
    }

    func cancelCount() -> Int {
        cancellations
    }
}
