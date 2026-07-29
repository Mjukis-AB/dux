import XCTest
@testable import DUX

final class SnapshotReviewControllerTests: XCTestCase {
    func testPlanReviewIsOwnedByExactParentAndExplicitlyReleased() async throws {
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one"
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            planReview: plan
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        let handle = try await controller.prepareRustTargetPlanReview(
            scanID: "scan:one",
            candidateID: "candidate:one"
        )
        XCTAssertEqual(handle.info.target.display, "/Users/example/project/target")
        let infoCount = await plan.infoCount()
        XCTAssertEqual(infoCount, 1)

        await controller.releaseRustTargetPlanReview(handle)
        await controller.releaseRustTargetPlanReview(handle)
        let planReleaseCount = await plan.releaseCount()
        XCTAssertEqual(planReleaseCount, 1)
        await controller.shutdown()
    }

    func testCleanupStartConsumesExactChildBeforeSuspending() async throws {
        let cleanup = ControllerCleanupTaskSpy()
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            cleanupTask: cleanup
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            planReview: plan
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        let handle = try await controller.prepareRustTargetPlanReview(
            scanID: "scan:one",
            candidateID: "candidate:one"
        )

        let task = try await controller.startRustTargetCleanup(handle)
        do {
            _ = try await controller.startRustTargetCleanup(handle)
            XCTFail("Expected the exact child to be consume-once")
        } catch {
            XCTAssertEqual(
                error as? ExplorerRustTargetCleanupStartError,
                .reviewUnavailable
            )
        }
        await controller.releaseRustTargetPlanReview(handle)

        let cleanupStartCount = await plan.cleanupStartCount()
        let releaseCount = await plan.releaseCount()
        let activeLeaseCount = await controller.activeLeaseCount()
        XCTAssertTrue((task as AnyObject) === cleanup)
        XCTAssertEqual(cleanupStartCount, 1)
        XCTAssertEqual(releaseCount, 0)
        XCTAssertEqual(activeLeaseCount, 1)
        await controller.shutdown()
    }

    func testCleanupStartRejectsAlteredInfoWithoutConsumingExactChild() async throws {
        let cleanup = ControllerCleanupTaskSpy()
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            cleanupTask: cleanup
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            planReview: plan
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        let handle = try await controller.prepareRustTargetPlanReview(
            scanID: "scan:one",
            candidateID: "candidate:one"
        )
        let alteredInfo = ExplorerRustTargetPlanReviewInfo(
            planID: "plan:altered",
            sourceScanID: handle.info.sourceScanID,
            candidateID: handle.info.candidateID,
            ruleID: handle.info.ruleID,
            ruleRevision: handle.info.ruleRevision,
            category: handle.info.category,
            mode: handle.info.mode,
            safety: handle.info.safety,
            action: handle.info.action,
            estimatedBytes: handle.info.estimatedBytes,
            newestMtime: handle.info.newestMtime,
            minimumAgeSeconds: handle.info.minimumAgeSeconds,
            minimumAgeNanoseconds: handle.info.minimumAgeNanoseconds,
            itemCount: handle.info.itemCount,
            pathCount: handle.info.pathCount,
            warnings: handle.info.warnings,
            createdAt: handle.info.createdAt,
            effectiveExpiresAt: handle.info.effectiveExpiresAt,
            scheduleEligible: handle.info.scheduleEligible,
            target: ExplorerRustTargetPlanReviewPath(
                encoding: handle.info.target.encoding,
                encodedBytes: handle.info.target.encodedBytes,
                display: "/Users/example/forged/target"
            )
        )
        let alteredHandle = ExplorerRustTargetPlanReviewHandle(
            id: handle.id,
            info: alteredInfo
        )

        do {
            _ = try await controller.startRustTargetCleanup(alteredHandle)
            XCTFail("Expected altered display information to be rejected")
        } catch {
            XCTAssertEqual(
                error as? ExplorerRustTargetCleanupStartError,
                .reviewUnavailable
            )
        }
        let startsAfterRejection = await plan.cleanupStartCount()
        XCTAssertEqual(startsAfterRejection, 0)

        let task = try await controller.startRustTargetCleanup(handle)
        let finalStartCount = await plan.cleanupStartCount()
        XCTAssertTrue((task as AnyObject) === cleanup)
        XCTAssertEqual(finalStartCount, 1)
        await controller.shutdown()
    }

