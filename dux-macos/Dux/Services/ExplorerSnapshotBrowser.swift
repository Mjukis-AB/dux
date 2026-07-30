import Foundation
import Observation

protocol ExplorerRustTargetPlanReviewClock: Sendable {
    func now() -> Date
    func sleep(until deadline: Date) async throws
}

struct ContinuousExplorerRustTargetPlanReviewClock: ExplorerRustTargetPlanReviewClock {
    func now() -> Date {
        Date()
    }

    func sleep(until deadline: Date) async throws {
        try await Task.sleep(for: .seconds(max(0, deadline.timeIntervalSince(now()))))
    }
}

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
    func prepareSnapshotDiffReview(
        scanID: String
    ) async throws -> ExplorerSnapshotDiffReviewHandle
    func snapshotDiffRootNode(
        _ handle: ExplorerSnapshotDiffReviewHandle
    ) async throws -> ExplorerSnapshotDiffNode
    func snapshotDiffChildNodes(
        _ handle: ExplorerSnapshotDiffReviewHandle,
        parentID: UInt64,
        sort: ExplorerSnapshotDiffSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotDiffNodePage
    func snapshotDiffTreemap(
        _ handle: ExplorerSnapshotDiffReviewHandle,
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotDiffTreemap
    func releaseSnapshotDiffReview(
        _ handle: ExplorerSnapshotDiffReviewHandle
    ) async
    func largeFiles(
        scanID: String,
        minimumLogicalBytes: UInt64,
        modifiedBefore: ExplorerSnapshotTimestamp?,
        maxResults: UInt16
    ) async throws -> ExplorerSnapshotLargeFilesPage
    func icloudObservationSource(
        scanID: String,
        scopeNodeID: UInt64,
        maxResults: UInt16
    ) async throws -> ExplorerICloudObservationSource
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
    func prepareRustTargetPlanReview(
        scanID: String,
        candidateID: String
    ) async throws -> ExplorerRustTargetPlanReviewHandle
    func refreshRustTargetPlanReview(
        _ handle: ExplorerRustTargetPlanReviewHandle
    ) async throws -> ExplorerRustTargetPlanReviewHandle
    func releaseRustTargetPlanReview(
        _ handle: ExplorerRustTargetPlanReviewHandle
    ) async
    func startRustTargetCleanup(
        _ handle: ExplorerRustTargetPlanReviewHandle
    ) async throws -> any DuxRustTargetCleanupTask
    func startRustTargetDryRun(
        _ handle: ExplorerRustTargetPlanReviewHandle
    ) async throws -> any DuxRustTargetDryRunTask
    func resolveLiveItem(
        scanID: String,
        nodeID: UInt64,
        purpose: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem
    func probeICloudLocalCopy(
        scanID: String,
        nodeID: UInt64
    ) async throws -> ExplorerICloudLocalCopyAssessment
    func executeTrash(scanID: String, nodeID: UInt64) async throws -> TrashPlatformResult
}

extension DuxSnapshotReviewBrowsing {
    func prepareSnapshotDiffReview(
        scanID _: String
    ) async throws -> ExplorerSnapshotDiffReviewHandle {
        throw ExplorerSnapshotDiffFailure.unavailable
    }

    func snapshotDiffRootNode(
        _: ExplorerSnapshotDiffReviewHandle
    ) async throws -> ExplorerSnapshotDiffNode {
        throw ExplorerSnapshotDiffFailure.unavailable
    }

    func snapshotDiffChildNodes(
        _: ExplorerSnapshotDiffReviewHandle,
        parentID _: UInt64,
        sort _: ExplorerSnapshotDiffSort,
        offset _: UInt64,
        limit _: UInt16
    ) async throws -> ExplorerSnapshotDiffNodePage {
        throw ExplorerSnapshotDiffFailure.unavailable
    }

    func snapshotDiffTreemap(
        _: ExplorerSnapshotDiffReviewHandle,
        parentID _: UInt64,
        maxCells _: UInt16
    ) async throws -> ExplorerSnapshotDiffTreemap {
        throw ExplorerSnapshotDiffFailure.unavailable
    }

    func releaseSnapshotDiffReview(
        _: ExplorerSnapshotDiffReviewHandle
    ) async {}

    func icloudObservationSource(
        scanID _: String,
        scopeNodeID _: UInt64,
        maxResults _: UInt16
    ) async throws -> ExplorerICloudObservationSource {
        throw ExplorerICloudObservationSourceError.reviewNotAcquired
    }

    func reviewCandidate(
        scanID _: String,
        candidateID _: String,
        command _: ExplorerCandidateReviewCommand
    ) async throws -> ExplorerCandidateReviewResult {
        throw ExplorerCandidateDetailError.unavailable
    }

    func prepareRustTargetPlanReview(
        scanID _: String,
        candidateID _: String
    ) async throws -> ExplorerRustTargetPlanReviewHandle {
        throw ExplorerRustTargetPlanReviewError.unavailable
    }

    func refreshRustTargetPlanReview(
        _: ExplorerRustTargetPlanReviewHandle
    ) async throws -> ExplorerRustTargetPlanReviewHandle {
        throw ExplorerRustTargetPlanReviewError.unavailable
    }

    func releaseRustTargetPlanReview(
        _: ExplorerRustTargetPlanReviewHandle
    ) async {}

    func startRustTargetCleanup(
        _: ExplorerRustTargetPlanReviewHandle
    ) async throws -> any DuxRustTargetCleanupTask {
        throw ExplorerRustTargetCleanupStartError.unavailable
    }

    func startRustTargetDryRun(
        _: ExplorerRustTargetPlanReviewHandle
    ) async throws -> any DuxRustTargetDryRunTask {
        throw ExplorerRustTargetDryRunStartError.unavailable
    }

    func resolveLiveItem(
        scanID _: String,
        nodeID _: UInt64,
        purpose _: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem {
        throw ExplorerSnapshotLivePathError.unavailable
    }

    func probeICloudLocalCopy(
        scanID _: String,
        nodeID _: UInt64
    ) async throws -> ExplorerICloudLocalCopyAssessment {
        throw ExplorerICloudLocalCopyProbeError.unavailable
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
    case iCloudObservation(UInt64)
    case other
}

enum ExplorerICloudLocalCopyReviewState: Equatable, Sendable {
    case idle
    case checking
    case observed(ExplorerICloudLocalCopyAssessment)
    case failed(ExplorerICloudLocalCopyProbeError)
}

enum ExplorerICloudObservationBatchPhase: Equatable, Sendable {
    case idle
    case checking(completed: Int, total: Int)
    case stopping(completed: Int, total: Int)
    case completed(total: Int)
    case cancelled(completed: Int, total: Int)
    case stopped(
        error: ExplorerICloudLocalCopyProbeError,
        completed: Int,
        total: Int
    )
}

enum ExplorerICloudObservationItemResult: Equatable, Sendable {
    case observed(ExplorerICloudLocalCopyAssessment)
    case failed(ExplorerICloudLocalCopyProbeError)
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

    func icloudObservationSource(
        scanID _: String,
        scopeNodeID _: UInt64,
        maxResults _: UInt16
    ) async throws -> ExplorerICloudObservationSource {
        throw ExplorerICloudObservationSourceError.reviewNotAcquired
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
    static let iCloudObservationResultLimit: UInt16 = 32

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
    private(set) var snapshotDiffPhase = ExplorerSnapshotDiffPhase.idle
    private(set) var snapshotDiffInfo: ExplorerSnapshotDiffInfo?
    private(set) var snapshotDiffBreadcrumbs: [ExplorerSnapshotDiffNode] = []
    private(set) var snapshotDiffPage: ExplorerSnapshotDiffNodePage?
    private(set) var snapshotDiffTreemap: ExplorerSnapshotDiffTreemap?
    private(set) var snapshotDiffOperationFailure: ExplorerSnapshotDiffFailure?
    private(set) var snapshotDiffSort = ExplorerSnapshotDiffSort.magnitudeDescending
    private(set) var snapshotDiffSelection: ExplorerSnapshotDiffSelection?
    private(set) var isSnapshotDiffNavigating = false
    private(set) var isSnapshotDiffPaging = false
    private(set) var candidatePage: ExplorerCandidateSummaryPage?
    private(set) var candidateFailure: ExplorerSnapshotBrowserFailure?
    private(set) var isCandidateLoading = false
    private(set) var candidateNotice: ExplorerLiveActionNotice?
    private(set) var selectedCandidateID: String?
    private(set) var candidatePathPage: ExplorerCandidatePathPage?
    private(set) var candidateEvidencePage: ExplorerCandidateEvidencePage?
    private(set) var candidateDetailFailure: ExplorerSnapshotBrowserFailure?
    private(set) var isCandidateDetailLoading = false
    private(set) var isCandidatePathPaging = false
    private(set) var isCandidateEvidencePaging = false
    private(set) var rustTargetPlanReviewState = ExplorerRustTargetPlanReviewState.idle
    private(set) var rustTargetCleanupState = ExplorerRustTargetCleanupState.idle
    private(set) var rustTargetDryRunState = ExplorerRustTargetDryRunState.idle
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
    private(set) var iCloudLocalCopyReviewState = ExplorerICloudLocalCopyReviewState.idle
    private(set) var iCloudObservationSource: ExplorerICloudObservationSource?
    private(set) var iCloudObservationSourceFailure: ExplorerICloudObservationSourceError?
    private(set) var isICloudObservationSourceLoading = false
    private(set) var iCloudObservationBatchPhase = ExplorerICloudObservationBatchPhase.idle
    private(set) var iCloudObservationResults:
        [UInt64: ExplorerICloudObservationItemResult] = [:]
    private(set) var checkingICloudObservationNodeID: UInt64?
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
    private let rustTargetPlanReviewClock: any ExplorerRustTargetPlanReviewClock
    private let rustTargetCleanupPollingClock: any ExplorerRustTargetCleanupPollingClock
    private let rustTargetCleanupTerminalObserver:
        (@MainActor @Sendable () async -> Void)?
    private let rustTargetDryRunPollingClock: any ExplorerRustTargetDryRunPollingClock
    private let rustTargetDryRunTerminalObserver:
        (@MainActor @Sendable () async -> Void)?

    @ObservationIgnored
    private var generation: UInt64 = 0
    @ObservationIgnored
    private var activePresentationID: UUID?
    @ObservationIgnored
    private var pendingExactScanID: String?
    @ObservationIgnored
    private var pendingExactContentMode: ExplorerSnapshotContentMode?
    @ObservationIgnored
    private var historyGeneration: UInt64 = 0
    @ObservationIgnored
    private var snapshotDiffGeneration: UInt64 = 0
    @ObservationIgnored
    private var snapshotDiffHandle: ExplorerSnapshotDiffReviewHandle?
    @ObservationIgnored
    private var largeFilesGeneration: UInt64 = 0
    @ObservationIgnored
    private var candidateGeneration: UInt64 = 0
    @ObservationIgnored
    private var candidateDetailGeneration: UInt64 = 0
    @ObservationIgnored
    private var candidatePathGeneration: UInt64 = 0
    @ObservationIgnored
    private var candidateEvidenceGeneration: UInt64 = 0
    @ObservationIgnored
    private var rustTargetPlanReviewGeneration: UInt64 = 0
    @ObservationIgnored
    private var rustTargetPlanReviewHandle: ExplorerRustTargetPlanReviewHandle?
    @ObservationIgnored
    private var rustTargetPlanReviewExpiryTask: Task<Void, Never>?
    @ObservationIgnored
    private var rustTargetCleanupGeneration: UInt64 = 0
    @ObservationIgnored
    private var rustTargetCleanupTask: (any DuxRustTargetCleanupTask)?
    @ObservationIgnored
    private var rustTargetCleanupDriverTask: Task<Void, Never>?
    @ObservationIgnored
    private var rustTargetCleanupCancellationRequested = false
    @ObservationIgnored
    private var rustTargetDryRunGeneration: UInt64 = 0
    @ObservationIgnored
    private var rustTargetDryRunTask: (any DuxRustTargetDryRunTask)?
    @ObservationIgnored
    private var rustTargetDryRunDriverTask: Task<Void, Never>?
    @ObservationIgnored
    private var rustTargetDryRunCancellationRequested = false
    @ObservationIgnored
    private var coverageGeneration: UInt64 = 0
    @ObservationIgnored
    private var liveActionGeneration: UInt64 = 0
    @ObservationIgnored
    private var iCloudLocalCopyReviewGeneration: UInt64 = 0
    @ObservationIgnored
    private var iCloudObservationGeneration: UInt64 = 0
    @ObservationIgnored
    private var iCloudObservationCancellationRequested = false
    @ObservationIgnored
    private var iCloudObservationBatchActive = false
    @ObservationIgnored
    private var iCloudObservationReloadPending = false
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
        scanDriver: (any ExplorerSubtreeScanDriving)? = nil,
        rustTargetPlanReviewClock: any ExplorerRustTargetPlanReviewClock =
            ContinuousExplorerRustTargetPlanReviewClock(),
        rustTargetCleanupPollingClock: any ExplorerRustTargetCleanupPollingClock =
            ContinuousExplorerRustTargetCleanupPollingClock(),
        rustTargetCleanupTerminalObserver:
            (@MainActor @Sendable () async -> Void)? = nil,
        rustTargetDryRunPollingClock: any ExplorerRustTargetDryRunPollingClock =
            ContinuousExplorerRustTargetDryRunPollingClock(),
        rustTargetDryRunTerminalObserver:
            (@MainActor @Sendable () async -> Void)? = nil
    ) {
        self.reviews = reviews
        self.history = history
        self.coverage = coverage
        self.liveActions = liveActions
        self.subtreeScans = subtreeScans
        self.scanDriver = scanDriver
        self.rustTargetPlanReviewClock = rustTargetPlanReviewClock
        self.rustTargetCleanupPollingClock = rustTargetCleanupPollingClock
        self.rustTargetCleanupTerminalObserver = rustTargetCleanupTerminalObserver
        self.rustTargetDryRunPollingClock = rustTargetDryRunPollingClock
        self.rustTargetDryRunTerminalObserver = rustTargetDryRunTerminalObserver
    }

    var currentDirectory: ExplorerSnapshotNode? {
        breadcrumbs.last
    }

    var currentSnapshotDiffDirectory: ExplorerSnapshotDiffNode? {
        snapshotDiffBreadcrumbs.last
    }

    var snapshotDiffNodes: [ExplorerSnapshotDiffNode] {
        snapshotDiffPage?.nodes ?? []
    }

    var selectedSnapshotDiffNodeID: UInt64? {
        guard case let .node(id) = snapshotDiffSelection else {
            return nil
        }
        return id
    }

    var selectedSnapshotDiffNode: ExplorerSnapshotDiffNode? {
        guard let selectedSnapshotDiffNodeID else {
            return nil
        }
        return snapshotDiffNodes.first { $0.id == selectedSnapshotDiffNodeID }
            ?? snapshotDiffTreemap?.cell(nodeID: selectedSnapshotDiffNodeID)?.node
    }

    var hasPreviousSnapshotDiffPage: Bool {
        (snapshotDiffPage?.offset ?? 0) > 0
    }

    var hasNextSnapshotDiffPage: Bool {
        guard let page = snapshotDiffPage else {
            return false
        }
        return page.offset + UInt64(page.nodes.count) < page.totalChildren
    }

    var isSnapshotDiffOtherGrowthSelected: Bool {
        snapshotDiffSelection == .otherGrowth
    }

    var isSnapshotDiffOtherShrinkageSelected: Bool {
        snapshotDiffSelection == .otherShrinkage
    }

    var hasPreviousPage: Bool {
        pageOffset > 0
    }

    var hasNextPage: Bool {
        pageOffset + UInt64(nodes.count) < totalChildren
    }

    var selectedNodeID: UInt64? {
        switch selection {
        case let .node(id), let .largeFile(id), let .iCloudObservation(id):
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

    var selectedICloudObservationResult: ExplorerICloudObservationItemResult? {
        guard let selectedNodeID else {
            return nil
        }
        return iCloudObservationResults[selectedNodeID]
    }

    var isOtherSelected: Bool {
        switch selection {
        case .other:
            true
        case let .node(id):
            treemap?.hasOther == true && treemap?.cell(nodeID: id) == nil
        case .largeFile:
            false
        case .iCloudObservation:
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

    var selectedCandidate: ExplorerCandidateSummary? {
        guard let selectedCandidateID else {
            return nil
        }
        return candidatePage?.candidates.first { $0.candidateID == selectedCandidateID }
    }

    var isSelectedRustTargetPlanReviewCandidate: Bool {
        guard let candidate = selectedCandidate else {
            return false
        }
        return candidate.ruleID == "developer.rust.target"
            && candidate.ruleRevision == 3
            && candidate.category == .developerArtifact
            && candidate.newestMtime != nil
            && candidate.safety == .safeRegenerable
            && candidate.action == .removeKnownRegenerableContents
            && !candidate.ruleScheduleEligible
            && candidate.pathCount == 1
            && candidate.evidenceKinds
                == [.matchedPath, .requiredMarker, .requiredMarker, .minimumAge]
            && candidate.blockers == [.protectedPath]
            && candidate.status == .discovered
    }

    var canPrepareRustTargetPlanReview: Bool {
        phase == .ready
            && contentMode == .candidates
            && !isSwitchingSnapshot
            && !isCandidateLoading
            && isSelectedRustTargetPlanReviewCandidate
            && rustTargetPlanReviewState != .preparing
            && !rustTargetCleanupState.isActive
            && rustTargetCleanupState == .idle
            && !rustTargetDryRunState.isActive
            && rustTargetDryRunState == .idle
    }

    var hasPreviousCandidatePage: Bool {
        (candidatePage?.cursor ?? 0) > 0
    }

    var hasNextCandidatePage: Bool {
        candidatePage?.nextCursor != nil
    }

    var hasPreviousCandidatePathPage: Bool {
        (candidatePathPage?.cursor ?? 0) > 0
    }

    var hasNextCandidatePathPage: Bool {
        candidatePathPage?.nextCursor != nil
    }

    var hasPreviousCandidateEvidencePage: Bool {
        (candidateEvidencePage?.cursor ?? 0) > 0
    }

    var hasNextCandidateEvidencePage: Bool {
        candidateEvidencePage?.nextCursor != nil
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

    var canCheckSelectedICloudLocalCopy: Bool {
        phase == .ready
            && selectedNode?.kind == .file
            && iCloudLocalCopyReviewState != .checking
            && !iCloudObservationBatchActive
            && !isSwitchingSnapshot
            && !isNavigating
            && !isPaging
    }

    var canStartICloudObservationBatch: Bool {
        phase == .ready
            && contentMode == .iCloudStatus
            && iCloudObservationSource?.targets.isEmpty == false
            && !iCloudObservationBatchActive
            && iCloudLocalCopyReviewState != .checking
            && !isSwitchingSnapshot
            && !isNavigating
    }

    var canStopICloudObservationBatch: Bool {
        iCloudObservationBatchActive && !iCloudObservationCancellationRequested
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
        let requestedScanID = pendingExactScanID
        let requestedContentMode = pendingExactContentMode
        pendingExactScanID = nil
        pendingExactContentMode = nil
        if let requestedContentMode {
            contentMode = requestedContentMode
        }
        async let history: Void = reloadHistory()
        if let requestedScanID {
            await openExactScan(requestedScanID)
        } else {
            await reloadLatest()
        }
        _ = await history
    }

    /// Queue one exact durable scan for the next Snapshot presentation.
    ///
    /// Targeted pressure scans are deliberately excluded from generic Home
    /// history, so their trusted scan IDs must be opened explicitly.
    func prepareExactScanReview(scanID: String) {
        pendingExactScanID = scanID
        pendingExactContentMode = .browse
    }

    /// Queue one exact targeted result directly in its deterministic findings
    /// view. This changes presentation intent only; it creates no candidate,
    /// plan, approval, or cleanup authority.
    func prepareExactCandidateReview(scanID: String) {
        pendingExactScanID = scanID
        pendingExactContentMode = .candidates
    }

    /// Queue one exact retained scan directly in its historical large-files
    /// view. The intent changes presentation only and never selects a file or
    /// cleanup action.
    func prepareExactLargeFilesReview(scanID: String) {
        pendingExactScanID = scanID
        pendingExactContentMode = .largeFiles
    }

    /// Queue one exact retained scan directly in its recorded-coverage view.
    /// Coverage remains historical evidence and does not infer current
    /// permission state.
    func prepareExactCoverageReview(scanID: String) {
        pendingExactScanID = scanID
        pendingExactContentMode = .coverage
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

        await openExactScan(historicalScan.scanID, markUnavailableInHistory: true)
    }

    private func openExactScan(
        _ requestedScanID: String,
        markUnavailableInHistory: Bool = false
    ) async {
        await releaseSnapshotDiffReview()
        await releaseRustTargetPlanReview()
        guard
            requestedScanID != scanID,
            phase != .loading,
            !isSwitchingSnapshot,
            !isNavigating,
            !isPaging
        else {
            return
        }
        invalidateLiveAction()
        invalidateICloudObservationContext()
        generation &+= 1
        let operation = generation
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

            clearCandidates()
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
            if contentMode == .changes {
                await prepareSnapshotDiffReview()
            } else if contentMode == .candidates {
                await reloadCandidates()
            } else if contentMode == .largeFiles {
                await reloadLargeFiles()
            } else if contentMode == .iCloudStatus {
                await reloadICloudObservationSource()
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
            if markUnavailableInHistory,
               failure == .noSnapshot || failure == .expired || failure == .unavailable
            {
                unavailableHistoricalScanIDs.insert(requestedScanID)
            }
            operationFailure = failure == .noSnapshot ? .unavailable : failure
        }
    }

    func reloadLatest() async {
        await releaseSnapshotDiffReview()
        await releaseRustTargetPlanReview()
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
            if contentMode == .changes {
                await prepareSnapshotDiffReview()
            } else if contentMode == .candidates {
                await reloadCandidates()
            } else if contentMode == .largeFiles {
                await reloadLargeFiles()
            } else if contentMode == .iCloudStatus {
                await reloadICloudObservationSource()
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
        if contentMode == .changes {
            await releaseSnapshotDiffReview()
        }
        await releaseRustTargetPlanReview()
        invalidateLiveAction()
        invalidateICloudObservationContext()
        contentMode = mode
        selection = nil
        selectedNodeSnapshot = nil
        largeFilesGeneration &+= 1
        isLargeFilesLoading = false
        clearCandidates()
        coverageGeneration &+= 1
        isCoverageLoading = false
        switch mode {
        case .browse:
            return
        case .changes:
            await prepareSnapshotDiffReview()
        case .candidates:
            if candidatePage == nil {
                await reloadCandidates()
            }
        case .largeFiles:
            if largeFilesPage == nil {
                await reloadLargeFiles()
            }
        case .iCloudStatus:
            await reloadICloudObservationSource()
        case .coverage:
            if coverageDetails == nil {
                await reloadCoverage()
            }
        }
    }

    func retrySnapshotDiffReview() async {
        guard contentMode == .changes, phase == .ready else {
            return
        }
        await releaseSnapshotDiffReview()
        await prepareSnapshotDiffReview()
    }

    func openSnapshotDiffDirectory(_ node: ExplorerSnapshotDiffNode) async {
        guard
            contentMode == .changes,
            snapshotDiffPhase == .ready,
            node.canDescend,
            snapshotDiffNodes.contains(node),
            !isSnapshotDiffNavigating,
            !isSnapshotDiffPaging
        else {
            return
        }
        await loadSnapshotDiffDirectory(node, breadcrumbIndex: nil)
    }

    func openSnapshotDiffBreadcrumb(at index: Int) async {
        guard
            snapshotDiffBreadcrumbs.indices.contains(index),
            index < snapshotDiffBreadcrumbs.count - 1
        else {
            return
        }
        await loadSnapshotDiffDirectory(
            snapshotDiffBreadcrumbs[index],
            breadcrumbIndex: index
        )
    }

    func goBackSnapshotDiff() async {
        guard snapshotDiffBreadcrumbs.count > 1 else {
            return
        }
        await openSnapshotDiffBreadcrumb(at: snapshotDiffBreadcrumbs.count - 2)
    }

    func selectSnapshotDiffSort(_ sort: ExplorerSnapshotDiffSort) async {
        guard
            sort != snapshotDiffSort,
            snapshotDiffPhase == .ready,
            let directory = currentSnapshotDiffDirectory,
            !isSnapshotDiffNavigating,
            !isSnapshotDiffPaging
        else {
            return
        }
        let previous = snapshotDiffSort
        snapshotDiffSort = sort
        do {
            try await loadSnapshotDiffPage(
                directory: directory,
                offset: 0,
                preserveTreemap: true
            )
        } catch {
            snapshotDiffSort = previous
            isSnapshotDiffPaging = false
            await publishSnapshotDiffFailure(error)
        }
    }

    func showPreviousSnapshotDiffPage() async {
        guard let page = snapshotDiffPage, page.offset > 0 else {
            return
        }
        let offset = page.offset >= UInt64(Self.pageLimit)
            ? page.offset - UInt64(Self.pageLimit)
            : 0
        await moveSnapshotDiffPage(to: offset)
    }

    func showNextSnapshotDiffPage() async {
        guard let page = snapshotDiffPage, page.hasMore else {
            return
        }
        await moveSnapshotDiffPage(to: page.offset + UInt64(page.nodes.count))
    }

    func selectSnapshotDiffNode(_ nodeID: UInt64?) {
        guard
            let nodeID,
            snapshotDiffNodes.contains(where: { $0.id == nodeID })
                || snapshotDiffTreemap?.cell(nodeID: nodeID) != nil
        else {
            snapshotDiffSelection = nil
            return
        }
        snapshotDiffSelection = .node(nodeID)
    }

    func selectSnapshotDiffTreemapCell(_ cell: ExplorerSnapshotDiffTreemapCell) {
        guard snapshotDiffTreemap?.cell(nodeID: cell.id) == cell else {
            return
        }
        snapshotDiffSelection = .node(cell.id)
    }

    func selectSnapshotDiffOtherGrowth() {
        guard snapshotDiffTreemap?.otherGrowthChildCount ?? 0 > 0 else {
            return
        }
        snapshotDiffSelection = .otherGrowth
    }

    func selectSnapshotDiffOtherShrinkage() {
        guard snapshotDiffTreemap?.otherShrinkageChildCount ?? 0 > 0 else {
            return
        }
        snapshotDiffSelection = .otherShrinkage
    }

    func reloadCandidates() async {
        await loadCandidatePage(cursor: 0)
    }

    func showNextCandidatePage() async {
        guard let cursor = candidatePage?.nextCursor else {
            return
        }
        await loadCandidatePage(cursor: cursor)
    }

    func showPreviousCandidatePage() async {
        guard let page = candidatePage, page.cursor > 0 else {
            return
        }
        let limit = ExplorerCandidateDetailAdapter.maximumPageLimit
        await loadCandidatePage(cursor: page.cursor >= limit ? page.cursor - limit : 0)
    }

    private func loadCandidatePage(cursor: UInt16) async {
        guard
            phase == .ready,
            contentMode == .candidates,
            let scanID,
            !isSwitchingSnapshot,
            !isCandidateLoading
        else { return }
        await releaseRustTargetPlanReview()
        candidateGeneration &+= 1
        let operation = candidateGeneration
        let snapshotGeneration = generation
        clearCandidateDetail()
        candidatePage = nil
        candidateFailure = nil
        isCandidateLoading = true
        do {
            let page = try await reviews.candidateSummaries(
                scanID: scanID,
                cursor: cursor,
                limit: ExplorerCandidateDetailAdapter.maximumPageLimit
            )
            guard
                operation == candidateGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .candidates,
                page.scanID == scanID,
                page.cursor == cursor,
                !Task.isCancelled
            else { return }
            candidatePage = page
            isCandidateLoading = false
        } catch {
            guard
                operation == candidateGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .candidates,
                !Task.isCancelled
            else { return }
            isCandidateLoading = false
            let failure = Self.failure(for: error)
            if failure == .expired {
                await releaseRustTargetPlanReview()
                await reviews.release(scanID: scanID)
                guard operation == candidateGeneration, self.scanID == scanID else { return }
                clearContent()
                phase = .failed(.expired)
            } else {
                candidateFailure = failure
            }
        }
    }

    func selectCandidate(_ candidateID: String?) async {
        await releaseRustTargetPlanReview()
        clearCandidateDetail()
        guard let candidateID else {
            return
        }
        guard
            phase == .ready,
            contentMode == .candidates,
            let scanID,
            let candidate = candidatePage?.candidates.first(where: {
                $0.candidateID == candidateID
            }),
            !isSwitchingSnapshot,
            !isCandidateLoading
        else {
            return
        }

        selectedCandidateID = candidateID
        isCandidateDetailLoading = true
        let operation = candidateDetailGeneration
        let snapshotGeneration = generation
        do {
            async let paths = reviews.candidatePaths(
                scanID: scanID,
                candidateID: candidateID,
                cursor: 0,
                limit: ExplorerCandidateDetailAdapter.maximumPageLimit
            )
            async let evidence = reviews.candidateEvidence(
                scanID: scanID,
                candidateID: candidateID,
                cursor: 0,
                limit: ExplorerCandidateDetailAdapter.maximumPageLimit
            )
            let (pathPage, evidencePage) = try await (paths, evidence)
            guard
                operation == candidateDetailGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .candidates,
                selectedCandidateID == candidateID
            else {
                return
            }
            guard !Task.isCancelled else {
                isCandidateDetailLoading = false
                return
            }
            guard
                Self.validCandidatePathPage(
                    pathPage,
                    candidate: candidate,
                    scanID: scanID,
                    cursor: 0
                ),
                Self.validCandidateEvidencePage(
                    evidencePage,
                    candidate: candidate,
                    scanID: scanID,
                    cursor: 0
                )
            else {
                isCandidateDetailLoading = false
                candidateDetailFailure = .invalidResponse
                return
            }
            candidatePathPage = pathPage
            candidateEvidencePage = evidencePage
            candidateDetailFailure = nil
            isCandidateDetailLoading = false
        } catch {
            guard
                operation == candidateDetailGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .candidates,
                selectedCandidateID == candidateID
            else {
                return
            }
            guard !Task.isCancelled else {
                isCandidateDetailLoading = false
                return
            }
            isCandidateDetailLoading = false
            await publishCandidateDetailFailure(
                error,
                scanID: scanID,
                detailOperation: operation
            )
        }
    }

    func reloadSelectedCandidateDetail() async {
        await selectCandidate(selectedCandidateID)
    }

    func showNextCandidatePathPage() async {
        guard let cursor = candidatePathPage?.nextCursor else {
            return
        }
        await loadCandidatePathPage(cursor: cursor)
    }

    func showPreviousCandidatePathPage() async {
        guard let page = candidatePathPage, page.cursor > 0 else {
            return
        }
        let limit = ExplorerCandidateDetailAdapter.maximumPageLimit
        await loadCandidatePathPage(cursor: page.cursor >= limit ? page.cursor - limit : 0)
    }

    func showNextCandidateEvidencePage() async {
        guard let cursor = candidateEvidencePage?.nextCursor else {
            return
        }
        await loadCandidateEvidencePage(cursor: cursor)
    }

    func showPreviousCandidateEvidencePage() async {
        guard let page = candidateEvidencePage, page.cursor > 0 else {
            return
        }
        let limit = ExplorerCandidateDetailAdapter.maximumPageLimit
        await loadCandidateEvidencePage(cursor: page.cursor >= limit ? page.cursor - limit : 0)
    }

    func reviewCandidate(
        candidateID: String,
        command: ExplorerCandidateReviewCommand
    ) async {
        guard
            phase == .ready,
            contentMode == .candidates,
            let scanID,
            candidatePage?.candidates.contains(where: { $0.candidateID == candidateID }) == true,
            !isSwitchingSnapshot,
            !isCandidateLoading
        else { return }
        await releaseRustTargetPlanReview()
        guard
            phase == .ready,
            contentMode == .candidates,
            self.scanID == scanID,
            candidatePage?.candidates.contains(where: {
                $0.candidateID == candidateID
            }) == true,
            !isSwitchingSnapshot,
            !isCandidateLoading
        else {
            return
        }
        let operation = candidateGeneration
        do {
            let result = try await reviews.reviewCandidate(
                scanID: scanID,
                candidateID: candidateID,
                command: command
            )
            guard
                operation == candidateGeneration,
                self.scanID == scanID,
                contentMode == .candidates,
                result.candidateID == candidateID
            else { return }
            if let page = candidatePage,
               let index = page.candidates.firstIndex(where: { $0.candidateID == candidateID })
            {
                var candidates = page.candidates
                let current = candidates[index]
                candidates[index] = ExplorerCandidateSummary(
                    candidateID: current.candidateID,
                    ruleID: current.ruleID,
                    ruleRevision: current.ruleRevision,
                    category: current.category,
                    estimatedBytes: current.estimatedBytes,
                    newestMtime: current.newestMtime,
                    safety: current.safety,
                    action: current.action,
                    ruleScheduleEligible: current.ruleScheduleEligible,
                    pathCount: current.pathCount,
                    evidenceKinds: current.evidenceKinds,
                    blockers: current.blockers,
                    createdAt: current.createdAt,
                    status: result.status
                )
                candidatePage = ExplorerCandidateSummaryPage(
                    scanID: page.scanID,
                    cursor: page.cursor,
                    nextCursor: page.nextCursor,
                    totalCandidates: page.totalCandidates,
                    candidates: candidates
                )
            }
            candidateNotice = ExplorerLiveActionNotice(
                message: "Review status updated. No cleanup action was performed.",
                isFailure: false
            )
        } catch {
            guard
                operation == candidateGeneration,
                self.scanID == scanID,
                contentMode == .candidates
            else {
                return
            }
            candidateNotice = ExplorerLiveActionNotice(
                message: "DUX could not record that review decision. No cleanup action was performed.",
                isFailure: true
            )
        }
    }

    func dismissCandidateNotice() {
        candidateNotice = nil
    }

    func prepareSelectedRustTargetPlanReview() async {
        guard
            canPrepareRustTargetPlanReview,
            let requestedScanID = scanID,
            let requestedCandidateID = selectedCandidateID
        else {
            return
        }
        await releaseRustTargetPlanReview()
        guard
            canPrepareRustTargetPlanReview,
            scanID == requestedScanID,
            selectedCandidateID == requestedCandidateID,
            let candidate = selectedCandidate,
            candidate.candidateID == requestedCandidateID
        else {
            return
        }
        let scanID = requestedScanID
        rustTargetPlanReviewGeneration &+= 1
        let operation = rustTargetPlanReviewGeneration
        let snapshotOperation = generation
        let candidateOperation = candidateGeneration
        let detailOperation = candidateDetailGeneration
        rustTargetPlanReviewState = .preparing
        do {
            let handle = try await reviews.prepareRustTargetPlanReview(
                scanID: scanID,
                candidateID: candidate.candidateID
            )
            guard
                operation == rustTargetPlanReviewGeneration,
                snapshotOperation == generation,
                candidateOperation == candidateGeneration,
                detailOperation == candidateDetailGeneration,
                self.scanID == scanID,
                contentMode == .candidates,
                selectedCandidateID == candidate.candidateID
            else {
                await reviews.releaseRustTargetPlanReview(handle)
                return
            }
            guard !Task.isCancelled else {
                await reviews.releaseRustTargetPlanReview(handle)
                if operation == rustTargetPlanReviewGeneration {
                    rustTargetPlanReviewState = .idle
                }
                return
            }
            guard
                handle.info.sourceScanID == scanID,
                ExplorerRustTargetPlanReviewAdapter.matches(
                    handle.info,
                    candidate: candidate
                )
            else {
                await reviews.releaseRustTargetPlanReview(handle)
                rustTargetPlanReviewState = .failed(.invalidResponse)
                return
            }
            rustTargetPlanReviewHandle = handle
            rustTargetPlanReviewState = .ready(handle.info)
            scheduleRustTargetPlanReviewExpiry(
                handle: handle,
                operation: operation
            )
        } catch {
            if Task.isCancelled {
                if
                    operation == rustTargetPlanReviewGeneration,
                    snapshotOperation == generation,
                    candidateOperation == candidateGeneration,
                    detailOperation == candidateDetailGeneration,
                    self.scanID == scanID,
                    contentMode == .candidates,
                    selectedCandidateID == candidate.candidateID
                {
                    rustTargetPlanReviewState = .idle
                }
                return
            }
            guard
                operation == rustTargetPlanReviewGeneration,
                snapshotOperation == generation,
                candidateOperation == candidateGeneration,
                detailOperation == candidateDetailGeneration,
                self.scanID == scanID,
                contentMode == .candidates,
                selectedCandidateID == candidate.candidateID
            else {
                return
            }
            let mapped = (error as? ExplorerRustTargetPlanReviewError) ?? .unavailable
            if mapped == .reviewExpired {
                await releaseRustTargetPlanReview()
                guard
                    snapshotOperation == generation,
                    candidateOperation == candidateGeneration,
                    detailOperation == candidateDetailGeneration,
                    self.scanID == scanID
                else {
                    return
                }
                rustTargetPlanReviewState = .expired
            } else if mapped == .parentReviewUnavailable || mapped == .reviewNotAcquired {
                await releaseRustTargetPlanReview()
                await reviews.release(scanID: scanID)
                guard
                    snapshotOperation == generation,
                    candidateOperation == candidateGeneration,
                    detailOperation == candidateDetailGeneration,
                    self.scanID == scanID
                else {
                    return
                }
                clearContent()
                phase = .failed(.expired)
            } else {
                rustTargetPlanReviewState = .failed(mapped)
            }
        }
    }

    func closeRustTargetPlanReview() async {
        await releaseRustTargetPlanReview()
    }

    func makeRustTargetCleanupConfirmation()
        -> ExplorerRustTargetCleanupConfirmation?
    {
        guard
            rustTargetCleanupState == .idle,
            rustTargetDryRunState == .idle,
            case let .ready(info) = rustTargetPlanReviewState,
            let handle = rustTargetPlanReviewHandle,
            handle.info == info
        else {
            return nil
        }
        return ExplorerRustTargetCleanupConfirmation(
            generation: rustTargetCleanupGeneration,
            reviewHandleID: handle.id,
            info: info
        )
    }

    /// Starts cleanup only from the exact confirmation currently displayed by
    /// SwiftUI. The controller, not these display facts, owns and consumes the
    /// opaque core review used as execution authority.
    func startConfirmedRustTargetCleanup(
        _ confirmation: ExplorerRustTargetCleanupConfirmation
    ) async {
        guard
            rustTargetCleanupState == .idle,
            rustTargetDryRunState == .idle,
            confirmation.generation == rustTargetCleanupGeneration,
            case let .ready(info) = rustTargetPlanReviewState,
            info == confirmation.info,
            let handle = rustTargetPlanReviewHandle,
            handle.id == confirmation.reviewHandleID,
            handle.info == confirmation.info
        else {
            return
        }

        rustTargetPlanReviewGeneration &+= 1
        rustTargetPlanReviewExpiryTask?.cancel()
        rustTargetPlanReviewExpiryTask = nil
        rustTargetPlanReviewHandle = nil
        rustTargetPlanReviewState = .idle

        rustTargetCleanupGeneration &+= 1
        let operation = rustTargetCleanupGeneration
        rustTargetCleanupCancellationRequested = false
        rustTargetCleanupState = .starting(info)
        let reviews = reviews
        let clock = rustTargetCleanupPollingClock
        let driver = Task { @MainActor [weak self] in
            guard let self else {
                return
            }
            do {
                let task = try await reviews.startRustTargetCleanup(handle)
                guard operation == self.rustTargetCleanupGeneration else {
                    _ = try? await task.requestCancellation()
                    return
                }
                self.rustTargetCleanupTask = task
                if self.rustTargetCleanupCancellationRequested {
                    _ = try? await task.requestCancellation()
                }

                while operation == self.rustTargetCleanupGeneration {
                    let poll = try await task.poll()
                    guard operation == self.rustTargetCleanupGeneration else {
                        _ = try? await task.requestCancellation()
                        return
                    }
                    self.rustTargetCleanupState = .observing(info, poll)
                    if poll.phase.isTerminal {
                        await self.rustTargetCleanupTerminalObserver?()
                        self.finishRustTargetCleanupDriver(operation: operation)
                        return
                    }
                    do {
                        try await clock.sleepUntilNextPoll()
                    } catch {
                        // A confirmed core task must not lose observation merely
                        // because the initiating Swift task was cancelled.
                        continue
                    }
                }
                _ = try? await task.requestCancellation()
            } catch {
                guard operation == self.rustTargetCleanupGeneration else {
                    return
                }
                if self.rustTargetCleanupTask == nil {
                    let startError = (error as? ExplorerRustTargetCleanupStartError)
                        ?? .invalidResponse
                    self.rustTargetCleanupState = .startFailed(
                        info,
                        startError.failure
                    )
                } else {
                    self.rustTargetCleanupState = .observationFailed(info)
                }
                self.finishRustTargetCleanupDriver(operation: operation)
            }
        }
        rustTargetCleanupDriverTask = driver
        await driver.value
    }

    func cancelRustTargetCleanup() async {
        guard rustTargetCleanupState.isActive else {
            return
        }
        rustTargetCleanupCancellationRequested = true
        guard let task = rustTargetCleanupTask else {
            return
        }
        _ = try? await task.requestCancellation()
    }

    func dismissRustTargetCleanupResult() async {
        guard
            !rustTargetCleanupState.isActive,
            rustTargetCleanupDriverTask == nil
        else {
            return
        }
        rustTargetCleanupGeneration &+= 1
        rustTargetCleanupState = .idle
        rustTargetCleanupCancellationRequested = false
        if phase == .ready, contentMode == .candidates {
            await reloadCandidates()
        }
    }

    /// App shutdown explicitly requests cancellation and waits for the
    /// confirmed operation observer. Ordinary Explorer dismissal deliberately
    /// does neither; dropping a window must not pretend to undo an effect.
    func shutdownRustTargetCleanup() async {
        rustTargetCleanupCancellationRequested = true
        if let task = rustTargetCleanupTask {
            _ = try? await task.requestCancellation()
        }
        if let driver = rustTargetCleanupDriverTask {
            await driver.value
        }
    }

    private func finishRustTargetCleanupDriver(operation: UInt64) {
        guard operation == rustTargetCleanupGeneration else {
            return
        }
        rustTargetCleanupTask = nil
        rustTargetCleanupDriverTask = nil
    }

    /// Consumes the exact displayed plan review into read-only validation.
    /// Unlike permanent cleanup, this requires no destructive confirmation and
    /// publishes only path-free task observations.
    func startRustTargetDryRun() async {
        guard
            rustTargetDryRunState == .idle,
            rustTargetCleanupState == .idle,
            case let .ready(info) = rustTargetPlanReviewState,
            let handle = rustTargetPlanReviewHandle,
            handle.info == info
        else {
            return
        }

        rustTargetPlanReviewGeneration &+= 1
        rustTargetPlanReviewExpiryTask?.cancel()
        rustTargetPlanReviewExpiryTask = nil
        rustTargetPlanReviewHandle = nil
        rustTargetPlanReviewState = .idle

        rustTargetDryRunGeneration &+= 1
        let operation = rustTargetDryRunGeneration
        rustTargetDryRunCancellationRequested = false
        rustTargetDryRunState = .starting(info)
        let reviews = reviews
        let clock = rustTargetDryRunPollingClock
        let driver = Task { @MainActor [weak self] in
            guard let self else {
                return
            }
            do {
                let task = try await reviews.startRustTargetDryRun(handle)
                guard operation == self.rustTargetDryRunGeneration else {
                    _ = try? await task.requestCancellation()
                    return
                }
                self.rustTargetDryRunTask = task
                if self.rustTargetDryRunCancellationRequested {
                    _ = try? await task.requestCancellation()
                }

                while operation == self.rustTargetDryRunGeneration {
                    let poll = try await task.poll()
                    guard operation == self.rustTargetDryRunGeneration else {
                        _ = try? await task.requestCancellation()
                        return
                    }
                    self.rustTargetDryRunState = .observing(info, poll)
                    if poll.phase.isTerminal {
                        await self.rustTargetDryRunTerminalObserver?()
                        self.finishRustTargetDryRunDriver(operation: operation)
                        return
                    }
                    do {
                        try await clock.sleepUntilNextPoll()
                    } catch {
                        // Once admitted, keep observing the core task even if
                        // the initiating Swift task is cancelled.
                        continue
                    }
                }
                _ = try? await task.requestCancellation()
            } catch {
                guard operation == self.rustTargetDryRunGeneration else {
                    return
                }
                if self.rustTargetDryRunTask == nil {
                    let startError = (error as? ExplorerRustTargetDryRunStartError)
                        ?? .invalidResponse
                    self.rustTargetDryRunState = .startFailed(
                        info,
                        startError.failure
                    )
                } else {
                    self.rustTargetDryRunState = .observationFailed(info)
                }
                self.finishRustTargetDryRunDriver(operation: operation)
            }
        }
        rustTargetDryRunDriverTask = driver
        await driver.value
    }

    func cancelRustTargetDryRun() async {
        guard rustTargetDryRunState.isActive else {
            return
        }
        rustTargetDryRunCancellationRequested = true
        guard let task = rustTargetDryRunTask else {
            return
        }
        _ = try? await task.requestCancellation()
    }

    func dismissRustTargetDryRunResult() async {
        guard
            !rustTargetDryRunState.isActive,
            rustTargetDryRunDriverTask == nil
        else {
            return
        }
        rustTargetDryRunGeneration &+= 1
        rustTargetDryRunState = .idle
        rustTargetDryRunCancellationRequested = false
        if phase == .ready, contentMode == .candidates {
            await reloadCandidates()
        }
    }

    func shutdownRustTargetDryRun() async {
        rustTargetDryRunCancellationRequested = true
        if let task = rustTargetDryRunTask {
            _ = try? await task.requestCancellation()
        }
        if let driver = rustTargetDryRunDriverTask {
            await driver.value
        }
    }

    private func finishRustTargetDryRunDriver(operation: UInt64) {
        guard operation == rustTargetDryRunGeneration else {
            return
        }
        rustTargetDryRunTask = nil
        rustTargetDryRunDriverTask = nil
    }

    private func loadCandidatePathPage(cursor: UInt16) async {
        guard
            phase == .ready,
            contentMode == .candidates,
            let scanID,
            let candidate = selectedCandidate,
            !isSwitchingSnapshot,
            !isCandidateDetailLoading,
            !isCandidatePathPaging
        else {
            return
        }
        candidatePathGeneration &+= 1
        let operation = candidatePathGeneration
        let detailOperation = candidateDetailGeneration
        let snapshotGeneration = generation
        isCandidatePathPaging = true
        candidateDetailFailure = nil
        do {
            let page = try await reviews.candidatePaths(
                scanID: scanID,
                candidateID: candidate.candidateID,
                cursor: cursor,
                limit: ExplorerCandidateDetailAdapter.maximumPageLimit
            )
            guard
                operation == candidatePathGeneration,
                detailOperation == candidateDetailGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .candidates,
                selectedCandidateID == candidate.candidateID
            else {
                return
            }
            guard !Task.isCancelled else {
                isCandidatePathPaging = false
                return
            }
            guard
                Self.validCandidatePathPage(
                    page,
                    candidate: candidate,
                    scanID: scanID,
                    cursor: cursor
                )
            else {
                isCandidatePathPaging = false
                candidateDetailFailure = .invalidResponse
                return
            }
            candidatePathPage = page
            isCandidatePathPaging = false
        } catch {
            guard
                operation == candidatePathGeneration,
                detailOperation == candidateDetailGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .candidates,
                selectedCandidateID == candidate.candidateID
            else {
                return
            }
            guard !Task.isCancelled else {
                isCandidatePathPaging = false
                return
            }
            isCandidatePathPaging = false
            await publishCandidateDetailFailure(
                error,
                scanID: scanID,
                detailOperation: detailOperation
            )
        }
    }

    private func loadCandidateEvidencePage(cursor: UInt16) async {
        guard
            phase == .ready,
            contentMode == .candidates,
            let scanID,
            let candidate = selectedCandidate,
            !isSwitchingSnapshot,
            !isCandidateDetailLoading,
            !isCandidateEvidencePaging
        else {
            return
        }
        candidateEvidenceGeneration &+= 1
        let operation = candidateEvidenceGeneration
        let detailOperation = candidateDetailGeneration
        let snapshotGeneration = generation
        isCandidateEvidencePaging = true
        candidateDetailFailure = nil
        do {
            let page = try await reviews.candidateEvidence(
                scanID: scanID,
                candidateID: candidate.candidateID,
                cursor: cursor,
                limit: ExplorerCandidateDetailAdapter.maximumPageLimit
            )
            guard
                operation == candidateEvidenceGeneration,
                detailOperation == candidateDetailGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .candidates,
                selectedCandidateID == candidate.candidateID
            else {
                return
            }
            guard !Task.isCancelled else {
                isCandidateEvidencePaging = false
                return
            }
            guard
                Self.validCandidateEvidencePage(
                    page,
                    candidate: candidate,
                    scanID: scanID,
                    cursor: cursor
                )
            else {
                isCandidateEvidencePaging = false
                candidateDetailFailure = .invalidResponse
                return
            }
            candidateEvidencePage = page
            isCandidateEvidencePaging = false
        } catch {
            guard
                operation == candidateEvidenceGeneration,
                detailOperation == candidateDetailGeneration,
                snapshotGeneration == generation,
                self.scanID == scanID,
                contentMode == .candidates,
                selectedCandidateID == candidate.candidateID
            else {
                return
            }
            guard !Task.isCancelled else {
                isCandidateEvidencePaging = false
                return
            }
            isCandidateEvidencePaging = false
            await publishCandidateDetailFailure(
                error,
                scanID: scanID,
                detailOperation: detailOperation
            )
        }
    }

    private func publishCandidateDetailFailure(
        _ error: Error,
        scanID: String,
        detailOperation: UInt64
    ) async {
        let failure = Self.failure(for: error)
        if failure == .expired {
            await releaseRustTargetPlanReview()
            await reviews.release(scanID: scanID)
            guard
                detailOperation == candidateDetailGeneration,
                self.scanID == scanID
            else {
                return
            }
            clearContent()
            phase = .failed(.expired)
        } else {
            guard
                detailOperation == candidateDetailGeneration,
                self.scanID == scanID,
                contentMode == .candidates
            else {
                return
            }
            candidateDetailFailure = failure
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
                await releaseRustTargetPlanReview()
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

    func reloadICloudObservationSource() async {
        guard
            phase == .ready,
            contentMode == .iCloudStatus,
            let scanID,
            let scopeNodeID = currentDirectory?.id,
            !isSwitchingSnapshot
        else {
            return
        }
        guard !iCloudObservationBatchActive else {
            iCloudObservationCancellationRequested = true
            iCloudObservationReloadPending = true
            if case let .checking(completed, total) = iCloudObservationBatchPhase {
                iCloudObservationBatchPhase = .stopping(
                    completed: completed,
                    total: total
                )
            }
            return
        }

        invalidateICloudLocalCopyReview()
        iCloudObservationGeneration &+= 1
        let operation = iCloudObservationGeneration
        let snapshotOperation = generation
        iCloudObservationSource = nil
        iCloudObservationSourceFailure = nil
        iCloudObservationResults = [:]
        checkingICloudObservationNodeID = nil
        iCloudObservationBatchPhase = .idle
        isICloudObservationSourceLoading = true

        do {
            let source = try await reviews.icloudObservationSource(
                scanID: scanID,
                scopeNodeID: scopeNodeID,
                maxResults: Self.iCloudObservationResultLimit
            )
            guard
                operation == iCloudObservationGeneration,
                snapshotOperation == generation,
                self.scanID == scanID,
                currentDirectory?.id == scopeNodeID,
                contentMode == .iCloudStatus,
                !Task.isCancelled
            else {
                return
            }
            iCloudObservationSource = source
            iCloudObservationSourceFailure = nil
            isICloudObservationSourceLoading = false
            selection = nil
            selectedNodeSnapshot = nil
        } catch {
            guard
                operation == iCloudObservationGeneration,
                snapshotOperation == generation,
                self.scanID == scanID,
                currentDirectory?.id == scopeNodeID,
                contentMode == .iCloudStatus,
                !Task.isCancelled
            else {
                return
            }
            isICloudObservationSourceLoading = false
            let mapped = (error as? ExplorerICloudObservationSourceError)
                ?? .unavailable
            if mapped == .expired {
                await reviews.release(scanID: scanID)
                guard
                    operation == iCloudObservationGeneration,
                    snapshotOperation == generation,
                    self.scanID == scanID
                else {
                    return
                }
                clearContent()
                phase = .failed(.expired)
            } else {
                iCloudObservationSourceFailure = mapped
            }
        }
    }

    func selectICloudObservationTarget(_ nodeID: UInt64?) {
        invalidateLiveAction()
        guard let nodeID else {
            selection = nil
            selectedNodeSnapshot = nil
            return
        }
        guard
            let target = iCloudObservationSource?.targets.first(where: {
                $0.id == nodeID
            })
        else {
            return
        }
        selection = .iCloudObservation(nodeID)
        selectedNodeSnapshot = target.node
    }

    func startICloudObservationBatch() async {
        guard
            canStartICloudObservationBatch,
            let source = iCloudObservationSource,
            let scanID,
            let scopeNodeID = currentDirectory?.id,
            source.scanID == scanID,
            source.scopeNodeID == scopeNodeID
        else {
            return
        }

        iCloudObservationGeneration &+= 1
        let operation = iCloudObservationGeneration
        let snapshotOperation = generation
        let total = source.targets.count
        iCloudObservationBatchActive = true
        iCloudObservationCancellationRequested = false
        iCloudObservationReloadPending = false
        iCloudObservationResults = [:]
        checkingICloudObservationNodeID = nil
        iCloudObservationBatchPhase = .checking(completed: 0, total: total)
        defer {
            iCloudObservationBatchActive = false
            checkingICloudObservationNodeID = nil
            if iCloudObservationReloadPending {
                iCloudObservationReloadPending = false
                Task { [weak self] in
                    await self?.reloadICloudObservationSource()
                }
            }
        }

        var completed = 0
        for target in source.targets {
            guard iCloudObservationContextMatches(
                operation: operation,
                snapshotOperation: snapshotOperation,
                scanID: scanID,
                scopeNodeID: scopeNodeID,
                source: source
            ) else {
                return
            }
            if iCloudObservationCancellationRequested || Task.isCancelled {
                iCloudObservationBatchPhase = .cancelled(
                    completed: completed,
                    total: total
                )
                return
            }

            checkingICloudObservationNodeID = target.id
            iCloudObservationBatchPhase = .checking(
                completed: completed,
                total: total
            )
            do {
                let assessment = try await reviews.probeICloudLocalCopy(
                    scanID: scanID,
                    nodeID: target.id
                )
                guard iCloudObservationContextMatches(
                    operation: operation,
                    snapshotOperation: snapshotOperation,
                    scanID: scanID,
                    scopeNodeID: scopeNodeID,
                    source: source
                ) else {
                    return
                }
                guard
                    !iCloudObservationCancellationRequested,
                    !Task.isCancelled
                else {
                    iCloudObservationBatchPhase = .cancelled(
                        completed: completed,
                        total: total
                    )
                    return
                }
                guard assessment.localAllocatedBytes == target.node.allocatedBytes else {
                    completed += 1
                    iCloudObservationResults[target.id] = .failed(.invalidResponse)
                    iCloudObservationBatchPhase = .stopped(
                        error: .invalidResponse,
                        completed: completed,
                        total: total
                    )
                    return
                }
                iCloudObservationResults[target.id] = .observed(assessment)
                completed += 1
            } catch is CancellationError {
                guard iCloudObservationContextMatches(
                    operation: operation,
                    snapshotOperation: snapshotOperation,
                    scanID: scanID,
                    scopeNodeID: scopeNodeID,
                    source: source
                ) else {
                    return
                }
                iCloudObservationBatchPhase = .cancelled(
                    completed: completed,
                    total: total
                )
                return
            } catch {
                guard iCloudObservationContextMatches(
                    operation: operation,
                    snapshotOperation: snapshotOperation,
                    scanID: scanID,
                    scopeNodeID: scopeNodeID,
                    source: source
                ) else {
                    return
                }
                guard
                    !iCloudObservationCancellationRequested,
                    !Task.isCancelled
                else {
                    iCloudObservationBatchPhase = .cancelled(
                        completed: completed,
                        total: total
                    )
                    return
                }
                let mapped = (error as? ExplorerICloudLocalCopyProbeError)
                    ?? .failed
                iCloudObservationResults[target.id] = .failed(mapped)
                completed += 1
                if Self.isSystemicICloudObservationFailure(mapped) {
                    iCloudObservationBatchPhase = .stopped(
                        error: mapped,
                        completed: completed,
                        total: total
                    )
                    return
                }
            }
        }
        checkingICloudObservationNodeID = nil
        iCloudObservationBatchPhase = .completed(total: completed)
    }

    func stopICloudObservationBatch() {
        guard canStopICloudObservationBatch else {
            return
        }
        iCloudObservationCancellationRequested = true
        if case let .checking(completed, total) = iCloudObservationBatchPhase {
            iCloudObservationBatchPhase = .stopping(
                completed: completed,
                total: total
            )
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

    func checkSelectedICloudLocalCopy(nodeID requestedNodeID: UInt64? = nil) async {
        guard
            canCheckSelectedICloudLocalCopy,
            let scanID,
            let node = liveActionNode(requestedNodeID),
            node.kind == .file
        else {
            return
        }

        iCloudLocalCopyReviewGeneration &+= 1
        let reviewOperation = iCloudLocalCopyReviewGeneration
        let snapshotOperation = generation
        let largeFilesOperation = largeFilesGeneration
        let contentModeOperation = contentMode
        let nodeID = node.id
        iCloudLocalCopyReviewState = .checking
        do {
            let assessment = try await reviews.probeICloudLocalCopy(
                scanID: scanID,
                nodeID: nodeID
            )
            guard
                reviewOperation == iCloudLocalCopyReviewGeneration,
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
            iCloudLocalCopyReviewState = .observed(assessment)
        } catch {
            guard
                reviewOperation == iCloudLocalCopyReviewGeneration,
                snapshotOperation == generation,
                largeFilesOperation == largeFilesGeneration,
                contentModeOperation == contentMode,
                self.scanID == scanID,
                selectedNodeID == nodeID,
                !Task.isCancelled
            else {
                return
            }
            let mapped = (error as? ExplorerICloudLocalCopyProbeError) ?? .failed
            iCloudLocalCopyReviewState = .failed(mapped)
        }
    }

    func trashSelectedItem(nodeID: UInt64? = nil) async {
        guard
            canTrashSelectedItem,
            let scanID,
            let node = liveActionNode(nodeID),
            node.kind == .file || node.kind == .directory || node.kind == .symlink
        else { return }
        invalidateICloudLocalCopyReview()
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
        pendingExactScanID = nil
        generation &+= 1
        historyGeneration &+= 1
        subtreeRefreshGeneration &+= 1
        let retainedScanID = scanID
        await releaseSnapshotDiffReview()
        await releaseRustTargetPlanReview()
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

        await releaseSnapshotDiffReview()
        await releaseRustTargetPlanReview()
        guard
            requestedScanID != sourceScanID,
            refreshOperation == subtreeRefreshGeneration,
            scanID == sourceScanID,
            !isSwitchingSnapshot
        else {
            return
        }
        invalidateLiveAction()
        invalidateICloudObservationContext()
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
            if contentMode == .changes {
                await prepareSnapshotDiffReview()
            } else if contentMode == .largeFiles {
                await reloadLargeFiles()
            } else if contentMode == .iCloudStatus {
                await reloadICloudObservationSource()
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
                await releaseRustTargetPlanReview()
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
        invalidateICloudObservationContext()
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

    private func prepareSnapshotDiffReview() async {
        guard
            contentMode == .changes,
            phase == .ready,
            let scanID,
            snapshotDiffHandle == nil
        else {
            return
        }
        snapshotDiffGeneration &+= 1
        let operation = snapshotDiffGeneration
        snapshotDiffPhase = .loading
        snapshotDiffOperationFailure = nil
        var preparedHandle: ExplorerSnapshotDiffReviewHandle?
        do {
            let handle = try await reviews.prepareSnapshotDiffReview(scanID: scanID)
            preparedHandle = handle
            guard
                operation == snapshotDiffGeneration,
                contentMode == .changes,
                self.scanID == scanID,
                phase == .ready,
                !Task.isCancelled
            else {
                await reviews.releaseSnapshotDiffReview(handle)
                return
            }
            let root = try await reviews.snapshotDiffRootNode(handle)
            async let page = reviews.snapshotDiffChildNodes(
                handle,
                parentID: root.id,
                sort: snapshotDiffSort,
                offset: 0,
                limit: Self.pageLimit
            )
            async let treemap = reviews.snapshotDiffTreemap(
                handle,
                parentID: root.id,
                maxCells: Self.treemapCellLimit
            )
            let (loadedPage, loadedTreemap) = try await (page, treemap)
            try validateSnapshotDiffPair(
                page: loadedPage,
                treemap: loadedTreemap,
                parentID: root.id
            )
            guard
                operation == snapshotDiffGeneration,
                contentMode == .changes,
                self.scanID == scanID,
                phase == .ready,
                !Task.isCancelled
            else {
                await reviews.releaseSnapshotDiffReview(handle)
                return
            }
            snapshotDiffHandle = handle
            snapshotDiffInfo = handle.info
            snapshotDiffBreadcrumbs = [root]
            snapshotDiffPage = loadedPage
            snapshotDiffTreemap = loadedTreemap
            snapshotDiffSelection = nil
            snapshotDiffPhase = .ready
            preparedHandle = nil
        } catch {
            if let preparedHandle {
                await reviews.releaseSnapshotDiffReview(preparedHandle)
            }
            guard
                operation == snapshotDiffGeneration,
                contentMode == .changes,
                self.scanID == scanID,
                !Task.isCancelled
            else {
                return
            }
            snapshotDiffPhase = .failed(Self.snapshotDiffFailure(error))
        }
    }

    private func loadSnapshotDiffDirectory(
        _ directory: ExplorerSnapshotDiffNode,
        breadcrumbIndex: Int?
    ) async {
        guard
            let handle = snapshotDiffHandle,
            directory.canDescend,
            snapshotDiffPhase == .ready
        else {
            return
        }
        snapshotDiffGeneration &+= 1
        let operation = snapshotDiffGeneration
        let scanID = scanID
        isSnapshotDiffNavigating = true
        snapshotDiffOperationFailure = nil
        do {
            async let page = reviews.snapshotDiffChildNodes(
                handle,
                parentID: directory.id,
                sort: snapshotDiffSort,
                offset: 0,
                limit: Self.pageLimit
            )
            async let treemap = reviews.snapshotDiffTreemap(
                handle,
                parentID: directory.id,
                maxCells: Self.treemapCellLimit
            )
            let (loadedPage, loadedTreemap) = try await (page, treemap)
            try validateSnapshotDiffPair(
                page: loadedPage,
                treemap: loadedTreemap,
                parentID: directory.id
            )
            guard
                operation == snapshotDiffGeneration,
                snapshotDiffHandle == handle,
                contentMode == .changes,
                self.scanID == scanID,
                !Task.isCancelled
            else {
                return
            }
            if let breadcrumbIndex {
                snapshotDiffBreadcrumbs = Array(
                    snapshotDiffBreadcrumbs.prefix(breadcrumbIndex + 1)
                )
            } else {
                snapshotDiffBreadcrumbs.append(directory)
            }
            snapshotDiffPage = loadedPage
            snapshotDiffTreemap = loadedTreemap
            snapshotDiffSelection = nil
            isSnapshotDiffNavigating = false
        } catch {
            guard
                operation == snapshotDiffGeneration,
                snapshotDiffHandle == handle,
                contentMode == .changes
            else {
                return
            }
            isSnapshotDiffNavigating = false
            await publishSnapshotDiffFailure(error)
        }
    }

    private func loadSnapshotDiffPage(
        directory: ExplorerSnapshotDiffNode,
        offset: UInt64,
        preserveTreemap: Bool
    ) async throws {
        guard
            let handle = snapshotDiffHandle,
            snapshotDiffPhase == .ready
        else {
            throw ExplorerSnapshotDiffFailure.expired
        }
        snapshotDiffGeneration &+= 1
        let operation = snapshotDiffGeneration
        let scanID = scanID
        isSnapshotDiffPaging = true
        snapshotDiffOperationFailure = nil
        let page = try await reviews.snapshotDiffChildNodes(
            handle,
            parentID: directory.id,
            sort: snapshotDiffSort,
            offset: offset,
            limit: Self.pageLimit
        )
        guard
            operation == snapshotDiffGeneration,
            snapshotDiffHandle == handle,
            contentMode == .changes,
            self.scanID == scanID,
            !Task.isCancelled
        else {
            throw CancellationError()
        }
        if preserveTreemap, let treemap = snapshotDiffTreemap {
            try validateSnapshotDiffPair(
                page: page,
                treemap: treemap,
                parentID: directory.id
            )
        }
        snapshotDiffPage = page
        snapshotDiffSelection = nil
        isSnapshotDiffPaging = false
    }

    private func moveSnapshotDiffPage(to offset: UInt64) async {
        guard
            let directory = currentSnapshotDiffDirectory,
            !isSnapshotDiffNavigating,
            !isSnapshotDiffPaging
        else {
            return
        }
        do {
            try await loadSnapshotDiffPage(
                directory: directory,
                offset: offset,
                preserveTreemap: true
            )
        } catch {
            isSnapshotDiffPaging = false
            await publishSnapshotDiffFailure(error)
        }
    }

    private func validateSnapshotDiffPair(
        page: ExplorerSnapshotDiffNodePage,
        treemap: ExplorerSnapshotDiffTreemap,
        parentID: UInt64
    ) throws {
        guard
            page.parentID == parentID,
            treemap.parentID == parentID,
            page.totalChildren == treemap.totalChildren,
            page.totalGrowthBytes == treemap.totalGrowthBytes,
            page.totalShrinkageBytes == treemap.totalShrinkageBytes,
            page.unchangedChildCount == treemap.unchangedChildCount,
            page.replacedChildCount == treemap.replacedChildCount
        else {
            throw ExplorerSnapshotDiffFailure.invalidResponse
        }
    }

    private func publishSnapshotDiffFailure(_ error: Error) async {
        if error is CancellationError {
            return
        }
        let failure = Self.snapshotDiffFailure(error)
        if failure == .expired || failure == .closed {
            await releaseSnapshotDiffReview()
            guard contentMode == .changes, phase == .ready else {
                return
            }
            snapshotDiffPhase = .failed(failure)
        } else {
            snapshotDiffOperationFailure = failure
        }
    }

    private func releaseSnapshotDiffReview() async {
        let handle = invalidateSnapshotDiffReview()
        if let handle {
            await reviews.releaseSnapshotDiffReview(handle)
        }
    }

    @discardableResult
    private func invalidateSnapshotDiffReview()
        -> ExplorerSnapshotDiffReviewHandle?
    {
        snapshotDiffGeneration &+= 1
        let handle = snapshotDiffHandle
        snapshotDiffHandle = nil
        snapshotDiffPhase = .idle
        snapshotDiffInfo = nil
        snapshotDiffBreadcrumbs = []
        snapshotDiffPage = nil
        snapshotDiffTreemap = nil
        snapshotDiffOperationFailure = nil
        snapshotDiffSelection = nil
        isSnapshotDiffNavigating = false
        isSnapshotDiffPaging = false
        return handle
    }

    private static func snapshotDiffFailure(_ error: Error) -> ExplorerSnapshotDiffFailure {
        if let error = error as? ExplorerSnapshotDiffFailure {
            return error
        }
        if error is CancellationError {
            return .unavailable
        }
        if let error = error as? EngineServiceError {
            return switch error {
            case .closed: .closed
            case .retryable: .unavailable
            case .unavailable, .invalidCapacityObservation,
                 .conflictingCapacityObservation, .supersededCapacityObservation,
                 .unexpected:
                .invalidResponse
            }
        }
        return .invalidResponse
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
            await releaseRustTargetPlanReview()
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
        _ = invalidateSnapshotDiffReview()
        invalidateLiveAction()
        invalidateICloudObservationContext()
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
        clearCandidates()
        clearLargeFiles()
        clearCoverage()
    }

    private func clearCandidates() {
        candidateGeneration &+= 1
        clearCandidateDetail()
        candidatePage = nil
        candidateFailure = nil
        isCandidateLoading = false
        candidateNotice = nil
    }

    private func clearCandidateDetail() {
        candidateDetailGeneration &+= 1
        candidatePathGeneration &+= 1
        candidateEvidenceGeneration &+= 1
        _ = invalidateRustTargetPlanReview()
        selectedCandidateID = nil
        candidatePathPage = nil
        candidateEvidencePage = nil
        candidateDetailFailure = nil
        isCandidateDetailLoading = false
        isCandidatePathPaging = false
        isCandidateEvidencePaging = false
    }

    private func releaseRustTargetPlanReview() async {
        let handle = invalidateRustTargetPlanReview()
        if let handle {
            await reviews.releaseRustTargetPlanReview(handle)
        }
    }

    private func invalidateRustTargetPlanReview()
        -> ExplorerRustTargetPlanReviewHandle?
    {
        rustTargetPlanReviewGeneration &+= 1
        rustTargetPlanReviewExpiryTask?.cancel()
        rustTargetPlanReviewExpiryTask = nil
        let handle = rustTargetPlanReviewHandle
        rustTargetPlanReviewHandle = nil
        rustTargetPlanReviewState = .idle
        return handle
    }

    private func scheduleRustTargetPlanReviewExpiry(
        handle: ExplorerRustTargetPlanReviewHandle,
        operation: UInt64
    ) {
        rustTargetPlanReviewExpiryTask?.cancel()
        let expiry = ExplorerRustTargetPlanReviewAdapter.date(
            for: handle.info.effectiveExpiresAt
        )
        let clock = rustTargetPlanReviewClock
        rustTargetPlanReviewExpiryTask = Task { [weak self] in
            while !Task.isCancelled {
                let now = clock.now()
                guard now < expiry else {
                    await self?.finishRustTargetPlanReview(
                        handle: handle,
                        operation: operation,
                        outcome: .expired
                    )
                    return
                }
                let refresh = min(
                    expiry,
                    now.addingTimeInterval(15)
                )
                do {
                    try await clock.sleep(until: refresh)
                } catch {
                    return
                }
                guard
                    !Task.isCancelled,
                    let self,
                    operation == self.rustTargetPlanReviewGeneration,
                    self.rustTargetPlanReviewHandle?.id == handle.id
                else {
                    return
                }
                guard clock.now() < expiry else {
                    await self.finishRustTargetPlanReview(
                        handle: handle,
                        operation: operation,
                        outcome: .expired
                    )
                    return
                }
                do {
                    let refreshed = try await self.reviews
                        .refreshRustTargetPlanReview(handle)
                    guard
                        operation == self.rustTargetPlanReviewGeneration,
                        self.rustTargetPlanReviewHandle?.id == handle.id
                    else {
                        await self.reviews
                            .releaseRustTargetPlanReview(refreshed)
                        return
                    }
                    guard
                        refreshed.id == handle.id,
                        refreshed.info == handle.info
                    else {
                        await self.finishRustTargetPlanReview(
                            handle: handle,
                            operation: operation,
                            outcome: .failed(.invalidResponse)
                        )
                        return
                    }
                    self.rustTargetPlanReviewHandle = refreshed
                    self.rustTargetPlanReviewState = .ready(refreshed.info)
                } catch {
                    guard !Task.isCancelled else {
                        return
                    }
                    let mapped = (error as? ExplorerRustTargetPlanReviewError)
                        ?? .unavailable
                    await self.finishRustTargetPlanReview(
                        handle: handle,
                        operation: operation,
                        outcome: mapped == .reviewExpired
                            ? .expired
                            : .failed(mapped)
                    )
                    return
                }
            }
        }
    }

    private func finishRustTargetPlanReview(
        handle: ExplorerRustTargetPlanReviewHandle,
        operation: UInt64,
        outcome: ExplorerRustTargetPlanReviewState
    ) async {
        guard
            operation == rustTargetPlanReviewGeneration,
            rustTargetPlanReviewHandle?.id == handle.id
        else {
            return
        }
        rustTargetPlanReviewExpiryTask = nil
        rustTargetPlanReviewHandle = nil
        rustTargetPlanReviewGeneration &+= 1
        let terminalOperation = rustTargetPlanReviewGeneration
        rustTargetPlanReviewState = outcome
        await reviews.releaseRustTargetPlanReview(handle)

        guard
            case let .failed(error) = outcome,
            error == .parentReviewUnavailable || error == .reviewNotAcquired,
            let scanID
        else {
            return
        }
        await reviews.release(scanID: scanID)
        guard
            terminalOperation == rustTargetPlanReviewGeneration,
            self.scanID == scanID
        else {
            return
        }
        clearContent()
        phase = .failed(.expired)
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
        invalidateICloudLocalCopyReview()
    }

    private func invalidateICloudLocalCopyReview() {
        iCloudLocalCopyReviewGeneration &+= 1
        iCloudLocalCopyReviewState = .idle
    }

    private func invalidateICloudObservationContext() {
        let priorPhase = iCloudObservationBatchPhase
        iCloudObservationGeneration &+= 1
        iCloudObservationCancellationRequested = true
        iCloudObservationReloadPending = false
        iCloudObservationSource = nil
        iCloudObservationSourceFailure = nil
        isICloudObservationSourceLoading = false
        iCloudObservationResults = [:]
        checkingICloudObservationNodeID = nil
        if iCloudObservationBatchActive {
            let progress = switch priorPhase {
            case let .checking(completed, total),
                 let .stopping(completed, total),
                 let .cancelled(completed, total):
                (completed, total)
            case let .stopped(_, completed, total):
                (completed, total)
            case let .completed(total):
                (total, total)
            case .idle:
                (0, 0)
            }
            iCloudObservationBatchPhase = .stopping(
                completed: progress.0,
                total: progress.1
            )
        } else {
            iCloudObservationBatchPhase = .idle
        }
    }

    private func iCloudObservationContextMatches(
        operation: UInt64,
        snapshotOperation: UInt64,
        scanID: String,
        scopeNodeID: UInt64,
        source: ExplorerICloudObservationSource
    ) -> Bool {
        operation == iCloudObservationGeneration
            && snapshotOperation == generation
            && self.scanID == scanID
            && currentDirectory?.id == scopeNodeID
            && contentMode == .iCloudStatus
            && iCloudObservationSource == source
            && phase == .ready
            && !isSwitchingSnapshot
    }

    private static func isSystemicICloudObservationFailure(
        _ error: ExplorerICloudLocalCopyProbeError
    ) -> Bool {
        switch error {
        case .invalidTarget, .changedSinceSnapshot, .failed:
            false
        case .unavailable, .unsupported, .invalidResponse:
            true
        }
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
                 .candidateNotFound, .notReviewable, .invalidResponse:
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

    private static func validCandidatePathPage(
        _ page: ExplorerCandidatePathPage,
        candidate: ExplorerCandidateSummary,
        scanID: String,
        cursor: UInt16
    ) -> Bool {
        page.scanID == scanID
            && page.cursor == cursor
            && page.totalPaths == candidate.pathCount
            && sameCandidateObservation(page.candidate, candidate)
            && validCandidatePageWindow(
                cursor: page.cursor,
                count: page.paths.count,
                total: page.totalPaths,
                nextCursor: page.nextCursor
            )
    }

    private static func validCandidateEvidencePage(
        _ page: ExplorerCandidateEvidencePage,
        candidate: ExplorerCandidateSummary,
        scanID: String,
        cursor: UInt16
    ) -> Bool {
        guard let expectedEvidence = UInt16(exactly: candidate.evidenceKinds.count) else {
            return false
        }
        return page.scanID == scanID
            && page.cursor == cursor
            && page.totalEvidence == expectedEvidence
            && sameCandidateObservation(page.candidate, candidate)
            && validCandidatePageWindow(
                cursor: page.cursor,
                count: page.evidence.count,
                total: page.totalEvidence,
                nextCursor: page.nextCursor
            )
    }

    private static func validCandidatePageWindow(
        cursor: UInt16,
        count: Int,
        total: UInt16,
        nextCursor: UInt16?
    ) -> Bool {
        let limit = ExplorerCandidateDetailAdapter.maximumPageLimit
        guard cursor <= total, count <= Int(limit) else {
            return false
        }
        let remaining = total - cursor
        guard count == Int(min(limit, remaining)) else {
            return false
        }
        let end = UInt32(cursor) + UInt32(count)
        guard end <= UInt32(total) else {
            return false
        }
        return end < UInt32(total) ? nextCursor == UInt16(end) : nextCursor == nil
    }

    private static func sameCandidateObservation(
        _ lhs: ExplorerCandidateSummary,
        _ rhs: ExplorerCandidateSummary
    ) -> Bool {
        lhs.candidateID == rhs.candidateID
            && lhs.ruleID == rhs.ruleID
            && lhs.ruleRevision == rhs.ruleRevision
            && lhs.category == rhs.category
            && lhs.estimatedBytes == rhs.estimatedBytes
            && lhs.newestMtime == rhs.newestMtime
            && lhs.safety == rhs.safety
            && lhs.action == rhs.action
            && lhs.ruleScheduleEligible == rhs.ruleScheduleEligible
            && lhs.pathCount == rhs.pathCount
            && lhs.evidenceKinds == rhs.evidenceKinds
            && lhs.blockers == rhs.blockers
            && lhs.createdAt == rhs.createdAt
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
