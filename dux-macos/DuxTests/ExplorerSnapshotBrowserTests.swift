import Foundation
import XCTest
@testable import DUX

@MainActor
final class ExplorerSnapshotBrowserTests: XCTestCase {
    func testOpeningLatestPublishesOneBoundedPageAfterExactAcquisitionSequence() async {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)

        await browser.openLatestIfNeeded()

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertTrue(browser.isLatestSnapshot)
        XCTAssertEqual(browser.breadcrumbs.map(\.id), [0])
        XCTAssertEqual(browser.nodes.count, 100)
        XCTAssertEqual(browser.pageOffset, 0)
        XCTAssertEqual(browser.totalChildren, 101)
        XCTAssertFalse(browser.hasPreviousPage)
        XCTAssertTrue(browser.hasNextPage)
        let calls = await reviews.recordedCalls()
        XCTAssertEqual(
            calls,
            [
                .acquireLatest,
                .root(scanID: "scan:latest"),
                .children(
                    scanID: "scan:latest",
                    parentID: 0,
                    sort: .logicalBytesDescending,
                    offset: 0,
                    limit: 100
                ),
                .treemap(scanID: "scan:latest", parentID: 0, maxCells: 48),
            ]
        )
        XCTAssertEqual(browser.treemap?.parentID, 0)
        XCTAssertEqual(browser.treemap?.cells.count, 48)
        XCTAssertEqual(browser.treemap?.otherChildCount, 53)
    }

    func testChangesLoadsLazilyAndLeavingReleasesOnlyTheComparison() async {
        let reviews = BrowserReviewStub(mode: .diffAvailable)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let browseNodes = browser.nodes

        let beforeEntry = await reviews.recordedCalls()
        XCTAssertFalse(beforeEntry.contains { call in
            if case .prepareDiff = call { return true }
            return false
        })

        await browser.selectContentMode(.changes)

        XCTAssertEqual(browser.contentMode, .changes)
        XCTAssertEqual(browser.snapshotDiffPhase, .ready)
        XCTAssertEqual(browser.snapshotDiffInfo?.currentScanID, "scan:latest")
        XCTAssertEqual(browser.snapshotDiffBreadcrumbs.map(\.id), [0])
        XCTAssertEqual(browser.snapshotDiffNodes.count, 100)
        XCTAssertEqual(browser.snapshotDiffPage?.totalChildren, 101)
        XCTAssertEqual(browser.snapshotDiffTreemap?.cells.count, 48)
        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.nodes, browseNodes)

        await browser.selectContentMode(.browse)

        XCTAssertEqual(browser.contentMode, .browse)
        XCTAssertEqual(browser.snapshotDiffPhase, .idle)
        XCTAssertNil(browser.snapshotDiffInfo)
        XCTAssertTrue(browser.snapshotDiffBreadcrumbs.isEmpty)
        XCTAssertEqual(browser.nodes, browseNodes)
        let releasedDiffCount = await reviews.releasedDiffReviewCount()
        let releasedScanIDs = await reviews.releasedScanIDs()
        XCTAssertEqual(releasedDiffCount, 1)
        XCTAssertTrue(releasedScanIDs.isEmpty)
    }

    func testUnavailableAndMalformedChangesKeepBrowseSnapshotReady() async {
        for (mode, expectedFailure, expectedReleaseCount) in [
            (BrowserReviewStub.Mode.diffUnavailable, .notAvailable, 0),
            (BrowserReviewStub.Mode.diffRootFailure, .invalidResponse, 1),
        ] as [(BrowserReviewStub.Mode, ExplorerSnapshotDiffFailure, Int)] {
            let reviews = BrowserReviewStub(mode: mode)
            let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
            await browser.reloadLatest()
            let browseNodes = browser.nodes
            let browseRoot = browser.breadcrumbs

            await browser.selectContentMode(.changes)

            XCTAssertEqual(browser.snapshotDiffPhase, .failed(expectedFailure))
            XCTAssertEqual(browser.phase, .ready)
            XCTAssertEqual(browser.scanID, "scan:latest")
            XCTAssertEqual(browser.nodes, browseNodes)
            XCTAssertEqual(browser.breadcrumbs, browseRoot)
            let releasedDiffCount = await reviews.releasedDiffReviewCount()
            XCTAssertEqual(releasedDiffCount, expectedReleaseCount)

            await browser.selectContentMode(.browse)
            XCTAssertEqual(browser.phase, .ready)
            XCTAssertEqual(browser.nodes, browseNodes)
        }
    }

    func testChangesNavigationBackBreadcrumbSortAndPagingStayComparisonLocal() async throws {
        let reviews = BrowserReviewStub(mode: .diffAvailable)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.changes)
        let browseBreadcrumbs = browser.breadcrumbs
        let directory = try XCTUnwrap(
            browser.snapshotDiffNodes.first(where: { $0.id == 1 })
        )

        await browser.openSnapshotDiffDirectory(directory)

        XCTAssertEqual(browser.snapshotDiffBreadcrumbs.map(\.id), [0, 1])
        XCTAssertEqual(browser.snapshotDiffNodes.map(\.id), [500])
        XCTAssertEqual(browser.breadcrumbs, browseBreadcrumbs)

        await browser.goBackSnapshotDiff()
        XCTAssertEqual(browser.snapshotDiffBreadcrumbs.map(\.id), [0])

        await browser.selectSnapshotDiffSort(.nameAscending)
        XCTAssertEqual(browser.snapshotDiffSort, .nameAscending)
        XCTAssertEqual(browser.snapshotDiffPage?.offset, 0)
        XCTAssertEqual(
            browser.snapshotDiffNodes.map(\.name.display),
            browser.snapshotDiffNodes.map(\.name.display).sorted()
        )
        XCTAssertTrue(browser.hasNextSnapshotDiffPage)

        await browser.showNextSnapshotDiffPage()
        XCTAssertEqual(browser.snapshotDiffPage?.offset, 100)
        XCTAssertEqual(browser.snapshotDiffNodes.count, 1)
        XCTAssertTrue(browser.hasPreviousSnapshotDiffPage)
        XCTAssertFalse(browser.hasNextSnapshotDiffPage)

        await browser.showPreviousSnapshotDiffPage()
        XCTAssertEqual(browser.snapshotDiffPage?.offset, 0)
        XCTAssertEqual(browser.snapshotDiffNodes.count, 100)
    }

    func testChangesTableAndTreemapSelectionRemainSynchronized() async throws {
        let reviews = BrowserReviewStub(mode: .diffAvailable)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.changes)
        let tableNode = try XCTUnwrap(
            browser.snapshotDiffNodes.first(where: { $0.id == 1 })
        )
        let treemapCell = try XCTUnwrap(
            browser.snapshotDiffTreemap?.cells.first(where: { $0.id != tableNode.id })
        )

        browser.selectSnapshotDiffNode(tableNode.id)
        XCTAssertEqual(browser.snapshotDiffSelection, .node(tableNode.id))
        XCTAssertEqual(browser.selectedSnapshotDiffNode, tableNode)

        browser.selectSnapshotDiffTreemapCell(treemapCell)
        XCTAssertEqual(browser.snapshotDiffSelection, .node(treemapCell.id))
        XCTAssertEqual(browser.selectedSnapshotDiffNode, treemapCell.node)

        browser.selectSnapshotDiffOtherGrowth()
        XCTAssertTrue(browser.isSnapshotDiffOtherGrowthSelected)
        browser.selectSnapshotDiffOtherShrinkage()
        XCTAssertTrue(browser.isSnapshotDiffOtherShrinkageSelected)
    }

    func testLateChangesPrepareAndSortResultsAreFencedAfterLeavingMode() async throws {
        let prepareReviews = BrowserReviewStub(mode: .suspendedDiffPrepare)
        let prepareBrowser = ExplorerSnapshotBrowserModel(reviews: prepareReviews)
        await prepareBrowser.reloadLatest()

        let entering = Task { await prepareBrowser.selectContentMode(.changes) }
        try await eventually { await prepareReviews.hasSuspendedDiffPrepare() }
        await prepareBrowser.selectContentMode(.browse)
        await prepareReviews.resumeDiffPrepare()
        await entering.value

        XCTAssertEqual(prepareBrowser.contentMode, .browse)
        XCTAssertEqual(prepareBrowser.snapshotDiffPhase, .idle)
        XCTAssertNil(prepareBrowser.snapshotDiffInfo)
        let prepareReleaseCount = await prepareReviews.releasedDiffReviewCount()
        XCTAssertEqual(prepareReleaseCount, 1)

        let sortReviews = BrowserReviewStub(mode: .suspendedDiffSort)
        let sortBrowser = ExplorerSnapshotBrowserModel(reviews: sortReviews)
        await sortBrowser.reloadLatest()
        await sortBrowser.selectContentMode(.changes)

        let sorting = Task {
            await sortBrowser.selectSnapshotDiffSort(.nameAscending)
        }
        try await eventually { await sortReviews.hasSuspendedDiffQuery() }
        await sortBrowser.selectContentMode(.browse)
        await sortReviews.resumeDiffQuery()
        await sorting.value

        XCTAssertEqual(sortBrowser.contentMode, .browse)
        XCTAssertEqual(sortBrowser.snapshotDiffPhase, .idle)
        XCTAssertNil(sortBrowser.snapshotDiffPage)
        let sortReleaseCount = await sortReviews.releasedDiffReviewCount()
        XCTAssertEqual(sortReleaseCount, 1)
    }

    func testChangesReviewReleasesOnCloseAndSnapshotReplacement() async throws {
        let closingReviews = BrowserReviewStub(mode: .diffAvailable)
        let closingBrowser = ExplorerSnapshotBrowserModel(reviews: closingReviews)
        await closingBrowser.reloadLatest()
        await closingBrowser.selectContentMode(.changes)

        await closingBrowser.close()

        XCTAssertEqual(closingBrowser.phase, .idle)
        XCTAssertEqual(closingBrowser.snapshotDiffPhase, .idle)
        let closeDiffReleaseCount = await closingReviews.releasedDiffReviewCount()
        let closeScanIDs = await closingReviews.releasedScanIDs()
        XCTAssertEqual(closeDiffReleaseCount, 1)
        XCTAssertEqual(closeScanIDs, ["scan:latest"])

        let replacementReviews = BrowserReviewStub(mode: .diffAvailable)
        let history = BrowserHistoryStub()
        let replacementBrowser = ExplorerSnapshotBrowserModel(
            reviews: replacementReviews,
            history: history
        )
        await replacementBrowser.reloadLatest()
        await replacementBrowser.reloadHistory()
        await replacementBrowser.selectContentMode(.changes)
        let older = try XCTUnwrap(
            replacementBrowser.historyScans.first { $0.scanID == "scan:older" }
        )

        await replacementBrowser.selectHistoricalScan(older)

        XCTAssertEqual(replacementBrowser.scanID, "scan:older")
        XCTAssertEqual(replacementBrowser.contentMode, .changes)
        XCTAssertEqual(replacementBrowser.snapshotDiffPhase, .ready)
        XCTAssertEqual(
            replacementBrowser.snapshotDiffInfo?.currentScanID,
            "scan:older"
        )
        let replacementDiffReleaseCount =
            await replacementReviews.releasedDiffReviewCount()
        let preparedScanIDs = await replacementReviews.preparedDiffScanIDs()
        XCTAssertEqual(replacementDiffReleaseCount, 1)
        XCTAssertEqual(preparedScanIDs, ["scan:latest", "scan:older"])
    }

    func testSubtreeReplacementReleasesChangesAndPreparesExactChildComparison() async throws {
        let reviews = BrowserReviewStub(mode: .diffAvailable)
        let driver = BrowserSubtreeScanDriver(
            outcome: .succeeded(browserScanSummary(scanID: "scan:refreshed"))
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            subtreeScans: BrowserSubtreeScanServiceStub(),
            scanDriver: driver
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.changes)
        let selectedNode = try XCTUnwrap(browser.snapshotDiffNodes.first)
        browser.selectSnapshotDiffNode(selectedNode.id)
        XCTAssertNotNil(browser.snapshotDiffSelection)

        await browser.refreshCurrentSubtree()

        XCTAssertEqual(browser.scanID, "scan:refreshed")
        XCTAssertEqual(browser.contentMode, .changes)
        XCTAssertEqual(browser.snapshotDiffPhase, .ready)
        XCTAssertEqual(
            browser.snapshotDiffInfo?.currentScanID,
            "scan:refreshed"
        )
        XCTAssertEqual(browser.snapshotDiffBreadcrumbs.map(\.id), [0])
        XCTAssertNil(browser.snapshotDiffSelection)
        let releasedDiffCount = await reviews.releasedDiffReviewCount()
        let preparedScanIDs = await reviews.preparedDiffScanIDs()
        XCTAssertEqual(releasedDiffCount, 1)
        XCTAssertEqual(preparedScanIDs, ["scan:latest", "scan:refreshed"])
    }

    func testSuspendedChangesNavigationCannotPublishAfterModeExit() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedDiffNavigation)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.changes)
        let directory = try XCTUnwrap(
            browser.snapshotDiffNodes.first(where: { $0.id == 1 })
        )
        browser.selectSnapshotDiffNode(directory.id)

        let navigating = Task {
            await browser.openSnapshotDiffDirectory(directory)
        }
        try await eventually {
            let hasPage = await reviews.hasSuspendedDiffNavigationPage()
            let hasTreemap = await reviews.hasSuspendedDiffNavigationTreemap()
            return hasPage && hasTreemap
        }
        XCTAssertTrue(browser.isSnapshotDiffNavigating)
        XCTAssertEqual(browser.snapshotDiffSelection, .node(directory.id))

        await browser.selectContentMode(.browse)
        XCTAssertNil(browser.snapshotDiffSelection)
        await reviews.resumeDiffNavigationPage()
        await reviews.resumeDiffNavigationTreemap()
        await navigating.value

        XCTAssertEqual(browser.contentMode, .browse)
        XCTAssertEqual(browser.snapshotDiffPhase, .idle)
        XCTAssertTrue(browser.snapshotDiffBreadcrumbs.isEmpty)
        XCTAssertNil(browser.snapshotDiffPage)
        XCTAssertNil(browser.snapshotDiffTreemap)
        XCTAssertNil(browser.snapshotDiffSelection)
        XCTAssertFalse(browser.isSnapshotDiffNavigating)
        let releasedDiffCount = await reviews.releasedDiffReviewCount()
        XCTAssertEqual(releasedDiffCount, 1)
    }

    func testSuspendedChangesPagingCannotPublishAfterClose() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedDiffPaging)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.changes)
        let selectedNode = try XCTUnwrap(browser.snapshotDiffNodes.first)
        browser.selectSnapshotDiffNode(selectedNode.id)

        let paging = Task { await browser.showNextSnapshotDiffPage() }
        try await eventually { await reviews.hasSuspendedDiffPaging() }
        XCTAssertTrue(browser.isSnapshotDiffPaging)
        XCTAssertEqual(browser.snapshotDiffSelection, .node(selectedNode.id))

        await browser.close()
        XCTAssertNil(browser.snapshotDiffSelection)
        await reviews.resumeDiffPaging()
        await paging.value

        XCTAssertEqual(browser.phase, .idle)
        XCTAssertEqual(browser.snapshotDiffPhase, .idle)
        XCTAssertTrue(browser.snapshotDiffBreadcrumbs.isEmpty)
        XCTAssertNil(browser.snapshotDiffPage)
        XCTAssertNil(browser.snapshotDiffTreemap)
        XCTAssertNil(browser.snapshotDiffSelection)
        XCTAssertFalse(browser.isSnapshotDiffPaging)
        let releasedDiffCount = await reviews.releasedDiffReviewCount()
        XCTAssertEqual(releasedDiffCount, 1)
    }

    func testCoverageLoadsLazilyForExactSnapshotAndFailureRemainsLocal() async {
        let reviews = BrowserReviewStub()
        let coverage = BrowserCoverageStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, coverage: coverage)
        await browser.reloadLatest()
        let requestsBeforeCoverage = await coverage.requestedScanIDs()
        XCTAssertEqual(requestsBeforeCoverage, [])

        await browser.selectContentMode(.coverage)

        let requestsAfterCoverage = await coverage.requestedScanIDs()
        XCTAssertEqual(requestsAfterCoverage, ["scan:latest"])
        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.contentMode, .coverage)
        XCTAssertEqual(browser.coverageDetails?.scanID, "scan:latest")
        XCTAssertEqual(browser.coverageDetails?.coverage, .complete)
        XCTAssertNil(browser.coverageFailure)
        XCTAssertFalse(browser.isCoverageLoading)

        await browser.selectContentMode(.browse)
        XCTAssertEqual(browser.phase, .ready)
        XCTAssertFalse(browser.nodes.isEmpty)
    }

    func testHistoricalSwitchClearsOldCoverageBeforeSuspendedRelease() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedReloadRelease)
        let history = BrowserHistoryStub()
        let coverage = BrowserCoverageStub()
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            history: history,
            coverage: coverage
        )
        await browser.reloadLatest()
        await browser.reloadHistory()
        await browser.selectContentMode(.coverage)
        XCTAssertEqual(browser.coverageDetails?.scanID, "scan:latest")
        let older = try XCTUnwrap(
            browser.historyScans.first { $0.scanID == "scan:older" }
        )

        let switching = Task { await browser.selectHistoricalScan(older) }
        try await eventually { await reviews.hasSuspendedRelease() }

        XCTAssertEqual(browser.scanID, "scan:older")
        XCTAssertNil(browser.coverageDetails)
        XCTAssertTrue(browser.isSwitchingSnapshot)

        await reviews.resumeRelease()
        await switching.value
        XCTAssertFalse(browser.isSwitchingSnapshot)
        XCTAssertEqual(browser.coverageDetails?.scanID, "scan:older")
        let requests = await coverage.requestedScanIDs()
        XCTAssertEqual(requests, ["scan:latest", "scan:older"])
    }

    func testLateCoverageResultIsDiscardedAfterLeavingCoverageMode() async throws {
        let reviews = BrowserReviewStub()
        let coverage = BrowserCoverageStub(suspends: true)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, coverage: coverage)
        await browser.reloadLatest()

        let loading = Task { await browser.selectContentMode(.coverage) }
        try await eventually { await coverage.hasSuspendedRequest() }
        XCTAssertTrue(browser.isCoverageLoading)

        await browser.selectContentMode(.browse)
        await coverage.resumeRequest()
        await loading.value

        XCTAssertEqual(browser.contentMode, .browse)
        XCTAssertNil(browser.coverageDetails)
        XCTAssertFalse(browser.isCoverageLoading)
    }

    func testLiveActionsResolveExactSelectionAndInvokeOnlyRequestedPresenter() async throws {
        let reviews = BrowserReviewStub()
        let presenter = BrowserLiveActionPresenterSpy()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, liveActions: presenter)
        await browser.reloadLatest()
        let file = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .file }))
        browser.selectTableNode(file.id)

        await browser.revealLiveItem()
        XCTAssertEqual(presenter.revealed.map(\.nodeID), [file.id])
        XCTAssertTrue(browser.liveActionNotice?.message.contains("Finder") == true)

        await browser.copyLiveItemPath()
        XCTAssertEqual(presenter.copied.map(\.nodeID), [file.id])

        await browser.quickLookLiveItem()
        XCTAssertEqual(presenter.previewed.map(\.nodeID), [file.id])
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.liveItem(
            scanID: "scan:latest",
            nodeID: file.id,
            purpose: .quickLook
        )))
    }

    func testICloudReviewIsManualExactAndObservationOnly() async throws {
        let reviews = BrowserReviewStub(mode: .iCloudEligible)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let callsBeforeCheck = await reviews.recordedCalls()
        XCTAssertFalse(callsBeforeCheck.contains {
            if case .iCloudProbe = $0 { return true }
            return false
        })

        let file = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .file }))
        browser.selectTableNode(file.id)
        XCTAssertTrue(browser.canCheckSelectedICloudLocalCopy)

        await browser.checkSelectedICloudLocalCopy()

        guard case let .observed(assessment) = browser.iCloudLocalCopyReviewState else {
            return XCTFail("Expected one favorable point-in-time observation")
        }
        XCTAssertTrue(assessment.isEligibleObservation)
        XCTAssertEqual(assessment.localAllocatedBytes, 8192)
        let callsAfterCheck = await reviews.recordedCalls()
        XCTAssertTrue(callsAfterCheck.contains(
            .iCloudProbe(scanID: "scan:latest", nodeID: file.id)
        ))
    }

    func testICloudReviewPublishesBlockersAndChangedFailureWithoutChangingSnapshot() async throws {
        let blockedReviews = BrowserReviewStub(mode: .iCloudBlocked)
        let blockedBrowser = ExplorerSnapshotBrowserModel(reviews: blockedReviews)
        await blockedBrowser.reloadLatest()
        let blockedFile = try XCTUnwrap(
            blockedBrowser.nodes.first(where: { $0.kind == .file })
        )
        blockedBrowser.selectTableNode(blockedFile.id)
        await blockedBrowser.checkSelectedICloudLocalCopy()
        guard case let .observed(blocked) = blockedBrowser.iCloudLocalCopyReviewState else {
            return XCTFail("Expected a blocked observation")
        }
        XCTAssertFalse(blocked.isEligibleObservation)
        XCTAssertEqual(blocked.blockers, [.uploadStateUnknown])

        let changedReviews = BrowserReviewStub(mode: .iCloudChanged)
        let changedBrowser = ExplorerSnapshotBrowserModel(reviews: changedReviews)
        await changedBrowser.reloadLatest()
        let changedFile = try XCTUnwrap(
            changedBrowser.nodes.first(where: { $0.kind == .file })
        )
        changedBrowser.selectTableNode(changedFile.id)
        await changedBrowser.checkSelectedICloudLocalCopy()
        XCTAssertEqual(
            changedBrowser.iCloudLocalCopyReviewState,
            .failed(.changedSinceSnapshot)
        )
        XCTAssertEqual(changedBrowser.phase, .ready)
        XCTAssertEqual(changedBrowser.scanID, "scan:latest")
    }

    func testLateICloudReviewIsDiscardedAfterSelectionChanges() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedICloudProbe)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let files = browser.nodes.filter { $0.kind == .file }
        let first = try XCTUnwrap(files.first)
        let second = try XCTUnwrap(files.dropFirst().first)
        browser.selectTableNode(first.id)

        let check = Task { await browser.checkSelectedICloudLocalCopy() }
        try await eventually { await reviews.hasSuspendedICloudProbe() }
        XCTAssertEqual(browser.iCloudLocalCopyReviewState, .checking)
        browser.selectTableNode(second.id)
        await reviews.resumeICloudProbe()
        await check.value

        XCTAssertEqual(browser.selectedNodeID, second.id)
        XCTAssertEqual(browser.iCloudLocalCopyReviewState, .idle)
    }

    func testLateICloudReviewIsDiscardedAfterEveryBrowserContextChange() async throws {
        enum ContextChange: CaseIterable {
            case contentMode
            case navigation
            case close
        }

        for change in ContextChange.allCases {
            let reviews = BrowserReviewStub(mode: .suspendedICloudProbe)
            let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
            await browser.reloadLatest()
            let file = try XCTUnwrap(
                browser.nodes.first(where: { $0.kind == .file })
            )
            let directory = try XCTUnwrap(
                browser.nodes.first(where: { $0.kind == .directory })
            )
            browser.selectTableNode(file.id)

            let check = Task { await browser.checkSelectedICloudLocalCopy() }
            try await eventually { await reviews.hasSuspendedICloudProbe() }
            switch change {
            case .contentMode:
                await browser.selectContentMode(.largeFiles)
            case .navigation:
                await browser.openDirectory(directory)
            case .close:
                await browser.close()
            }
            await reviews.resumeICloudProbe()
            await check.value

            XCTAssertEqual(
                browser.iCloudLocalCopyReviewState,
                .idle,
                "stale iCloud result after \(change)"
            )
        }
    }

    func testICloudReviewRequiresASelectedRegularFileAndClearsOnModeChange() async throws {
        let reviews = BrowserReviewStub(mode: .iCloudEligible)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let directory = try XCTUnwrap(
            browser.nodes.first(where: { $0.kind == .directory })
        )
        browser.selectTableNode(directory.id)
        XCTAssertFalse(browser.canCheckSelectedICloudLocalCopy)
        await browser.checkSelectedICloudLocalCopy()

        let file = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .file }))
        browser.selectTableNode(file.id)
        await browser.checkSelectedICloudLocalCopy()
        guard case .observed = browser.iCloudLocalCopyReviewState else {
            return XCTFail("Expected an observation before leaving Browse")
        }
        await browser.selectContentMode(.largeFiles)
        XCTAssertEqual(browser.iCloudLocalCopyReviewState, .idle)

        let probes = await reviews.recordedCalls().filter {
            if case .iCloudProbe = $0 { return true }
            return false
        }
        XCTAssertEqual(probes, [
            .iCloudProbe(scanID: "scan:latest", nodeID: file.id),
        ])
    }

    func testICloudStatusLoadsBoundedSourceWithoutAutomaticProbes() async throws {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        await browser.selectContentMode(.iCloudStatus)

        let source = try XCTUnwrap(browser.iCloudObservationSource)
        XCTAssertEqual(source.scanID, "scan:latest")
        XCTAssertEqual(source.scopeNodeID, 0)
        XCTAssertEqual(source.requestedMaxResults, 32)
        XCTAssertEqual(source.targets.map(\.rank), [0, 1, 2])
        XCTAssertEqual(source.targets.map(\.id), [910, 911, 912])
        XCTAssertEqual(browser.iCloudObservationBatchPhase, .idle)
        XCTAssertTrue(browser.iCloudObservationResults.isEmpty)
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.iCloudObservationSource(
            scanID: "scan:latest",
            scopeNodeID: 0,
            maxResults: 32
        )))
        XCTAssertFalse(calls.contains {
            if case .iCloudProbe = $0 { return true }
            return false
        })
    }

    func testICloudObservationBatchChecksTargetsSeriallyInSourceRankOrder() async throws {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.iCloudStatus)

        await browser.startICloudObservationBatch()

        XCTAssertEqual(browser.iCloudObservationBatchPhase, .completed(total: 3))
        XCTAssertEqual(browser.iCloudObservationResults.count, 3)
        for target in try XCTUnwrap(browser.iCloudObservationSource).targets {
            guard case let .observed(assessment) = browser.iCloudObservationResults[target.id] else {
                return XCTFail("Expected an observation for rank \(target.rank)")
            }
            XCTAssertEqual(assessment.localAllocatedBytes, target.node.allocatedBytes)
        }
        let probes = await reviews.recordedCalls().filter {
            if case .iCloudProbe = $0 { return true }
            return false
        }
        XCTAssertEqual(probes, [
            .iCloudProbe(scanID: "scan:latest", nodeID: 910),
            .iCloudProbe(scanID: "scan:latest", nodeID: 911),
            .iCloudProbe(scanID: "scan:latest", nodeID: 912),
        ])
        let maximumConcurrentProbes = await reviews.maximumConcurrentICloudProbeCount()
        XCTAssertEqual(maximumConcurrentProbes, 1)
    }

    func testICloudObservationBatchContinuesAfterPerItemFailures() async throws {
        let failures: [UInt64: ExplorerICloudLocalCopyProbeError] = [
            910: .invalidTarget,
            911: .changedSinceSnapshot,
            912: .failed,
        ]
        let reviews = BrowserReviewStub(iCloudProbeFailures: failures)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.iCloudStatus)

        await browser.startICloudObservationBatch()

        XCTAssertEqual(browser.iCloudObservationBatchPhase, .completed(total: 3))
        XCTAssertEqual(browser.iCloudObservationResults, [
            910: .failed(.invalidTarget),
            911: .failed(.changedSinceSnapshot),
            912: .failed(.failed),
        ])
        let probes = await reviews.recordedCalls().filter {
            if case .iCloudProbe = $0 { return true }
            return false
        }
        XCTAssertEqual(probes, [
            .iCloudProbe(scanID: "scan:latest", nodeID: 910),
            .iCloudProbe(scanID: "scan:latest", nodeID: 911),
            .iCloudProbe(scanID: "scan:latest", nodeID: 912),
        ])
    }

    func testICloudObservationBatchStopsAfterSystemicFailure() async {
        for failure in [
            ExplorerICloudLocalCopyProbeError.unsupported,
            .unavailable,
            .invalidResponse,
        ] {
            let reviews = BrowserReviewStub(iCloudProbeFailures: [910: failure])
            let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
            await browser.reloadLatest()
            await browser.selectContentMode(.iCloudStatus)

            await browser.startICloudObservationBatch()

            XCTAssertEqual(
                browser.iCloudObservationBatchPhase,
                .stopped(error: failure, completed: 1, total: 3)
            )
            XCTAssertEqual(browser.iCloudObservationResults, [
                910: .failed(failure),
            ])
            let probes = await reviews.recordedCalls().filter {
                if case .iCloudProbe = $0 { return true }
                return false
            }
            XCTAssertEqual(
                probes,
                [.iCloudProbe(scanID: "scan:latest", nodeID: 910)],
                "systemic \(failure) must stop later checks"
            )
        }
    }

    func testICloudObservationBatchRejectsAllocationMismatchAndStops() async {
        let reviews = BrowserReviewStub(
            iCloudProbeAllocationOverrides: [910: 1]
        )
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.iCloudStatus)

        await browser.startICloudObservationBatch()

        XCTAssertEqual(
            browser.iCloudObservationBatchPhase,
            .stopped(error: .invalidResponse, completed: 1, total: 3)
        )
        XCTAssertEqual(browser.iCloudObservationResults, [
            910: .failed(.invalidResponse),
        ])
        let probes = await reviews.recordedCalls().filter {
            if case .iCloudProbe = $0 { return true }
            return false
        }
        XCTAssertEqual(probes, [
            .iCloudProbe(scanID: "scan:latest", nodeID: 910),
        ])
    }

    func testICloudObservationBatchDiscardsLeaseGenerationCancellation() async throws {
        let reviews = BrowserReviewStub(
            cancelledICloudProbeNodeIDs: [910],
            suspendedICloudProbeNodeID: 910
        )
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.iCloudStatus)

        let checking = Task { await browser.startICloudObservationBatch() }
        try await eventually { await reviews.hasSuspendedICloudProbe() }
        await reviews.resumeICloudProbe()
        await checking.value

        XCTAssertEqual(
            browser.iCloudObservationBatchPhase,
            .cancelled(completed: 0, total: 3)
        )
        XCTAssertTrue(browser.iCloudObservationResults.isEmpty)
        XCTAssertNil(browser.checkingICloudObservationNodeID)
        let probes = await reviews.recordedCalls().filter {
            if case .iCloudProbe = $0 { return true }
            return false
        }
        XCTAssertEqual(probes, [
            .iCloudProbe(scanID: "scan:latest", nodeID: 910),
        ])
    }

    func testStoppingICloudObservationBatchRejectsInFlightAndRemainingResults() async throws {
        let reviews = BrowserReviewStub(suspendedICloudProbeNodeID: 910)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.iCloudStatus)

        let checking = Task { await browser.startICloudObservationBatch() }
        try await eventually { await reviews.hasSuspendedICloudProbe() }
        XCTAssertEqual(
            browser.iCloudObservationBatchPhase,
            .checking(completed: 0, total: 3)
        )

        browser.stopICloudObservationBatch()
        XCTAssertEqual(
            browser.iCloudObservationBatchPhase,
            .stopping(completed: 0, total: 3)
        )
        await reviews.resumeICloudProbe()
        await checking.value

        XCTAssertEqual(
            browser.iCloudObservationBatchPhase,
            .cancelled(completed: 0, total: 3)
        )
        XCTAssertTrue(browser.iCloudObservationResults.isEmpty)
        XCTAssertNil(browser.checkingICloudObservationNodeID)
        let probes = await reviews.recordedCalls().filter {
            if case .iCloudProbe = $0 { return true }
            return false
        }
        XCTAssertEqual(probes, [
            .iCloudProbe(scanID: "scan:latest", nodeID: 910),
        ])
    }

    func testSecondICloudObservationBatchStartDoesNotOverlapActiveRun() async throws {
        let reviews = BrowserReviewStub(suspendedICloudProbeNodeID: 910)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.iCloudStatus)

        let first = Task { await browser.startICloudObservationBatch() }
        try await eventually { await reviews.hasSuspendedICloudProbe() }
        await browser.startICloudObservationBatch()

        let probesWhileSuspended = await reviews.recordedCalls().filter {
            if case .iCloudProbe = $0 { return true }
            return false
        }
        XCTAssertEqual(probesWhileSuspended, [
            .iCloudProbe(scanID: "scan:latest", nodeID: 910),
        ])
        let maximumConcurrentProbes = await reviews.maximumConcurrentICloudProbeCount()
        XCTAssertEqual(maximumConcurrentProbes, 1)

        await reviews.resumeICloudProbe()
        await first.value
        XCTAssertEqual(browser.iCloudObservationBatchPhase, .completed(total: 3))
    }

    func testICloudObservationSelectionChangeDoesNotCancelBatch() async throws {
        let reviews = BrowserReviewStub(suspendedICloudProbeNodeID: 910)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.iCloudStatus)

        let checking = Task { await browser.startICloudObservationBatch() }
        try await eventually { await reviews.hasSuspendedICloudProbe() }
        browser.selectICloudObservationTarget(911)
        XCTAssertEqual(browser.selectedNodeID, 911)

        await reviews.resumeICloudProbe()
        await checking.value

        XCTAssertEqual(browser.selectedNodeID, 911)
        XCTAssertEqual(browser.iCloudObservationBatchPhase, .completed(total: 3))
        XCTAssertEqual(browser.iCloudObservationResults.count, 3)
    }

    func testICloudObservationBatchRejectsLateResultsAfterEveryContextChange() async throws {
        enum ContextChange: CaseIterable {
            case mode
            case navigation
            case snapshot
            case close
        }

        for change in ContextChange.allCases {
            let reviews = BrowserReviewStub(suspendedICloudProbeNodeID: 910)
            let history = BrowserHistoryStub()
            let browser = ExplorerSnapshotBrowserModel(
                reviews: reviews,
                history: history
            )
            await browser.reloadLatest()
            await browser.reloadHistory()
            await browser.selectContentMode(.iCloudStatus)
            let originalSource = try XCTUnwrap(browser.iCloudObservationSource)
            let checking = Task { await browser.startICloudObservationBatch() }
            try await eventually { await reviews.hasSuspendedICloudProbe() }

            switch change {
            case .mode:
                await browser.selectContentMode(.browse)
            case .navigation:
                let directory = try XCTUnwrap(
                    browser.nodes.first(where: { $0.kind == .directory })
                )
                await browser.openDirectory(directory)
            case .snapshot:
                let older = try XCTUnwrap(
                    browser.historyScans.first(where: { $0.scanID == "scan:older" })
                )
                await browser.selectHistoricalScan(older)
            case .close:
                await browser.close()
            }

            await reviews.resumeICloudProbe()
            await checking.value

            XCTAssertTrue(
                browser.iCloudObservationResults.isEmpty,
                "published stale result after \(change)"
            )
            XCTAssertNil(
                browser.checkingICloudObservationNodeID,
                "retained stale checking row after \(change)"
            )
            XCTAssertNotEqual(
                browser.iCloudObservationSource,
                originalSource,
                "retained stale source after \(change)"
            )
            let probes = await reviews.recordedCalls().filter {
                if case .iCloudProbe = $0 { return true }
                return false
            }
            XCTAssertEqual(
                probes,
                [.iCloudProbe(scanID: "scan:latest", nodeID: 910)],
                "started a later probe after \(change)"
            )
        }
    }

    func testCategoryIsConsistentAcrossPageTreemapLargeFilesAndSelection() async throws {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        let folder = try XCTUnwrap(browser.nodes.first(where: { $0.id == 1 }))
        XCTAssertEqual(folder.category, .developerArtifact)
        XCTAssertEqual(browser.treemap?.cell(nodeID: 1)?.node.category, .developerArtifact)
        browser.selectTableNode(1)
        XCTAssertEqual(browser.selectedNode?.category, .developerArtifact)

        await browser.selectContentMode(.largeFiles)
        XCTAssertEqual(
            Set(browser.largeFilesPage?.files.map(\.node.category) ?? []),
            [.installerAndDownload]
        )
    }

    func testTreemapAccessibilityCopyDoesNotInventCategoriesForOther() async throws {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let treemap = try XCTUnwrap(browser.treemap)
        let node = try XCTUnwrap(treemap.cells.first?.node)

        XCTAssertTrue(treemap.otherAccessibilitySummary.contains("categories not summarized"))
        XCTAssertTrue(treemap.otherAccessibilitySummary.contains("of the current folder"))
        XCTAssertFalse(treemap.otherAccessibilitySummary.contains("unclassified"))
        XCTAssertTrue(
            node.treemapAccessibilitySummary(total: treemap.totalChildLogicalBytes)
                .contains("of the current folder")
        )
    }

    func testSuspendedLiveActionIsDiscardedAfterSelectionChanges() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedLiveAction)
        let presenter = BrowserLiveActionPresenterSpy()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, liveActions: presenter)
        await browser.reloadLatest()
        let files = browser.nodes.filter { $0.kind == .file }
        let first = try XCTUnwrap(files.first)
        let second = try XCTUnwrap(files.dropFirst().first)
        browser.selectTableNode(first.id)

        let action = Task { await browser.revealLiveItem() }
        try await eventually { await reviews.hasSuspendedLiveAction() }
        XCTAssertTrue(browser.isLiveActionLoading)
        browser.selectTableNode(second.id)
        await reviews.resumeLiveAction()
        await action.value

        XCTAssertTrue(presenter.revealed.isEmpty)
        XCTAssertNil(browser.liveActionNotice)
        XCTAssertFalse(browser.isLiveActionLoading)
    }

    func testSuspendedLargeFileActionIsDiscardedAfterEveryQueryChange() async throws {
        enum QueryChange: CaseIterable {
            case threshold
            case age
            case refresh
        }

        for change in QueryChange.allCases {
            let reviews = BrowserReviewStub(mode: .suspendedLiveAction)
            let presenter = BrowserLiveActionPresenterSpy()
            let browser = ExplorerSnapshotBrowserModel(reviews: reviews, liveActions: presenter)
            await browser.reloadLatest()
            await browser.selectContentMode(.largeFiles)
            let file = try XCTUnwrap(browser.largeFilesPage?.files.first)
            browser.selectLargeFile(file.id)

            let action = Task { await browser.revealLiveItem() }
            try await eventually { await reviews.hasSuspendedLiveAction() }
            switch change {
            case .threshold:
                await browser.selectLargeFileThreshold(.gibibytes5)
            case .age:
                await browser.selectLargeFileAge(
                    .days90,
                    now: Date(timeIntervalSince1970: 1_800_000_000)
                )
            case .refresh:
                await browser.reloadLargeFiles()
            }
            await reviews.resumeLiveAction()
            await action.value

            XCTAssertTrue(presenter.revealed.isEmpty, "stale presenter call after \(change)")
            XCTAssertNil(browser.liveActionNotice)
            XCTAssertFalse(browser.isLiveActionLoading)
        }
    }

    func testChangedLiveItemPreservesHistoricalBrowserAndPerformsNoAction() async throws {
        let reviews = BrowserReviewStub(mode: .liveActionChanged)
        let presenter = BrowserLiveActionPresenterSpy()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, liveActions: presenter)
        await browser.reloadLatest()
        let file = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .file }))
        browser.selectTableNode(file.id)

        await browser.quickLookLiveItem()

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertTrue(browser.liveActionNotice?.isFailure == true)
        XCTAssertTrue(browser.liveActionNotice?.message.contains("changed since the scan") == true)
        XCTAssertTrue(presenter.previewed.isEmpty)
    }

    func testCopyPathDoesNotClaimSuccessForNonUnicodeFilesystemName() async throws {
        let reviews = BrowserReviewStub(mode: .nonUnicodeLiveItem)
        let presenter = BrowserLiveActionPresenterSpy()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, liveActions: presenter)
        await browser.reloadLatest()
        let file = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .file }))
        browser.selectTableNode(file.id)

        await browser.copyLiveItemPath()

        XCTAssertTrue(browser.liveActionNotice?.isFailure == true)
        XCTAssertTrue(browser.liveActionNotice?.message.contains("cannot be copied as text") == true)
        XCTAssertEqual(presenter.copied.count, 1)
    }

    func testPagingSortingAndBreadcrumbNavigationReplaceOnlyConfirmedPage() async throws {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let directory = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .directory }))
        let file = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .file }))

        await browser.showNextPage()
        XCTAssertEqual(browser.pageOffset, 100)
        XCTAssertEqual(browser.nodes.count, 1)
        XCTAssertTrue(browser.hasPreviousPage)
        XCTAssertFalse(browser.hasNextPage)

        await browser.showPreviousPage()
        XCTAssertEqual(browser.pageOffset, 0)
        XCTAssertEqual(browser.nodes.count, 100)

        for sort in ExplorerSnapshotNodeSort.allCases where sort != browser.sort {
            await browser.selectSort(sort)
            XCTAssertEqual(browser.sort, sort)
            XCTAssertEqual(browser.pageOffset, 0)
            XCTAssertLessThanOrEqual(browser.nodes.count, Int(ExplorerSnapshotBrowserModel.pageLimit))
        }

        let callsBeforeFile = await reviews.recordedCalls().count
        await browser.openDirectory(file)
        let callsAfterFile = await reviews.recordedCalls().count
        XCTAssertEqual(callsAfterFile, callsBeforeFile)

        await browser.openDirectory(directory)
        XCTAssertEqual(browser.breadcrumbs.map(\.id), [0, directory.id])
        XCTAssertEqual(browser.nodes.map(\.name.display), ["nested.log"])

        await browser.goToBreadcrumb(at: 0)
        XCTAssertEqual(browser.breadcrumbs.map(\.id), [0])
        XCTAssertEqual(browser.pageOffset, 0)
    }

    func testNoSnapshotIsTruthfulAndDistinctFromCorruptResponse() async {
        let reviews = BrowserReviewStub(mode: .noSnapshot)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)

        await browser.reloadLatest()

        XCTAssertEqual(browser.phase, .failed(.noSnapshot))
        XCTAssertNil(browser.scanID)
        XCTAssertTrue(browser.nodes.isEmpty)
        let calls = await reviews.recordedCalls()
        XCTAssertEqual(calls, [.acquireLatest])
    }

    func testFailureAfterAcquisitionReleasesLeaseAndExpiryRequiresReacquisition() async {
        let reviews = BrowserReviewStub(mode: .rootExpired)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)

        await browser.reloadLatest()

        XCTAssertEqual(browser.phase, .failed(.expired))
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest"])
        XCTAssertNil(browser.scanID)
    }

    func testCloseReturnsToIdleAndReleasesCurrentReview() async {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        await browser.close()

        XCTAssertEqual(browser.phase, .idle)
        XCTAssertNil(browser.scanID)
        XCTAssertTrue(browser.breadcrumbs.isEmpty)
        XCTAssertTrue(browser.nodes.isEmpty)
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest"])
    }

    func testLargeFilesLoadsLazilyOnSharedReviewAndSelectsHistoricalObservation() async throws {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        XCTAssertNil(browser.largeFilesPage)
        await browser.selectContentMode(.largeFiles)

        let page = try XCTUnwrap(browser.largeFilesPage)
        XCTAssertEqual(page.totalMatchingFiles, 2)
        XCTAssertEqual(page.totalMatchingLogicalBytes, 8_589_934_592)
        XCTAssertEqual(page.files.map(\.node.name.display), ["archive.mov", "image.dmg"])
        XCTAssertFalse(page.hasMore)
        XCTAssertFalse(browser.isLargeFilesLoading)
        let largeFileCalls = await reviews.recordedCalls().filter {
            if case .largeFiles = $0 { true } else { false }
        }
        XCTAssertEqual(
            largeFileCalls,
            [.largeFiles(
                scanID: "scan:latest",
                minimumLogicalBytes: ExplorerSnapshotLargeFileThreshold.gibibyte1.rawValue,
                modifiedBefore: nil,
                maxResults: 100
            )]
        )

        browser.selectLargeFile(page.files.last?.id)
        XCTAssertEqual(browser.selectedNode, page.files.last?.node)
        XCTAssertFalse(browser.isOtherSelected)
    }

    func testLargeFileAgeAndSizeFiltersSendBoundedStrictRequest() async throws {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.largeFiles)
        let now = Date(timeIntervalSince1970: 1_800_000_000)

        await browser.selectLargeFileThreshold(.gibibytes5)
        await browser.selectLargeFileAge(.days90, now: now)

        let expectedCutoff = try XCTUnwrap(
            ExplorerSnapshotLargeFileAge.days90.modifiedBefore(now: now)
        )
        XCTAssertEqual(browser.largeFilesPage?.minimumLogicalBytes, 5_368_709_120)
        XCTAssertEqual(browser.largeFilesPage?.modifiedBefore, expectedCutoff)
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.largeFiles(
            scanID: "scan:latest",
            minimumLogicalBytes: 5_368_709_120,
            modifiedBefore: expectedCutoff,
            maxResults: 100
        )))
    }

    func testLeavingLargeFilesDiscardsSuspendedResult() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedLargeFiles)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        let loading = Task { await browser.selectContentMode(.largeFiles) }
        try await eventually { await reviews.hasSuspendedLargeFiles() }
        await browser.selectContentMode(.browse)
        await reviews.resumeLargeFiles()
        await loading.value

        XCTAssertEqual(browser.contentMode, .browse)
        XCTAssertNil(browser.largeFilesPage)
        XCTAssertFalse(browser.isLargeFilesLoading)
    }

    func testLargeFileExpiryClosesSharedReview() async {
        let reviews = BrowserReviewStub(mode: .largeFilesExpired)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        await browser.selectContentMode(.largeFiles)

        XCTAssertEqual(browser.phase, .failed(.expired))
        XCTAssertNil(browser.scanID)
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest"])
    }

    func testFailedDirectoryBudgetKeepsLastConfirmedPage() async throws {
        let reviews = BrowserReviewStub(mode: .directoryBudget)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let directory = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .directory }))
        let confirmedIDs = browser.nodes.map(\.id)

        await browser.openDirectory(directory)

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.breadcrumbs.map(\.id), [0])
        XCTAssertEqual(browser.nodes.map(\.id), confirmedIDs)
        XCTAssertEqual(browser.operationFailure, .budgetExceeded)
    }

    func testCloseDuringAcquisitionReleasesLateLeaseWithoutPublishing() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedAcquire)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        let loading = Task { await browser.reloadLatest() }
        try await eventually { await reviews.hasSuspendedAcquisition() }

        await browser.close()
        await reviews.resumeAcquisition()
        await loading.value

        XCTAssertEqual(browser.phase, .idle)
        XCTAssertNil(browser.scanID)
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest"])
    }

    func testOlderDismissCannotCloseNewPresentationAndHiddenReloadIsIgnored() async {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        let first = UUID()
        let second = UUID()
        await browser.present(id: first)

        await browser.present(id: second)
        await browser.dismiss(id: first)
        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")

        await browser.dismiss(id: second)
        let acquisitionsBeforeHiddenReload = await reviews.recordedCalls().filter {
            $0 == .acquireLatest
        }.count
        await browser.reloadLatest(ifPresented: second)
        let acquisitionsAfterHiddenReload = await reviews.recordedCalls().filter {
            $0 == .acquireLatest
        }.count
        XCTAssertEqual(browser.phase, .idle)
        XCTAssertEqual(acquisitionsAfterHiddenReload, acquisitionsBeforeHiddenReload)
    }

    func testStaleSortCannotRollbackNewerReloadOrdering() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedSort)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        let sorting = Task { await browser.selectSort(.nameAscending) }
        try await eventually { await reviews.hasSuspendedQuery() }
        await browser.reloadLatest()
        await reviews.resumeQuery()
        await sorting.value

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.sort, .nameAscending)
        XCTAssertEqual(browser.pageOffset, 0)
        XCTAssertFalse(browser.isNavigating)
    }

    func testBreadcrumbNavigationCannotOverlapPaging() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedNextPage)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let directory = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .directory }))

        let paging = Task { await browser.showNextPage() }
        try await eventually { await reviews.hasSuspendedQuery() }
        await browser.openDirectory(directory)
        XCTAssertEqual(browser.breadcrumbs.map(\.id), [0])

        await reviews.resumeQuery()
        await paging.value
        XCTAssertFalse(browser.isPaging)
        XCTAssertEqual(browser.pageOffset, 100)
    }

    func testTreemapFailureKeepsConfirmedTextualPage() async {
        let reviews = BrowserReviewStub(mode: .treemapBudget)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)

        await browser.reloadLatest()

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.nodes.count, 100)
        XCTAssertNil(browser.treemap)
        XCTAssertEqual(browser.treemapFailure, .budgetExceeded)
        XCTAssertNil(browser.operationFailure)
    }

    func testConflictingCategoryAcrossPageAndTreemapRejectsTreemap() async {
        let reviews = BrowserReviewStub(mode: .treemapCategoryMismatch)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)

        await browser.reloadLatest()

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.nodes.first(where: { $0.id == 1 })?.category, .developerArtifact)
        XCTAssertNil(browser.treemap)
        XCTAssertEqual(browser.treemapFailure, .invalidResponse)
        XCTAssertNil(browser.operationFailure)
    }

    func testTreemapSelectionRestoresLogicalPageAndTableSelectionHighlightsOther() async throws {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let treemap = try XCTUnwrap(browser.treemap)
        let cell = try XCTUnwrap(treemap.cells.first)

        await browser.showNextPage()
        XCTAssertEqual(browser.pageOffset, 100)
        await browser.selectTreemapCell(cell)

        XCTAssertEqual(browser.sort, .logicalBytesDescending)
        XCTAssertEqual(browser.pageOffset, 0)
        XCTAssertEqual(browser.selectedNodeID, cell.id)
        XCTAssertTrue(browser.nodes.contains(where: { $0.id == cell.id }))

        let omitted = try XCTUnwrap(
            browser.nodes.last(where: { treemap.cell(nodeID: $0.id) == nil })
        )
        browser.selectTableNode(omitted.id)
        XCTAssertEqual(browser.selectedNodeID, omitted.id)
        XCTAssertTrue(browser.isOtherSelected)
        XCTAssertEqual(browser.selectedNode?.id, omitted.id)
    }

    func testOffPageTreemapSelectionRejectsPageWithConflictingCategory() async throws {
        let reviews = BrowserReviewStub(mode: .offPageCategoryMismatch)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        let cell = try XCTUnwrap(browser.treemap?.cell(nodeID: 1))

        await browser.showNextPage()
        XCTAssertEqual(browser.pageOffset, 100)
        await browser.selectTreemapCell(cell)

        XCTAssertEqual(browser.pageOffset, 100)
        XCTAssertFalse(browser.nodes.contains(where: { $0.id == cell.id }))
        XCTAssertEqual(browser.selectedNode, cell.node)
        XCTAssertEqual(browser.operationFailure, .invalidResponse)
    }

    func testPagingAndSortingDoNotReloadStableTreemap() async {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        await browser.showNextPage()
        await browser.showPreviousPage()
        await browser.selectSort(.nameAscending)

        let treemapCalls = await reviews.recordedCalls().filter {
            if case .treemap = $0 { true } else { false }
        }
        XCTAssertEqual(treemapCalls.count, 1)
        XCTAssertEqual(browser.treemap?.parentID, 0)
    }

    func testTreemapExpiryInvalidatesReviewAndLateTreemapCannotPublishAfterClose() async throws {
        let expiredReviews = BrowserReviewStub(mode: .treemapExpired)
        let expiredBrowser = ExplorerSnapshotBrowserModel(reviews: expiredReviews)
        await expiredBrowser.reloadLatest()
        XCTAssertEqual(expiredBrowser.phase, .failed(.expired))
        let expiredReleases = await expiredReviews.releasedScanIDs()
        XCTAssertEqual(expiredReleases, ["scan:latest"])

        let suspendedReviews = BrowserReviewStub(mode: .suspendedTreemap)
        let suspendedBrowser = ExplorerSnapshotBrowserModel(reviews: suspendedReviews)
        let opening = Task { await suspendedBrowser.reloadLatest() }
        try await eventually { await suspendedReviews.hasSuspendedTreemap() }
        await suspendedBrowser.close()
        await suspendedReviews.resumeTreemap()
        await opening.value
        XCTAssertEqual(suspendedBrowser.phase, .idle)
        XCTAssertNil(suspendedBrowser.treemap)
        let suspendedReleases = await suspendedReviews.releasedScanIDs()
        XCTAssertEqual(suspendedReleases, ["scan:latest"])
    }

    func testStaleExpiryCannotResurrectClosedOrReplaceReloadedBrowser() async throws {
        let closingReviews = BrowserReviewStub(mode: .pageExpiredWithSuspendedRelease)
        let closingBrowser = ExplorerSnapshotBrowserModel(reviews: closingReviews)
        await closingBrowser.reloadLatest()
        let expiringPage = Task { await closingBrowser.showNextPage() }
        try await eventually { await closingReviews.hasSuspendedRelease() }
        await closingBrowser.close()
        await closingReviews.resumeRelease()
        await expiringPage.value
        XCTAssertEqual(closingBrowser.phase, .idle)
        XCTAssertNil(closingBrowser.scanID)

        let reloadingReviews = BrowserReviewStub(mode: .pageExpiredWithSuspendedRelease)
        let reloadingBrowser = ExplorerSnapshotBrowserModel(reviews: reloadingReviews)
        await reloadingBrowser.reloadLatest()
        let staleExpiry = Task { await reloadingBrowser.showNextPage() }
        try await eventually { await reloadingReviews.hasSuspendedRelease() }
        await reloadingBrowser.reloadLatest()
        await reloadingReviews.resumeRelease()
        await staleExpiry.value
        XCTAssertEqual(reloadingBrowser.phase, .ready)
        XCTAssertEqual(reloadingBrowser.scanID, "scan:latest")
        XCTAssertNotNil(reloadingBrowser.treemap)
    }

    func testStaleReloadWaitingOnReleaseCannotSupersedeNewerLatestAcquisition() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedReloadRelease)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()

        let staleReload = Task { await browser.reloadLatest() }
        try await eventually { await reviews.hasSuspendedRelease() }
        await browser.reloadLatest()
        await reviews.resumeRelease()
        await staleReload.value

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        let latestAcquisitions = await reviews.recordedCalls().filter { $0 == .acquireLatest }
        XCTAssertEqual(latestAcquisitions.count, 2)
    }

    func testRecentHistoryIsBoundedAndDoesNotDelayOrReplaceLatestSnapshot() async {
        let reviews = BrowserReviewStub()
        let history = BrowserHistoryStub(hasMore: true)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, history: history)

        await browser.present(id: UUID())

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertEqual(browser.historyScans.map(\.scanID), ["scan:latest", "scan:older"])
        XCTAssertTrue(browser.historyHasMore)
        XCTAssertNil(browser.historyFailure)
        let limits = await history.requestedLimits()
        XCTAssertEqual(limits, [ExplorerSnapshotBrowserModel.historyLimit])
    }

    func testPreparedTargetedScanOpensExactSnapshotWithoutLoadingLatestHome() async {
        let reviews = BrowserReviewStub()
        let history = BrowserHistoryStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, history: history)
        let targetedScanID = "scan:targeted:focused"

        browser.prepareExactScanReview(scanID: targetedScanID)
        await browser.present(id: UUID())

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, targetedScanID)
        XCTAssertFalse(browser.isLatestSnapshot)
        XCTAssertNil(browser.selectedHistoricalScan)
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.acquire(scanID: targetedScanID)))
        XCTAssertFalse(calls.contains(.acquireLatest))
    }

    func testPreparedTargetedFindingReviewOpensExactCandidatesWithoutLoadingLatestHome() async {
        let reviews = BrowserReviewStub(mode: .candidatesAvailable)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        let targetedScanID = "scan:targeted:caches"

        browser.prepareExactCandidateReview(scanID: targetedScanID)
        await browser.present(id: UUID())

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, targetedScanID)
        XCTAssertEqual(browser.contentMode, .candidates)
        XCTAssertEqual(browser.candidatePage?.scanID, targetedScanID)
        XCTAssertEqual(browser.candidatePage?.candidates.count, 2)
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.acquire(scanID: targetedScanID)))
        XCTAssertTrue(calls.contains(.candidateSummaries(
            scanID: targetedScanID,
            cursor: 0,
            limit: ExplorerCandidateDetailAdapter.maximumPageLimit
        )))
        XCTAssertFalse(calls.contains(.acquireLatest))
    }

    func testPreparedLargeFilesReviewOpensExactModeWithoutLoadingLatestHome() async {
        let reviews = BrowserReviewStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        let scanID = "scan:home:large-files"

        browser.prepareExactLargeFilesReview(scanID: scanID)
        await browser.present(id: UUID())

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, scanID)
        XCTAssertEqual(browser.contentMode, .largeFiles)
        XCTAssertEqual(browser.largeFilesPage?.files.count, 2)
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.acquire(scanID: scanID)))
        XCTAssertTrue(calls.contains(.largeFiles(
            scanID: scanID,
            minimumLogicalBytes: ExplorerSnapshotLargeFileThreshold.gibibyte1.rawValue,
            modifiedBefore: nil,
            maxResults: ExplorerSnapshotBrowserModel.largeFileResultLimit
        )))
        XCTAssertFalse(calls.contains(.acquireLatest))
    }

    func testPreparedCoverageReviewOpensExactModeWithoutLoadingLatestHome() async {
        let reviews = BrowserReviewStub()
        let coverage = BrowserCoverageStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, coverage: coverage)
        let scanID = "scan:home:coverage"

        browser.prepareExactCoverageReview(scanID: scanID)
        await browser.present(id: UUID())

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, scanID)
        XCTAssertEqual(browser.contentMode, .coverage)
        XCTAssertEqual(browser.coverageDetails?.scanID, scanID)
        let requestedScanIDs = await coverage.requestedScanIDs()
        XCTAssertEqual(requestedScanIDs, [scanID])
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.acquire(scanID: scanID)))
        XCTAssertFalse(calls.contains(.acquireLatest))
    }

    func testHistoryFailureLeavesConfirmedSnapshotReady() async {
        let reviews = BrowserReviewStub()
        let history = BrowserHistoryStub(fails: true)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, history: history)

        await browser.present(id: UUID())

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertEqual(browser.historyFailure, .unavailable)
        XCTAssertTrue(browser.historyScans.isEmpty)
    }

    func testOverLimitHistoryFailsClosedWithoutReplacingLatestSnapshot() async {
        let reviews = BrowserReviewStub()
        let history = BrowserHistoryStub(overLimit: true)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, history: history)

        await browser.present(id: UUID())

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertEqual(browser.historyFailure, .invalidResponse)
        XCTAssertTrue(browser.historyScans.isEmpty)
    }

    func testExactHistoricalSwitchPublishesThenReleasesPreviousReview() async throws {
        let reviews = BrowserReviewStub()
        let history = BrowserHistoryStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, history: history)
        await browser.present(id: UUID())
        let older = try XCTUnwrap(browser.historyScans.last)

        await browser.selectHistoricalScan(older)

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:older")
        XCTAssertFalse(browser.isLatestSnapshot)
        XCTAssertEqual(browser.selectedHistoricalScan?.scanID, "scan:older")
        XCTAssertFalse(browser.isSwitchingSnapshot)
        let calls = await reviews.recordedCalls()
        XCTAssertEqual(
            Array(calls.suffix(5)),
            [
                .acquire(scanID: "scan:older"),
                .root(scanID: "scan:older"),
                .children(
                    scanID: "scan:older",
                    parentID: 0,
                    sort: .logicalBytesDescending,
                    offset: 0,
                    limit: 100
                ),
                .treemap(scanID: "scan:older", parentID: 0, maxCells: 48),
                .release(scanID: "scan:latest"),
            ]
        )
    }

    func testUnavailableHistoricalSelectionKeepsCurrentSnapshotAndMarksOnlyTarget() async throws {
        let reviews = BrowserReviewStub(mode: .exactUnavailable)
        let history = BrowserHistoryStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, history: history)
        await browser.present(id: UUID())
        let confirmedIDs = browser.nodes.map(\.id)
        let older = try XCTUnwrap(browser.historyScans.last)

        await browser.selectHistoricalScan(older)

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertEqual(browser.nodes.map(\.id), confirmedIDs)
        XCTAssertEqual(browser.operationFailure, .unavailable)
        XCTAssertEqual(browser.unavailableHistoricalScanIDs, ["scan:older"])
        let released = await reviews.releasedScanIDs()
        XCTAssertTrue(released.isEmpty)

        await browser.reloadHistory()
        XCTAssertTrue(browser.unavailableHistoricalScanIDs.isEmpty)
    }

    func testCloseDuringExactSelectionReleasesLateTargetAndCurrentReview() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedExactAcquire)
        let history = BrowserHistoryStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, history: history)
        await browser.present(id: UUID())
        let older = try XCTUnwrap(browser.historyScans.last)
        let switching = Task { await browser.selectHistoricalScan(older) }
        try await eventually { await reviews.hasSuspendedAcquisition() }

        await browser.close()
        await reviews.resumeAcquisition()
        await switching.value

        XCTAssertEqual(browser.phase, .idle)
        XCTAssertNil(browser.scanID)
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest", "scan:older"])
    }

    func testCancelledExactSelectionRestoresInteractionAndReleasesTarget() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedExactAcquire)
        let history = BrowserHistoryStub()
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews, history: history)
        await browser.present(id: UUID())
        let older = try XCTUnwrap(browser.historyScans.last)
        let confirmedIDs = browser.nodes.map(\.id)
        let switching = Task { await browser.selectHistoricalScan(older) }
        try await eventually { await reviews.hasSuspendedAcquisition() }

        switching.cancel()
        await reviews.resumeAcquisition()
        await switching.value

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertEqual(browser.nodes.map(\.id), confirmedIDs)
        XCTAssertFalse(browser.isSwitchingSnapshot)
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:older"])
    }

    func testSubtreeRefreshAtomicallyInstallsExactSnapshotThenReleasesSource() async {
        let reviews = BrowserReviewStub()
        let driver = BrowserSubtreeScanDriver(
            outcome: .succeeded(browserScanSummary(scanID: "scan:refreshed"))
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            subtreeScans: BrowserSubtreeScanServiceStub(),
            scanDriver: driver
        )
        await browser.reloadLatest()
        let confirmedNodes = browser.nodes

        await browser.refreshCurrentSubtree()

        XCTAssertEqual(driver.requests, [
            .init(sourceScanID: "scan:latest", nodeID: 0, displayName: "/Users/example"),
        ])
        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:refreshed")
        XCTAssertFalse(browser.isLatestSnapshot)
        XCTAssertEqual(browser.breadcrumbs.map(\.id), [0])
        XCTAssertEqual(browser.nodes, confirmedNodes)
        XCTAssertNotNil(browser.treemap)
        XCTAssertFalse(browser.isSubtreeRefreshRunning)
        XCTAssertTrue(browser.subtreeRefreshNotice?.message.contains("standalone snapshot root") == true)
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.acquire(scanID: "scan:refreshed")))
        XCTAssertTrue(calls.contains(.root(scanID: "scan:refreshed")))
        XCTAssertTrue(calls.contains(.release(scanID: "scan:latest")))
    }

    func testSubtreeFailureAndCancellationPreserveConfirmedSnapshot() async {
        for outcome in [
            AppScanRunOutcome.failed(.rootUnavailable),
            .cancelled,
        ] {
            let reviews = BrowserReviewStub()
            let driver = BrowserSubtreeScanDriver(outcome: outcome)
            let browser = ExplorerSnapshotBrowserModel(
                reviews: reviews,
                subtreeScans: BrowserSubtreeScanServiceStub(),
                scanDriver: driver
            )
            await browser.reloadLatest()
            let confirmedNodes = browser.nodes

            await browser.refreshCurrentSubtree()

            XCTAssertEqual(browser.phase, .ready)
            XCTAssertEqual(browser.scanID, "scan:latest")
            XCTAssertEqual(browser.nodes, confirmedNodes)
            XCTAssertNotNil(browser.subtreeRefreshNotice)
            let released = await reviews.releasedScanIDs()
            XCTAssertTrue(released.isEmpty)
        }
    }

    func testSubtreeResultDoesNotReplaceViewAfterDirectoryNavigation() async throws {
        let reviews = BrowserReviewStub()
        let driver = BrowserSubtreeScanDriver(
            outcome: .succeeded(browserScanSummary(scanID: "scan:refreshed")),
            suspends: true
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            subtreeScans: BrowserSubtreeScanServiceStub(),
            scanDriver: driver
        )
        await browser.reloadLatest()
        let directory = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .directory }))

        let refresh = Task { await browser.refreshCurrentSubtree() }
        try await eventually { await driver.hasSuspendedRequest() }
        XCTAssertTrue(browser.isSubtreeRefreshRunning)
        await browser.openDirectory(directory)
        driver.resume()
        await refresh.value

        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertEqual(browser.breadcrumbs.map(\.id), [0, directory.id])
        XCTAssertTrue(browser.subtreeRefreshNotice?.message.contains("Recent Scans") == true)
        let calls = await reviews.recordedCalls()
        XCTAssertFalse(calls.contains(.acquire(scanID: "scan:refreshed")))
    }

    func testUnvalidatedSubtreeReplacementKeepsSourceLeaseAndContent() async {
        let reviews = BrowserReviewStub(mode: .exactUnavailable)
        let driver = BrowserSubtreeScanDriver(
            outcome: .succeeded(browserScanSummary(scanID: "scan:refreshed"))
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            subtreeScans: BrowserSubtreeScanServiceStub(),
            scanDriver: driver
        )
        await browser.reloadLatest()
        let confirmedNodes = browser.nodes

        await browser.refreshCurrentSubtree()

        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertEqual(browser.nodes, confirmedNodes)
        XCTAssertTrue(browser.subtreeRefreshNotice?.isFailure == true)
        let released = await reviews.releasedScanIDs()
        XCTAssertTrue(released.isEmpty)
    }

    func testCloseSuppressesSuspendedSubtreeResult() async throws {
        let reviews = BrowserReviewStub()
        let driver = BrowserSubtreeScanDriver(
            outcome: .succeeded(browserScanSummary(scanID: "scan:refreshed")),
            suspends: true
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            subtreeScans: BrowserSubtreeScanServiceStub(),
            scanDriver: driver
        )
        await browser.reloadLatest()
        let refresh = Task { await browser.refreshCurrentSubtree() }
        try await eventually { await driver.hasSuspendedRequest() }

        await browser.close()
        driver.resume()
        await refresh.value

        XCTAssertEqual(browser.phase, .idle)
        XCTAssertNil(browser.scanID)
        XCTAssertFalse(browser.isSubtreeRefreshRunning)
        let calls = await reviews.recordedCalls()
        XCTAssertFalse(calls.contains(.acquire(scanID: "scan:refreshed")))
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest"])
    }

    func testTerminalQuiescenceFencesAdmissionsCancelsAndJoinsSubtreeRefresh() async throws {
        let reviews = BrowserReviewStub()
        let driver = BrowserSubtreeScanDriver(
            outcome: .cancelled,
            suspends: true
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            subtreeScans: BrowserSubtreeScanServiceStub(),
            scanDriver: driver
        )
        await browser.reloadLatest()
        let refresh = Task { await browser.refreshCurrentSubtree() }
        try await eventually { await driver.hasSuspendedRequest() }

        let retained = browser.beginTerminalRuntimeQuiescence()
        let joiningCaller = Task { await browser.quiesceForTerminalRuntime() }
        joiningCaller.cancel()
        await browser.reloadLatest()
        await retained.value
        await joiningCaller.value
        await refresh.value

        XCTAssertEqual(driver.cancellationCount, 1)
        XCTAssertEqual(driver.requests.count, 1)
        XCTAssertEqual(browser.phase, .idle)
        XCTAssertNil(browser.scanID)
        XCTAssertFalse(browser.isSubtreeRefreshRunning)
        let calls = await reviews.recordedCalls()
        let releasedScanIDs = await reviews.releasedScanIDs()
        XCTAssertEqual(calls.filter { $0 == .acquireLatest }.count, 1)
        XCTAssertEqual(releasedScanIDs, ["scan:latest"])
    }

    func testTerminalQuiescenceJoinsHistoryAndCoverageBeforeLeaseRelease() async throws {
        let reviews = BrowserReviewStub()
        let history = BrowserHistoryStub(suspends: true)
        let coverage = BrowserCoverageStub(suspends: true)
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            history: history,
            coverage: coverage
        )
        await browser.reloadLatest()
        let historyLoad = Task { await browser.reloadHistory() }
        let coverageLoad = Task { await browser.selectContentMode(.coverage) }
        try await eventually {
            let historySuspended = await history.hasSuspendedRequest()
            let coverageSuspended = await coverage.hasSuspendedRequest()
            return historySuspended && coverageSuspended
        }

        let drain = browser.beginTerminalRuntimeQuiescence()
        await Task.yield()
        await browser.close()
        var released = await reviews.releasedScanIDs()
        XCTAssertTrue(released.isEmpty)

        await history.resumeRequest()
        await coverage.resumeRequest()
        await historyLoad.value
        await coverageLoad.value
        await drain.value

        released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest"])
        XCTAssertEqual(browser.phase, .idle)
    }

    func testTerminalQuiescenceJoinsLiveResolutionBeforeLeaseRelease() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedLiveAction)
        let presenter = BrowserLiveActionPresenterSpy()
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            liveActions: presenter
        )
        await browser.reloadLatest()
        let file = try XCTUnwrap(browser.nodes.first(where: { $0.kind == .file }))
        browser.selectTableNode(file.id)
        let reveal = Task { await browser.revealLiveItem() }
        try await eventually { await reviews.hasSuspendedLiveAction() }

        let drain = browser.beginTerminalRuntimeQuiescence()
        await Task.yield()
        var released = await reviews.releasedScanIDs()
        XCTAssertTrue(released.isEmpty)

        await reviews.resumeLiveAction()
        await reveal.value
        await drain.value

        released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest"])
        XCTAssertTrue(presenter.revealed.isEmpty)
    }

    func testCloseDuringSubtreeSnapshotAcquisitionReleasesBothReviews() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedExactAcquire)
        let driver = BrowserSubtreeScanDriver(
            outcome: .succeeded(browserScanSummary(scanID: "scan:refreshed"))
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            subtreeScans: BrowserSubtreeScanServiceStub(),
            scanDriver: driver
        )
        await browser.reloadLatest()
        let refresh = Task { await browser.refreshCurrentSubtree() }
        try await eventually { await reviews.hasSuspendedAcquisition() }

        await browser.close()
        await reviews.resumeAcquisition()
        await refresh.value

        XCTAssertEqual(browser.phase, .idle)
        XCTAssertNil(browser.scanID)
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest", "scan:refreshed"])
    }

    func testSubtreeTreemapMismatchRejectsReplacementAndKeepsSource() async {
        let reviews = BrowserReviewStub(mode: .refreshedTreemapMismatch)
        let driver = BrowserSubtreeScanDriver(
            outcome: .succeeded(browserScanSummary(scanID: "scan:refreshed"))
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            subtreeScans: BrowserSubtreeScanServiceStub(),
            scanDriver: driver
        )
        await browser.reloadLatest()
        let confirmedNodes = browser.nodes

        await browser.refreshCurrentSubtree()

        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertEqual(browser.nodes, confirmedNodes)
        XCTAssertTrue(browser.subtreeRefreshNotice?.isFailure == true)
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:refreshed"])
    }

    func testCandidateDetailLoadsAndPagesExactHistoricalPathsAndEvidence() async throws {
        let reviews = BrowserReviewStub(mode: .candidatesAvailable)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)

        await browser.selectCandidate(candidate.candidateID)

        XCTAssertEqual(browser.selectedCandidateID, candidate.candidateID)
        XCTAssertEqual(browser.selectedCandidate, candidate)
        XCTAssertEqual(browser.candidatePathPage?.cursor, 0)
        XCTAssertEqual(browser.candidatePathPage?.paths.count, 64)
        XCTAssertEqual(browser.candidatePathPage?.nextCursor, 64)
        XCTAssertEqual(browser.candidateEvidencePage?.cursor, 0)
        XCTAssertEqual(browser.candidateEvidencePage?.evidence.count, 64)
        XCTAssertEqual(browser.candidateEvidencePage?.nextCursor, 64)
        XCTAssertNil(browser.candidateDetailFailure)
        XCTAssertFalse(browser.isCandidateDetailLoading)

        await browser.showNextCandidatePathPage()
        await browser.showNextCandidateEvidencePage()

        XCTAssertEqual(browser.candidatePathPage?.cursor, 64)
        XCTAssertEqual(browser.candidatePathPage?.paths.count, 1)
        XCTAssertNil(browser.candidatePathPage?.nextCursor)
        XCTAssertTrue(browser.hasPreviousCandidatePathPage)
        XCTAssertEqual(browser.candidateEvidencePage?.cursor, 64)
        XCTAssertEqual(browser.candidateEvidencePage?.evidence.map(\.ordinal), [64])
        XCTAssertNil(browser.candidateEvidencePage?.nextCursor)
        XCTAssertTrue(browser.hasPreviousCandidateEvidencePage)

        await browser.showPreviousCandidatePathPage()
        await browser.showPreviousCandidateEvidencePage()

        XCTAssertEqual(browser.candidatePathPage?.cursor, 0)
        XCTAssertEqual(browser.candidateEvidencePage?.cursor, 0)
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.candidatePaths(
            scanID: "scan:latest",
            candidateID: candidate.candidateID,
            cursor: 64,
            limit: ExplorerCandidateDetailAdapter.maximumPageLimit
        )))
        XCTAssertTrue(calls.contains(.candidateEvidence(
            scanID: "scan:latest",
            candidateID: candidate.candidateID,
            cursor: 64,
            limit: ExplorerCandidateDetailAdapter.maximumPageLimit
        )))
    }

    func testCandidateSummaryPagesMoveThroughBoundedFindingsAndClearSelection() async throws {
        let reviews = BrowserReviewStub(mode: .candidateSummaryPages)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)

        XCTAssertEqual(browser.candidatePage?.cursor, 0)
        XCTAssertEqual(browser.candidatePage?.candidates.count, 64)
        XCTAssertEqual(browser.candidatePage?.nextCursor, 64)
        XCTAssertFalse(browser.hasPreviousCandidatePage)
        XCTAssertTrue(browser.hasNextCandidatePage)
        let selected = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(selected.candidateID)
        XCTAssertEqual(browser.selectedCandidateID, selected.candidateID)

        await browser.showNextCandidatePage()

        XCTAssertEqual(browser.candidatePage?.cursor, 64)
        XCTAssertEqual(browser.candidatePage?.candidates.count, 1)
        XCTAssertNil(browser.candidatePage?.nextCursor)
        XCTAssertTrue(browser.hasPreviousCandidatePage)
        XCTAssertFalse(browser.hasNextCandidatePage)
        XCTAssertNil(browser.selectedCandidateID)

        await browser.showPreviousCandidatePage()

        XCTAssertEqual(browser.candidatePage?.cursor, 0)
        XCTAssertEqual(browser.candidatePage?.candidates.count, 64)
        let calls = await reviews.recordedCalls()
        XCTAssertTrue(calls.contains(.candidateSummaries(
            scanID: "scan:latest",
            cursor: 64,
            limit: ExplorerCandidateDetailAdapter.maximumPageLimit
        )))
    }

    func testLateCandidateDetailIsDiscardedAfterLeavingCandidates() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedCandidateDetail)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)

        let loading = Task { await browser.selectCandidate(candidate.candidateID) }
        try await eventually { await reviews.hasSuspendedCandidateDetail() }
        XCTAssertTrue(browser.isCandidateDetailLoading)

        await browser.selectContentMode(.browse)
        await reviews.resumeCandidateDetail()
        await loading.value

        XCTAssertEqual(browser.contentMode, .browse)
        XCTAssertNil(browser.selectedCandidateID)
        XCTAssertNil(browser.candidatePathPage)
        XCTAssertNil(browser.candidateEvidencePage)
        XCTAssertNil(browser.candidateDetailFailure)
        XCTAssertFalse(browser.isCandidateDetailLoading)
    }

    func testLateCandidateDetailCannotReplaceAChangedSelection() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedCandidateDetail)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidates = try XCTUnwrap(browser.candidatePage?.candidates)
        XCTAssertGreaterThanOrEqual(candidates.count, 2)

        let firstLoad = Task { await browser.selectCandidate(candidates[0].candidateID) }
        try await eventually { await reviews.hasSuspendedCandidateDetail() }

        await browser.selectCandidate(candidates[1].candidateID)
        await reviews.resumeCandidateDetail()
        await firstLoad.value

        XCTAssertEqual(browser.selectedCandidateID, candidates[1].candidateID)
        XCTAssertEqual(browser.candidatePathPage?.candidate.candidateID, candidates[1].candidateID)
        XCTAssertEqual(
            browser.candidateEvidencePage?.candidate.candidateID,
            candidates[1].candidateID
        )
        XCTAssertFalse(browser.isCandidateDetailLoading)
    }

    func testCancelledCurrentCandidateDetailStopsLoadingWithoutPublishing() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedCandidateDetail)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)

        let loading = Task { await browser.selectCandidate(candidate.candidateID) }
        try await eventually { await reviews.hasSuspendedCandidateDetail() }
        loading.cancel()
        await reviews.resumeCandidateDetail()
        await loading.value

        XCTAssertEqual(browser.selectedCandidateID, candidate.candidateID)
        XCTAssertNil(browser.candidatePathPage)
        XCTAssertNil(browser.candidateEvidencePage)
        XCTAssertNil(browser.candidateDetailFailure)
        XCTAssertFalse(browser.isCandidateDetailLoading)
    }

    func testCandidateDetailExpiryInvalidatesAndReleasesWholeReview() async throws {
        let reviews = BrowserReviewStub(mode: .candidateDetailExpired)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)

        await browser.selectCandidate(candidate.candidateID)

        XCTAssertEqual(browser.phase, .failed(.expired))
        XCTAssertNil(browser.scanID)
        XCTAssertNil(browser.selectedCandidateID)
        let released = await reviews.releasedScanIDs()
        XCTAssertEqual(released, ["scan:latest"])
    }

    func testMalformedCandidateDetailStopsLoadingWithoutPublishingEitherPage() async throws {
        let reviews = BrowserReviewStub(mode: .candidateDetailMalformed)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)

        await browser.selectCandidate(candidate.candidateID)

        XCTAssertEqual(browser.selectedCandidateID, candidate.candidateID)
        XCTAssertNil(browser.candidatePathPage)
        XCTAssertNil(browser.candidateEvidencePage)
        XCTAssertEqual(browser.candidateDetailFailure, .invalidResponse)
        XCTAssertFalse(browser.isCandidateDetailLoading)
    }

    func testRustTargetPlanReviewPublishesExactObservationAndReleasesOnClose() async throws {
        let reviews = BrowserReviewStub(mode: .rustTargetPlanReviewAvailable)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)

        await browser.prepareSelectedRustTargetPlanReview()

        guard case let .ready(info) = browser.rustTargetPlanReviewState else {
            return XCTFail("Expected an exact Rust-target plan observation")
        }
        XCTAssertEqual(info.candidateID, candidate.candidateID)
        XCTAssertEqual(info.target.display, "/Users/example/project/target")
        XCTAssertEqual(browser.phase, .ready)

        await browser.closeRustTargetPlanReview()

        let releasedPlanReviewCount = await reviews.releasedPlanReviewCount()
        let releasedScanIDs = await reviews.releasedScanIDs()
        XCTAssertEqual(browser.rustTargetPlanReviewState, .idle)
        XCTAssertEqual(releasedPlanReviewCount, 1)
        XCTAssertTrue(releasedScanIDs.isEmpty)
    }

    func testRustTargetPlanReviewRejectsAndReleasesMismatchedRecency() async throws {
        let reviews = BrowserReviewStub(mode: .rustTargetPlanReviewMismatchedRecency)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)

        await browser.prepareSelectedRustTargetPlanReview()

        let released = await reviews.releasedPlanReviewCount()
        let starts = await reviews.cleanupStartCount()
        XCTAssertEqual(browser.rustTargetPlanReviewState, .failed(.invalidResponse))
        XCTAssertEqual(released, 1)
        XCTAssertEqual(starts, 0)
    }

    func testReadyRustTargetPlanReviewRevalidatesAndFailsClosedOnDrift() async throws {
        let reviews = BrowserReviewStub(mode: .rustTargetPlanReviewChangesOnRefresh)
        let clock = SuspendedRustTargetPlanReviewClock(now: Date())
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetPlanReviewClock: clock
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)

        await browser.prepareSelectedRustTargetPlanReview()

        guard case .ready = browser.rustTargetPlanReviewState else {
            return XCTFail("Expected a prepared plan before revalidation")
        }
        try await eventually { await clock.hasSuspendedSleep() }
        await clock.resumeSleep()
        try await eventually {
            await MainActor.run {
                browser.rustTargetPlanReviewState == .failed(.changedDuringReview)
            }
        }

        let releasedPlanReviewCount = await reviews.releasedPlanReviewCount()
        let releasedScanIDs = await reviews.releasedScanIDs()
        XCTAssertEqual(releasedPlanReviewCount, 1)
        XCTAssertTrue(releasedScanIDs.isEmpty)
        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
    }

    func testExpiredRustTargetPlanReviewKeepsRenewableParentSnapshot() async throws {
        let reviews = BrowserReviewStub(mode: .rustTargetPlanReviewExpired)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)

        await browser.prepareSelectedRustTargetPlanReview()

        let releasedScanIDs = await reviews.releasedScanIDs()
        XCTAssertEqual(browser.rustTargetPlanReviewState, .expired)
        XCTAssertEqual(browser.phase, .ready)
        XCTAssertEqual(browser.scanID, "scan:latest")
        XCTAssertTrue(releasedScanIDs.isEmpty)
    }

    func testLateRustTargetPlanReviewIsReleasedAfterCandidateChanges() async throws {
        let reviews = BrowserReviewStub(mode: .suspendedRustTargetPlanReview)
        let browser = ExplorerSnapshotBrowserModel(reviews: reviews)
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidates = try XCTUnwrap(browser.candidatePage?.candidates)
        let preparing = Task {
            await browser.selectCandidate(candidates[0].candidateID)
            await browser.prepareSelectedRustTargetPlanReview()
        }
        try await eventually { await reviews.hasSuspendedPlanReview() }

        await browser.selectCandidate(candidates[1].candidateID)
        await reviews.resumePlanReview()
        await preparing.value

        let releasedPlanReviewCount = await reviews.releasedPlanReviewCount()
        XCTAssertEqual(browser.selectedCandidateID, candidates[1].candidateID)
        XCTAssertEqual(browser.rustTargetPlanReviewState, .idle)
        XCTAssertEqual(releasedPlanReviewCount, 1)
    }

    func testExactCleanupConfirmationIsConsumeOnceAndPublishesPathFreeResult() async throws {
        let cleanup = BrowserCleanupTaskStub(
            polls: [
                ExplorerRustTargetCleanupPoll(
                    phase: .succeeded,
                    cancellationRequested: false,
                    revision: 1,
                    failure: nil,
                    result: ExplorerRustTargetCleanupResult(
                        sessionID:
                        "cleanup:rust-target:0123456789abcdef0123456789abcdef",
                        status: .completed,
                        removedEntries: 4,
                        removedLogicalBytes: 8_192,
                        verifiedCapacityDeltaBytes: 7_000
                    )
                ),
            ]
        )
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            cleanupTask: cleanup
        )
        let observer = DryRunTerminalObserverSpy()
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetCleanupTerminalObserver: {
                await observer.observe()
            }
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        let confirmation = try XCTUnwrap(
            browser.makeRustTargetCleanupConfirmation()
        )
        let stale = ExplorerRustTargetCleanupConfirmation(
            generation: confirmation.generation &+ 1,
            reviewHandleID: confirmation.reviewHandleID,
            info: confirmation.info
        )

        await browser.startConfirmedRustTargetCleanup(stale)
        let startsAfterStaleConfirmation = await reviews.cleanupStartCount()
        XCTAssertEqual(startsAfterStaleConfirmation, 0)
        await browser.startConfirmedRustTargetCleanup(confirmation)

        guard case let .observing(_, poll) = browser.rustTargetCleanupState else {
            return XCTFail("Expected a terminal path-free cleanup observation")
        }
        let cleanupStartCount = await reviews.cleanupStartCount()
        let releaseCount = await reviews.releasedPlanReviewCount()
        let observations = await observer.count()
        XCTAssertEqual(cleanupStartCount, 1)
        XCTAssertEqual(releaseCount, 0)
        XCTAssertEqual(observations, 1)
        XCTAssertEqual(poll.phase, .succeeded)
        XCTAssertEqual(poll.result?.removedEntries, 4)
        XCTAssertEqual(poll.result?.removedLogicalBytes, 8_192)
        XCTAssertFalse(browser.canPrepareRustTargetPlanReview)

        await browser.startConfirmedRustTargetCleanup(confirmation)
        let startsAfterRepeatedConfirmation = await reviews.cleanupStartCount()
        let observationsAfterRepeatedConfirmation = await observer.count()
        XCTAssertEqual(startsAfterRepeatedConfirmation, 1)
        XCTAssertEqual(observationsAfterRepeatedConfirmation, 1)
    }

    func testCancelledCleanupRetainsItsPathFreeCorrelatedResult() async throws {
        let result = ExplorerRustTargetCleanupResult(
            sessionID: "cleanup:rust-target:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            status: .cancelled,
            removedEntries: 2,
            removedLogicalBytes: 4_096,
            verifiedCapacityDeltaBytes: 3_000
        )
        let cleanup = BrowserCleanupTaskStub(
            polls: [
                ExplorerRustTargetCleanupPoll(
                    phase: .cancelled,
                    cancellationRequested: true,
                    revision: 1,
                    failure: nil,
                    result: result
                ),
            ]
        )
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            cleanupTask: cleanup
        )
        let observer = DryRunTerminalObserverSpy()
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetCleanupTerminalObserver: {
                await observer.observe()
            }
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        let confirmation = try XCTUnwrap(
            browser.makeRustTargetCleanupConfirmation()
        )

        await browser.startConfirmedRustTargetCleanup(confirmation)

        guard case let .observing(_, poll) = browser.rustTargetCleanupState else {
            return XCTFail("Expected a terminal cancelled cleanup observation")
        }
        XCTAssertEqual(poll.phase, .cancelled)
        XCTAssertEqual(poll.result, result)
        XCTAssertEqual(browser.rustTargetCleanupState.correlatedResult, result)
        let observations = await observer.count()
        XCTAssertEqual(observations, 1)
    }

    func testTerminalCleanupCannotDismissWhileHistoryFinalizes() async throws {
        let cleanup = BrowserCleanupTaskStub(
            polls: [
                ExplorerRustTargetCleanupPoll(
                    phase: .succeeded,
                    cancellationRequested: false,
                    revision: 1,
                    failure: nil,
                    result: ExplorerRustTargetCleanupResult(
                        sessionID:
                        "cleanup:rust-target:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        status: .completed,
                        removedEntries: 1,
                        removedLogicalBytes: 1024,
                        verifiedCapacityDeltaBytes: nil
                    )
                ),
            ]
        )
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            cleanupTask: cleanup
        )
        let observer = SuspendedCleanupTerminalObserver()
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetCleanupTerminalObserver: {
                await observer.observe()
            }
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        let confirmation = try XCTUnwrap(
            browser.makeRustTargetCleanupConfirmation()
        )
        let execution = Task {
            await browser.startConfirmedRustTargetCleanup(confirmation)
        }
        try await eventually { await observer.isSuspended() }

        XCTAssertTrue(browser.rustTargetCleanupHistoryFinalizationInProgress)
        await browser.dismissRustTargetCleanupResult()
        XCTAssertNotEqual(browser.rustTargetCleanupState, .idle)

        await observer.resume()
        await execution.value
        XCTAssertFalse(browser.rustTargetCleanupHistoryFinalizationInProgress)
        await browser.dismissRustTargetCleanupResult()
        XCTAssertEqual(browser.rustTargetCleanupState, .idle)
    }

    func testRustTargetDryRunConsumesPreviewAndRefreshesHistoryExactlyOnce() async throws {
        let dryRun = BrowserDryRunTaskStub(
            polls: [
                ExplorerRustTargetDryRunPoll(
                    phase: .succeeded,
                    cancellationRequested: false,
                    revision: 1,
                    failure: nil,
                    result: ExplorerRustTargetDryRunResult(
                        sessionID:
                        "cleanup:rust-target-dry-run:0123456789abcdef0123456789abcdef",
                        status: .dryRun
                    )
                ),
            ]
        )
        let observer = DryRunTerminalObserverSpy()
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            dryRunTask: dryRun
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetDryRunTerminalObserver: {
                await observer.observe()
            }
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()

        await browser.startRustTargetDryRun()

        guard case let .observing(_, poll) = browser.rustTargetDryRunState else {
            return XCTFail("Expected a terminal path-free dry-run observation")
        }
        XCTAssertEqual(poll.phase, .succeeded)
        XCTAssertEqual(poll.result?.status, .dryRun)
        let starts = await reviews.dryRunStartCount()
        let observations = await observer.count()
        XCTAssertEqual(starts, 1)
        XCTAssertEqual(observations, 1)
        XCTAssertEqual(browser.rustTargetPlanReviewState, .idle)
        XCTAssertFalse(browser.canPrepareRustTargetPlanReview)

        await browser.startRustTargetDryRun()
        let repeatedStarts = await reviews.dryRunStartCount()
        let repeatedObservations = await observer.count()
        XCTAssertEqual(repeatedStarts, 1)
        XCTAssertEqual(repeatedObservations, 1)
    }

    func testClosingExplorerKeepsDryRunObservedAndRefreshesHistoryOnce() async throws {
        let dryRun = BrowserDryRunTaskStub(
            polls: [
                ExplorerRustTargetDryRunPoll(
                    phase: .running,
                    cancellationRequested: false,
                    revision: 1,
                    failure: nil,
                    result: nil
                ),
                ExplorerRustTargetDryRunPoll(
                    phase: .succeeded,
                    cancellationRequested: false,
                    revision: 2,
                    failure: nil,
                    result: ExplorerRustTargetDryRunResult(
                        sessionID:
                        "cleanup:rust-target-dry-run:fedcba9876543210fedcba9876543210",
                        status: .dryRun
                    )
                ),
            ]
        )
        let clock = SuspendedRustTargetDryRunPollingClock()
        let observer = DryRunTerminalObserverSpy()
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            dryRunTask: dryRun
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetDryRunPollingClock: clock,
            rustTargetDryRunTerminalObserver: {
                await observer.observe()
            }
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        let execution = Task {
            await browser.startRustTargetDryRun()
        }
        try await eventually { await clock.hasSuspendedSleep() }

        await browser.close()

        let cancellations = await dryRun.cancellationCount()
        XCTAssertEqual(cancellations, 0)
        await clock.resumeSleep()
        await execution.value
        guard case let .observing(_, poll) = browser.rustTargetDryRunState else {
            return XCTFail("Expected dry-run observation to survive Explorer close")
        }
        let observations = await observer.count()
        XCTAssertEqual(poll.phase, .succeeded)
        XCTAssertEqual(observations, 1)
    }

    func testDryRunCancellationBeforeTaskAttachmentIsForwardedOnce() async throws {
        let dryRun = BrowserDryRunTaskStub(
            polls: [
                ExplorerRustTargetDryRunPoll(
                    phase: .cancelled,
                    cancellationRequested: true,
                    revision: 1,
                    failure: nil,
                    result: nil
                ),
            ]
        )
        let observer = DryRunTerminalObserverSpy()
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            dryRunTask: dryRun,
            suspendDryRunStart: true
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetDryRunTerminalObserver: {
                await observer.observe()
            }
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        let execution = Task {
            await browser.startRustTargetDryRun()
        }
        try await eventually { await reviews.hasSuspendedDryRunStart() }

        await browser.cancelRustTargetDryRun()
        await reviews.resumeDryRunStart()
        await execution.value

        let cancellations = await dryRun.cancellationCount()
        let observations = await observer.count()
        XCTAssertEqual(cancellations, 1)
        XCTAssertEqual(observations, 1)
        guard case let .observing(_, poll) = browser.rustTargetDryRunState else {
            return XCTFail("Expected cancellation terminal observation")
        }
        XCTAssertEqual(poll.phase, .cancelled)
    }

    func testClosingExplorerDoesNotCancelConfirmedCleanup() async throws {
        let cleanup = BrowserCleanupTaskStub(
            polls: [
                ExplorerRustTargetCleanupPoll(
                    phase: .running,
                    cancellationRequested: false,
                    revision: 1,
                    failure: nil,
                    result: nil
                ),
                ExplorerRustTargetCleanupPoll(
                    phase: .succeeded,
                    cancellationRequested: false,
                    revision: 2,
                    failure: nil,
                    result: ExplorerRustTargetCleanupResult(
                        sessionID:
                        "cleanup:rust-target:fedcba9876543210fedcba9876543210",
                        status: .completed,
                        removedEntries: 1,
                        removedLogicalBytes: 10,
                        verifiedCapacityDeltaBytes: nil
                    )
                ),
            ]
        )
        let clock = SuspendedRustTargetCleanupPollingClock()
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            cleanupTask: cleanup
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetCleanupPollingClock: clock
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        let confirmation = try XCTUnwrap(
            browser.makeRustTargetCleanupConfirmation()
        )
        let execution = Task {
            await browser.startConfirmedRustTargetCleanup(confirmation)
        }
        try await eventually { await clock.hasSuspendedSleep() }

        await browser.close()

        let cancellationCount = await cleanup.cancellationCount()
        XCTAssertEqual(cancellationCount, 0)
        await clock.resumeSleep()
        await execution.value
        guard case let .observing(_, poll) = browser.rustTargetCleanupState else {
            return XCTFail("Expected cleanup observation to survive Explorer close")
        }
        XCTAssertEqual(poll.phase, .succeeded)
    }

    func testAppShutdownCancelsAndWaitsForConfirmedCleanup() async throws {
        let cleanup = BrowserCleanupTaskStub(
            polls: [
                ExplorerRustTargetCleanupPoll(
                    phase: .running,
                    cancellationRequested: false,
                    revision: 1,
                    failure: nil,
                    result: nil
                ),
                ExplorerRustTargetCleanupPoll(
                    phase: .cancelled,
                    cancellationRequested: true,
                    revision: 2,
                    failure: nil,
                    result: nil
                ),
            ]
        )
        let clock = SuspendedRustTargetCleanupPollingClock()
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            cleanupTask: cleanup
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetCleanupPollingClock: clock
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        let confirmation = try XCTUnwrap(
            browser.makeRustTargetCleanupConfirmation()
        )
        let execution = Task {
            await browser.startConfirmedRustTargetCleanup(confirmation)
        }
        try await eventually { await clock.hasSuspendedSleep() }

        let shutdown = Task {
            await browser.shutdownRustTargetCleanup()
        }
        try await eventually { await cleanup.cancellationCount() == 1 }
        await clock.resumeSleep()
        await shutdown.value
        await execution.value

        guard case let .observing(_, poll) = browser.rustTargetCleanupState else {
            return XCTFail("Expected shutdown to retain the terminal observation")
        }
        XCTAssertEqual(poll.phase, .cancelled)
        let cancellationCount = await cleanup.cancellationCount()
        XCTAssertEqual(cancellationCount, 1)
    }

    func testTerminalQuiescenceRetainsAndJoinsConfirmedCleanupBeforeReviewRelease() async throws {
        let cleanup = BrowserCleanupTaskStub(
            polls: [
                ExplorerRustTargetCleanupPoll(
                    phase: .running,
                    cancellationRequested: false,
                    revision: 1,
                    failure: nil,
                    result: nil
                ),
                ExplorerRustTargetCleanupPoll(
                    phase: .cancelled,
                    cancellationRequested: true,
                    revision: 2,
                    failure: nil,
                    result: nil
                ),
            ]
        )
        let clock = SuspendedRustTargetCleanupPollingClock()
        let reviews = BrowserReviewStub(
            mode: .rustTargetPlanReviewAvailable,
            cleanupTask: cleanup
        )
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetCleanupPollingClock: clock
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        let confirmation = try XCTUnwrap(browser.makeRustTargetCleanupConfirmation())
        let execution = Task {
            await browser.startConfirmedRustTargetCleanup(confirmation)
        }
        try await eventually { await clock.hasSuspendedSleep() }

        let first = Task { await browser.quiesceForTerminalRuntime() }
        first.cancel()
        let second = Task { await browser.quiesceForTerminalRuntime() }
        try await eventually { await cleanup.cancellationCount() == 1 }
        let releasedBeforeCompletion = await reviews.releasedScanIDs()
        XCTAssertTrue(releasedBeforeCompletion.isEmpty)

        await clock.resumeSleep()
        await execution.value
        await first.value
        await second.value

        let cancellationCount = await cleanup.cancellationCount()
        let releasedScanIDs = await reviews.releasedScanIDs()
        XCTAssertEqual(cancellationCount, 1)
        XCTAssertEqual(releasedScanIDs, ["scan:latest"])
        XCTAssertEqual(browser.phase, .idle)
        XCTAssertNil(browser.scanID)
    }

    func testTerminalFenceCancelsAndJoinsPlanExpiryWithoutLateRefresh() async throws {
        let clock = SuspendedRustTargetPlanReviewClock(now: Date())
        let reviews = BrowserReviewStub(mode: .rustTargetPlanReviewAvailable)
        let browser = ExplorerSnapshotBrowserModel(
            reviews: reviews,
            rustTargetPlanReviewClock: clock
        )
        await browser.reloadLatest()
        await browser.selectContentMode(.candidates)
        let candidate = try XCTUnwrap(browser.candidatePage?.candidates.first)
        await browser.selectCandidate(candidate.candidateID)
        await browser.prepareSelectedRustTargetPlanReview()
        try await eventually { await clock.hasSuspendedSleep() }

        let drain = browser.beginTerminalRuntimeQuiescence()
        await Task.yield()
        let releasesBeforeResume = await reviews.releasedScanIDs()
        XCTAssertTrue(releasesBeforeResume.isEmpty)

        await clock.resumeSleep()
        await drain.value

        let refreshCount = await reviews.planReviewRefreshCount()
        let planReleaseCount = await reviews.releasedPlanReviewCount()
        XCTAssertEqual(refreshCount, 0)
        XCTAssertEqual(planReleaseCount, 1)
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

private struct BrowserSubtreeScanServiceStub: DuxSnapshotSubtreeScanServing {
    func startSubtreeScan(
        sourceScanID _: String,
        nodeID _: UInt64
    ) async throws -> HomeScanStartDisposition {
        throw HomeScanServiceError.internalState
    }
}

@MainActor
private final class BrowserSubtreeScanDriver: ExplorerSubtreeScanDriving {
    struct Request: Equatable {
        let sourceScanID: String
        let nodeID: UInt64
        let displayName: String
    }

    private(set) var requests: [Request] = []
    private let outcome: AppScanRunOutcome
    private let suspends: Bool
    private var continuation: CheckedContinuation<Void, Never>?
    private(set) var cancellationCount = 0

    init(outcome: AppScanRunOutcome, suspends: Bool = false) {
        self.outcome = outcome
        self.suspends = suspends
    }

    func startSubtreeScan(
        sourceScanID: String,
        nodeID: UInt64,
        displayName: String,
        using _: any DuxSnapshotSubtreeScanServing
    ) async -> AppScanRunOutcome {
        requests.append(Request(
            sourceScanID: sourceScanID,
            nodeID: nodeID,
            displayName: displayName
        ))
        if suspends {
            await withCheckedContinuation { continuation in
                self.continuation = continuation
            }
        }
        return outcome
    }

    func hasSuspendedRequest() -> Bool {
        continuation != nil
    }

    func cancelSubtreeScan() async {
        cancellationCount += 1
        resume()
    }

    func resume() {
        continuation?.resume()
        continuation = nil
    }
}

private actor BrowserReviewStub: DuxSnapshotReviewBrowsing {
    enum Mode: Equatable, Sendable {
        case available
        case noSnapshot
        case rootExpired
        case directoryBudget
        case suspendedAcquire
        case suspendedSort
        case suspendedNextPage
        case treemapBudget
        case treemapExpired
        case suspendedTreemap
        case pageExpiredWithSuspendedRelease
        case suspendedReloadRelease
        case exactUnavailable
        case suspendedExactAcquire
        case largeFilesExpired
        case suspendedLargeFiles
        case suspendedLiveAction
        case liveActionChanged
        case nonUnicodeLiveItem
        case iCloudEligible
        case iCloudBlocked
        case iCloudChanged
        case suspendedICloudProbe
        case treemapCategoryMismatch
        case offPageCategoryMismatch
        case refreshedTreemapMismatch
        case candidatesAvailable
        case candidateSummaryPages
        case suspendedCandidateDetail
        case candidateDetailExpired
        case candidateDetailMalformed
        case rustTargetPlanReviewAvailable
        case rustTargetPlanReviewChangesOnRefresh
        case rustTargetPlanReviewMismatchedRecency
        case rustTargetPlanReviewExpired
        case suspendedRustTargetPlanReview
        case diffAvailable
        case diffUnavailable
        case diffRootFailure
        case suspendedDiffPrepare
        case suspendedDiffSort
        case suspendedDiffNavigation
        case suspendedDiffPaging
    }

    enum Call: Equatable, Sendable {
        case acquire(scanID: String)
        case acquireLatest
        case root(scanID: String)
        case children(
            scanID: String,
            parentID: UInt64,
            sort: ExplorerSnapshotNodeSort,
            offset: UInt64,
            limit: UInt16
        )
        case treemap(scanID: String, parentID: UInt64, maxCells: UInt16)
        case candidateSummaries(scanID: String, cursor: UInt16, limit: UInt16)
        case candidatePaths(
            scanID: String,
            candidateID: String,
            cursor: UInt16,
            limit: UInt16
        )
        case candidateEvidence(
            scanID: String,
            candidateID: String,
            cursor: UInt16,
            limit: UInt16
        )
        case largeFiles(
            scanID: String,
            minimumLogicalBytes: UInt64,
            modifiedBefore: ExplorerSnapshotTimestamp?,
            maxResults: UInt16
        )
        case iCloudObservationSource(
            scanID: String,
            scopeNodeID: UInt64,
            maxResults: UInt16
        )
        case liveItem(
            scanID: String,
            nodeID: UInt64,
            purpose: ExplorerSnapshotLivePathPurpose
        )
        case iCloudProbe(scanID: String, nodeID: UInt64)
        case prepareDiff(scanID: String)
        case diffRoot(handleID: UUID)
        case diffChildren(
            handleID: UUID,
            parentID: UInt64,
            sort: ExplorerSnapshotDiffSort,
            offset: UInt64,
            limit: UInt16
        )
        case diffTreemap(handleID: UUID, parentID: UInt64, maxCells: UInt16)
        case releaseDiff(handleID: UUID)
        case release(scanID: String)
    }

    private let mode: Mode
    private let cleanupTask: BrowserCleanupTaskStub
    private let dryRunTask: BrowserDryRunTaskStub
    private var calls: [Call] = []
    private var released: [String] = []
    private var acquisitionContinuation: CheckedContinuation<Void, Never>?
    private var queryContinuation: CheckedContinuation<Void, Never>?
    private var treemapContinuation: CheckedContinuation<Void, Never>?
    private var releaseContinuation: CheckedContinuation<Void, Never>?
    private var largeFilesContinuation: CheckedContinuation<Void, Never>?
    private var liveActionContinuation: CheckedContinuation<Void, Never>?
    private var iCloudProbeContinuation: CheckedContinuation<Void, Never>?
    private let iCloudProbeFailures: [UInt64: ExplorerICloudLocalCopyProbeError]
    private let iCloudProbeAllocationOverrides: [UInt64: UInt64]
    private let cancelledICloudProbeNodeIDs: Set<UInt64>
    private let suspendedICloudProbeNodeID: UInt64?
    private var didSuspendICloudProbe = false
    private var activeICloudProbes = 0
    private var maximumConcurrentICloudProbes = 0
    private var candidateDetailContinuation: CheckedContinuation<Void, Never>?
    private var planReviewContinuation: CheckedContinuation<Void, Never>?
    private var dryRunStartContinuation: CheckedContinuation<Void, Never>?
    private var diffPrepareContinuation: CheckedContinuation<Void, Never>?
    private var diffQueryContinuation: CheckedContinuation<Void, Never>?
    private var diffNavigationPageContinuation:
        CheckedContinuation<Void, Never>?
    private var diffNavigationTreemapContinuation:
        CheckedContinuation<Void, Never>?
    private var diffPagingContinuation: CheckedContinuation<Void, Never>?
    private var releasedPlanReviewIDs: [UUID] = []
    private var releasedDiffReviewIDs: [UUID] = []
    private var diffScanIDs: [String] = []
    private var didSuspendRelease = false
    private var didSuspendQuery = false
    private var didSuspendDiffQuery = false
    private var didSuspendCandidateDetail = false
    private var rootFirstPageRequestCount = 0
    private var cleanupStarts = 0
    private var dryRunStarts = 0
    private var planReviewRefreshes = 0
    private let suspendDryRunStart: Bool

    init(
        mode: Mode = .available,
        cleanupTask: BrowserCleanupTaskStub = BrowserCleanupTaskStub(),
        dryRunTask: BrowserDryRunTaskStub = BrowserDryRunTaskStub(),
        suspendDryRunStart: Bool = false,
        iCloudProbeFailures: [UInt64: ExplorerICloudLocalCopyProbeError] = [:],
        iCloudProbeAllocationOverrides: [UInt64: UInt64] = [:],
        cancelledICloudProbeNodeIDs: Set<UInt64> = [],
        suspendedICloudProbeNodeID: UInt64? = nil
    ) {
        self.mode = mode
        self.cleanupTask = cleanupTask
        self.dryRunTask = dryRunTask
        self.suspendDryRunStart = suspendDryRunStart
        self.iCloudProbeFailures = iCloudProbeFailures
        self.iCloudProbeAllocationOverrides = iCloudProbeAllocationOverrides
        self.cancelledICloudProbeNodeIDs = cancelledICloudProbeNodeIDs
        self.suspendedICloudProbeNodeID = suspendedICloudProbeNodeID
    }

    func acquire(scanID: String) async throws {
        calls.append(.acquire(scanID: scanID))
        if mode == .exactUnavailable {
            throw ExplorerSnapshotReviewAcquisitionError.snapshotUnavailable
        }
        if mode == .suspendedExactAcquire {
            await withCheckedContinuation { continuation in
                acquisitionContinuation = continuation
            }
        }
    }

    func acquireLatest() async throws -> String {
        calls.append(.acquireLatest)
        if case .noSnapshot = mode {
            throw ExplorerSnapshotReviewAcquisitionError.snapshotUnavailable
        }
        if case .suspendedAcquire = mode {
            await withCheckedContinuation { continuation in
                acquisitionContinuation = continuation
            }
        }
        return "scan:latest"
    }

    func release(scanID: String) async {
        calls.append(.release(scanID: scanID))
        released.append(scanID)
        if (mode == .pageExpiredWithSuspendedRelease || mode == .suspendedReloadRelease),
           !didSuspendRelease
        {
            didSuspendRelease = true
            await withCheckedContinuation { continuation in
                releaseContinuation = continuation
            }
        }
    }

    func rootNode(scanID: String) async throws -> ExplorerSnapshotNode {
        calls.append(.root(scanID: scanID))
        if case .rootExpired = mode {
            throw ExplorerSnapshotNodeError.reviewExpired
        }
        return browserNode(
            id: 0,
            parentID: nil,
            depth: 0,
            kind: .directory,
            name: "/Users/example",
            logicalBytes: 99_850,
            childCount: 101,
            fileCount: 101
        )
    }

    func candidateSummaries(
        scanID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateSummaryPage {
        calls.append(.candidateSummaries(scanID: scanID, cursor: cursor, limit: limit))
        guard
            mode == .candidatesAvailable
                || mode == .candidateSummaryPages
                || mode == .suspendedCandidateDetail
                || mode == .candidateDetailExpired
                || mode == .candidateDetailMalformed
                || mode == .rustTargetPlanReviewAvailable
                || mode == .rustTargetPlanReviewChangesOnRefresh
                || mode == .rustTargetPlanReviewMismatchedRecency
                || mode == .rustTargetPlanReviewExpired
                || mode == .suspendedRustTargetPlanReview
        else {
            throw ExplorerCandidateDetailError.reviewNotAcquired
        }
        let candidates = candidateSummaries()
        let start = Int(cursor)
        guard start <= candidates.count else {
            throw ExplorerCandidateDetailError.invalidRequest
        }
        let end = min(start + Int(limit), candidates.count)
        let page = Array(candidates[start ..< end])
        return ExplorerCandidateSummaryPage(
            scanID: scanID,
            cursor: cursor,
            nextCursor: end < candidates.count ? UInt16(end) : nil,
            totalCandidates: UInt16(candidates.count),
            candidates: page
        )
    }

    func candidatePaths(
        scanID: String,
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidatePathPage {
        calls.append(.candidatePaths(
            scanID: scanID,
            candidateID: candidateID,
            cursor: cursor,
            limit: limit
        ))
        if mode == .candidateDetailExpired {
            throw ExplorerCandidateDetailError.reviewExpired
        }
        guard
            mode == .candidatesAvailable
                || mode == .candidateSummaryPages
                || mode == .suspendedCandidateDetail
                || mode == .candidateDetailMalformed
                || mode == .rustTargetPlanReviewAvailable
                || mode == .rustTargetPlanReviewChangesOnRefresh
                || mode == .rustTargetPlanReviewMismatchedRecency
                || mode == .rustTargetPlanReviewExpired
                || mode == .suspendedRustTargetPlanReview
        else {
            throw ExplorerCandidateDetailError.reviewNotAcquired
        }
        if mode == .suspendedCandidateDetail, !didSuspendCandidateDetail {
            didSuspendCandidateDetail = true
            await withCheckedContinuation { continuation in
                candidateDetailContinuation = continuation
            }
        }
        let candidate = candidateSummary(id: candidateID)
        let allPaths = (0 ..< Int(candidate.pathCount)).map { index in
            ExplorerCandidateObservedPath(
                encoding: .utf8,
                encodedBytes: Data("/Users/example/target/\(index)".utf8),
                display: "/Users/example/target/\(index)"
            )
        }
        let start = min(Int(cursor), allPaths.count)
        let end = min(start + Int(limit), allPaths.count)
        return ExplorerCandidatePathPage(
            scanID: mode == .candidateDetailMalformed ? "scan:wrong" : scanID,
            candidate: candidate,
            cursor: cursor,
            nextCursor: end < allPaths.count ? UInt16(end) : nil,
            totalPaths: UInt16(allPaths.count),
            paths: Array(allPaths[start ..< end])
        )
    }

    func candidateEvidence(
        scanID: String,
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateEvidencePage {
        calls.append(.candidateEvidence(
            scanID: scanID,
            candidateID: candidateID,
            cursor: cursor,
            limit: limit
        ))
        if mode == .candidateDetailExpired {
            throw ExplorerCandidateDetailError.reviewExpired
        }
        guard
            mode == .candidatesAvailable
                || mode == .candidateSummaryPages
                || mode == .suspendedCandidateDetail
                || mode == .candidateDetailMalformed
                || mode == .rustTargetPlanReviewAvailable
                || mode == .rustTargetPlanReviewChangesOnRefresh
                || mode == .rustTargetPlanReviewMismatchedRecency
                || mode == .rustTargetPlanReviewExpired
                || mode == .suspendedRustTargetPlanReview
        else {
            throw ExplorerCandidateDetailError.reviewNotAcquired
        }
        let candidate = candidateSummary(id: candidateID)
        let allEvidence = candidate.evidenceKinds.indices.map { index in
            ExplorerCandidateEvidence(
                ordinal: UInt16(index),
                kind: .matchedPath,
                path: ExplorerCandidateObservedPath(
                    encoding: .utf8,
                    encodedBytes: Data("/Users/example/target/\(index)".utf8),
                    display: "/Users/example/target/\(index)"
                ),
                identifier: nil,
                newestMtime: nil,
                minimumAgeSeconds: nil,
                minimumAgeNanoseconds: nil,
                observedBytes: nil,
                minimumBytes: nil
            )
        }
        let start = min(Int(cursor), allEvidence.count)
        let end = min(start + Int(limit), allEvidence.count)
        return ExplorerCandidateEvidencePage(
            scanID: scanID,
            candidate: candidate,
            cursor: cursor,
            nextCursor: end < allEvidence.count ? UInt16(end) : nil,
            totalEvidence: UInt16(allEvidence.count),
            evidence: Array(allEvidence[start ..< end])
        )
    }

    func childNodes(
        scanID: String,
        parentID: UInt64,
        sort: ExplorerSnapshotNodeSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotNodePage {
        calls.append(.children(
            scanID: scanID,
            parentID: parentID,
            sort: sort,
            offset: offset,
            limit: limit
        ))
        if parentID == 0, offset == 0 {
            rootFirstPageRequestCount += 1
        }
        let shouldSuspend = !didSuspendQuery && (
            mode == .suspendedSort && parentID == 0 && sort == .nameAscending
                || mode == .suspendedNextPage && parentID == 0 && offset == 100
        )
        if shouldSuspend {
            didSuspendQuery = true
            await withCheckedContinuation { continuation in
                queryContinuation = continuation
            }
        }
        if parentID == 1, case .directoryBudget = mode {
            throw ExplorerSnapshotNodeError.budgetExceeded
        }
        if mode == .pageExpiredWithSuspendedRelease, parentID == 0, offset == 100 {
            throw ExplorerSnapshotNodeError.reviewExpired
        }
        let allNodes: [ExplorerSnapshotNode]
        if parentID == 1 {
            allNodes = [browserNode(
                id: 500,
                parentID: 1,
                depth: 2,
                kind: .file,
                name: "nested.log",
                logicalBytes: 5_000,
                category: .developerArtifact
            )]
        } else {
            var rootNodes = [browserNode(
                id: 1,
                parentID: 0,
                depth: 1,
                kind: .directory,
                name: "Folder",
                logicalBytes: 5_000,
                category: .developerArtifact,
                childCount: 1,
                fileCount: 1
            )]
            rootNodes.append(contentsOf: (2 ... 101).map { index in
                browserNode(
                    id: UInt64(index),
                    parentID: 0,
                    depth: 1,
                    kind: .file,
                    name: "item-\(index)",
                    logicalBytes: UInt64(1_000 - index)
                )
            })
            let sortedNodes = sort == .nameAscending
                ? rootNodes.sorted { $0.name.display < $1.name.display }
                : rootNodes
            if mode == .offPageCategoryMismatch, rootFirstPageRequestCount > 1 {
                allNodes = sortedNodes.map { node in
                    guard node.id == 1 else { return node }
                    return browserNode(node, replacingCategoryWith: .browserCache)
                }
            } else {
                allNodes = sortedNodes
            }
        }
        let start = min(Int(offset), allNodes.count)
        let end = min(start + Int(limit), allNodes.count)
        return ExplorerSnapshotNodePage(
            parentID: parentID,
            offset: offset,
            totalChildren: UInt64(allNodes.count),
            hasMore: end < allNodes.count,
            nodes: Array(allNodes[start ..< end])
        )
    }

    func treemap(
        scanID: String,
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotTreemap {
        calls.append(.treemap(scanID: scanID, parentID: parentID, maxCells: maxCells))
        if case .treemapBudget = mode {
            throw ExplorerSnapshotTreemapError.budgetExceeded
        }
        if case .treemapExpired = mode {
            throw ExplorerSnapshotTreemapError.reviewExpired
        }
        if case .suspendedTreemap = mode {
            await withCheckedContinuation { continuation in
                treemapContinuation = continuation
            }
        }
        let allNodes: [ExplorerSnapshotNode]
        if parentID == 1 {
            allNodes = [browserNode(
                id: 500,
                parentID: 1,
                depth: 2,
                kind: .file,
                name: "nested.log",
                logicalBytes: 5_000,
                category: .developerArtifact
            )]
        } else {
            var rootNodes = [browserNode(
                id: 1,
                parentID: 0,
                depth: 1,
                kind: .directory,
                name: "Folder",
                logicalBytes: 5_000,
                category: .developerArtifact,
                childCount: 1,
                fileCount: 1
            )]
            rootNodes.append(contentsOf: (2 ... 101).map { index in
                browserNode(
                    id: UInt64(index),
                    parentID: 0,
                    depth: 1,
                    kind: .file,
                    name: "item-\(index)",
                    logicalBytes: UInt64(1_000 - index)
                )
            })
            allNodes = rootNodes.sorted {
                $0.logicalBytes == $1.logicalBytes
                    ? $0.id < $1.id
                    : $0.logicalBytes > $1.logicalBytes
            }
        }
        let represented = Array(allNodes.prefix(Int(maxCells)))
        let omitted = allNodes.dropFirst(represented.count)
        let cells = represented.enumerated().map { index, node in
            let projectedNode = (
                mode == .treemapCategoryMismatch
                    || mode == .refreshedTreemapMismatch && scanID == "scan:refreshed"
            ) && node.id == 1
                ? browserNode(node, replacingCategoryWith: .browserCache)
                : node
            return ExplorerSnapshotTreemapCell(
                node: projectedNode,
                logicalRank: UInt64(index)
            )
        }
        return ExplorerSnapshotTreemap(
            parentID: parentID,
            totalChildren: UInt64(allNodes.count),
            totalChildLogicalBytes: allNodes.reduce(0) { $0 + $1.logicalBytes },
            otherChildCount: UInt64(omitted.count),
            otherLogicalBytes: omitted.reduce(0) { $0 + $1.logicalBytes },
            zeroLogicalChildCount: 0,
            cells: cells
        )
    }

    func prepareSnapshotDiffReview(
        scanID: String
    ) async throws -> ExplorerSnapshotDiffReviewHandle {
        calls.append(.prepareDiff(scanID: scanID))
        diffScanIDs.append(scanID)
        if mode == .diffUnavailable {
            throw ExplorerSnapshotDiffFailure.notAvailable
        }
        guard
            mode == .diffAvailable
                || mode == .diffRootFailure
                || mode == .suspendedDiffPrepare
                || mode == .suspendedDiffSort
                || mode == .suspendedDiffNavigation
                || mode == .suspendedDiffPaging
        else {
            throw ExplorerSnapshotDiffFailure.unavailable
        }
        if mode == .suspendedDiffPrepare {
            await withCheckedContinuation { continuation in
                diffPrepareContinuation = continuation
            }
        }
        return ExplorerSnapshotDiffReviewHandle(
            id: UUID(),
            info: browserDiffInfo(currentScanID: scanID)
        )
    }

    func snapshotDiffRootNode(
        _ handle: ExplorerSnapshotDiffReviewHandle
    ) async throws -> ExplorerSnapshotDiffNode {
        calls.append(.diffRoot(handleID: handle.id))
        if mode == .diffRootFailure {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }
        return browserDiffNode(
            id: 0,
            parentID: nil,
            depth: 0,
            kind: .directory,
            name: "/Users/example",
            change: .grew,
            currentLogicalBytes: 99_850,
            baselineLogicalBytes: 98_550,
            childCount: 101,
            canDescend: true
        )
    }

    func snapshotDiffChildNodes(
        _ handle: ExplorerSnapshotDiffReviewHandle,
        parentID: UInt64,
        sort: ExplorerSnapshotDiffSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotDiffNodePage {
        calls.append(.diffChildren(
            handleID: handle.id,
            parentID: parentID,
            sort: sort,
            offset: offset,
            limit: limit
        ))
        if mode == .suspendedDiffSort,
           sort == .nameAscending,
           !didSuspendDiffQuery
        {
            didSuspendDiffQuery = true
            await withCheckedContinuation { continuation in
                diffQueryContinuation = continuation
            }
        }
        if mode == .suspendedDiffNavigation, parentID == 1 {
            await withCheckedContinuation { continuation in
                diffNavigationPageContinuation = continuation
            }
        }
        if mode == .suspendedDiffPaging, parentID == 0, offset == 100 {
            await withCheckedContinuation { continuation in
                diffPagingContinuation = continuation
            }
        }
        let allNodes = browserDiffNodes(parentID: parentID, sort: sort)
        let start = min(Int(offset), allNodes.count)
        let end = min(start + Int(limit), allNodes.count)
        let totals = browserDiffTotals(allNodes)
        return ExplorerSnapshotDiffNodePage(
            parentID: parentID,
            offset: offset,
            totalChildren: UInt64(allNodes.count),
            hasMore: end < allNodes.count,
            totalGrowthBytes: totals.growth,
            totalShrinkageBytes: totals.shrinkage,
            unchangedChildCount: totals.unchanged,
            replacedChildCount: totals.replaced,
            nodes: Array(allNodes[start ..< end])
        )
    }

    func snapshotDiffTreemap(
        _ handle: ExplorerSnapshotDiffReviewHandle,
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotDiffTreemap {
        calls.append(.diffTreemap(
            handleID: handle.id,
            parentID: parentID,
            maxCells: maxCells
        ))
        if mode == .suspendedDiffNavigation, parentID == 1 {
            await withCheckedContinuation { continuation in
                diffNavigationTreemapContinuation = continuation
            }
        }
        let allNodes = browserDiffNodes(
            parentID: parentID,
            sort: .magnitudeDescending
        )
        let changed = allNodes.filter {
            $0.logicalChange.magnitudeBytes > 0
        }
        let represented = Array(changed.prefix(Int(maxCells)))
        let omitted = changed.dropFirst(represented.count)
        let totals = browserDiffTotals(allNodes)
        return ExplorerSnapshotDiffTreemap(
            parentID: parentID,
            totalChildren: UInt64(allNodes.count),
            changedChildCount: UInt64(changed.count),
            totalGrowthBytes: totals.growth,
            totalShrinkageBytes: totals.shrinkage,
            otherGrowthChildCount: UInt64(omitted.filter {
                $0.logicalChange.direction == .growth
            }.count),
            otherGrowthBytes: omitted.reduce(into: UInt64(0)) { total, node in
                if node.logicalChange.direction == .growth {
                    total += node.logicalChange.magnitudeBytes
                }
            },
            otherShrinkageChildCount: UInt64(omitted.filter {
                $0.logicalChange.direction == .shrinkage
            }.count),
            otherShrinkageBytes: omitted.reduce(into: UInt64(0)) { total, node in
                if node.logicalChange.direction == .shrinkage {
                    total += node.logicalChange.magnitudeBytes
                }
            },
            unchangedChildCount: totals.unchanged,
            replacedChildCount: totals.replaced,
            cells: represented.enumerated().map { index, node in
                ExplorerSnapshotDiffTreemapCell(
                    node: node,
                    magnitudeRank: UInt64(index)
                )
            }
        )
    }

    func releaseSnapshotDiffReview(
        _ handle: ExplorerSnapshotDiffReviewHandle
    ) {
        calls.append(.releaseDiff(handleID: handle.id))
        releasedDiffReviewIDs.append(handle.id)
    }

    func largeFiles(
        scanID: String,
        minimumLogicalBytes: UInt64,
        modifiedBefore: ExplorerSnapshotTimestamp?,
        maxResults: UInt16
    ) async throws -> ExplorerSnapshotLargeFilesPage {
        calls.append(.largeFiles(
            scanID: scanID,
            minimumLogicalBytes: minimumLogicalBytes,
            modifiedBefore: modifiedBefore,
            maxResults: maxResults
        ))
        if mode == .largeFilesExpired {
            throw ExplorerSnapshotLargeFilesError.reviewExpired
        }
        if mode == .suspendedLargeFiles {
            await withCheckedContinuation { continuation in
                largeFilesContinuation = continuation
            }
        }
        let candidates = [
            ExplorerSnapshotLargeFile(
                node: browserNode(
                    id: 900,
                    parentID: 1,
                    depth: 2,
                    kind: .file,
                    name: "archive.mov",
                    logicalBytes: 6_442_450_944,
                    category: .installerAndDownload
                ),
                parentContext: [browserNodeName("Downloads")],
                contextTruncated: false
            ),
            ExplorerSnapshotLargeFile(
                node: browserNode(
                    id: 901,
                    parentID: 1,
                    depth: 2,
                    kind: .file,
                    name: "image.dmg",
                    logicalBytes: 2_147_483_648,
                    category: .installerAndDownload
                ),
                parentContext: [browserNodeName("Downloads")],
                contextTruncated: false
            ),
        ].filter { file in
            file.node.logicalBytes >= minimumLogicalBytes
                && modifiedBefore.map { cutoff in
                    guard let modified = file.node.modifiedAt else { return false }
                    return modified.secondsSinceUnixEpoch < cutoff.secondsSinceUnixEpoch
                        || (modified.secondsSinceUnixEpoch == cutoff.secondsSinceUnixEpoch
                            && modified.nanoseconds < cutoff.nanoseconds)
                } ?? true
        }
        let files = Array(candidates.prefix(Int(maxResults)))
        return ExplorerSnapshotLargeFilesPage(
            minimumLogicalBytes: minimumLogicalBytes,
            modifiedBefore: modifiedBefore,
            totalMatchingFiles: UInt64(candidates.count),
            totalMatchingLogicalBytes: candidates.reduce(0) { $0 + $1.node.logicalBytes },
            hasMore: candidates.count > files.count,
            files: files
        )
    }

    func icloudObservationSource(
        scanID: String,
        scopeNodeID: UInt64,
        maxResults: UInt16
    ) async throws -> ExplorerICloudObservationSource {
        calls.append(.iCloudObservationSource(
            scanID: scanID,
            scopeNodeID: scopeNodeID,
            maxResults: maxResults
        ))
        let targets = Array(browserICloudObservationTargets().prefix(Int(maxResults)))
        return ExplorerICloudObservationSource(
            scanID: scanID,
            scopeNodeID: scopeNodeID,
            requestedMaxResults: maxResults,
            visitedNodeCount: 104,
            totalRankedFiles: UInt64(browserICloudObservationTargets().count),
            hasMore: browserICloudObservationTargets().count > targets.count,
            targets: targets
        )
    }

    func resolveLiveItem(
        scanID: String,
        nodeID: UInt64,
        purpose: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem {
        calls.append(.liveItem(scanID: scanID, nodeID: nodeID, purpose: purpose))
        if mode == .liveActionChanged {
            throw ExplorerSnapshotLivePathError.changedSinceScan
        }
        if mode == .suspendedLiveAction {
            await withCheckedContinuation { continuation in
                liveActionContinuation = continuation
            }
        }
        return ExplorerResolvedLiveItem(
            nodeID: nodeID,
            kind: .file,
            url: URL(fileURLWithPath: "/Users/example/item-\(nodeID)"),
            exactTextPath: mode == .nonUnicodeLiveItem
                ? nil
                : "/Users/example/item-\(nodeID)"
        )
    }

    func probeICloudLocalCopy(
        scanID: String,
        nodeID: UInt64
    ) async throws -> ExplorerICloudLocalCopyAssessment {
        calls.append(.iCloudProbe(scanID: scanID, nodeID: nodeID))
        activeICloudProbes += 1
        maximumConcurrentICloudProbes = max(
            maximumConcurrentICloudProbes,
            activeICloudProbes
        )
        defer {
            activeICloudProbes -= 1
        }
        if mode == .iCloudChanged {
            throw ExplorerICloudLocalCopyProbeError.changedSinceSnapshot
        }
        if (
            mode == .suspendedICloudProbe
                || suspendedICloudProbeNodeID == nodeID
        ) && !didSuspendICloudProbe {
            didSuspendICloudProbe = true
            await withCheckedContinuation { continuation in
                iCloudProbeContinuation = continuation
            }
        }
        if let failure = iCloudProbeFailures[nodeID] {
            throw failure
        }
        if cancelledICloudProbeNodeIDs.contains(nodeID) {
            throw CancellationError()
        }
        guard
            mode == .iCloudEligible
                || mode == .iCloudBlocked
                || mode == .suspendedICloudProbe
                || !iCloudProbeFailures.isEmpty
                || suspendedICloudProbeNodeID != nil
                || mode == .available
        else {
            throw ExplorerICloudLocalCopyProbeError.unavailable
        }
        let blocked = mode == .iCloudBlocked
        return ExplorerICloudLocalCopyAssessment(
            localAllocatedBytes: iCloudProbeAllocationOverrides[nodeID]
                ?? browserICloudObservationTargets()
                    .first(where: { $0.id == nodeID })?
                    .node.allocatedBytes ?? 8192,
            observedAtUnixMilliseconds: 1_234_000,
            ubiquitous: .yes,
            uploaded: blocked ? .unknown : .yes,
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
            isEligibleObservation: !blocked,
            blockers: blocked ? [.uploadStateUnknown] : [],
            isIdentityReady: false,
            identityBlockers: [.containerIdentityUnsupported]
        )
    }

    func prepareRustTargetPlanReview(
        scanID: String,
        candidateID: String
    ) async throws -> ExplorerRustTargetPlanReviewHandle {
        guard scanID == "scan:latest" else {
            throw ExplorerRustTargetPlanReviewError.parentReviewUnavailable
        }
        if mode == .rustTargetPlanReviewExpired {
            throw ExplorerRustTargetPlanReviewError.reviewExpired
        }
        guard
            mode == .rustTargetPlanReviewAvailable
                || mode == .rustTargetPlanReviewChangesOnRefresh
                || mode == .rustTargetPlanReviewMismatchedRecency
                || mode == .suspendedRustTargetPlanReview
        else {
            throw ExplorerRustTargetPlanReviewError.unavailable
        }
        if mode == .suspendedRustTargetPlanReview {
            await withCheckedContinuation { continuation in
                planReviewContinuation = continuation
            }
        }
        let now = Date()
        let info = ExplorerRustTargetPlanReviewInfo(
            planID: "plan:browser",
            sourceScanID: scanID,
            candidateID: candidateID,
            ruleID: "developer.rust.target",
            ruleRevision: 3,
            category: .developerArtifact,
            mode: .permanentSafe,
            safety: .safeRegenerable,
            action: .removeKnownRegenerableContents,
            estimatedBytes: 42_000,
            newestMtime: ExplorerSnapshotTimestamp(
                secondsSinceUnixEpoch: 1_700_000_000,
                nanoseconds: mode == .rustTargetPlanReviewMismatchedRecency ? 1 : 0
            ),
            minimumAgeSeconds: 604_800,
            minimumAgeNanoseconds: 0,
            itemCount: 1,
            pathCount: 1,
            warnings: [
                .estimatedBytesUnverified,
                .permanentRemovalCannotBeUndone,
            ],
            createdAt: ExplorerRustTargetPlanReviewAdapter.timestamp(for: now),
            effectiveExpiresAt: ExplorerRustTargetPlanReviewAdapter.timestamp(
                for: now.addingTimeInterval(60)
            ),
            scheduleEligible: false,
            target: ExplorerRustTargetPlanReviewPath(
                encoding: .unixBytes,
                encodedBytes: Data("/Users/example/project/target".utf8),
                display: "/Users/example/project/target"
            )
        )
        return ExplorerRustTargetPlanReviewHandle(id: UUID(), info: info)
    }

    func refreshRustTargetPlanReview(
        _ handle: ExplorerRustTargetPlanReviewHandle
    ) throws -> ExplorerRustTargetPlanReviewHandle {
        planReviewRefreshes += 1
        if mode == .rustTargetPlanReviewChangesOnRefresh {
            throw ExplorerRustTargetPlanReviewError.changedDuringReview
        }
        guard mode == .rustTargetPlanReviewAvailable else {
            throw ExplorerRustTargetPlanReviewError.unavailable
        }
        return handle
    }

    func releaseRustTargetPlanReview(
        _ handle: ExplorerRustTargetPlanReviewHandle
    ) {
        releasedPlanReviewIDs.append(handle.id)
    }

    func startRustTargetCleanup(
        _: ExplorerRustTargetPlanReviewHandle
    ) throws -> any DuxRustTargetCleanupTask {
        guard mode == .rustTargetPlanReviewAvailable else {
            throw ExplorerRustTargetCleanupStartError.unavailable
        }
        cleanupStarts += 1
        return cleanupTask
    }

    func startRustTargetDryRun(
        _: ExplorerRustTargetPlanReviewHandle
    ) async throws -> any DuxRustTargetDryRunTask {
        guard mode == .rustTargetPlanReviewAvailable else {
            throw ExplorerRustTargetDryRunStartError.unavailable
        }
        dryRunStarts += 1
        if suspendDryRunStart {
            await withCheckedContinuation { continuation in
                dryRunStartContinuation = continuation
            }
        }
        return dryRunTask
    }

    func recordedCalls() -> [Call] {
        calls
    }

    func releasedScanIDs() -> [String] {
        released
    }

    func hasSuspendedAcquisition() -> Bool {
        acquisitionContinuation != nil
    }

    func resumeAcquisition() {
        acquisitionContinuation?.resume()
        acquisitionContinuation = nil
    }

    func hasSuspendedQuery() -> Bool {
        queryContinuation != nil
    }

    func resumeQuery() {
        queryContinuation?.resume()
        queryContinuation = nil
    }

    func hasSuspendedTreemap() -> Bool {
        treemapContinuation != nil
    }

    func resumeTreemap() {
        treemapContinuation?.resume()
        treemapContinuation = nil
    }

    func hasSuspendedDiffPrepare() -> Bool {
        diffPrepareContinuation != nil
    }

    func resumeDiffPrepare() {
        diffPrepareContinuation?.resume()
        diffPrepareContinuation = nil
    }

    func hasSuspendedDiffQuery() -> Bool {
        diffQueryContinuation != nil
    }

    func resumeDiffQuery() {
        diffQueryContinuation?.resume()
        diffQueryContinuation = nil
    }

    func hasSuspendedDiffNavigationPage() -> Bool {
        diffNavigationPageContinuation != nil
    }

    func hasSuspendedDiffNavigationTreemap() -> Bool {
        diffNavigationTreemapContinuation != nil
    }

    func resumeDiffNavigationPage() {
        diffNavigationPageContinuation?.resume()
        diffNavigationPageContinuation = nil
    }

    func resumeDiffNavigationTreemap() {
        diffNavigationTreemapContinuation?.resume()
        diffNavigationTreemapContinuation = nil
    }

    func hasSuspendedDiffPaging() -> Bool {
        diffPagingContinuation != nil
    }

    func resumeDiffPaging() {
        diffPagingContinuation?.resume()
        diffPagingContinuation = nil
    }

    func releasedDiffReviewCount() -> Int {
        releasedDiffReviewIDs.count
    }

    func preparedDiffScanIDs() -> [String] {
        diffScanIDs
    }

    func hasSuspendedLargeFiles() -> Bool {
        largeFilesContinuation != nil
    }

    func resumeLargeFiles() {
        largeFilesContinuation?.resume()
        largeFilesContinuation = nil
    }

    func hasSuspendedLiveAction() -> Bool {
        liveActionContinuation != nil
    }

    func resumeLiveAction() {
        liveActionContinuation?.resume()
        liveActionContinuation = nil
    }

    func hasSuspendedICloudProbe() -> Bool {
        iCloudProbeContinuation != nil
    }

    func resumeICloudProbe() {
        iCloudProbeContinuation?.resume()
        iCloudProbeContinuation = nil
    }

    func maximumConcurrentICloudProbeCount() -> Int {
        maximumConcurrentICloudProbes
    }

    func hasSuspendedCandidateDetail() -> Bool {
        candidateDetailContinuation != nil
    }

    func resumeCandidateDetail() {
        candidateDetailContinuation?.resume()
        candidateDetailContinuation = nil
    }

    func hasSuspendedPlanReview() -> Bool {
        planReviewContinuation != nil
    }

    func resumePlanReview() {
        planReviewContinuation?.resume()
        planReviewContinuation = nil
    }

    func releasedPlanReviewCount() -> Int {
        releasedPlanReviewIDs.count
    }

    func planReviewRefreshCount() -> Int {
        planReviewRefreshes
    }

    func cleanupStartCount() -> Int {
        cleanupStarts
    }

    func dryRunStartCount() -> Int {
        dryRunStarts
    }

    func hasSuspendedDryRunStart() -> Bool {
        dryRunStartContinuation != nil
    }

    func resumeDryRunStart() {
        dryRunStartContinuation?.resume()
        dryRunStartContinuation = nil
    }

    func hasSuspendedRelease() -> Bool {
        releaseContinuation != nil
    }

    func resumeRelease() {
        releaseContinuation?.resume()
        releaseContinuation = nil
    }

    private func candidateSummaries() -> [ExplorerCandidateSummary] {
        if mode == .candidateSummaryPages {
            return (0 ... 64).map {
                browserCandidateSummary(id: "candidate:page-\($0)")
            }
        }
        return [
            candidateSummary(id: "candidate:browser-0"),
            candidateSummary(id: "candidate:browser-1"),
        ]
    }

    private func candidateSummary(id: String) -> ExplorerCandidateSummary {
        switch mode {
        case .rustTargetPlanReviewAvailable, .rustTargetPlanReviewChangesOnRefresh,
             .rustTargetPlanReviewMismatchedRecency,
             .rustTargetPlanReviewExpired,
             .suspendedRustTargetPlanReview:
            browserRustTargetCandidateSummary(id: id)
        default:
            browserCandidateSummary(id: id)
        }
    }
}

private actor BrowserDryRunTaskStub: DuxRustTargetDryRunTask {
    private let polls: [ExplorerRustTargetDryRunPoll]
    private var pollIndex = 0
    private var cancellations = 0

    init(
        polls: [ExplorerRustTargetDryRunPoll] = [
            ExplorerRustTargetDryRunPoll(
                phase: .cancelled,
                cancellationRequested: true,
                revision: 1,
                failure: nil,
                result: nil
            ),
        ]
    ) {
        precondition(!polls.isEmpty)
        self.polls = polls
    }

    func poll() -> ExplorerRustTargetDryRunPoll {
        let poll = polls[min(pollIndex, polls.count - 1)]
        pollIndex += 1
        return poll
    }

    func requestCancellation() -> ExplorerRustTargetDryRunCancelOutcome {
        cancellations += 1
        return .requested
    }

    func cancellationCount() -> Int {
        cancellations
    }
}

private actor DryRunTerminalObserverSpy {
    private var observations = 0

    func observe() {
        observations += 1
    }

    func count() -> Int {
        observations
    }
}

private actor SuspendedCleanupTerminalObserver {
    private var continuation: CheckedContinuation<Void, Never>?

    func observe() async {
        await withCheckedContinuation { continuation in
            self.continuation = continuation
        }
    }

    func isSuspended() -> Bool {
        continuation != nil
    }

    func resume() {
        continuation?.resume()
        continuation = nil
    }
}

private actor BrowserCleanupTaskStub: DuxRustTargetCleanupTask {
    private let polls: [ExplorerRustTargetCleanupPoll]
    private var pollIndex = 0
    private var cancellations = 0

    init(
        polls: [ExplorerRustTargetCleanupPoll] = [
            ExplorerRustTargetCleanupPoll(
                phase: .cancelled,
                cancellationRequested: true,
                revision: 1,
                failure: nil,
                result: nil
            ),
        ]
    ) {
        precondition(!polls.isEmpty)
        self.polls = polls
    }

    func poll() throws -> ExplorerRustTargetCleanupPoll {
        let poll = polls[min(pollIndex, polls.count - 1)]
        pollIndex += 1
        return poll
    }

    func requestCancellation() -> ExplorerRustTargetCleanupCancelOutcome {
        cancellations += 1
        return .requested
    }

    func cancellationCount() -> Int {
        cancellations
    }
}

private struct SuspendedRustTargetDryRunPollingClock:
    ExplorerRustTargetDryRunPollingClock
{
    private let sleeper = SuspendedRustTargetDryRunPollingSleeper()

    func sleepUntilNextPoll() async throws {
        await sleeper.sleep()
    }

    func hasSuspendedSleep() async -> Bool {
        await sleeper.hasSuspendedSleep()
    }

    func resumeSleep() async {
        await sleeper.resumeSleep()
    }
}

private actor SuspendedRustTargetDryRunPollingSleeper {
    private var continuation: CheckedContinuation<Void, Never>?

    func sleep() async {
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

private struct SuspendedRustTargetCleanupPollingClock:
    ExplorerRustTargetCleanupPollingClock
{
    private let sleeper = SuspendedRustTargetCleanupPollingSleeper()

    func sleepUntilNextPoll() async throws {
        await sleeper.sleep()
    }

    func hasSuspendedSleep() async -> Bool {
        await sleeper.hasSuspendedSleep()
    }

    func resumeSleep() async {
        await sleeper.resumeSleep()
    }
}

private actor SuspendedRustTargetCleanupPollingSleeper {
    private var continuation: CheckedContinuation<Void, Never>?

    func sleep() async {
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

private struct SuspendedRustTargetPlanReviewClock:
    ExplorerRustTargetPlanReviewClock
{
    private let instant: Date
    private let sleeper = SuspendedRustTargetPlanReviewSleeper()

    init(now: Date) {
        instant = now
    }

    func now() -> Date {
        instant
    }

    func sleep(until _: Date) async throws {
        try Task.checkCancellation()
        await sleeper.sleep()
        try Task.checkCancellation()
    }

    func hasSuspendedSleep() async -> Bool {
        await sleeper.hasSuspendedSleep()
    }

    func resumeSleep() async {
        await sleeper.resumeSleep()
    }
}

private actor SuspendedRustTargetPlanReviewSleeper {
    private var continuation: CheckedContinuation<Void, Never>?

    func sleep() async {
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

@MainActor
private final class BrowserLiveActionPresenterSpy: ExplorerLiveFileActionPresenting {
    private(set) var revealed: [ExplorerResolvedLiveItem] = []
    private(set) var copied: [ExplorerResolvedLiveItem] = []
    private(set) var previewed: [ExplorerResolvedLiveItem] = []
    private(set) var dismissCount = 0

    func reveal(_ item: ExplorerResolvedLiveItem) -> Bool {
        revealed.append(item)
        return true
    }

    func copyPath(_ item: ExplorerResolvedLiveItem) -> Bool {
        copied.append(item)
        return item.exactTextPath != nil
    }

    func quickLook(_ item: ExplorerResolvedLiveItem) -> Bool {
        previewed.append(item)
        return true
    }

    func dismissQuickLook() {
        dismissCount += 1
    }
}

private actor BrowserCoverageStub: DuxScanCoverageServing {
    private var scanIDs: [String] = []
    private let suspends: Bool
    private var continuation: CheckedContinuation<Void, Never>?

    init(suspends: Bool = false) {
        self.suspends = suspends
    }

    func loadScanCoverageDetails(scanID: String) async throws
        -> ExplorerScanCoverageDetails
    {
        scanIDs.append(scanID)
        if suspends {
            await withCheckedContinuation { continuation in
                self.continuation = continuation
            }
        }
        return ExplorerScanCoverageDetails(
            scanID: scanID,
            coverage: .complete,
            measuredPermille: 1_000,
            totalIssueRecords: 0,
            totalIssueOccurrences: 0,
            issues: []
        )
    }

    func requestedScanIDs() -> [String] {
        scanIDs
    }

    func hasSuspendedRequest() -> Bool {
        continuation != nil
    }

    func resumeRequest() {
        continuation?.resume()
        continuation = nil
    }
}

private actor BrowserHistoryStub: DuxSnapshotHistoryServing {
    private let hasMore: Bool
    private let fails: Bool
    private let overLimit: Bool
    private let suspends: Bool
    private var limits: [UInt16] = []
    private var continuation: CheckedContinuation<Void, Never>?

    init(
        hasMore: Bool = false,
        fails: Bool = false,
        overLimit: Bool = false,
        suspends: Bool = false
    ) {
        self.hasMore = hasMore
        self.fails = fails
        self.overLimit = overLimit
        self.suspends = suspends
    }

    func loadRecentSnapshotHistory(limit: UInt16) async throws -> ExplorerSnapshotHistoryPage {
        limits.append(limit)
        if suspends {
            await withCheckedContinuation { continuation in
                self.continuation = continuation
            }
        }
        if fails {
            throw EngineServiceError.unavailable
        }
        let scans: [ExplorerHistoricalScan] = overLimit
            ? (0 ... 50).map { index in
                historicalBrowserScan(id: "scan:\(index)", startedAt: TimeInterval(2_000 - index))
            }
            : [
                historicalBrowserScan(id: "scan:latest", startedAt: 2_000),
                historicalBrowserScan(id: "scan:older", startedAt: 1_000),
            ]
        return ExplorerSnapshotHistoryPage(scans: scans, hasMore: hasMore)
    }

    func requestedLimits() -> [UInt16] {
        limits
    }

    func hasSuspendedRequest() -> Bool {
        continuation != nil
    }

    func resumeRequest() {
        continuation?.resume()
        continuation = nil
    }
}

private func browserCandidateSummary(id: String) -> ExplorerCandidateSummary {
    ExplorerCandidateSummary(
        candidateID: id,
        ruleID: "developer.rust.target",
        ruleRevision: 2,
        category: .developerArtifact,
        estimatedBytes: 42_000,
        newestMtime: ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: 1_700_000_000,
            nanoseconds: 0
        ),
        safety: .safeRegenerable,
        action: .removeKnownRegenerableContents,
        ruleScheduleEligible: false,
        pathCount: 65,
        evidenceKinds: Array(repeating: .matchedPath, count: 65),
        blockers: [.protectedPath],
        createdAt: ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: 1_700_000_001,
            nanoseconds: 0
        ),
        status: .discovered
    )
}

private func browserRustTargetCandidateSummary(id: String) -> ExplorerCandidateSummary {
    ExplorerCandidateSummary(
        candidateID: id,
        ruleID: "developer.rust.target",
        ruleRevision: 3,
        category: .developerArtifact,
        estimatedBytes: 42_000,
        newestMtime: ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: 1_700_000_000,
            nanoseconds: 0
        ),
        safety: .safeRegenerable,
        action: .removeKnownRegenerableContents,
        ruleScheduleEligible: false,
        pathCount: 1,
        evidenceKinds: [.matchedPath, .requiredMarker, .requiredMarker, .minimumAge],
        blockers: [.protectedPath],
        createdAt: ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: 1_700_000_001,
            nanoseconds: 0
        ),
        status: .discovered
    )
}

private func historicalBrowserScan(id: String, startedAt: TimeInterval) -> ExplorerHistoricalScan {
    ExplorerHistoricalScan(
        scanID: id,
        startedAt: Date(timeIntervalSince1970: startedAt),
        completedAt: Date(timeIntervalSince1970: startedAt + 100),
        status: .succeeded,
        counts: ExplorerHistoricalScanCounts(
            directoryCount: 1,
            fileCount: 101,
            logicalBytes: 99_850,
            allocatedBytes: 99_850
        ),
        coverage: .complete,
        coveragePermille: 1_000,
        issueCount: 0,
        snapshotRecorded: true
    )
}

private func browserScanSummary(scanID: String) -> AppScanSummary {
    AppScanSummary(
        scanID: scanID,
        startedAt: Date(timeIntervalSince1970: 2_000),
        completedAt: Date(timeIntervalSince1970: 2_100),
        progress: ScanProgressFacts(
            files: 101,
            directories: 1,
            knownAllocatedBytes: 99_850,
            issueCount: 0
        ),
        logicalBytes: 99_850,
        coverage: .complete,
        coveragePermille: 1_000,
        snapshotAvailable: true
    )
}

private func browserNode(
    id: UInt64,
    parentID: UInt64?,
    depth: UInt32,
    kind: ExplorerSnapshotNodeKind,
    name: String,
    logicalBytes: UInt64,
    category: ExplorerStorageCategory = .unclassified,
    childCount: UInt64 = 0,
    fileCount: UInt64 = 1
) -> ExplorerSnapshotNode {
    ExplorerSnapshotNode(
        id: id,
        parentID: parentID,
        depth: depth,
        kind: kind,
        category: category,
        name: ExplorerSnapshotNodeName(
            encoding: .unixBytes,
            encodedBytes: Data(name.utf8),
            display: name
        ),
        logicalBytes: logicalBytes,
        allocatedBytes: logicalBytes,
        fileCount: fileCount,
        childCount: childCount,
        modifiedAt: ExplorerSnapshotTimestamp(secondsSinceUnixEpoch: 1_700_000_000, nanoseconds: 0),
        accessedAt: nil,
        scanFlags: ExplorerSnapshotScanFlags(
            inaccessible: false,
            timedOut: false,
            hardLinkDuplicate: false,
            mountBoundary: false
        )
    )
}

private func browserDiffInfo(currentScanID: String) -> ExplorerSnapshotDiffInfo {
    ExplorerSnapshotDiffInfo(
        currentScanID: currentScanID,
        baselineScanID: "scan:baseline:\(currentScanID)",
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
            status: .partial,
            measuredPermille: 950,
            issueRecordCount: 1,
            issueOccurrenceCount: 1
        )
    )
}

private func browserDiffNode(
    id: UInt64,
    parentID: UInt64?,
    depth: UInt32,
    kind: ExplorerSnapshotNodeKind,
    name: String,
    category: ExplorerStorageCategory = .unclassified,
    change: ExplorerSnapshotDiffChange,
    currentLogicalBytes: UInt64?,
    baselineLogicalBytes: UInt64?,
    childCount: UInt64 = 0,
    canDescend: Bool = false
) -> ExplorerSnapshotDiffNode {
    let logicalChange: ExplorerSnapshotDiffValue
    switch (currentLogicalBytes, baselineLogicalBytes) {
    case let (current?, baseline?) where current > baseline:
        logicalChange = ExplorerSnapshotDiffValue(
            direction: .growth,
            magnitudeBytes: current - baseline
        )
    case let (current?, baseline?) where baseline > current:
        logicalChange = ExplorerSnapshotDiffValue(
            direction: .shrinkage,
            magnitudeBytes: baseline - current
        )
    case let (current?, nil):
        logicalChange = ExplorerSnapshotDiffValue(
            direction: .growth,
            magnitudeBytes: current
        )
    case let (nil, baseline?):
        logicalChange = ExplorerSnapshotDiffValue(
            direction: .shrinkage,
            magnitudeBytes: baseline
        )
    default:
        logicalChange = ExplorerSnapshotDiffValue(
            direction: .unchanged,
            magnitudeBytes: 0
        )
    }
    let flags = ExplorerSnapshotScanFlags(
        inaccessible: false,
        timedOut: false,
        hardLinkDuplicate: false,
        mountBoundary: false
    )
    return ExplorerSnapshotDiffNode(
        id: id,
        parentID: parentID,
        depth: depth,
        name: browserNodeName(name),
        kind: kind,
        currentKind: currentLogicalBytes == nil ? nil : kind,
        baselineKind: baselineLogicalBytes == nil ? nil : kind,
        category: category,
        change: change,
        logicalChange: logicalChange,
        currentLogicalBytes: currentLogicalBytes,
        baselineLogicalBytes: baselineLogicalBytes,
        currentAllocatedBytes: currentLogicalBytes,
        baselineAllocatedBytes: baselineLogicalBytes,
        allocatedChange: logicalChange,
        currentFileCount: currentLogicalBytes == nil ? nil : max(childCount, 1),
        baselineFileCount: baselineLogicalBytes == nil ? nil : max(childCount, 1),
        currentChildCount: currentLogicalBytes == nil ? nil : childCount,
        baselineChildCount: baselineLogicalBytes == nil ? nil : childCount,
        currentScanFlags: currentLogicalBytes == nil ? nil : flags,
        baselineScanFlags: baselineLogicalBytes == nil ? nil : flags,
        canDescend: canDescend
    )
}

private func browserDiffNodes(
    parentID: UInt64,
    sort: ExplorerSnapshotDiffSort
) -> [ExplorerSnapshotDiffNode] {
    let nodes: [ExplorerSnapshotDiffNode]
    if parentID == 1 {
        nodes = [
            browserDiffNode(
                id: 500,
                parentID: 1,
                depth: 2,
                kind: .file,
                name: "nested.log",
                category: .developerArtifact,
                change: .removed,
                currentLogicalBytes: nil,
                baselineLogicalBytes: 5_000
            ),
        ]
    } else {
        var rootNodes = [
            browserDiffNode(
                id: 1,
                parentID: 0,
                depth: 1,
                kind: .directory,
                name: "Folder",
                category: .developerArtifact,
                change: .grew,
                currentLogicalBytes: 5_000,
                baselineLogicalBytes: 4_700,
                childCount: 1,
                canDescend: true
            ),
        ]
        rootNodes.append(contentsOf: (2 ... 101).map { index in
            let current = UInt64(1_000 - index)
            let grew = index.isMultiple(of: 2)
            return browserDiffNode(
                id: UInt64(index),
                parentID: 0,
                depth: 1,
                kind: .file,
                name: "item-\(String(format: "%03d", index))",
                change: grew ? .grew : .shrank,
                currentLogicalBytes: current,
                baselineLogicalBytes: grew ? current - 10 : current + 10
            )
        })
        nodes = rootNodes
    }
    return switch sort {
    case .magnitudeDescending:
        nodes.sorted {
            if $0.logicalChange.magnitudeBytes == $1.logicalChange.magnitudeBytes {
                return $0.id < $1.id
            }
            return $0.logicalChange.magnitudeBytes > $1.logicalChange.magnitudeBytes
        }
    case .nameAscending:
        nodes.sorted {
            if $0.name.display == $1.name.display {
                return $0.id < $1.id
            }
            return $0.name.display < $1.name.display
        }
    case .currentBytesDescending:
        nodes.sorted {
            let lhs = $0.currentLogicalBytes ?? 0
            let rhs = $1.currentLogicalBytes ?? 0
            return lhs == rhs ? $0.id < $1.id : lhs > rhs
        }
    }
}

private func browserDiffTotals(
    _ nodes: [ExplorerSnapshotDiffNode]
) -> (growth: UInt64, shrinkage: UInt64, unchanged: UInt64, replaced: UInt64) {
    nodes.reduce(
        into: (UInt64(0), UInt64(0), UInt64(0), UInt64(0))
    ) { totals, node in
        switch node.logicalChange.direction {
        case .growth:
            totals.0 += node.logicalChange.magnitudeBytes
        case .shrinkage:
            totals.1 += node.logicalChange.magnitudeBytes
        case .unchanged:
            totals.2 += 1
        }
        if node.change == .replaced {
            totals.3 += 1
        }
    }
}

private func browserNodeName(_ value: String) -> ExplorerSnapshotNodeName {
    ExplorerSnapshotNodeName(
        encoding: .unixBytes,
        encodedBytes: Data(value.utf8),
        display: value
    )
}

private func browserICloudObservationTargets() -> [ExplorerICloudObservationTarget] {
    [
        ExplorerICloudObservationTarget(
            rank: 0,
            node: browserNode(
                id: 910,
                parentID: 0,
                depth: 1,
                kind: .file,
                name: "local-video.mov",
                logicalBytes: 24_576
            ),
            parentContext: [browserNodeName("/Users/example")],
            contextTruncated: false
        ),
        ExplorerICloudObservationTarget(
            rank: 1,
            node: browserNode(
                id: 911,
                parentID: 0,
                depth: 1,
                kind: .file,
                name: "local-archive.zip",
                logicalBytes: 16_384
            ),
            parentContext: [browserNodeName("/Users/example")],
            contextTruncated: false
        ),
        ExplorerICloudObservationTarget(
            rank: 2,
            node: browserNode(
                id: 912,
                parentID: 0,
                depth: 1,
                kind: .file,
                name: "local-document.pdf",
                logicalBytes: 8_192
            ),
            parentContext: [browserNodeName("/Users/example")],
            contextTruncated: false
        ),
    ]
}

private func browserNode(
    _ node: ExplorerSnapshotNode,
    replacingCategoryWith category: ExplorerStorageCategory
) -> ExplorerSnapshotNode {
    ExplorerSnapshotNode(
        id: node.id,
        parentID: node.parentID,
        depth: node.depth,
        kind: node.kind,
        category: category,
        name: node.name,
        logicalBytes: node.logicalBytes,
        allocatedBytes: node.allocatedBytes,
        fileCount: node.fileCount,
        childCount: node.childCount,
        modifiedAt: node.modifiedAt,
        accessedAt: node.accessedAt,
        scanFlags: node.scanFlags
    )
}
