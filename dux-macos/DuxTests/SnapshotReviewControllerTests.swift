import XCTest
@testable import DUX

final class SnapshotReviewControllerTests: XCTestCase {
    func testICloudObservationSourceUsesExactRetainedLeaseAndRequest() async throws {
        let expected = controllerICloudObservationSource(
            scanID: "scan:one",
            scopeNodeID: 42,
            maxResults: 32
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            iCloudObservationSource: .success(expected)
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        let actual = try await controller.icloudObservationSource(
            scanID: "scan:one",
            scopeNodeID: 42,
            maxResults: 32
        )

        XCTAssertEqual(actual, expected)
        let requests = await lease.requestedICloudObservationSources()
        XCTAssertEqual(requests.map(\.scopeNodeID), [42])
        XCTAssertEqual(requests.map(\.maxResults), [32])
        await controller.shutdown()
    }

    func testICloudObservationSourceFailsWithoutExactRetainedLease() async {
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: []),
            clock: SuspendedSnapshotReviewClock()
        )

        do {
            _ = try await controller.icloudObservationSource(
                scanID: "scan:missing",
                scopeNodeID: 42,
                maxResults: 32
            )
            XCTFail("Expected missing exact review to fail closed")
        } catch {
            XCTAssertEqual(
                error as? ExplorerICloudObservationSourceError,
                .reviewNotAcquired
            )
        }
        await controller.shutdown()
    }

    func testICloudObservationSourceFencesLateResultAfterLeaseRelease() async throws {
        let source = controllerICloudObservationSource(
            scanID: "scan:one",
            scopeNodeID: 42,
            maxResults: 32
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            iCloudObservationSource: .success(source),
            suspendsICloudObservationSource: true
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        let loading = Task {
            try await controller.icloudObservationSource(
                scanID: "scan:one",
                scopeNodeID: 42,
                maxResults: 32
            )
        }
        for _ in 0 ..< 2_000 where !(await lease.hasSuspendedICloudObservationSource()) {
            await Task.yield()
        }
        let didSuspend = await lease.hasSuspendedICloudObservationSource()
        XCTAssertTrue(didSuspend)

        await controller.release(scanID: "scan:one")
        await lease.resumeICloudObservationSource()

        do {
            _ = try await loading.value
            XCTFail("Expected the released generation to fence its late result")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
        await controller.shutdown()
    }

    func testICloudProbeUsesExactRetainedLeaseAndNode() async throws {
        let expected = ExplorerICloudLocalCopyAssessment(
            localAllocatedBytes: 4096,
            observedAtUnixMilliseconds: 1_000,
            ubiquitous: .yes,
            uploaded: .yes,
            uploading: .no,
            uploadError: .absent,
            unresolvedConflicts: .no,
            localCopyState: .current,
            downloadRequested: .no,
            downloading: .no,
            downloadError: .absent,
            excludedFromSync: .no,
            accountIdentity: .stable,
            containerIdentity: .unsupported,
            itemGeneration: .stable,
            fileVersion: .stable,
            shared: .no,
            syncPaused: .no,
            isEligibleObservation: true,
            blockers: [],
            isIdentityReady: false,
            identityBlockers: [.containerIdentityUnsupported]
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            iCloudProbe: .success(expected)
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        let actual = try await controller.probeICloudLocalCopy(
            scanID: "scan:one",
            nodeID: 42
        )

        XCTAssertEqual(actual, expected)
        let requestedNodeIDs = await lease.requestedICloudNodeIDs()
        XCTAssertEqual(requestedNodeIDs, [42])
        await controller.shutdown()
    }

    func testICloudProbeFailsWithoutExactRetainedLease() async {
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: []),
            clock: SuspendedSnapshotReviewClock()
        )

        do {
            _ = try await controller.probeICloudLocalCopy(
                scanID: "scan:missing",
                nodeID: 42
            )
            XCTFail("Expected missing exact review to fail closed")
        } catch {
            XCTAssertEqual(
                error as? ExplorerICloudLocalCopyProbeError,
                .unavailable
            )
        }
        await controller.shutdown()
    }

    func testSnapshotDiffPreparationIsLazyReusedAndExplicitlyReleasedOnce() async throws {
        let diff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one")
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            diffReview: diff
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        let preparationsBeforeEntry = await lease.diffPreparationCount()
        let activeBeforeEntry = await controller.activeDiffReviewCount()
        XCTAssertEqual(preparationsBeforeEntry, 0)
        XCTAssertEqual(activeBeforeEntry, 0)

        let first = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")
        let second = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")

        let preparationsAfterEntry = await lease.diffPreparationCount()
        let activeAfterEntry = await controller.activeDiffReviewCount()
        XCTAssertEqual(first, second)
        XCTAssertEqual(preparationsAfterEntry, 1)
        XCTAssertEqual(activeAfterEntry, 1)

        await controller.releaseSnapshotDiffReview(first)
        await controller.releaseSnapshotDiffReview(first)

        let releases = await diff.releaseCount()
        let finalActive = await controller.activeDiffReviewCount()
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(finalActive, 0)
        await controller.shutdown()
    }

    func testFinalParentReleaseDrainsOwnedSnapshotDiffBeforeParent() async throws {
        let events = ControllerReleaseEvents()
        let diff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one"),
            releaseEvents: events
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            diffReview: diff,
            releaseEvents: events
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        try await controller.acquire(scanID: "scan:one")
        _ = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")

        await controller.release(scanID: "scan:one")
        let eventsWithOneOwner = await events.values()
        XCTAssertEqual(eventsWithOneOwner, [])

        await controller.release(scanID: "scan:one")

        let releaseEvents = await events.values()
        let diffReleases = await diff.releaseCount()
        let parentReleases = await lease.releaseCount()
        XCTAssertEqual(releaseEvents, ["diff", "parent"])
        XCTAssertEqual(diffReleases, 1)
        XCTAssertEqual(parentReleases, 1)
    }

    func testExpiredParentNavigationDrainsSnapshotDiffBeforeParent() async throws {
        let events = ControllerReleaseEvents()
        let diff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one"),
            releaseEvents: events
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            navigationExpires: true,
            diffReview: diff,
            releaseEvents: events
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        _ = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")

        do {
            _ = try await controller.rootNode(scanID: "scan:one")
            XCTFail("Expected parent navigation expiry")
        } catch {
            XCTAssertEqual(error as? ExplorerSnapshotNodeError, .reviewExpired)
        }

        let releaseEvents = await events.values()
        let activeParents = await controller.activeLeaseCount()
        let activeDiffs = await controller.activeDiffReviewCount()
        XCTAssertEqual(releaseEvents, ["diff", "parent"])
        XCTAssertEqual(activeParents, 0)
        XCTAssertEqual(activeDiffs, 0)
    }

    func testParentRenewFailureDrainsSnapshotDiffBeforeParent() async throws {
        let events = ControllerReleaseEvents()
        let diff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one"),
            releaseEvents: events
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            renewFails: true,
            diffReview: diff,
            releaseEvents: events
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        _ = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")

        await controller.renewNow()

        let releaseEvents = await events.values()
        let diffRenewals = await diff.renewCount()
        let activeParents = await controller.activeLeaseCount()
        let activeDiffs = await controller.activeDiffReviewCount()
        XCTAssertEqual(releaseEvents, ["diff", "parent"])
        XCTAssertEqual(diffRenewals, 0)
        XCTAssertEqual(activeParents, 0)
        XCTAssertEqual(activeDiffs, 0)
    }

    func testSnapshotDiffRenewsWithParentAndExplicitReleaseKeepsParent() async throws {
        let diff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one")
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            diffReview: diff
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        let handle = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")

        await controller.renewNow()

        let parentRenewals = await lease.renewCount()
        let diffRenewals = await diff.renewCount()
        XCTAssertEqual(parentRenewals, 1)
        XCTAssertEqual(diffRenewals, 1)

        await controller.releaseSnapshotDiffReview(handle)

        let diffReleases = await diff.releaseCount()
        let parentReleases = await lease.releaseCount()
        let activeParents = await controller.activeLeaseCount()
        let activeDiffs = await controller.activeDiffReviewCount()
        XCTAssertEqual(diffReleases, 1)
        XCTAssertEqual(parentReleases, 0)
        XCTAssertEqual(activeParents, 1)
        XCTAssertEqual(activeDiffs, 0)
        await controller.shutdown()
    }

    func testSnapshotDiffRenewFailureReleasesOnlyChild() async throws {
        let diff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one"),
            renewError: .expired
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            diffReview: diff
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        _ = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")

        await controller.renewNow()

        let diffRenewals = await diff.renewCount()
        let diffReleases = await diff.releaseCount()
        let parentReleases = await lease.releaseCount()
        let activeParents = await controller.activeLeaseCount()
        let activeDiffs = await controller.activeDiffReviewCount()
        XCTAssertEqual(diffRenewals, 1)
        XCTAssertEqual(diffReleases, 1)
        XCTAssertEqual(parentReleases, 0)
        XCTAssertEqual(activeParents, 1)
        XCTAssertEqual(activeDiffs, 0)
        await controller.shutdown()
    }

    func testLateSnapshotDiffRootResultIsFencedAfterParentGenerationReplacement() async throws {
        let oldDiff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one"),
            rootNode: .success(controllerSnapshotDiffRoot()),
            suspendsRootNode: true
        )
        let oldLease = StubSnapshotReviewLease(
            scanID: "scan:one",
            diffReview: oldDiff
        )
        let replacementLease = StubSnapshotReviewLease(scanID: "scan:one")
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [oldLease, replacementLease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        let handle = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")

        let loading = Task {
            try await controller.snapshotDiffRootNode(handle)
        }
        try await eventually { await oldDiff.hasSuspendedRootNode() }

        await controller.release(scanID: "scan:one")
        try await controller.acquire(scanID: "scan:one")
        await oldDiff.resumeRootNode()

        do {
            _ = try await loading.value
            XCTFail("Expected old diff generation to fence its late root result")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }

        let oldDiffReleases = await oldDiff.releaseCount()
        let replacementReleases = await replacementLease.releaseCount()
        let activeParents = await controller.activeLeaseCount()
        XCTAssertEqual(oldDiffReleases, 1)
        XCTAssertEqual(replacementReleases, 0)
        XCTAssertEqual(activeParents, 1)
        await controller.shutdown()
    }

    func testLateSnapshotDiffPreparationIsRejectedAfterParentGenerationReplacement() async throws {
        let oldDiff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one")
        )
        let oldLease = StubSnapshotReviewLease(
            scanID: "scan:one",
            diffReview: oldDiff,
            suspendsDiffPreparation: true
        )
        let replacementLease = StubSnapshotReviewLease(scanID: "scan:one")
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [oldLease, replacementLease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")

        let preparation = Task {
            try await controller.prepareSnapshotDiffReview(scanID: "scan:one")
        }
        try await eventually { await oldLease.hasSuspendedDiffPreparation() }

        await controller.release(scanID: "scan:one")
        try await controller.acquire(scanID: "scan:one")
        await oldLease.resumeDiffPreparation()

        do {
            _ = try await preparation.value
            XCTFail("Expected old parent generation to reject its late diff child")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }

        let childReleases = await oldDiff.releaseCount()
        let activeDiffs = await controller.activeDiffReviewCount()
        let activeParents = await controller.activeLeaseCount()
        XCTAssertEqual(childReleases, 1)
        XCTAssertEqual(activeDiffs, 0)
        XCTAssertEqual(activeParents, 1)
        await controller.shutdown()
    }

    func testShutdownReleasesSnapshotDiffBeforeParentExactlyOnce() async throws {
        let events = ControllerReleaseEvents()
        let diff = StubSnapshotDiffReviewSession(
            info: controllerSnapshotDiffInfo(currentScanID: "scan:one"),
            releaseEvents: events
        )
        let lease = StubSnapshotReviewLease(
            scanID: "scan:one",
            diffReview: diff,
            releaseEvents: events
        )
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: SuspendedSnapshotReviewClock()
        )
        try await controller.acquire(scanID: "scan:one")
        _ = try await controller.prepareSnapshotDiffReview(scanID: "scan:one")

        await controller.shutdown()
        await controller.shutdown()

        let releaseEvents = await events.values()
        let diffReleases = await diff.releaseCount()
        let parentReleases = await lease.releaseCount()
        let activeParents = await controller.activeLeaseCount()
        let activeDiffs = await controller.activeDiffReviewCount()
        XCTAssertEqual(releaseEvents, ["diff", "parent"])
        XCTAssertEqual(diffReleases, 1)
        XCTAssertEqual(parentReleases, 1)
        XCTAssertEqual(activeParents, 0)
        XCTAssertEqual(activeDiffs, 0)
    }

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

    func testDryRunStartConsumesExactChildAndCannotRaceCleanup() async throws {
        let dryRun = ControllerDryRunTaskSpy()
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            dryRunTask: dryRun
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

        let task = try await controller.startRustTargetDryRun(handle)
        do {
            _ = try await controller.startRustTargetCleanup(handle)
            XCTFail("Expected dry run to consume the exact child")
        } catch {
            XCTAssertEqual(
                error as? ExplorerRustTargetCleanupStartError,
                .reviewUnavailable
            )
        }
        do {
            _ = try await controller.startRustTargetDryRun(handle)
            XCTFail("Expected the exact child to be consume-once")
        } catch {
            XCTAssertEqual(
                error as? ExplorerRustTargetDryRunStartError,
                .reviewUnavailable
            )
        }

        let dryRunStartCount = await plan.dryRunStartCount()
        let cleanupStartCount = await plan.cleanupStartCount()
        XCTAssertTrue((task as AnyObject) === dryRun)
        XCTAssertEqual(dryRunStartCount, 1)
        XCTAssertEqual(cleanupStartCount, 0)
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

    func testConcurrentShutdownWaitsForPendingAcquisitionAndReleasesLateLease() async throws {
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

        let firstShutdown = Task { await controller.shutdown() }
        firstShutdown.cancel()
        let secondShutdown = Task { await controller.shutdown() }
        await Task.yield()
        var releaseCount = await lease.releaseCount()
        XCTAssertEqual(releaseCount, 0)

        await service.resumeAcquisitions()
        do {
            try await acquisition.value
            XCTFail("Expected terminal acquisition cancellation")
        } catch is CancellationError {
            // The late child was released before either shutdown completed.
        }
        await firstShutdown.value
        await secondShutdown.value

        releaseCount = await lease.releaseCount()
        let activeLeaseCount = await controller.activeLeaseCount()
        XCTAssertEqual(releaseCount, 1)
        XCTAssertEqual(activeLeaseCount, 0)
        do {
            try await controller.acquire(scanID: "scan:one")
            XCTFail("Expected permanent shutdown admission fence")
        } catch let error as EngineServiceError {
            XCTAssertEqual(error, .closed)
        }
    }

    func testShutdownJoinsSuspendedRenewalBeforeExactRelease() async throws {
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
        let renewal = Task { await controller.renewNow() }
        try await eventually { await lease.hasSuspendedRenewal() }

        let firstShutdown = Task { await controller.shutdown() }
        let secondShutdown = Task { await controller.shutdown() }
        await Task.yield()
        var releaseCount = await lease.releaseCount()
        XCTAssertEqual(releaseCount, 0)

        await lease.resumeRenewal()
        await renewal.value
        await firstShutdown.value
        await secondShutdown.value

        releaseCount = await lease.releaseCount()
        let activeLeaseCount = await controller.activeLeaseCount()
        XCTAssertEqual(releaseCount, 1)
        XCTAssertEqual(activeLeaseCount, 0)
    }

    func testShutdownRetainsLateCleanupTaskUntilCancellationIsTerminal() async throws {
        let cleanup = SuspendedControllerCleanupTask()
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            cleanupTask: cleanup,
            suspendsCleanupStart: true
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
        let start = Task { try await controller.startRustTargetCleanup(handle) }
        try await eventually { await plan.hasSuspendedCleanupStart() }

        let shutdown = Task { await controller.shutdown() }
        await plan.resumeCleanupStart()
        try await eventually { await cleanup.hasSuspendedPoll() }
        var releases = await lease.releaseCount()
        XCTAssertEqual(releases, 0)

        await cleanup.resumeTerminalPoll()
        do {
            _ = try await start.value
            XCTFail("Expected the late cleanup task to remain terminal-owned")
        } catch is CancellationError {}
        await shutdown.value

        releases = await lease.releaseCount()
        let cancellations = await cleanup.cancellationCount()
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(cancellations, 1)
    }

    func testShutdownRetainsLateDryRunTaskUntilCancellationIsTerminal() async throws {
        let dryRun = SuspendedControllerDryRunTask()
        let plan = StubRustTargetPlanReviewSession(
            scanID: "scan:one",
            candidateID: "candidate:one",
            dryRunTask: dryRun,
            suspendsDryRunStart: true
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
        let start = Task { try await controller.startRustTargetDryRun(handle) }
        try await eventually { await plan.hasSuspendedDryRunStart() }

        let shutdown = Task { await controller.shutdown() }
        try await eventually {
            do {
                _ = try await controller.rootNode(scanID: "scan:one")
                return false
            } catch EngineServiceError.closed {
                return true
            } catch {
                return false
            }
        }
        await plan.resumeDryRunStart()
        try await eventually { await dryRun.hasSuspendedPoll() }
        var releases = await lease.releaseCount()
        XCTAssertEqual(releases, 0)

        await dryRun.resumeTerminalPoll()
        do {
            _ = try await start.value
            XCTFail("Expected the late dry-run task to remain terminal-owned")
        } catch is CancellationError {}
        await shutdown.value

        releases = await lease.releaseCount()
        let cancellations = await dryRun.cancellationCount()
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(cancellations, 1)
    }

    func testShutdownRetainsLateSubtreeTaskUntilCancellationIsTerminal() async throws {
        let scanTask = ControllerScanTaskSpy(suspendsTerminalPoll: true)
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

        let shutdown = Task { await controller.shutdown() }
        await lease.resumeSubtreeStart()
        try await eventually { await scanTask.hasSuspendedPoll() }
        var releases = await lease.releaseCount()
        XCTAssertEqual(releases, 0)

        await scanTask.resumeTerminalPoll()
        do {
            _ = try await start.value
            XCTFail("Expected the late subtree task to remain terminal-owned")
        } catch is CancellationError {}
        await shutdown.value

        releases = await lease.releaseCount()
        let cancellations = await scanTask.cancelCount()
        XCTAssertEqual(releases, 1)
        XCTAssertEqual(cancellations, 1)
    }

    func testShutdownJoinsCancellationInsensitiveRenewalOwner() async throws {
        let clock = ControlledSnapshotReviewClock()
        let lease = StubSnapshotReviewLease(scanID: "scan:one")
        let controller = DuxSnapshotReviewController(
            service: StubSnapshotReviewService(leases: [lease]),
            clock: clock
        )
        try await controller.acquire(scanID: "scan:one")
        try await eventually { await clock.hasSuspendedSleep() }

        let shutdown = Task { await controller.shutdown() }
        await Task.yield()
        var releases = await lease.releaseCount()
        XCTAssertEqual(releases, 0)

        await clock.resumeSleep()
        await shutdown.value

        releases = await lease.releaseCount()
        XCTAssertEqual(releases, 1)
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

private actor ControlledSnapshotReviewClock: DuxSnapshotReviewRenewalClock {
    private var continuation: CheckedContinuation<Void, Never>?

    func sleepForRenewalInterval() async throws {
        await withCheckedContinuation { continuation in
            self.continuation = continuation
        }
    }

    func hasSuspendedSleep() -> Bool {
        continuation != nil
    }

    func resumeSleep() {
        continuation?.resume()
        continuation = nil
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
    struct ICloudObservationSourceRequest: Sendable {
        let scopeNodeID: UInt64
        let maxResults: UInt16
    }

    nonisolated let scanID: String

    private let renewFails: Bool
    private let suspendsFirstRenewal: Bool
    private let navigationExpires: Bool
    private let subtreeStart: Result<HomeScanStartDisposition, ExplorerSnapshotSubtreeScanError>
    private let suspendsSubtreeStart: Bool
    private let planReview: StubRustTargetPlanReviewSession?
    private let diffReview: StubSnapshotDiffReviewSession?
    private let suspendsDiffPreparation: Bool
    private let iCloudProbe:
        Result<ExplorerICloudLocalCopyAssessment, ExplorerICloudLocalCopyProbeError>?
    private let iCloudObservationSource:
        Result<ExplorerICloudObservationSource, ExplorerICloudObservationSourceError>?
    private let suspendsICloudObservationSource: Bool
    private let releaseEvents: ControllerReleaseEvents?
    private var renewals = 0
    private var releases = 0
    private var renewalContinuation: CheckedContinuation<Void, Never>?
    private var subtreeStartContinuation: CheckedContinuation<Void, Never>?
    private var requestedSubtreeNodeIDs: [UInt64] = []
    private var diffPreparations = 0
    private var diffPreparationContinuation: CheckedContinuation<Void, Never>?
    private var iCloudNodeIDs: [UInt64] = []
    private var iCloudObservationSourceRequests: [ICloudObservationSourceRequest] = []
    private var iCloudObservationSourceContinuation: CheckedContinuation<Void, Never>?

    init(
        scanID: String,
        renewFails: Bool = false,
        suspendsFirstRenewal: Bool = false,
        navigationExpires: Bool = false,
        subtreeStart: Result<HomeScanStartDisposition, ExplorerSnapshotSubtreeScanError> =
            .failure(.unavailable),
        suspendsSubtreeStart: Bool = false,
        planReview: StubRustTargetPlanReviewSession? = nil,
        diffReview: StubSnapshotDiffReviewSession? = nil,
        suspendsDiffPreparation: Bool = false,
        iCloudProbe:
            Result<ExplorerICloudLocalCopyAssessment, ExplorerICloudLocalCopyProbeError>? = nil,
        iCloudObservationSource:
            Result<ExplorerICloudObservationSource, ExplorerICloudObservationSourceError>? = nil,
        suspendsICloudObservationSource: Bool = false,
        releaseEvents: ControllerReleaseEvents? = nil
    ) {
        self.scanID = scanID
        self.renewFails = renewFails
        self.suspendsFirstRenewal = suspendsFirstRenewal
        self.navigationExpires = navigationExpires
        self.subtreeStart = subtreeStart
        self.suspendsSubtreeStart = suspendsSubtreeStart
        self.planReview = planReview
        self.diffReview = diffReview
        self.suspendsDiffPreparation = suspendsDiffPreparation
        self.iCloudProbe = iCloudProbe
        self.iCloudObservationSource = iCloudObservationSource
        self.suspendsICloudObservationSource = suspendsICloudObservationSource
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

    func prepareSnapshotDiffReview() async throws -> any DuxSnapshotDiffReviewSession {
        diffPreparations += 1
        if suspendsDiffPreparation {
            await withCheckedContinuation { continuation in
                diffPreparationContinuation = continuation
            }
        }
        guard let diffReview else {
            throw ExplorerSnapshotDiffFailure.notAvailable
        }
        return diffReview
    }

    func probeICloudLocalCopy(
        nodeID: UInt64
    ) throws -> ExplorerICloudLocalCopyAssessment {
        iCloudNodeIDs.append(nodeID)
        guard let iCloudProbe else {
            throw ExplorerICloudLocalCopyProbeError.unavailable
        }
        return try iCloudProbe.get()
    }

    func icloudObservationSource(
        scopeNodeID: UInt64,
        maxResults: UInt16
    ) async throws -> ExplorerICloudObservationSource {
        iCloudObservationSourceRequests.append(
            ICloudObservationSourceRequest(
                scopeNodeID: scopeNodeID,
                maxResults: maxResults
            )
        )
        if suspendsICloudObservationSource {
            await withCheckedContinuation { continuation in
                iCloudObservationSourceContinuation = continuation
            }
        }
        guard let iCloudObservationSource else {
            throw ExplorerICloudObservationSourceError.unavailable
        }
        return try iCloudObservationSource.get()
    }

    func release() async {
        releases += 1
        await releaseEvents?.append("parent")
    }

    func renewCount() -> Int {
        renewals
    }

    func requestedICloudNodeIDs() -> [UInt64] {
        iCloudNodeIDs
    }

    func requestedICloudObservationSources() -> [ICloudObservationSourceRequest] {
        iCloudObservationSourceRequests
    }

    func hasSuspendedICloudObservationSource() -> Bool {
        iCloudObservationSourceContinuation != nil
    }

    func resumeICloudObservationSource() {
        iCloudObservationSourceContinuation?.resume()
        iCloudObservationSourceContinuation = nil
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

    func diffPreparationCount() -> Int {
        diffPreparations
    }

    func hasSuspendedDiffPreparation() -> Bool {
        diffPreparationContinuation != nil
    }

    func resumeDiffPreparation() {
        diffPreparationContinuation?.resume()
        diffPreparationContinuation = nil
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

private func controllerICloudObservationSource(
    scanID: String,
    scopeNodeID: UInt64,
    maxResults: UInt16
) -> ExplorerICloudObservationSource {
    ExplorerICloudObservationSource(
        scanID: scanID,
        scopeNodeID: scopeNodeID,
        requestedMaxResults: maxResults,
        visitedNodeCount: 1,
        totalRankedFiles: 0,
        hasMore: false,
        targets: []
    )
}

private actor StubSnapshotDiffReviewSession: DuxSnapshotDiffReviewSession {
    nonisolated let info: ExplorerSnapshotDiffInfo

    private let renewedInfo: ExplorerSnapshotDiffInfo?
    private let renewError: ExplorerSnapshotDiffFailure?
    private let rootResult: Result<ExplorerSnapshotDiffNode, ExplorerSnapshotDiffFailure>
    private let suspendsRootNode: Bool
    private let releaseEvents: ControllerReleaseEvents?
    private var renewals = 0
    private var releases = 0
    private var rootContinuation: CheckedContinuation<Void, Never>?

    init(
        info: ExplorerSnapshotDiffInfo,
        renewedInfo: ExplorerSnapshotDiffInfo? = nil,
        renewError: ExplorerSnapshotDiffFailure? = nil,
        rootNode: Result<ExplorerSnapshotDiffNode, ExplorerSnapshotDiffFailure> =
            .failure(.invalidRequest),
        suspendsRootNode: Bool = false,
        releaseEvents: ControllerReleaseEvents? = nil
    ) {
        self.info = info
        self.renewedInfo = renewedInfo
        self.renewError = renewError
        rootResult = rootNode
        self.suspendsRootNode = suspendsRootNode
        self.releaseEvents = releaseEvents
    }

    func renew() throws -> ExplorerSnapshotDiffInfo {
        renewals += 1
        if let renewError {
            throw renewError
        }
        return renewedInfo ?? info
    }

    func rootNode() async throws -> ExplorerSnapshotDiffNode {
        if suspendsRootNode {
            await withCheckedContinuation { continuation in
                rootContinuation = continuation
            }
        }
        return try rootResult.get()
    }

    func childNodes(
        parentID _: UInt64,
        sort _: ExplorerSnapshotDiffSort,
        offset _: UInt64,
        limit _: UInt16
    ) throws -> ExplorerSnapshotDiffNodePage {
        throw ExplorerSnapshotDiffFailure.invalidRequest
    }

    func treemap(
        parentID _: UInt64,
        maxCells _: UInt16
    ) throws -> ExplorerSnapshotDiffTreemap {
        throw ExplorerSnapshotDiffFailure.invalidRequest
    }

    func release() async {
        releases += 1
        await releaseEvents?.append("diff")
    }

    func renewCount() -> Int {
        renewals
    }

    func releaseCount() -> Int {
        releases
    }

    func hasSuspendedRootNode() -> Bool {
        rootContinuation != nil
    }

    func resumeRootNode() {
        rootContinuation?.resume()
        rootContinuation = nil
    }
}

private actor StubRustTargetPlanReviewSession: DuxRustTargetPlanReviewSession {
    nonisolated let scanID: String
    nonisolated let candidateID: String

    private let events: ControllerReleaseEvents?
    private let infoError: ExplorerRustTargetPlanReviewError?
    private let refreshInfoError: ExplorerRustTargetPlanReviewError?
    private let cleanupTask: (any DuxRustTargetCleanupTask)?
    private let dryRunTask: (any DuxRustTargetDryRunTask)?
    private let suspendsCleanupStart: Bool
    private let suspendsDryRunStart: Bool
    private let record: ExplorerRustTargetPlanReviewRecord
    private var infos = 0
    private var releases = 0
    private var cleanupStarts = 0
    private var dryRunStarts = 0
    private var cleanupStartContinuation: CheckedContinuation<Void, Never>?
    private var dryRunStartContinuation: CheckedContinuation<Void, Never>?

    init(
        scanID: String,
        candidateID: String,
        recordCandidateID: String? = nil,
        infoError: ExplorerRustTargetPlanReviewError? = nil,
        refreshInfoError: ExplorerRustTargetPlanReviewError? = nil,
        cleanupTask: (any DuxRustTargetCleanupTask)? = nil,
        dryRunTask: (any DuxRustTargetDryRunTask)? = nil,
        suspendsCleanupStart: Bool = false,
        suspendsDryRunStart: Bool = false,
        events: ControllerReleaseEvents? = nil
    ) {
        self.scanID = scanID
        self.candidateID = candidateID
        self.infoError = infoError
        self.refreshInfoError = refreshInfoError
        self.cleanupTask = cleanupTask
        self.dryRunTask = dryRunTask
        self.suspendsCleanupStart = suspendsCleanupStart
        self.suspendsDryRunStart = suspendsDryRunStart
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

    func startCleanup() async throws -> any DuxRustTargetCleanupTask {
        cleanupStarts += 1
        if suspendsCleanupStart {
            await withCheckedContinuation { continuation in
                cleanupStartContinuation = continuation
            }
        }
        guard let cleanupTask else {
            throw ExplorerRustTargetCleanupStartError.unavailable
        }
        return cleanupTask
    }

    func startDryRun() async throws -> any DuxRustTargetDryRunTask {
        dryRunStarts += 1
        if suspendsDryRunStart {
            await withCheckedContinuation { continuation in
                dryRunStartContinuation = continuation
            }
        }
        guard let dryRunTask else {
            throw ExplorerRustTargetDryRunStartError.unavailable
        }
        return dryRunTask
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

    func dryRunStartCount() -> Int {
        dryRunStarts
    }

    func hasSuspendedCleanupStart() -> Bool {
        cleanupStartContinuation != nil
    }

    func resumeCleanupStart() {
        cleanupStartContinuation?.resume()
        cleanupStartContinuation = nil
    }

    func hasSuspendedDryRunStart() -> Bool {
        dryRunStartContinuation != nil
    }

    func resumeDryRunStart() {
        dryRunStartContinuation?.resume()
        dryRunStartContinuation = nil
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

private actor SuspendedControllerCleanupTask: DuxRustTargetCleanupTask {
    private var cancellations = 0
    private var pollContinuation: CheckedContinuation<Void, Never>?

    func poll() async throws -> ExplorerRustTargetCleanupPoll {
        await withCheckedContinuation { continuation in
            pollContinuation = continuation
        }
        return ExplorerRustTargetCleanupPoll(
            phase: .cancelled,
            cancellationRequested: true,
            revision: 1,
            failure: nil,
            result: nil
        )
    }

    func requestCancellation() async throws -> ExplorerRustTargetCleanupCancelOutcome {
        cancellations += 1
        return .requested
    }

    func hasSuspendedPoll() -> Bool {
        pollContinuation != nil
    }

    func resumeTerminalPoll() {
        pollContinuation?.resume()
        pollContinuation = nil
    }

    func cancellationCount() -> Int {
        cancellations
    }
}

private actor SuspendedControllerDryRunTask: DuxRustTargetDryRunTask {
    private var cancellations = 0
    private var pollContinuation: CheckedContinuation<Void, Never>?

    func poll() async throws -> ExplorerRustTargetDryRunPoll {
        await withCheckedContinuation { continuation in
            pollContinuation = continuation
        }
        return ExplorerRustTargetDryRunPoll(
            phase: .cancelled,
            cancellationRequested: true,
            revision: 1,
            failure: nil,
            result: nil
        )
    }

    func requestCancellation() async throws -> ExplorerRustTargetDryRunCancelOutcome {
        cancellations += 1
        return .requested
    }

    func hasSuspendedPoll() -> Bool {
        pollContinuation != nil
    }

    func resumeTerminalPoll() {
        pollContinuation?.resume()
        pollContinuation = nil
    }

    func cancellationCount() -> Int {
        cancellations
    }
}

private final class ControllerDryRunTaskSpy:
    DuxRustTargetDryRunTask,
    @unchecked Sendable
{
    func poll() async throws -> ExplorerRustTargetDryRunPoll {
        ExplorerRustTargetDryRunPoll(
            phase: .cancelled,
            cancellationRequested: true,
            revision: 1,
            failure: nil,
            result: nil
        )
    }

    func requestCancellation() async throws -> ExplorerRustTargetDryRunCancelOutcome {
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

private func controllerSnapshotDiffInfo(
    currentScanID: String,
    baselineScanID: String = "scan:baseline"
) -> ExplorerSnapshotDiffInfo {
    ExplorerSnapshotDiffInfo(
        currentScanID: currentScanID,
        baselineScanID: baselineScanID,
        currentStartedAt: Date(timeIntervalSince1970: 2_000),
        currentCompletedAt: Date(timeIntervalSince1970: 2_100),
        baselineStartedAt: Date(timeIntervalSince1970: 1_000),
        baselineCompletedAt: Date(timeIntervalSince1970: 1_100),
        currentCoverage: ExplorerSnapshotDiffCoverage(
            status: .complete,
            measuredPermille: 1_000,
            issueRecordCount: 0,
            issueOccurrenceCount: 0
        ),
        baselineCoverage: ExplorerSnapshotDiffCoverage(
            status: .complete,
            measuredPermille: 1_000,
            issueRecordCount: 0,
            issueOccurrenceCount: 0
        )
    )
}

private func controllerSnapshotDiffRoot() -> ExplorerSnapshotDiffNode {
    ExplorerSnapshotDiffNode(
        id: 0,
        parentID: nil,
        depth: 0,
        name: ExplorerSnapshotNodeName(
            encoding: .unixBytes,
            encodedBytes: Data("/".utf8),
            display: "/"
        ),
        kind: .directory,
        currentKind: .directory,
        baselineKind: .directory,
        category: .unclassified,
        change: .grew,
        logicalChange: ExplorerSnapshotDiffValue(
            direction: .growth,
            magnitudeBytes: 512
        ),
        currentLogicalBytes: 1_536,
        baselineLogicalBytes: 1_024,
        currentAllocatedBytes: nil,
        baselineAllocatedBytes: nil,
        allocatedChange: nil,
        currentFileCount: 2,
        baselineFileCount: 1,
        currentChildCount: 1,
        baselineChildCount: 1,
        currentScanFlags: ExplorerSnapshotScanFlags(
            inaccessible: false,
            timedOut: false,
            hardLinkDuplicate: false,
            mountBoundary: false
        ),
        baselineScanFlags: ExplorerSnapshotScanFlags(
            inaccessible: false,
            timedOut: false,
            hardLinkDuplicate: false,
            mountBoundary: false
        ),
        canDescend: true
    )
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
    private let suspendsTerminalPoll: Bool
    private var cancellations = 0
    private var pollContinuation: CheckedContinuation<Void, Never>?

    init(suspendsTerminalPoll: Bool = false) {
        self.suspendsTerminalPoll = suspendsTerminalPoll
    }

    func poll() async throws -> HomeScanTaskPoll {
        if suspendsTerminalPoll {
            await withCheckedContinuation { continuation in
                pollContinuation = continuation
            }
            return HomeScanTaskPoll(
                phase: .cancelled,
                stage: .terminal,
                cancellationRequested: true,
                revision: 1,
                progress: nil,
                eventsTruncated: false,
                failure: nil,
                result: nil
            )
        }
        throw HomeScanServiceError.invalidResponse
    }

    func requestCancellation() async throws -> HomeScanCancelOutcome {
        cancellations += 1
        return .requested
    }

    func cancelCount() -> Int {
        cancellations
    }

    func hasSuspendedPoll() -> Bool {
        pollContinuation != nil
    }

    func resumeTerminalPoll() {
        pollContinuation?.resume()
        pollContinuation = nil
    }
}
