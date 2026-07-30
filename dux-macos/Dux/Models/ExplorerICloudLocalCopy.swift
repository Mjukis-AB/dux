import Foundation

enum ExplorerICloudBooleanFact: Equatable, Sendable {
    case yes
    case no
    case unknown
}

enum ExplorerICloudTransferErrorState: Equatable, Sendable {
    case absent
    case present
    case unknown
}

enum ExplorerICloudLocalCopyState: Equatable, Sendable {
    case current
    case stale
    case notDownloaded
    case unknown
}

enum ExplorerICloudLocalCopyBlockReason: Equatable, Sendable {
    case unsupportedItemKind
    case ubiquityUnknown
    case notUbiquitous
    case uploadStateUnknown
    case uploadIncomplete
    case uploadActivityUnknown
    case uploadInProgress
    case uploadErrorUnknown
    case uploadErrorPresent
    case conflictStateUnknown
    case unresolvedConflicts
    case localCopyStateUnknown
    case staleLocalCopy
    case noLocalCopy
    case downloadRequestUnknown
    case downloadRequested
    case downloadActivityUnknown
    case downloadInProgress
    case downloadErrorUnknown
    case downloadErrorPresent
    case syncExclusionUnknown
    case excludedFromSync
    case allocationUnknown
    case noLocalAllocation
    case invalidObservationTime
}

/// Path-free, point-in-time iCloud metadata classified by Rust.
///
/// A favorable observation is discovery only. It is not a candidate,
/// approval, cleanup plan, or eviction capability.
struct ExplorerICloudLocalCopyAssessment: Equatable, Sendable {
    let localAllocatedBytes: UInt64
    let observedAtUnixMilliseconds: Int64
    let ubiquitous: ExplorerICloudBooleanFact
    let uploaded: ExplorerICloudBooleanFact
    let uploading: ExplorerICloudBooleanFact
    let uploadError: ExplorerICloudTransferErrorState
    let unresolvedConflicts: ExplorerICloudBooleanFact
    let localCopyState: ExplorerICloudLocalCopyState
    let downloadRequested: ExplorerICloudBooleanFact
    let downloading: ExplorerICloudBooleanFact
    let downloadError: ExplorerICloudTransferErrorState
    let excludedFromSync: ExplorerICloudBooleanFact
    let isEligibleObservation: Bool
    let blockers: [ExplorerICloudLocalCopyBlockReason]
}

enum ExplorerICloudLocalCopyProbeError: Error, Equatable, Sendable {
    case unavailable
    case invalidTarget
    case changedSinceSnapshot
    case unsupported
    case failed
    case invalidResponse
}

/// One path-free historical file selected for an explicit iCloud metadata
/// observation. Snapshot allocation is only a ranking input; it is not a
/// reclaim estimate or evidence that the file belongs to iCloud.
struct ExplorerICloudObservationTarget: Equatable, Identifiable, Sendable {
    var id: UInt64 { node.id }

    let rank: UInt16
    let node: ExplorerSnapshotNode
    let parentContext: [ExplorerSnapshotNodeName]
    let contextTruncated: Bool

    var parentDisplay: String {
        let joined = parentContext.map(\.display).joined(separator: " / ")
        return contextTruncated ? "… / \(joined)" : joined
    }
}

/// A bounded, deterministic, retained-snapshot source for manual iCloud
/// observations. It contains no live paths, provider conclusions, cleanup
/// candidates, or destructive authority.
struct ExplorerICloudObservationSource: Equatable, Sendable {
    let scanID: String
    let scopeNodeID: UInt64
    let requestedMaxResults: UInt16
    let visitedNodeCount: UInt64
    let totalRankedFiles: UInt64
    let hasMore: Bool
    let targets: [ExplorerICloudObservationTarget]
}

enum ExplorerICloudObservationSourceError: Error, Equatable, Sendable {
    case invalidRequest
    case reviewNotAcquired
    case expired
    case budgetExceeded
    case unavailable
    case invalidResponse
}

/// Strict conversion boundary for the Rust-owned, path-free observation
/// source. Any inconsistent accounting, ranking, name, or node record fails
/// closed before it reaches Explorer state.
enum ExplorerICloudObservationSourceAdapter {
    static let maximumResults: UInt16 = 32
    static let maximumVisitedNodes: UInt64 = 200_000

