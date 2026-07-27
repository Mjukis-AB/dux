import Foundation
import Observation

protocol DuxSnapshotReviewBrowsing: Sendable {
    func acquire(scanID: String) async throws
    func acquireLatest() async throws -> String
    func release(scanID: String) async
    func rootNode(scanID: String) async throws -> ExplorerSnapshotNode
    func candidateSummaries(
        scanID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateSummaryPage
    func reviewCandidate(
        scanID: String,
        candidateID: String,
        command: ExplorerCandidateReviewCommand
    ) async throws -> ExplorerCandidateReviewResult
    func childNodes(
        scanID: String,
        parentID: UInt64,
        sort: ExplorerSnapshotNodeSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotNodePage
    func treemap(
        scanID: String,
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotTreemap
    func largeFiles(
        scanID: String,
        minimumLogicalBytes: UInt64,
        modifiedBefore: ExplorerSnapshotTimestamp?,
        maxResults: UInt16
    ) async throws -> ExplorerSnapshotLargeFilesPage
    func candidatePaths(
        scanID: String,
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidatePathPage
    func candidateEvidence(
        scanID: String,
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateEvidencePage
    func resolveLiveItem(
        scanID: String,
        nodeID: UInt64,
        purpose: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem
    func executeTrash(scanID: String, nodeID: UInt64) async throws -> TrashPlatformResult
}

extension DuxSnapshotReviewBrowsing {
    func reviewCandidate(
        scanID _: String,
        candidateID _: String,
        command _: ExplorerCandidateReviewCommand
    ) async throws -> ExplorerCandidateReviewResult {
        throw ExplorerCandidateDetailError.unavailable
    }

    func resolveLiveItem(
        scanID _: String,
        nodeID _: UInt64,
        purpose _: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem {
        throw ExplorerSnapshotLivePathError.unavailable
    }

    func executeTrash(scanID _: String, nodeID _: UInt64) async throws -> TrashPlatformResult {
        throw ExplorerTrashError.unavailable
    }
}

struct UnavailableDuxSnapshotHistoryBrowser: DuxSnapshotHistoryServing {
    func loadRecentSnapshotHistory(limit _: UInt16) async throws -> ExplorerSnapshotHistoryPage {
        throw EngineServiceError.unavailable
    }
}

struct UnavailableDuxScanCoverageBrowser: DuxScanCoverageServing {
    func loadScanCoverageDetails(scanID _: String) async throws
        -> ExplorerScanCoverageDetails
    {
        throw ExplorerScanCoverageError.unavailable
    }
}

@MainActor
protocol ExplorerSubtreeScanDriving: AnyObject {
    func startSubtreeScan(
        sourceScanID: String,
        nodeID: UInt64,
        displayName: String,
        using service: any DuxSnapshotSubtreeScanServing
    ) async -> AppScanRunOutcome
}

extension AppModel: ExplorerSubtreeScanDriving {}

private struct UnavailableDuxSnapshotSubtreeScanService: DuxSnapshotSubtreeScanServing {
    func startSubtreeScan(
        sourceScanID _: String,
        nodeID _: UInt64
    ) async throws -> HomeScanStartDisposition {
        throw HomeScanServiceError.internalState
    }
}

enum ExplorerSnapshotBrowserFailure: Equatable, Sendable {
    case noSnapshot
    case expired
    case busy
    case budgetExceeded
    case unavailable
    case invalidResponse

    var title: String {
        switch self {
        case .noSnapshot: "No saved scan yet"
        case .expired: "Snapshot review expired"
        case .busy: "Explorer is busy"
        case .budgetExceeded: "Snapshot exceeds the browsing budget"
        case .unavailable: "Snapshot unavailable"
        case .invalidResponse: "Snapshot could not be displayed"
        }
    }

    var detail: String {
        switch self {
        case .noSnapshot:
            "Run a Home scan to create a read-only snapshot, then try again."
        case .expired:
            "The retained review ended. Reload the latest available snapshot."
        case .busy:
            "DUX is protecting its resource budget. Wait a moment and retry."
        case .budgetExceeded:
            "This folder or snapshot is too large for the current bounded browser. No files were changed."
        case .unavailable:
            "The saved snapshot cannot be read right now. No files were changed."
        case .invalidResponse:
            "DUX rejected inconsistent snapshot data instead of presenting it."
        }
    }
}

enum ExplorerSnapshotBrowserPhase: Equatable, Sendable {
    case idle
    case loading
    case ready
    case failed(ExplorerSnapshotBrowserFailure)
}

enum ExplorerSnapshotSelection: Equatable, Sendable {
    case node(UInt64)
    case largeFile(UInt64)
    case other
}

struct ExplorerLiveActionNotice: Equatable, Sendable {
    let message: String
    let isFailure: Bool
}

enum ExplorerTrashError: Error, Equatable, Sendable {
    case closed
    case reviewNotAcquired
    case unavailable
    case invalidRequest
    case changedSincePlan
    case busy
    case storageUnavailable
    case unsafeStorage
    case incompatibleSchema
    case corruptData
    case outcomeUnknown
    case failed

    init(_ error: Error) {
        guard let error = error as? TrashExecutionError else {
            self = (error as? ExplorerTrashError) ?? .failed
            return
        }
        self = switch error {
        case .Closed: .closed
        case .InvalidRequest: .invalidRequest
        case .ChangedSincePlan: .changedSincePlan
        case .ReviewUnavailable: .unavailable
        case .Busy: .busy
        case .StorageUnavailable: .storageUnavailable
        case .UnsafeStorage: .unsafeStorage
        case .IncompatibleSchema: .incompatibleSchema
        case .CorruptData: .corruptData
        case .OutcomeUnknown: .outcomeUnknown
        case .InternalState: .failed
        }
    }
}

private enum ExplorerSnapshotTreemapLoadResult: Sendable {
    case ready(ExplorerSnapshotTreemap)
    case failed(ExplorerSnapshotBrowserFailure)
}

struct UnavailableDuxSnapshotReviewBrowser: DuxSnapshotReviewBrowsing {
    func acquire(scanID _: String) async throws {
        throw ExplorerSnapshotReviewAcquisitionError.snapshotUnavailable
    }

    func acquireLatest() async throws -> String {
        throw ExplorerSnapshotReviewAcquisitionError.snapshotUnavailable
    }

    func release(scanID _: String) async {}

    func rootNode(scanID _: String) async throws -> ExplorerSnapshotNode {
        throw ExplorerSnapshotNodeError.reviewNotAcquired
    }

    func candidateSummaries(
        scanID _: String,
        cursor _: UInt16,
        limit _: UInt16
    ) async throws -> ExplorerCandidateSummaryPage {
        throw ExplorerCandidateDetailError.reviewNotAcquired
    }

    func reviewCandidate(
        scanID _: String,
        candidateID _: String,
        command _: ExplorerCandidateReviewCommand
    ) async throws -> ExplorerCandidateReviewResult {
        throw ExplorerCandidateDetailError.reviewNotAcquired
    }

    func childNodes(
        scanID _: String,
        parentID _: UInt64,
        sort _: ExplorerSnapshotNodeSort,
        offset _: UInt64,
        limit _: UInt16
    ) async throws -> ExplorerSnapshotNodePage {
        throw ExplorerSnapshotNodeError.reviewNotAcquired
    }

    func treemap(
        scanID _: String,
        parentID _: UInt64,
        maxCells _: UInt16
    ) async throws -> ExplorerSnapshotTreemap {
        throw ExplorerSnapshotTreemapError.reviewNotAcquired
    }

    func largeFiles(
        scanID _: String,
        minimumLogicalBytes _: UInt64,
        modifiedBefore _: ExplorerSnapshotTimestamp?,
        maxResults _: UInt16
    ) async throws -> ExplorerSnapshotLargeFilesPage {
        throw ExplorerSnapshotLargeFilesError.reviewNotAcquired
    }

    func candidatePaths(
        scanID _: String,
        candidateID _: String,
        cursor _: UInt16,
        limit _: UInt16
    ) async throws -> ExplorerCandidatePathPage {
        throw ExplorerCandidateDetailError.reviewNotAcquired
    }

    func candidateEvidence(
        scanID _: String,
        candidateID _: String,
        cursor _: UInt16,
        limit _: UInt16
    ) async throws -> ExplorerCandidateEvidencePage {
        throw ExplorerCandidateDetailError.reviewNotAcquired
    }

    func resolveLiveItem(
        scanID _: String,
        nodeID _: UInt64,
        purpose _: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem {
        throw ExplorerSnapshotLivePathError.reviewNotAcquired
    }
}

@MainActor
@Observable
final class ExplorerSnapshotBrowserModel {
    static let pageLimit: UInt16 = 100
    static let treemapCellLimit: UInt16 = 48
    static let historyLimit: UInt16 = 50
    static let largeFileResultLimit: UInt16 = 100

    private(set) var phase = ExplorerSnapshotBrowserPhase.idle
    private(set) var scanID: String?
    private(set) var isLatestSnapshot = false
    private(set) var breadcrumbs: [ExplorerSnapshotNode] = []
    private(set) var nodes: [ExplorerSnapshotNode] = []
    private(set) var pageOffset: UInt64 = 0
    private(set) var totalChildren: UInt64 = 0
    private(set) var sort = ExplorerSnapshotNodeSort.logicalBytesDescending
    private(set) var isNavigating = false
    private(set) var isPaging = false
    private(set) var operationFailure: ExplorerSnapshotBrowserFailure?
    private(set) var treemap: ExplorerSnapshotTreemap?
    private(set) var treemapFailure: ExplorerSnapshotBrowserFailure?
    private(set) var isTreemapLoading = false
    private(set) var selection: ExplorerSnapshotSelection?
    private(set) var selectedNodeSnapshot: ExplorerSnapshotNode?
    private(set) var historyScans: [ExplorerHistoricalScan] = []
    private(set) var historyHasMore = false
    private(set) var isHistoryLoading = false
    private(set) var historyFailure: ExplorerSnapshotBrowserFailure?
    private(set) var unavailableHistoricalScanIDs: Set<String> = []
    private(set) var isSwitchingSnapshot = false
    private(set) var contentMode = ExplorerSnapshotContentMode.browse
    private(set) var largeFileThreshold = ExplorerSnapshotLargeFileThreshold.gibibyte1
    private(set) var largeFileAge = ExplorerSnapshotLargeFileAge.any
    private(set) var largeFilesPage: ExplorerSnapshotLargeFilesPage?
    private(set) var largeFilesFailure: ExplorerSnapshotBrowserFailure?
    private(set) var isLargeFilesLoading = false
    private(set) var coverageDetails: ExplorerScanCoverageDetails?
    private(set) var coverageFailure: ExplorerSnapshotBrowserFailure?
    private(set) var isCoverageLoading = false
    private(set) var isLiveActionLoading = false
    private(set) var liveActionNotice: ExplorerLiveActionNotice?
    private(set) var isTrashLoading = false
    private(set) var trashNotice: ExplorerLiveActionNotice?
    private(set) var isSubtreeRefreshRunning = false
    private(set) var subtreeRefreshNotice: ExplorerLiveActionNotice?

    private let reviews: any DuxSnapshotReviewBrowsing
    private let history: any DuxSnapshotHistoryServing
    private let coverage: any DuxScanCoverageServing
    private let liveActions: any ExplorerLiveFileActionPresenting
    private let subtreeScans: any DuxSnapshotSubtreeScanServing
    private let scanDriver: (any ExplorerSubtreeScanDriving)?

    @ObservationIgnored
    private var generation: UInt64 = 0
    @ObservationIgnored
    private var activePresentationID: UUID?
    @ObservationIgnored
    private var historyGeneration: UInt64 = 0
    @ObservationIgnored
    private var largeFilesGeneration: UInt64 = 0
    @ObservationIgnored
    private var coverageGeneration: UInt64 = 0
    @ObservationIgnored
    private var liveActionGeneration: UInt64 = 0
    @ObservationIgnored
    private var subtreeRefreshGeneration: UInt64 = 0

    init(
        reviews: any DuxSnapshotReviewBrowsing,
        history: any DuxSnapshotHistoryServing = UnavailableDuxSnapshotHistoryBrowser(),
        coverage: any DuxScanCoverageServing = UnavailableDuxScanCoverageBrowser(),
        liveActions: any ExplorerLiveFileActionPresenting =
            UnavailableExplorerLiveFileActionPresenter(),
        subtreeScans: any DuxSnapshotSubtreeScanServing =
            UnavailableDuxSnapshotSubtreeScanService(),
        scanDriver: (any ExplorerSubtreeScanDriving)? = nil
    ) {
        self.reviews = reviews
        self.history = history
        self.coverage = coverage
        self.liveActions = liveActions
        self.subtreeScans = subtreeScans
        self.scanDriver = scanDriver
    }

    var currentDirectory: ExplorerSnapshotNode? {
        breadcrumbs.last
    }

    var hasPreviousPage: Bool {
        pageOffset > 0
    }

    var hasNextPage: Bool {
        pageOffset + UInt64(nodes.count) < totalChildren
    }

    var selectedNodeID: UInt64? {
        switch selection {
        case let .node(id), let .largeFile(id):
            return id
        case .other, nil:
            return nil
        }
    }

    var selectedNode: ExplorerSnapshotNode? {
        guard let selectedNodeID else {
            return nil
        }
        return nodes.first { $0.id == selectedNodeID }
            ?? treemap?.cell(nodeID: selectedNodeID)?.node
            ?? selectedNodeSnapshot
    }

    var isOtherSelected: Bool {
        switch selection {
        case .other:
            true
        case let .node(id):
            treemap?.hasOther == true && treemap?.cell(nodeID: id) == nil
        case .largeFile:
            false
        case nil:
            false
        }
    }

    var selectedHistoricalScan: ExplorerHistoricalScan? {
        guard let scanID else {
            return nil
        }
        return historyScans.first { $0.scanID == scanID }
    }

    var canRevealSelectedLiveItem: Bool {
        canUseLiveAction(selectedNode, requiresFile: false)
    }

    var canCopySelectedLivePath: Bool {
        canUseLiveAction(selectedNode, requiresFile: false)
    }

    var canQuickLookSelectedLiveItem: Bool {
        canUseLiveAction(selectedNode, requiresFile: true)
    }

    var canTrashSelectedItem: Bool {
        phase == .ready
            && selectedNode.map { $0.kind == .file || $0.kind == .directory || $0.kind == .symlink } == true
            && !isTrashLoading
            && !isSwitchingSnapshot
            && !isNavigating
            && !isPaging
    }

    var canRefreshCurrentSubtree: Bool {
        phase == .ready
            && currentDirectory?.kind == .directory
            && scanDriver != nil
            && !isSubtreeRefreshRunning
            && !isSwitchingSnapshot
            && !isNavigating
            && !isPaging
    }

    func openLatestIfNeeded() async {
        guard phase == .idle else {
            return
        }
        await reloadLatest()
    }

    func present(id: UUID) async {
        guard activePresentationID != id else {
            return
        }
        activePresentationID = id
        async let snapshot: Void = reloadLatest()
        async let history: Void = reloadHistory()
        _ = await (snapshot, history)
    }

    func dismiss(id: UUID) async {
        guard activePresentationID == id else {
            return
        }
        activePresentationID = nil
        await close()
    }

    func reloadLatest(ifPresented id: UUID) async {
        guard activePresentationID == id else {
            return
        }
        async let snapshot: Void = reloadLatest()
        async let history: Void = reloadHistory()
        _ = await (snapshot, history)
    }

    func reloadHistory() async {
        historyGeneration &+= 1
        let operation = historyGeneration
        isHistoryLoading = true
        historyFailure = nil
        do {
            let page = try await history.loadRecentSnapshotHistory(limit: Self.historyLimit)
            guard operation == historyGeneration, !Task.isCancelled else {
                return
            }
            guard page.scans.count <= Int(Self.historyLimit) else {
                throw ExplorerSnapshotHistoryError.invalidResponse
            }
            historyScans = page.scans
            historyHasMore = page.hasMore
            unavailableHistoricalScanIDs = []
            isHistoryLoading = false
        } catch {
            guard operation == historyGeneration, !Task.isCancelled else {
                return
            }
            isHistoryLoading = false
            historyFailure = Self.failure(for: error)
        }
    }

    func selectHistoricalScan(_ historicalScan: ExplorerHistoricalScan) async {
        guard
            let confirmedScan = historyScans.first(where: {
                $0.scanID == historicalScan.scanID && $0 == historicalScan
            }),
            confirmedScan.canRequestReview,
            phase != .loading,
            historicalScan.scanID != scanID,
            !unavailableHistoricalScanIDs.contains(historicalScan.scanID),
            !isSwitchingSnapshot,
            !isNavigating,
            !isPaging
        else {
            return
        }

        invalidateLiveAction()
        generation &+= 1
        let operation = generation
        let requestedScanID = historicalScan.scanID
        let previousScanID = scanID
        isSwitchingSnapshot = true
        operationFailure = nil
        var acquired = false
        do {
            try await reviews.acquire(scanID: requestedScanID)
            acquired = true
            guard operation == generation, !Task.isCancelled else {
                await reviews.release(scanID: requestedScanID)
                if operation == generation {
                    isSwitchingSnapshot = false
                }
                return
            }
            let root = try await reviews.rootNode(scanID: requestedScanID)
            let page = try await reviews.childNodes(
                scanID: requestedScanID,
                parentID: root.id,
                sort: sort,
                offset: 0,
                limit: Self.pageLimit
            )
            guard page.totalChildren == root.childCount else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            let treemapResult = try await loadTreemap(scanID: requestedScanID, directory: root)
            guard operation == generation, !Task.isCancelled else {
                await reviews.release(scanID: requestedScanID)
                if operation == generation {
                    isSwitchingSnapshot = false
                }
                return
            }

            clearLargeFiles()
            clearCoverage()
            scanID = requestedScanID
            isLatestSnapshot = false
            breadcrumbs = [root]
            publishPage(page)
            publishTreemap(treemapResult, directory: root, page: page)
            selection = nil
            selectedNodeSnapshot = nil
            phase = .ready
            if let previousScanID {
                await reviews.release(scanID: previousScanID)
            }
            guard operation == generation, self.scanID == requestedScanID else {
                return
            }
            isSwitchingSnapshot = false
            if contentMode == .largeFiles {
                await reloadLargeFiles()
            } else if contentMode == .coverage {
                await reloadCoverage()
            }
        } catch {
            if acquired {
                await reviews.release(scanID: requestedScanID)
            }
            guard operation == generation, !Task.isCancelled else {
                if operation == generation {
                    isSwitchingSnapshot = false
                }
                return
            }
            isSwitchingSnapshot = false
            let failure = Self.failure(for: error)
            if failure == .noSnapshot || failure == .expired || failure == .unavailable {
                unavailableHistoricalScanIDs.insert(requestedScanID)
            }
            operationFailure = failure == .noSnapshot ? .unavailable : failure
        }
    }

    func reloadLatest() async {
        generation &+= 1
        let operation = generation
        let previousScanID = scanID
        clearContent()
        phase = .loading
        if let previousScanID {
            await reviews.release(scanID: previousScanID)
            guard operation == generation, !Task.isCancelled else {
                return
            }
        }

        var acquiredScanID: String?
        do {
            let latestScanID = try await reviews.acquireLatest()
            acquiredScanID = latestScanID
            guard operation == generation, !Task.isCancelled else {
                await reviews.release(scanID: latestScanID)
                return
            }
            let root = try await reviews.rootNode(scanID: latestScanID)
            let page = try await reviews.childNodes(
                scanID: latestScanID,
                parentID: root.id,
                sort: sort,
                offset: 0,
                limit: Self.pageLimit
            )
            guard page.totalChildren == root.childCount else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            let treemapResult = try await loadTreemap(
                scanID: latestScanID,
                directory: root
            )
            guard operation == generation, !Task.isCancelled else {
                await reviews.release(scanID: latestScanID)
                return
            }
            scanID = latestScanID
            isLatestSnapshot = true
            breadcrumbs = [root]
            publishPage(page)
            publishTreemap(treemapResult, directory: root, page: page)
            phase = .ready
            if contentMode == .largeFiles {
                await reloadLargeFiles()
            } else if contentMode == .coverage {
                await reloadCoverage()
            }
        } catch {
            if let acquiredScanID {
                await reviews.release(scanID: acquiredScanID)
            }
            guard operation == generation, !Task.isCancelled else {
                return
            }
            clearContent()
            phase = .failed(Self.failure(for: error))
        }
    }

    func openDirectory(_ node: ExplorerSnapshotNode) async {
        guard node.kind == .directory, !isSwitchingSnapshot else {
            return
        }
        await replaceCurrentDirectory(with: node, breadcrumbIndex: nil)
    }

    func goToBreadcrumb(at index: Int) async {
        guard breadcrumbs.indices.contains(index), !isSwitchingSnapshot else {
            return
        }
        await replaceCurrentDirectory(
            with: breadcrumbs[index],
            breadcrumbIndex: index
        )
    }

    func goBack() async {
        guard breadcrumbs.count > 1, !isSwitchingSnapshot else {
            return
        }
        await goToBreadcrumb(at: breadcrumbs.count - 2)
    }

    func selectSort(_ newSort: ExplorerSnapshotNodeSort) async {
        guard
            newSort != sort,
            let currentDirectory,
            !isSwitchingSnapshot,
            !isNavigating,
            !isPaging
        else {
            return
        }
        let previousSort = sort
        sort = newSort
        let result = await reloadDirectory(currentDirectory, reloadTreemap: false)
        if !result.succeeded, result.operation == generation {
            sort = previousSort
        }
    }

    func showNextPage() async {
        guard
            phase == .ready,
            hasNextPage,
            !isPaging,
            !isNavigating,
            !isSwitchingSnapshot,
            !nodes.isEmpty
        else {
            return
        }
        await moveToPage(offset: pageOffset + UInt64(nodes.count))
    }

    func showPreviousPage() async {
        guard
            phase == .ready,
            hasPreviousPage,
            !isPaging,
            !isNavigating,
            !isSwitchingSnapshot
        else {
            return
        }
        await moveToPage(
            offset: pageOffset >= UInt64(Self.pageLimit)
                ? pageOffset - UInt64(Self.pageLimit)
                : 0
        )
    }

    func dismissOperationFailure() {
        operationFailure = nil
    }

    func dismissTreemapFailure() {
        treemapFailure = nil
    }

    func selectContentMode(_ mode: ExplorerSnapshotContentMode) async {
        guard mode != contentMode, phase == .ready, !isSwitchingSnapshot else {
            return
        }
        invalidateLiveAction()
        contentMode = mode
        selection = nil
        selectedNodeSnapshot = nil
        largeFilesGeneration &+= 1
        isLargeFilesLoading = false
        coverageGeneration &+= 1
        isCoverageLoading = false
        switch mode {
        case .browse:
            return
        case .largeFiles:
            if largeFilesPage == nil {
                await reloadLargeFiles()
            }
        case .coverage:
            if coverageDetails == nil {
                await reloadCoverage()
            }
        }
    }

    func reloadCoverage() async {
        guard
            phase == .ready,
            contentMode == .coverage,
            let scanID,
            !isSwitchingSnapshot
        else {
            return
        }
        coverageGeneration &+= 1
        let operation = coverageGeneration
        let snapshotGeneration = generation
        coverageDetails = nil
        coverageFailure = nil
        isCoverageLoading = true
        do {
            let details = try await coverage.loadScanCoverageDetails(scanID: scanID)
            guard
                operation == coverageGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .coverage,
                details.scanID == scanID,
                !Task.isCancelled
            else {
                return
            }
            coverageDetails = details
            coverageFailure = nil
            isCoverageLoading = false
        } catch {
            guard
                operation == coverageGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .coverage,
                !Task.isCancelled
            else {
                return
            }
            isCoverageLoading = false
            coverageFailure = Self.failure(for: error)
        }
    }

    func selectLargeFileThreshold(_ threshold: ExplorerSnapshotLargeFileThreshold) async {
        guard threshold != largeFileThreshold else {
            return
        }
        invalidateLiveAction()
        largeFileThreshold = threshold
        clearLargeFiles()
        if contentMode == .largeFiles {
            await reloadLargeFiles()
        }
    }

    func selectLargeFileAge(_ age: ExplorerSnapshotLargeFileAge, now: Date = .now) async {
        guard age != largeFileAge else {
            return
        }
        invalidateLiveAction()
        largeFileAge = age
        clearLargeFiles()
        if contentMode == .largeFiles {
            await reloadLargeFiles(now: now)
        }
    }

    func reloadLargeFiles(now: Date = .now) async {
        guard
            phase == .ready,
            contentMode == .largeFiles,
            let scanID,
            !isSwitchingSnapshot
        else {
            return
        }
        invalidateLiveAction()
        largeFilesGeneration &+= 1
        let operation = largeFilesGeneration
        let snapshotGeneration = generation
        let threshold = largeFileThreshold.rawValue
        let modifiedBefore = largeFileAge.modifiedBefore(now: now)
        largeFilesPage = nil
        isLargeFilesLoading = true
        largeFilesFailure = nil
        do {
            let page = try await reviews.largeFiles(
                scanID: scanID,
                minimumLogicalBytes: threshold,
                modifiedBefore: modifiedBefore,
                maxResults: Self.largeFileResultLimit
            )
            guard
                operation == largeFilesGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .largeFiles,
                largeFileThreshold.rawValue == threshold,
                largeFileAge.modifiedBefore(now: now) == modifiedBefore,
                !Task.isCancelled
            else {
                return
            }
            largeFilesPage = page
            largeFilesFailure = nil
            isLargeFilesLoading = false
            selection = nil
            selectedNodeSnapshot = nil
        } catch {
            guard
                operation == largeFilesGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                !Task.isCancelled
            else {
                return
            }
            isLargeFilesLoading = false
            let failure = Self.failure(for: error)
            if failure == .expired {
                await reviews.release(scanID: scanID)
                guard
                    operation == largeFilesGeneration,
                    snapshotGeneration == generation,
                    self.scanID == scanID
                else {
                    return
                }
                clearContent()
                phase = .failed(.expired)
            } else {
                largeFilesFailure = failure
            }
        }
    }

    func selectTableNode(_ nodeID: UInt64?) {
        invalidateLiveAction()
        guard let nodeID else {
            selection = nil
            selectedNodeSnapshot = nil
            return
        }
        guard let node = nodes.first(where: { $0.id == nodeID }) else {
            return
        }
        selection = .node(nodeID)
        selectedNodeSnapshot = node
    }

    func selectLargeFile(_ nodeID: UInt64?) {
        invalidateLiveAction()
        guard let nodeID else {
            selection = nil
            selectedNodeSnapshot = nil
            return
        }
        guard let file = largeFilesPage?.files.first(where: { $0.id == nodeID }) else {
            return
        }
        selection = .largeFile(nodeID)
        selectedNodeSnapshot = file.node
    }

    func selectOther() {
        guard treemap?.hasOther == true else {
            return
        }
        invalidateLiveAction()
        selection = .other
        selectedNodeSnapshot = nil
    }

    func selectTreemapCell(_ cell: ExplorerSnapshotTreemapCell) async {
        guard
            phase == .ready,
            treemap?.cell(nodeID: cell.id) == cell,
            !isNavigating,
            !isSwitchingSnapshot,
            !isPaging
        else {
            return
        }
        invalidateLiveAction()
        selection = .node(cell.id)
        selectedNodeSnapshot = cell.node
        guard !nodes.contains(where: { $0.id == cell.id }) else {
            return
        }
        await revealTreemapCell(cell)
    }

    func refreshCurrentSubtree() async {
        guard
            canRefreshCurrentSubtree,
            let scanDriver,
            let sourceScanID = scanID,
            let sourceDirectory = currentDirectory
        else {
            return
        }

        subtreeRefreshGeneration &+= 1
        let refreshOperation = subtreeRefreshGeneration
        let sourceGeneration = generation
        let sourceNodeID = sourceDirectory.id
        let displayName = sourceDirectory.name.display
        isSubtreeRefreshRunning = true
        subtreeRefreshNotice = nil

        let outcome = await scanDriver.startSubtreeScan(
            sourceScanID: sourceScanID,
            nodeID: sourceNodeID,
            displayName: displayName,
            using: subtreeScans
        )
        guard refreshOperation == subtreeRefreshGeneration else {
            return
        }
        isSubtreeRefreshRunning = false

        switch outcome {
        case let .succeeded(summary):
            guard summary.snapshotAvailable else {
                subtreeRefreshNotice = Self.subtreeRefreshFailureNotice(.snapshotRejected)
                return
            }
            guard
                sourceGeneration == generation,
                scanID == sourceScanID,
                currentDirectory?.id == sourceNodeID,
                phase == .ready,
                !Task.isCancelled
            else {
                subtreeRefreshNotice = ExplorerLiveActionNotice(
                    message: "The folder scan finished and is available in Recent Scans. This view was not replaced because you navigated elsewhere.",
                    isFailure: false
                )
                return
            }
            await installCompletedSubtreeSnapshot(
                scanID: summary.scanID,
                sourceScanID: sourceScanID,
                refreshOperation: refreshOperation
            )
        case .cancelled:
            subtreeRefreshNotice = ExplorerLiveActionNotice(
                message: "Folder scan stopped. The previous snapshot remains visible.",
                isFailure: false
            )
        case let .failed(failure):
            subtreeRefreshNotice = Self.subtreeRefreshFailureNotice(failure)
        case .superseded:
            subtreeRefreshNotice = ExplorerLiveActionNotice(
                message: "The folder scan changed before it could update this view. The previous snapshot remains visible.",
                isFailure: true
            )
        }
    }

    func dismissSubtreeRefreshNotice() {
        subtreeRefreshNotice = nil
    }

    func revealLiveItem(nodeID: UInt64? = nil) async {
        await performLiveAction(.reveal, requestedNodeID: nodeID)
    }

    func copyLiveItemPath(nodeID: UInt64? = nil) async {
        await performLiveAction(.copyPath, requestedNodeID: nodeID)
    }

    func quickLookLiveItem(nodeID: UInt64? = nil) async {
        await performLiveAction(.quickLook, requestedNodeID: nodeID)
    }

    func trashSelectedItem(nodeID: UInt64? = nil) async {
        guard
            canTrashSelectedItem,
            let scanID,
            let node = liveActionNode(nodeID),
            node.kind == .file || node.kind == .directory || node.kind == .symlink
        else { return }
        let snapshotOperation = generation
        let nodeID = node.id
        isTrashLoading = true
        trashNotice = nil
        do {
            let result = try await reviews.executeTrash(scanID: scanID, nodeID: nodeID)
            guard snapshotOperation == generation, self.scanID == scanID, !Task.isCancelled else { return }
            isTrashLoading = false
            trashNotice = switch result {
            case .completed:
                ExplorerLiveActionNotice(message: "Moved to Trash. Space is reclaimed after macOS empties Trash.", isFailure: false)
            case .unsupported:
                ExplorerLiveActionNotice(message: "This item cannot be moved to Trash on this volume.", isFailure: true)
            case .failed:
                ExplorerLiveActionNotice(message: "macOS could not move this item to Trash. No retry was attempted.", isFailure: true)
            case .outcomeUnknown:
                ExplorerLiveActionNotice(message: "The Trash result is unknown. Check Finder before trying anything else.", isFailure: true)
            }
        } catch {
            guard snapshotOperation == generation, self.scanID == scanID, !Task.isCancelled else { return }
            isTrashLoading = false
            let mapped = ExplorerTrashError(error)
            trashNotice = ExplorerLiveActionNotice(message: Self.trashFailureMessage(mapped), isFailure: true)
        }
    }

    func dismissLiveActionNotice() {
        liveActionNotice = nil
    }

    func dismissTrashNotice() {
        trashNotice = nil
    }

    func close() async {
        activePresentationID = nil
        generation &+= 1
        historyGeneration &+= 1
        subtreeRefreshGeneration &+= 1
        let retainedScanID = scanID
        clearContent()
        historyScans = []
        historyHasMore = false
        isHistoryLoading = false
        historyFailure = nil
        unavailableHistoricalScanIDs = []
        isSubtreeRefreshRunning = false
        subtreeRefreshNotice = nil
        phase = .idle
        if let retainedScanID {
            await reviews.release(scanID: retainedScanID)
        }
    }

    private func installCompletedSubtreeSnapshot(
        scanID requestedScanID: String,
        sourceScanID: String,
        refreshOperation: UInt64
    ) async {
        guard
            requestedScanID != sourceScanID,
            refreshOperation == subtreeRefreshGeneration,
            scanID == sourceScanID,
            !isSwitchingSnapshot
        else {
            return
        }

        invalidateLiveAction()
        generation &+= 1
        let installOperation = generation
        isSwitchingSnapshot = true
        operationFailure = nil
        var acquired = false
        do {
            try await reviews.acquire(scanID: requestedScanID)
            acquired = true
            guard
                installOperation == generation,
                refreshOperation == subtreeRefreshGeneration,
                !Task.isCancelled
            else {
                await reviews.release(scanID: requestedScanID)
                return
            }
            let root = try await reviews.rootNode(scanID: requestedScanID)
            guard root.kind == .directory else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            let page = try await reviews.childNodes(
                scanID: requestedScanID,
                parentID: root.id,
                sort: sort,
                offset: 0,
                limit: Self.pageLimit
            )
            guard page.totalChildren == root.childCount else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            let replacementTreemap = try await requiredTreemap(
                scanID: requestedScanID,
                directory: root,
                page: page
            )
            guard
                installOperation == generation,
                refreshOperation == subtreeRefreshGeneration,
                self.scanID == sourceScanID,
                !Task.isCancelled
            else {
                await reviews.release(scanID: requestedScanID)
                return
            }

            clearLargeFiles()
            clearCoverage()
            self.scanID = requestedScanID
            // This is an exact result, but another global scan may become the
            // newest while its review is being acquired and validated.
            isLatestSnapshot = false
            breadcrumbs = [root]
            publishPage(page)
            treemap = replacementTreemap
            treemapFailure = nil
            selection = nil
            selectedNodeSnapshot = nil
            phase = .ready
            subtreeRefreshNotice = ExplorerLiveActionNotice(
                message: "Folder scan complete. This refreshed folder is now a standalone snapshot root; its former parent remains in the previous snapshot.",
                isFailure: false
            )
            await reviews.release(scanID: sourceScanID)
            guard
                installOperation == generation,
                refreshOperation == subtreeRefreshGeneration,
                self.scanID == requestedScanID
            else {
                return
            }
            isSwitchingSnapshot = false
            async let historyReload: Void = reloadHistory()
            if contentMode == .largeFiles {
                await reloadLargeFiles()
            } else if contentMode == .coverage {
                await reloadCoverage()
            }
            await historyReload
        } catch {
            if acquired {
                await reviews.release(scanID: requestedScanID)
            }
            guard
                installOperation == generation,
                refreshOperation == subtreeRefreshGeneration,
                self.scanID == sourceScanID,
                !Task.isCancelled
            else {
                return
            }
            isSwitchingSnapshot = false
            subtreeRefreshNotice = ExplorerLiveActionNotice(
                message: "The new folder snapshot could not be validated. The previous snapshot remains visible.",
                isFailure: true
            )
        }
    }

    private func requiredTreemap(
        scanID: String,
        directory: ExplorerSnapshotNode,
        page: ExplorerSnapshotNodePage
    ) async throws -> ExplorerSnapshotTreemap {
        let value = try await reviews.treemap(
            scanID: scanID,
            parentID: directory.id,
            maxCells: Self.treemapCellLimit
        )
        let pageNodesByID = Dictionary(uniqueKeysWithValues: page.nodes.map { ($0.id, $0) })
        guard
            value.parentID == directory.id,
            value.totalChildren == directory.childCount,
            value.totalChildLogicalBytes == directory.logicalBytes,
            value.cells.allSatisfy({ cell in
                pageNodesByID[cell.id].map { $0 == cell.node } ?? true
            })
        else {
            throw ExplorerSnapshotTreemapError.invalidResponse
        }
        return value
    }

    private func performLiveAction(
        _ action: ExplorerLiveFileAction,
        requestedNodeID: UInt64?
    ) async {
        guard
            phase == .ready,
            let scanID,
            let node = liveActionNode(requestedNodeID),
            !isSwitchingSnapshot,
            !isNavigating,
            !isPaging,
            !isLiveActionLoading,
            action != .quickLook || node.kind == .file
        else {
            return
        }

        liveActionGeneration &+= 1
        let actionOperation = liveActionGeneration
        let snapshotOperation = generation
        let largeFilesOperation = largeFilesGeneration
        let contentModeOperation = contentMode
        let nodeID = node.id
        isLiveActionLoading = true
        liveActionNotice = nil
        do {
            let item = try await reviews.resolveLiveItem(
                scanID: scanID,
                nodeID: nodeID,
                purpose: action.livePathPurpose
            )
            guard
                actionOperation == liveActionGeneration,
                snapshotOperation == generation,
                largeFilesOperation == largeFilesGeneration,
                contentModeOperation == contentMode,
                self.scanID == scanID,
                selectedNodeID == nodeID,
                phase == .ready,
                !isSwitchingSnapshot,
                !Task.isCancelled
            else {
                return
            }
            let succeeded = switch action {
            case .reveal: liveActions.reveal(item)
            case .copyPath: liveActions.copyPath(item)
            case .quickLook: liveActions.quickLook(item)
            }
            isLiveActionLoading = false
            liveActionNotice = ExplorerLiveActionNotice(
                message: action.resultMessage(succeeded: succeeded, item: item),
                isFailure: !succeeded
            )
        } catch {
            guard
                actionOperation == liveActionGeneration,
                snapshotOperation == generation,
                largeFilesOperation == largeFilesGeneration,
                contentModeOperation == contentMode,
                self.scanID == scanID,
                selectedNodeID == nodeID,
                !Task.isCancelled
            else {
                return
            }
            isLiveActionLoading = false
            if error as? ExplorerSnapshotLivePathError == .reviewExpired {
                await reviews.release(scanID: scanID)
                guard
                    actionOperation == liveActionGeneration,
                    snapshotOperation == generation,
                    self.scanID == scanID
                else {
                    return
                }
                clearContent()
                phase = .failed(.expired)
            } else {
                liveActionNotice = ExplorerLiveActionNotice(
                    message: Self.liveActionFailureMessage(error),
                    isFailure: true
                )
            }
        }
    }

    @discardableResult
    private func reloadDirectory(
        _ directory: ExplorerSnapshotNode,
        reloadTreemap: Bool
    ) async -> (succeeded: Bool, operation: UInt64?) {
        guard
            phase == .ready,
            let scanID,
            !isNavigating,
            !isPaging,
            !isSwitchingSnapshot
        else {
            return (false, nil)
        }
        invalidateLiveAction()
        generation &+= 1
        let operation = generation
        isNavigating = true
        operationFailure = nil
        do {
            let page = try await reviews.childNodes(
                scanID: scanID,
                parentID: directory.id,
                sort: sort,
                offset: 0,
                limit: Self.pageLimit
            )
            guard page.totalChildren == directory.childCount else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            let treemapResult: ExplorerSnapshotTreemapLoadResult?
            if reloadTreemap {
                treemapResult = try await loadTreemap(scanID: scanID, directory: directory)
            } else {
                treemapResult = nil
            }
            guard operation == generation, self.scanID == scanID else {
                return (false, operation)
            }
            publishPage(page)
            if reloadTreemap {
                publishTreemap(treemapResult, directory: directory, page: page)
                selection = nil
                selectedNodeSnapshot = nil
            }
            isNavigating = false
            return (true, operation)
        } catch {
            guard operation == generation else {
                return (false, operation)
            }
            isNavigating = false
            await publishOperationFailure(error, scanID: scanID, operation: operation)
            return (false, operation)
        }
    }

    private func replaceCurrentDirectory(
        with directory: ExplorerSnapshotNode,
        breadcrumbIndex: Int?
    ) async {
        let result = await reloadDirectory(directory, reloadTreemap: true)
        guard result.succeeded else {
            return
        }
        if let breadcrumbIndex {
            breadcrumbs.removeSubrange((breadcrumbIndex + 1)..<breadcrumbs.endIndex)
        } else {
            breadcrumbs.append(directory)
        }
    }

    private func moveToPage(offset: UInt64) async {
        guard let scanID, let currentDirectory else {
            return
        }
        invalidateLiveAction()
        let operation = generation
        isPaging = true
        operationFailure = nil
        do {
            let page = try await reviews.childNodes(
                scanID: scanID,
                parentID: currentDirectory.id,
                sort: sort,
                offset: offset,
                limit: Self.pageLimit
            )
            guard operation == generation, self.scanID == scanID else {
                return
            }
            guard page.totalChildren == totalChildren else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            publishPage(page)
            isPaging = false
        } catch {
            guard operation == generation else {
                return
            }
            isPaging = false
            await publishOperationFailure(error, scanID: scanID, operation: operation)
        }
    }

    private func revealTreemapCell(_ cell: ExplorerSnapshotTreemapCell) async {
        guard let scanID, let currentDirectory else {
            return
        }
        let operation = generation
        let pageSize = UInt64(Self.pageLimit)
        let offset = (cell.logicalRank / pageSize) * pageSize
        isPaging = true
        operationFailure = nil
        do {
            let page = try await reviews.childNodes(
                scanID: scanID,
                parentID: currentDirectory.id,
                sort: .logicalBytesDescending,
                offset: offset,
                limit: Self.pageLimit
            )
            guard
                operation == generation,
                self.scanID == scanID,
                treemap?.parentID == currentDirectory.id
            else {
                return
            }
            guard
                page.totalChildren == totalChildren,
                page.nodes.contains(where: { $0 == cell.node })
            else {
                throw ExplorerSnapshotNodeError.invalidResponse
            }
            sort = .logicalBytesDescending
            publishPage(page)
            selection = .node(cell.id)
            selectedNodeSnapshot = cell.node
            isPaging = false
        } catch {
            guard operation == generation else {
                return
            }
            isPaging = false
            await publishOperationFailure(error, scanID: scanID, operation: operation)
        }
    }

    private func loadTreemap(
        scanID: String,
        directory: ExplorerSnapshotNode
    ) async throws -> ExplorerSnapshotTreemapLoadResult {
        isTreemapLoading = true
        defer { isTreemapLoading = false }
        do {
            let treemap = try await reviews.treemap(
                scanID: scanID,
                parentID: directory.id,
                maxCells: Self.treemapCellLimit
            )
            guard
                treemap.parentID == directory.id,
                treemap.totalChildren == directory.childCount,
                treemap.totalChildLogicalBytes == directory.logicalBytes
            else {
                throw ExplorerSnapshotTreemapError.invalidResponse
            }
            return .ready(treemap)
        } catch {
            let failure = Self.failure(for: error)
            if failure == .expired {
                throw error
            }
            return .failed(failure)
        }
    }

    private func publishTreemap(
        _ result: ExplorerSnapshotTreemapLoadResult?,
        directory: ExplorerSnapshotNode,
        page: ExplorerSnapshotNodePage
    ) {
        guard page.parentID == directory.id else {
            treemap = nil
            treemapFailure = .invalidResponse
            return
        }
        switch result {
        case let .ready(value):
            let pageNodesByID = Dictionary(uniqueKeysWithValues: page.nodes.map { ($0.id, $0) })
            guard value.cells.allSatisfy({ cell in
                guard let pageNode = pageNodesByID[cell.id] else {
                    return true
                }
                return pageNode == cell.node
            }) else {
                treemap = nil
                treemapFailure = .invalidResponse
                return
            }
            treemap = value
            treemapFailure = nil
        case let .failed(failure):
            treemap = nil
            treemapFailure = failure
        case nil:
            break
        }
    }

    private func publishPage(_ page: ExplorerSnapshotNodePage) {
        nodes = page.nodes
        pageOffset = page.offset
        totalChildren = page.totalChildren
        isPaging = false
        operationFailure = nil
    }

    private func publishOperationFailure(
        _ error: Error,
        scanID: String,
        operation: UInt64
    ) async {
        let failure = Self.failure(for: error)
        if failure == .expired {
            await reviews.release(scanID: scanID)
            guard generation == operation, self.scanID == scanID else {
                return
            }
            clearContent()
            phase = .failed(.expired)
        } else {
            guard generation == operation, self.scanID == scanID else {
                return
            }
            operationFailure = failure
        }
    }

    private func clearContent() {
        invalidateLiveAction()
        scanID = nil
        isLatestSnapshot = false
        breadcrumbs = []
        nodes = []
        pageOffset = 0
        totalChildren = 0
        isNavigating = false
        isPaging = false
        operationFailure = nil
        treemap = nil
        treemapFailure = nil
        isTreemapLoading = false
        isSwitchingSnapshot = false
        selection = nil
        selectedNodeSnapshot = nil
        clearLargeFiles()
        clearCoverage()
    }

    private func clearLargeFiles() {
        largeFilesGeneration &+= 1
        largeFilesPage = nil
        largeFilesFailure = nil
        isLargeFilesLoading = false
    }

    private func clearCoverage() {
        coverageGeneration &+= 1
        coverageDetails = nil
        coverageFailure = nil
        isCoverageLoading = false
    }

    private func liveActionNode(_ requestedNodeID: UInt64?) -> ExplorerSnapshotNode? {
        let nodeID = requestedNodeID ?? selectedNodeID
        guard nodeID == selectedNodeID else {
            return nil
        }
        return selectedNode
    }

    private func canUseLiveAction(
        _ node: ExplorerSnapshotNode?,
        requiresFile: Bool
    ) -> Bool {
        guard
            phase == .ready,
            let node,
            !isSwitchingSnapshot,
            !isNavigating,
            !isPaging,
            !isLiveActionLoading
        else {
            return false
        }
        return requiresFile ? node.kind == .file : node.kind == .file || node.kind == .directory
    }

    private func invalidateLiveAction() {
        liveActionGeneration &+= 1
        isLiveActionLoading = false
        liveActionNotice = nil
        liveActions.dismissQuickLook()
        isTrashLoading = false
        trashNotice = nil
    }

    private static func liveActionFailureMessage(_ error: Error) -> String {
        guard let error = error as? ExplorerSnapshotLivePathError else {
            return "The current item could not be validated. No external action was performed."
        }
        return switch error {
        case .changedSinceScan:
            "This item or one of its folders changed since the scan. Run a new scan before trying again."
        case .accessDenied:
            "macOS did not allow DUX to validate the current item. No external action was performed."
        case .identityUnavailable:
            "The item could not be matched with reliable identity evidence. Run a new scan before using live actions."
        case .unsupportedItem:
            "Live actions are available only for identity-checked files and folders."
        case .nodeNotFound, .invalidResponse:
            "DUX rejected inconsistent item data. No external action was performed."
        case .reviewExpired, .reviewNotAcquired:
            "The retained snapshot review expired. Reload Explorer and try again."
        case .unavailable:
            "The item is missing or cannot be safely matched to this scan."
        }
    }

    private static func subtreeRefreshFailureNotice(
        _ failure: AppScanFailure
    ) -> ExplorerLiveActionNotice {
        let reason = switch failure {
        case .busy:
            "Another scan is already using this location."
        case .rootUnavailable:
            "The current folder no longer matches the retained snapshot."
        case .storageUnavailable, .incompatibleStorage:
            "Snapshot storage is unavailable for this scan."
        case .snapshotRejected:
            "The scan finished without a usable replacement snapshot."
        case .outcomeUnknown:
            "DUX could not confirm whether the folder scan completed."
        case .closed, .taskExpired, .scanFailed, .invalidResponse, .unexpected:
            "The folder scan could not finish."
        }
        return ExplorerLiveActionNotice(
            message: "\(reason) The previous snapshot remains visible.",
            isFailure: true
        )
    }

    private static func failure(for error: Error) -> ExplorerSnapshotBrowserFailure {
        if error is CancellationError {
            return .unavailable
        }
        if let error = error as? ExplorerSnapshotReviewAcquisitionError {
            return switch error {
            case .snapshotUnavailable, .scanNotFound: .noSnapshot
            case .retryable: .busy
            case .budgetExceeded: .budgetExceeded
            case .closed, .unavailable: .unavailable
            case .invalidResponse: .invalidResponse
            }
        }
        if let error = error as? ExplorerSnapshotNodeError {
            return switch error {
            case .reviewExpired, .reviewNotAcquired: .expired
            case .budgetExceeded: .budgetExceeded
            case .invalidLimit, .invalidPage, .invalidResponse,
                 .nodeNotFound, .nodeNotDirectory:
                .invalidResponse
            }
        }
        if let error = error as? ExplorerSnapshotTreemapError {
            return switch error {
            case .reviewExpired, .reviewNotAcquired: .expired
            case .budgetExceeded: .budgetExceeded
            case .invalidBudget, .invalidResponse, .nodeNotFound, .nodeNotDirectory:
                .invalidResponse
            }
        }
        if let error = error as? ExplorerSnapshotLargeFilesError {
            return switch error {
            case .reviewExpired, .reviewNotAcquired: .expired
            case .budgetExceeded: .budgetExceeded
            case .invalidRequest, .invalidResponse: .invalidResponse
            }
        }
        if let error = error as? ExplorerCandidateDetailError {
            return switch error {
            case .reviewExpired, .reviewNotAcquired: .expired
            case .budgetExceeded: .budgetExceeded
            case .invalidLimit, .invalidRequest, .evaluationUnavailable,
                 .candidateNotFound, .invalidResponse:
                .invalidResponse
            case .unavailable: .unavailable
            }
        }
        if let error = error as? ExplorerScanCoverageError {
            return switch error {
            case .retryable: .busy
            case .invalidRequest, .invalidResponse: .invalidResponse
            case .scanNotFound, .unavailable: .unavailable
            }
        }
        if let error = error as? EngineServiceError {
            return switch error {
            case .retryable: .busy
            case .closed, .unavailable: .unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                .invalidResponse
            }
        }
        return .invalidResponse
    }

    static func trashFailureMessage(_ error: ExplorerTrashError) -> String {
        switch error {
        case .closed: "DUX is closing. No Trash action was performed."
        case .reviewNotAcquired, .unavailable: "The retained snapshot review is unavailable. Reload Explorer before trying again."
        case .invalidRequest: "DUX rejected this Trash request. No action was performed."
        case .changedSincePlan: "This item changed after it was reviewed. No Trash action was performed; scan again before retrying."
        case .busy: "Cleanup is busy. Try again when the current operation finishes."
        case .storageUnavailable, .unsafeStorage, .incompatibleSchema, .corruptData, .failed:
            "DUX could not safely record this Trash action. No retry was attempted."
        case .outcomeUnknown:
            "The Trash result is unknown. Check Finder before trying anything else."
        }
    }
}

extension DuxSnapshotReviewController: DuxSnapshotReviewBrowsing {}

private extension ExplorerLiveFileAction {
    var livePathPurpose: ExplorerSnapshotLivePathPurpose {
        switch self {
        case .reveal: .reveal
        case .copyPath: .copyPath
        case .quickLook: .quickLook
        }
    }

    func resultMessage(succeeded: Bool, item: ExplorerResolvedLiveItem) -> String {
        guard succeeded else {
            if self == .copyPath, item.exactTextPath == nil {
                return "This filesystem name cannot be copied as text without changing it."
            }
            return "macOS could not perform the requested action."
        }
        return switch self {
        case .reveal:
            "Asked Finder to reveal the identity-checked current item."
        case .copyPath:
            "Copied the identity-checked current path."
        case .quickLook:
            "Quick Look is showing current contents; sizes in Explorer remain historical."
        }
    }
}