    func testFinalParentReleaseDrainsOwnedPlanReviewBeforeParent() async throws {
        let events = ControllerReleaseEvents()
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            events: events
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            planReview: plan,
            releaseEvents: events
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        _ = try await controller.prepareRustTargetPlanReview(
            scanID: "scan:one",
            candidateID: "candidate:one"
        )

        await controller.release(scanID: "scan:one")

        let releaseEvents = await events.values()
        let planReleaseCount = await plan.releaseCount()
        let leaseReleaseCount = await lease.releaseCount()
        XCTAssertEqual(releaseEvents, ["plan", "parent"])
        XCTAssertEqual(planReleaseCount, 1)
        XCTAssertEqual(leaseReleaseCount, 1)
    }

    func testMalformedPlanReviewInfoReleasesChildWithoutPublishingHandle() async throws {
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            recordCandidateID: "candidate:wrong"
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            planReview: plan
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        do {
            _ = try await controller.prepareRustTargetPlanReview(
                scanID: "scan:one",
                candidateID: "candidate:one"
            )
            XCTFail("expected invalid response")
        } catch {
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .invalidResponse
            )
        }
        let planReleaseCount = await plan.releaseCount()
        XCTAssertEqual(planReleaseCount, 1)
        await controller.shutdown()
    }

    func testExpiredChildReviewDoesNotInvalidateSharedRenewableParent() async throws {
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            infoError: .reviewExpired
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            planReview: plan
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        try await controller.acquire(scanID: "scan:one")

        do {
            _ = try await controller.prepareRustTargetPlanReview(
                scanID: "scan:one",
                candidateID: "candidate:one"
            )
            XCTFail("expected child review expiry")
        } catch {
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .reviewExpired
            )
        }
        let childReleaseCount = await plan.releaseCount()
        XCTAssertEqual(childReleaseCount, 1)

        await controller.release(scanID: "scan:one")
        let releaseCountWithOneOwner = await lease.releaseCount()
        XCTAssertEqual(releaseCountWithOneOwner, 0)

        await controller.release(scanID: "scan:one")
        let finalReleaseCount = await lease.releaseCount()
        XCTAssertEqual(finalReleaseCount, 1)
    }

    func testPlanRefreshDriftReleasesOnlyChildAndKeepsParentReview() async throws {
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            refreshInfoError: .changedDuringReview
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            planReview: plan
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        let handle = try await controller.prepareRustTargetPlanReview(
            scanID: "scan:one",
            candidateID: "candidate:one"
        )

        do {
            _ = try await controller.refreshRustTargetPlanReview(handle)
            XCTFail("expected plan drift")
        } catch {
            XCTAssertEqual(
                error as? ExplorerRustTargetPlanReviewError,
                .changedDuringReview
            )
        }

        let infoCount = await plan.infoCount()
        let planReleaseCount = await plan.releaseCount()
        let parentReleaseCount = await lease.releaseCount()
        XCTAssertEqual(infoCount, 2)
        XCTAssertEqual(planReleaseCount, 1)
        XCTAssertEqual(parentReleaseCount, 0)

        await controller.release(scanID: "scan:one")
        let finalParentReleaseCount = await lease.releaseCount()
        XCTAssertEqual(finalParentReleaseCount, 1)
    }

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
    private let planReview: StubRustTargetPlanReviewSession?
    private let releaseEvents: ControllerReleaseEvents?
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
        suspendsSubtreeStart: Bool = false,
        planReview: StubRustTargetPlanReviewSession? = nil,
        releaseEvents: ControllerReleaseEvents? = nil
    ) {
        self.scanID = scanID
        self.renewFails = renewFails
        self.suspendsFirstRenewal = suspendsFirstRenewal
        self.navigationExpires = navigationExpires
        self.subtreeStart = subtreeStart
        self.suspendsSubtreeStart = suspendsSubtreeStart
        self.planReview = planReview
        self.releaseEvents = releaseEvents
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

    func prepareRustTargetPlanReview(
        candidateID: String
    ) async throws -> any DuxRustTargetPlanReviewSession {
        guard let planReview, planReview.candidateID == candidateID else {
            throw ExplorerRustTargetPlanReviewError.candidateUnavailable
        }
        return planReview
    }

    func release() async {
        releases += 1
        await releaseEvents?.append("parent")
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

private actor StubRustTargetPlanReviewSession: DuxRustTargetPlanReviewSession {
    nonisolated let scanID: String
    nonisolated let candidateID: String

    private let events: ControllerReleaseEvents?
    private let infoError: ExplorerRustTargetPlanReviewError?
    private let refreshInfoError: ExplorerRustTargetPlanReviewError?
    private let cleanupTask: (any DuxRustTargetCleanupTask)?
    private let record: ExplorerRustTargetPlanReviewRecord
    private var infos = 0
    private var releases = 0
    private var cleanupStarts = 0

    init(
        scanID: String,
        candidateID: String,
        recordCandidateID: String? = nil,
        infoError: ExplorerRustTargetPlanReviewError? = nil,
        refreshInfoError: ExplorerRustTargetPlanReviewError? = nil,
        cleanupTask: (any DuxRustTargetCleanupTask)? = nil,
        events: ControllerReleaseEvents? = nil
    ) {
        self.scanID = scanID
        self.candidateID = candidateID
        self.infoError = infoError
        self.refreshInfoError = refreshInfoError
        self.cleanupTask = cleanupTask
        self.events = events
        let now = Date()
        record = controllerPlanReviewRecord(
            scanID: scanID,
            candidateID: recordCandidateID ?? candidateID,
            createdAt: now,
            expiresAt: now.addingTimeInterval(60)
        )
    }

    func info() throws -> ExplorerRustTargetPlanReviewRecord {
        infos += 1
        if let infoError {
            throw infoError
        }
        if infos > 1, let refreshInfoError {
            throw refreshInfoError
        }
        return record
    }

    func release() async {
        releases += 1
        await events?.append("plan")
    }

    func startCleanup() throws -> any DuxRustTargetCleanupTask {
        cleanupStarts += 1
        guard let cleanupTask else {
            throw ExplorerRustTargetCleanupStartError.unavailable
        }
        return cleanupTask
    }

    func infoCount() -> Int {
        infos
    }

    func releaseCount() -> Int {
        releases
    }

    func cleanupStartCount() -> Int {
        cleanupStarts
    }
}

private final class ControllerCleanupTaskSpy:
    DuxRustTargetCleanupTask,
    @unchecked Sendable
{
    func poll() async throws -> ExplorerRustTargetCleanupPoll {
        ExplorerRustTargetCleanupPoll(
            phase: .cancelled,
            cancellationRequested: true,
            revision: 1,
            failure: nil,
            result: nil
        )
    }

    func requestCancellation() async throws -> ExplorerRustTargetCleanupCancelOutcome {
        .alreadyTerminal
    }
}

private actor ControllerReleaseEvents {
    private var events: [String] = []

    func append(_ value: String) {
        events.append(value)
    }

    func values() -> [String] {
        events
    }
}

private func controllerPlanReviewRecord(
    scanID: String,
    candidateID: String,
    createdAt: Date,
    expiresAt: Date
) -> ExplorerRustTargetPlanReviewRecord {
    ExplorerRustTargetPlanReviewRecord(
        recordVersion: 1,
        planID: "plan:controller",
        sourceScanID: scanID,
        candidateID: candidateID,
        ruleID: "developer.rust.target",
        ruleRevision: 3,
        category: .developerArtifact,
        mode: .permanentSafe,
        safety: .safeRegenerable,
        action: .removeKnownRegenerableContents,
        estimatedBytes: 42,
        newestMtime: ExplorerRustTargetPlanReviewAdapter.timestamp(
            for: createdAt.addingTimeInterval(-604_800)
        ),
        minimumAgeSeconds: 604_800,
        minimumAgeNanoseconds: 0,
        itemCount: 1,
        pathCount: 1,
        warnings: [.estimatedBytesUnverified, .permanentRemovalCannotBeUndone],
        createdAt: ExplorerRustTargetPlanReviewAdapter.timestamp(for: createdAt),
        effectiveExpiresAt: ExplorerRustTargetPlanReviewAdapter.timestamp(for: expiresAt),
        scheduleEligible: false,
        target: ExplorerRustTargetPlanReviewPath(
            encoding: .unixBytes,
            encodedBytes: Data("/Users/example/project/target".utf8),
            display: "/Users/example/project/target"
        )
    )
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