    private static let recordVersion: UInt32 = 1
    private static let maximumContextComponents = 8

    static func request(
        scopeNodeID: UInt64,
        maxResults: UInt16
    ) throws -> SnapshotICloudObservationSourceRequest {
        guard (1 ... maximumResults).contains(maxResults) else {
            throw ExplorerICloudObservationSourceError.invalidRequest
        }
        return SnapshotICloudObservationSourceRequest(
            recordVersion: recordVersion,
            scopeNodeId: scopeNodeID,
            maxResults: maxResults
        )
    }

    static func map(
        _ raw: SnapshotICloudObservationSource,
        expectedScanID: String,
        expectedScopeNodeID: UInt64,
        requestedMaxResults: UInt16
    ) throws -> ExplorerICloudObservationSource {
        let returnedCount = UInt64(raw.targets.count)
        guard
            !expectedScanID.isEmpty,
            (1 ... maximumResults).contains(requestedMaxResults),
            raw.recordVersion == recordVersion,
            raw.scanId == expectedScanID,
            raw.scopeNodeId == expectedScopeNodeID,
            raw.requestedMaxResults == requestedMaxResults,
            raw.visitedNodeCount <= maximumVisitedNodes,
            raw.targets.count <= Int(maximumResults),
            raw.targets.count <= Int(requestedMaxResults),
            raw.totalRankedFiles >= returnedCount,
            raw.totalRankedFiles <= raw.visitedNodeCount,
            returnedCount == min(UInt64(requestedMaxResults), raw.totalRankedFiles),
            raw.hasMore == (raw.totalRankedFiles > returnedCount)
        else {
            throw ExplorerICloudObservationSourceError.invalidResponse
        }

        var targets: [ExplorerICloudObservationTarget] = []
        targets.reserveCapacity(raw.targets.count)
        for (index, rawTarget) in raw.targets.enumerated() {
            let node: ExplorerSnapshotNode
            let parentContext: [ExplorerSnapshotNodeName]
            do {
                node = try ExplorerSnapshotNodeAdapter.mapNode(
                    rawTarget.node,
                    maximumNameBytes: 1024
                )
                parentContext = try rawTarget.parentContext.map {
                    try ExplorerSnapshotNodeAdapter.mapName(
                        $0,
                        maximumNameBytes: 1024
                    )
                }
            } catch {
                throw ExplorerICloudObservationSourceError.invalidResponse
            }

            let parentDepth = Int(node.depth) - 1
            let contextMatches = rawTarget.contextTruncated
                ? parentContext.count == maximumContextComponents
                && parentDepth > maximumContextComponents
                : parentContext.count == parentDepth
            guard
                rawTarget.recordVersion == recordVersion,
                rawTarget.rank == UInt16(index),
                node.id != 0,
                node.parentID != nil,
                node.depth > 0,
                node.kind == .file,
                node.fileCount == 1,
                node.childCount == 0,
                node.allocatedBytes.map({ $0 > 0 }) == true,
                !node.scanFlags.inaccessible,
                !node.scanFlags.timedOut,
                !node.scanFlags.hardLinkDuplicate,
                !node.scanFlags.mountBoundary,
                parentContext.count <= maximumContextComponents,
                contextMatches
            else {
                throw ExplorerICloudObservationSourceError.invalidResponse
            }

            targets.append(
                ExplorerICloudObservationTarget(
                    rank: rawTarget.rank,
                    node: node,
                    parentContext: parentContext,
                    contextTruncated: rawTarget.contextTruncated
                )
            )
        }

        guard
            Set(targets.map(\.id)).count == targets.count,
            Set(targets.map(\.rank)).count == targets.count,
            zip(targets, targets.dropFirst()).allSatisfy(orderedBefore)
        else {
            throw ExplorerICloudObservationSourceError.invalidResponse
        }

        return ExplorerICloudObservationSource(
            scanID: raw.scanId,
            scopeNodeID: raw.scopeNodeId,
            requestedMaxResults: raw.requestedMaxResults,
            visitedNodeCount: raw.visitedNodeCount,
            totalRankedFiles: raw.totalRankedFiles,
            hasMore: raw.hasMore,
            targets: targets
        )
    }

