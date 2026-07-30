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
