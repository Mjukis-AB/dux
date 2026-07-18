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
        case treemapCategoryMismatch
        case offPageCategoryMismatch
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
        case largeFiles(
            scanID: String,
            minimumLogicalBytes: UInt64,
            modifiedBefore: ExplorerSnapshotTimestamp?,
            maxResults: UInt16
        )
        case liveItem(
            scanID: String,
            nodeID: UInt64,
            purpose: ExplorerSnapshotLivePathPurpose
        )
        case release(scanID: String)
    }

    private let mode: Mode
    private var calls: [Call] = []
    private var released: [String] = []
    private var acquisitionContinuation: CheckedContinuation<Void, Never>?
    private var queryContinuation: CheckedContinuation<Void, Never>?
    private var treemapContinuation: CheckedContinuation<Void, Never>?
    private var releaseContinuation: CheckedContinuation<Void, Never>?
    private var largeFilesContinuation: CheckedContinuation<Void, Never>?
    private var liveActionContinuation: CheckedContinuation<Void, Never>?
    private var didSuspendRelease = false
    private var didSuspendQuery = false
    private var rootFirstPageRequestCount = 0

    init(mode: Mode = .available) {
        self.mode = mode
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
            let projectedNode = mode == .treemapCategoryMismatch && node.id == 1
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

    func hasSuspendedRelease() -> Bool {
        releaseContinuation != nil
    }

    func resumeRelease() {
        releaseContinuation?.resume()
        releaseContinuation = nil
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
    private var limits: [UInt16] = []

    init(hasMore: Bool = false, fails: Bool = false, overLimit: Bool = false) {
        self.hasMore = hasMore
        self.fails = fails
        self.overLimit = overLimit
    }

    func loadRecentSnapshotHistory(limit: UInt16) async throws -> ExplorerSnapshotHistoryPage {
        limits.append(limit)
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

private func browserNodeName(_ value: String) -> ExplorerSnapshotNodeName {
    ExplorerSnapshotNodeName(
        encoding: .unixBytes,
        encodedBytes: Data(value.utf8),
        display: value
    )
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