    private static func orderedBefore(
        _ pair: (
            ExplorerICloudObservationTarget,
            ExplorerICloudObservationTarget
        )
    ) -> Bool {
        let (lhs, rhs) = pair
        let lhsAllocated = lhs.node.allocatedBytes ?? 0
        let rhsAllocated = rhs.node.allocatedBytes ?? 0
        if lhsAllocated != rhsAllocated {
            return lhsAllocated > rhsAllocated
        }
        if lhs.node.logicalBytes != rhs.node.logicalBytes {
            return lhs.node.logicalBytes > rhs.node.logicalBytes
        }
        return lhs.node.id < rhs.node.id
    }
}

enum ExplorerICloudLocalCopyAssessmentAdapter {
    static func map(
        _ raw: ICloudLocalCopyAssessment
    ) throws -> ExplorerICloudLocalCopyAssessment {
        guard
            raw.recordVersion == 1,
            raw.provider == .iCloudDrive,
            raw.itemKind == .regularFile,
            raw.localAllocatedBytes > 0,
            raw.observedAtUnixMs >= 0
        else {
            throw ExplorerICloudLocalCopyProbeError.invalidResponse
        }

        let ubiquitous = boolean(raw.ubiquitous)
        let uploaded = boolean(raw.uploaded)
        let uploading = boolean(raw.uploading)
        let uploadError = transferError(raw.uploadError)
        let unresolvedConflicts = boolean(raw.unresolvedConflicts)
        let localCopyState = localCopy(raw.localCopyState)
        let downloadRequested = boolean(raw.downloadRequested)
        let downloading = boolean(raw.downloading)
        let downloadError = transferError(raw.downloadError)
        let excludedFromSync = boolean(raw.excludedFromSync)
        let blockers = raw.blockers.map(blockReason)

        var expected: [ExplorerICloudLocalCopyBlockReason] = []
        switch ubiquitous {
        case .yes: break
        case .no: expected.append(.notUbiquitous)
        case .unknown: expected.append(.ubiquityUnknown)
        }
        switch uploaded {
        case .yes: break
        case .no: expected.append(.uploadIncomplete)
        case .unknown: expected.append(.uploadStateUnknown)
        }
        switch uploading {
        case .no: break
        case .yes: expected.append(.uploadInProgress)
        case .unknown: expected.append(.uploadActivityUnknown)
        }
        switch uploadError {
        case .absent: break
        case .present: expected.append(.uploadErrorPresent)
        case .unknown: expected.append(.uploadErrorUnknown)
        }
        switch unresolvedConflicts {
        case .no: break
        case .yes: expected.append(.unresolvedConflicts)
        case .unknown: expected.append(.conflictStateUnknown)
        }
        switch localCopyState {
        case .current: break
        case .stale: expected.append(.staleLocalCopy)
        case .notDownloaded: expected.append(.noLocalCopy)
        case .unknown: expected.append(.localCopyStateUnknown)
        }
        switch downloadRequested {
        case .no: break
        case .yes: expected.append(.downloadRequested)
        case .unknown: expected.append(.downloadRequestUnknown)
        }
        switch downloading {
        case .no: break
        case .yes: expected.append(.downloadInProgress)
        case .unknown: expected.append(.downloadActivityUnknown)
        }
        switch downloadError {
        case .absent: break
        case .present: expected.append(.downloadErrorPresent)
        case .unknown: expected.append(.downloadErrorUnknown)
        }
        switch excludedFromSync {
        case .no: break
        case .yes: expected.append(.excludedFromSync)
        case .unknown: expected.append(.syncExclusionUnknown)
        }
        guard
            blockers == expected,
            raw.isEligibleObservation == expected.isEmpty
        else {
            throw ExplorerICloudLocalCopyProbeError.invalidResponse
        }

        return ExplorerICloudLocalCopyAssessment(
            localAllocatedBytes: raw.localAllocatedBytes,
            observedAtUnixMilliseconds: raw.observedAtUnixMs,
            ubiquitous: ubiquitous,
            uploaded: uploaded,
            uploading: uploading,
            uploadError: uploadError,
            unresolvedConflicts: unresolvedConflicts,
            localCopyState: localCopyState,
            downloadRequested: downloadRequested,
            downloading: downloading,
            downloadError: downloadError,
            excludedFromSync: excludedFromSync,
            isEligibleObservation: raw.isEligibleObservation,
            blockers: blockers
        )
    }

    private static func boolean(_ value: ICloudBooleanState) -> ExplorerICloudBooleanFact {
        switch value {
        case .`true`: .yes
        case .`false`: .no
        case .unknown: .unknown
        }
    }

    private static func transferError(
        _ value: ICloudErrorState
    ) -> ExplorerICloudTransferErrorState {
        switch value {
        case .absent: .absent
        case .present: .present
        case .unknown: .unknown
        }
    }

    private static func localCopy(
        _ value: ICloudLocalCopyState
    ) -> ExplorerICloudLocalCopyState {
        switch value {
        case .current: .current
        case .stale: .stale
        case .notDownloaded: .notDownloaded
        case .unknown: .unknown
        }
    }

    private static func blockReason(
        _ value: ICloudLocalCopyBlockReason
    ) -> ExplorerICloudLocalCopyBlockReason {
        switch value {
        case .unsupportedItemKind: .unsupportedItemKind
        case .ubiquityUnknown: .ubiquityUnknown
        case .notUbiquitous: .notUbiquitous
        case .uploadStateUnknown: .uploadStateUnknown
        case .uploadIncomplete: .uploadIncomplete
        case .uploadActivityUnknown: .uploadActivityUnknown
        case .uploadInProgress: .uploadInProgress
        case .uploadErrorUnknown: .uploadErrorUnknown
        case .uploadErrorPresent: .uploadErrorPresent
        case .conflictStateUnknown: .conflictStateUnknown
        case .unresolvedConflicts: .unresolvedConflicts
        case .localCopyStateUnknown: .localCopyStateUnknown
        case .staleLocalCopy: .staleLocalCopy
        case .noLocalCopy: .noLocalCopy
        case .downloadRequestUnknown: .downloadRequestUnknown
        case .downloadRequested: .downloadRequested
        case .downloadActivityUnknown: .downloadActivityUnknown
        case .downloadInProgress: .downloadInProgress
        case .downloadErrorUnknown: .downloadErrorUnknown
        case .downloadErrorPresent: .downloadErrorPresent
        case .syncExclusionUnknown: .syncExclusionUnknown
        case .excludedFromSync: .excludedFromSync
        case .allocationUnknown: .allocationUnknown
        case .noLocalAllocation: .noLocalAllocation
        case .invalidObservationTime: .invalidObservationTime
        }
    }
}

struct ExplorerICloudLocalCopyFactRow: Identifiable, Equatable, Sendable {
    let id: String
    let label: String
    let value: String
}

enum ExplorerICloudLocalCopyDisclosure {
    static let action = "Remove local copy"
    static let retention = "Stays in iCloud"
    static let redownload = "Requires a network connection to download again"
    static let unavailable =
        "No cleanup action is available yet. This check created no candidate, approval, plan, or guaranteed reclaim estimate."
}

extension ExplorerICloudLocalCopyAssessment {
    var reviewTitle: String {
        isEligibleObservation
            ? "Currently supports review"
            : "Not currently ready for review"
    }

    var reviewDetail: String {
        if isEligibleObservation {
            return "Every required iCloud fact was known and favorable at the observation time. This is discovery only; it does not authorize cleanup."
        }
        return "One or more required facts were unfavorable or unknown. DUX failed closed and did not create a cleanup candidate."
    }

    var observedAt: Date {
        Date(timeIntervalSince1970: Double(observedAtUnixMilliseconds) / 1_000)
    }

    var factRows: [ExplorerICloudLocalCopyFactRow] {
        [
            ExplorerICloudLocalCopyFactRow(
                id: "ubiquitous",
                label: "iCloud item",
                value: ubiquitous.factText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "uploaded",
                label: "Uploaded",
                value: uploaded.factText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "uploading",
                label: "Uploading",
                value: uploading.activityText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "upload-error",
                label: "Upload error",
                value: uploadError.factText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "conflicts",
                label: "Unresolved conflicts",
                value: unresolvedConflicts.activityText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "local-copy",
                label: "Local copy",
                value: localCopyState.factText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "download-requested",
                label: "Download requested",
                value: downloadRequested.activityText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "downloading",
                label: "Downloading",
                value: downloading.activityText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "download-error",
                label: "Download error",
                value: downloadError.factText
            ),
            ExplorerICloudLocalCopyFactRow(
                id: "excluded",
                label: "Excluded from sync",
                value: excludedFromSync.activityText
            ),
        ]
    }
}

extension ExplorerICloudBooleanFact {
    fileprivate var factText: String {
        switch self {
        case .yes: "Yes"
        case .no: "No"
        case .unknown: "Unknown"
        }
    }

    fileprivate var activityText: String {
        switch self {
        case .yes: "Yes"
        case .no: "No"
        case .unknown: "Unknown"
        }
    }
}

extension ExplorerICloudTransferErrorState {
    fileprivate var factText: String {
        switch self {
        case .absent: "None reported"
        case .present: "Reported"
        case .unknown: "Unknown"
        }
    }
}

extension ExplorerICloudLocalCopyState {
    fileprivate var factText: String {
        switch self {
        case .current: "Current and downloaded"
        case .stale: "Stale"
        case .notDownloaded: "Remote only"
        case .unknown: "Unknown"
        }
    }
}

extension ExplorerICloudLocalCopyBlockReason {
    var displayText: String {
        switch self {
        case .unsupportedItemKind: "The item is not a supported regular file."
        case .ubiquityUnknown: "iCloud item status is unknown."
        case .notUbiquitous: "The file is not reported as an iCloud item."
        case .uploadStateUnknown: "Upload completion is unknown."
        case .uploadIncomplete: "The upload is not complete."
        case .uploadActivityUnknown: "Upload activity is unknown."
        case .uploadInProgress: "An upload is in progress."
        case .uploadErrorUnknown: "Upload error status is unknown."
        case .uploadErrorPresent: "An upload error is reported."
        case .conflictStateUnknown: "Conflict status is unknown."
        case .unresolvedConflicts: "The file has unresolved conflicts."
        case .localCopyStateUnknown: "Local-copy status is unknown."
        case .staleLocalCopy: "The local copy is stale."
        case .noLocalCopy: "The file is already remote only."
        case .downloadRequestUnknown: "Download-request status is unknown."
        case .downloadRequested: "A download has been requested."
        case .downloadActivityUnknown: "Download activity is unknown."
        case .downloadInProgress: "A download is in progress."
        case .downloadErrorUnknown: "Download error status is unknown."
        case .downloadErrorPresent: "A download error is reported."
        case .syncExclusionUnknown: "Sync-exclusion status is unknown."
        case .excludedFromSync: "The file is excluded from sync."
        case .allocationUnknown: "Local allocation is unknown."
        case .noLocalAllocation: "No local allocation was observed."
        case .invalidObservationTime: "The observation time is invalid."
        }
    }
}

extension ExplorerICloudLocalCopyProbeError {
    var title: String {
        switch self {
        case .unavailable: "iCloud status unavailable"
        case .invalidTarget: "File cannot be checked"
        case .changedSinceSnapshot: "File changed since the scan"
        case .unsupported: "iCloud check unsupported"
        case .failed: "iCloud check failed"
        case .invalidResponse: "iCloud response rejected"
        }
    }

    var detail: String {
        switch self {
        case .unavailable:
            "The retained review or platform metadata service is unavailable. Reload Explorer and try again."
        case .invalidTarget:
            "Only an exact, non-root, regular, single-link file with known local allocation can be checked."
        case .changedSinceSnapshot:
            "The file or one of its folders no longer matches this snapshot. Run a new scan before checking again."
        case .unsupported:
            "This macOS environment cannot read the required iCloud metadata."
        case .failed:
            "macOS could not read a complete iCloud status. No cleanup candidate was created."
        case .invalidResponse:
            "DUX rejected inconsistent iCloud metadata instead of treating it as favorable."
        }
    }
}
