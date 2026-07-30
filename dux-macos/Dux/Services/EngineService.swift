import Dispatch
import Foundation
import OSLog

protocol DuxVolumeStatusServing: Sendable {
    func observeVolumeCapacity(_ snapshot: VolumeCapacitySnapshot) async throws
        -> VolumeCapacitySnapshot
}

protocol DuxCapacityTrendServing: Sendable {
    func loadCapacityTrend(stableVolumeID: String, at: Date) async throws -> VolumeCapacityTrend
}

protocol DuxPressureEpisodeServing: Sendable {
    func loadPressureEpisodeHistory(
        stableVolumeID: String,
        at: Date,
        limit: UInt16
    ) async throws -> VolumePressureHistory
}

extension DuxPressureEpisodeServing {
    func loadPressureEpisodeHistory(
        stableVolumeID _: String,
        at _: Date,
        limit _: UInt16
    ) async throws -> VolumePressureHistory {
        throw EngineServiceError.unavailable
    }
}

extension DuxCapacityTrendServing {
    func loadCapacityTrend(stableVolumeID _: String, at _: Date) async throws -> VolumeCapacityTrend {
        throw EngineServiceError.unavailable
    }
}

protocol DuxPressurePolicyServing: Sendable {
    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy
    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult
    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult
}

protocol DuxSnapshotRetentionCapServing: Sendable {
    func loadSnapshotRetentionCap() async throws -> SnapshotRetentionCapModel
    func setSnapshotRetentionCap(
        _ capBytes: UInt64
    ) async throws -> SnapshotRetentionCapUpdateResultModel
    func resetSnapshotRetentionCap() async throws -> SnapshotRetentionCapUpdateResultModel
}

protocol DuxManagedScanCacheClearPreviewLease: AnyObject, Sendable {
    var preview: DuxManagedScanCacheClearPreviewModel { get }
    func release() async
}

protocol DuxOwnedStorageFootprintServing: Sendable {
    func loadOwnedStorageFootprint() async throws
        -> DuxOwnedStorageFootprintModel
    func prepareManagedScanCacheClear() async throws
        -> any DuxManagedScanCacheClearPreviewLease
    func clearManagedScanCache(
        _ preview: any DuxManagedScanCacheClearPreviewLease
    ) async throws -> DuxManagedScanCacheClearResultModel
}

extension DuxOwnedStorageFootprintServing {
    func loadOwnedStorageFootprint() async throws
        -> DuxOwnedStorageFootprintModel
    {
        throw DuxOwnedStorageFootprintServiceError.unavailable
    }

    func prepareManagedScanCacheClear() async throws
        -> any DuxManagedScanCacheClearPreviewLease
    {
        throw DuxManagedScanCacheClearServiceError.unavailable
    }

    func clearManagedScanCache(
        _: any DuxManagedScanCacheClearPreviewLease
    ) async throws -> DuxManagedScanCacheClearResultModel {
        throw DuxManagedScanCacheClearServiceError.unavailable
    }
}

extension DuxSnapshotRetentionCapServing {
    func loadSnapshotRetentionCap() async throws -> SnapshotRetentionCapModel {
        throw SnapshotRetentionCapServiceError.unavailable
    }

    func setSnapshotRetentionCap(
        _: UInt64
    ) async throws -> SnapshotRetentionCapUpdateResultModel {
        throw SnapshotRetentionCapServiceError.unavailable
    }

    func resetSnapshotRetentionCap() async throws -> SnapshotRetentionCapUpdateResultModel {
        throw SnapshotRetentionCapServiceError.unavailable
    }
}

protocol DuxPermanentCleanupPolicyServing: Sendable {
    func loadPermanentCleanupPolicy() async throws -> PermanentCleanupPolicy
    func setPermanentCleanupEnabled(
        _ enabled: Bool
    ) async throws -> PermanentCleanupPolicyUpdateResult
    func resetPermanentCleanup() async throws -> PermanentCleanupPolicyUpdateResult
}

protocol DuxCleanupExclusionsServing: Sendable {
    func loadCleanupExclusions() async throws -> CleanupExclusionsPolicy
    func setCleanupExclusions(
        _ paths: [CleanupExclusionPathObservation]
    ) async throws -> CleanupExclusionsUpdateResult
    func resetCleanupExclusions() async throws -> CleanupExclusionsUpdateResult
}

protocol DuxProjectDiscoveryRootsServing: Sendable {
    func loadProjectDiscoveryRoots() async throws -> ProjectDiscoveryRoots
    func setProjectDiscoveryRoots(
        _ roots: [ProjectDiscoveryRoot]
    ) async throws -> ProjectDiscoveryRootsUpdateResult
    func resetProjectDiscoveryRoots() async throws -> ProjectDiscoveryRootsUpdateResult
}

protocol DuxPersistentRecoveryDebtServing: Sendable {
    func loadPersistentRecoveryDebt() async throws -> PersistentRecoveryDebt
}

extension DuxPersistentRecoveryDebtServing {
    func loadPersistentRecoveryDebt() async throws -> PersistentRecoveryDebt {
        throw PersistentRecoveryDebtServiceError.unavailable
    }
}

protocol DuxClaimedRunningScanProvenanceServing: Sendable {
    func loadClaimedRunningScanProvenance() async throws
        -> ClaimedRunningScanProvenance
}

extension DuxClaimedRunningScanProvenanceServing {
    func loadClaimedRunningScanProvenance() async throws
        -> ClaimedRunningScanProvenance
    {
        throw ClaimedRunningScanProvenanceServiceError.unavailable
    }
}

protocol DuxDirectCargoEnrollmentServing: Sendable {
    func loadDirectCargoEnrollmentStatus() async throws
        -> DirectCargoEnrollmentStatusModel
    func inspectDirectCargoExecutable(
        _ selection: DirectCargoExecutableSelection
    ) async throws -> any DuxDirectCargoEnrollmentPreviewLease
    func enrollDirectCargo(
        _ preview: any DuxDirectCargoEnrollmentPreviewLease
    ) async throws -> DirectCargoEnrollmentUpdateModel
    func revokeDirectCargoEnrollment() async throws -> DirectCargoEnrollmentUpdateModel
}

extension DuxDirectCargoEnrollmentServing {
    func loadDirectCargoEnrollmentStatus() async throws
        -> DirectCargoEnrollmentStatusModel
    {
        throw DirectCargoEnrollmentServiceError.unavailable
    }

    func inspectDirectCargoExecutable(
        _: DirectCargoExecutableSelection
    ) async throws -> any DuxDirectCargoEnrollmentPreviewLease {
        throw DirectCargoEnrollmentServiceError.unavailable
    }

    func enrollDirectCargo(
        _: any DuxDirectCargoEnrollmentPreviewLease
    ) async throws -> DirectCargoEnrollmentUpdateModel {
        throw DirectCargoEnrollmentServiceError.unavailable
    }

    func revokeDirectCargoEnrollment() async throws -> DirectCargoEnrollmentUpdateModel {
        throw DirectCargoEnrollmentServiceError.unavailable
    }
}

extension DuxCleanupExclusionsServing {
    func loadCleanupExclusions() async throws -> CleanupExclusionsPolicy {
        throw CleanupExclusionsServiceError.unavailable
    }

    func setCleanupExclusions(
        _: [CleanupExclusionPathObservation]
    ) async throws -> CleanupExclusionsUpdateResult {
        throw CleanupExclusionsServiceError.unavailable
    }

    func resetCleanupExclusions() async throws -> CleanupExclusionsUpdateResult {
        throw CleanupExclusionsServiceError.unavailable
    }
}

extension DuxProjectDiscoveryRootsServing {
    func loadProjectDiscoveryRoots() async throws -> ProjectDiscoveryRoots {
        throw ProjectDiscoveryRootsServiceError.unavailable
    }

    func setProjectDiscoveryRoots(
        _: [ProjectDiscoveryRoot]
    ) async throws -> ProjectDiscoveryRootsUpdateResult {
        throw ProjectDiscoveryRootsServiceError.unavailable
    }

    func resetProjectDiscoveryRoots() async throws -> ProjectDiscoveryRootsUpdateResult {
        throw ProjectDiscoveryRootsServiceError.unavailable
    }
}

extension DuxPermanentCleanupPolicyServing {
    func loadPermanentCleanupPolicy() async throws -> PermanentCleanupPolicy {
        throw PermanentCleanupPolicyServiceError.unavailable
    }

    func setPermanentCleanupEnabled(
        _: Bool
    ) async throws -> PermanentCleanupPolicyUpdateResult {
        throw PermanentCleanupPolicyServiceError.unavailable
    }

    func resetPermanentCleanup() async throws -> PermanentCleanupPolicyUpdateResult {
        throw PermanentCleanupPolicyServiceError.unavailable
    }
}

protocol EngineServing: DuxVolumeStatusServing, DuxCapacityTrendServing,
    DuxPressureEpisodeServing, DuxPressurePolicyServing, DuxSnapshotRetentionCapServing,
    DuxOwnedStorageFootprintServing, DuxPermanentCleanupPolicyServing,
    DuxCleanupExclusionsServing, DuxProjectDiscoveryRootsServing,
    DuxDirectCargoEnrollmentServing, DuxTargetedReclaimScanServing,
    DuxCleanupHistoryServing, DuxCleanupHistoryClearing, DuxPersistentRecoveryDebtServing,
    DuxClaimedRunningScanProvenanceServing, Sendable
{
    func loadStatus() async throws -> EngineStatus
}

protocol DuxSnapshotReviewServing: Sendable {
    func acquireExplorerReview(scanID: String) async throws -> any DuxSnapshotReviewLease
    func acquireLatestExplorerReview() async throws -> any DuxSnapshotReviewLease
}

protocol DuxSnapshotHistoryServing: Sendable {
    func loadRecentSnapshotHistory(limit: UInt16) async throws -> ExplorerSnapshotHistoryPage
}

protocol DuxCleanupHistoryServing: Sendable {
    func loadRecentCleanupHistory(
        cursor: CleanupHistoryCursorModel?,
        limit: UInt16
    ) async throws -> CleanupHistoryPageModel
    func loadCleanupHistorySession(
        sessionID: String
    ) async throws -> CleanupHistorySessionDetailModel
    func loadCleanupHistoryRuleOutcomes(
        sessionID: String,
        detail: CleanupHistorySessionDetailModel
    ) async throws -> CleanupHistoryRuleOutcomeBatchModel
    func loadRecurringStorageThieves() async throws
        -> CleanupHistoryStorageThiefRankingModel
}

extension DuxCleanupHistoryServing {
    func loadRecentCleanupHistory(
        cursor _: CleanupHistoryCursorModel?,
        limit _: UInt16
    ) async throws -> CleanupHistoryPageModel {
        throw CleanupHistoryServiceError.unavailable
    }

    func loadCleanupHistorySession(
        sessionID _: String
    ) async throws -> CleanupHistorySessionDetailModel {
        throw CleanupHistoryServiceError.unavailable
    }

    func loadCleanupHistoryRuleOutcomes(
        sessionID _: String,
        detail _: CleanupHistorySessionDetailModel
    ) async throws -> CleanupHistoryRuleOutcomeBatchModel {
        throw CleanupHistoryServiceError.unavailable
    }

    func loadRecurringStorageThieves() async throws
        -> CleanupHistoryStorageThiefRankingModel
    {
        throw CleanupHistoryServiceError.unavailable
    }
}

protocol DuxCleanupHistoryClearPreviewLease: AnyObject, Sendable {
    var preview: CleanupHistoryClearPreviewModel { get }
    func release() async
}

protocol DuxCleanupHistoryClearing: Sendable {
    func prepareCleanupHistoryClear() async throws
        -> any DuxCleanupHistoryClearPreviewLease
    func clearCleanupHistory(
        _ preview: any DuxCleanupHistoryClearPreviewLease
    ) async throws -> CleanupHistoryClearResultModel
}

extension DuxCleanupHistoryClearing {
    func prepareCleanupHistoryClear() async throws
        -> any DuxCleanupHistoryClearPreviewLease
    {
        throw CleanupHistoryClearServiceError.unavailable
    }

    func clearCleanupHistory(
        _: any DuxCleanupHistoryClearPreviewLease
    ) async throws -> CleanupHistoryClearResultModel {
        throw CleanupHistoryClearServiceError.unavailable
    }
}

protocol DuxScanCoverageServing: Sendable {
    func loadScanCoverageDetails(scanID: String) async throws -> ExplorerScanCoverageDetails
}

protocol DuxSnapshotReviewLease: AnyObject, Sendable {
    var scanID: String { get }
    func renew() async throws -> Int64
    func rootNode() async throws -> ExplorerSnapshotNode
    func candidateSummaries(
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateSummaryPage
    func reviewCandidate(
        candidateID: String,
        command: ExplorerCandidateReviewCommand
    ) async throws -> ExplorerCandidateReviewResult
    func childNodes(
        parentID: UInt64,
        sort: ExplorerSnapshotNodeSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotNodePage
    func treemap(
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotTreemap
    func largeFiles(
        minimumLogicalBytes: UInt64,
        modifiedBefore: ExplorerSnapshotTimestamp?,
        maxResults: UInt16
    ) async throws -> ExplorerSnapshotLargeFilesPage
    func icloudObservationSource(
        scopeNodeID: UInt64,
        maxResults: UInt16
    ) async throws -> ExplorerICloudObservationSource
    func candidatePaths(
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidatePathPage
    func candidateEvidence(
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateEvidencePage
    func prepareRustTargetPlanReview(
        candidateID: String
    ) async throws -> any DuxRustTargetPlanReviewSession
    func prepareSnapshotDiffReview() async throws -> any DuxSnapshotDiffReviewSession
    func resolveLiveItem(
        nodeID: UInt64,
        purpose: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem
    func probeICloudLocalCopy(
        nodeID: UInt64
    ) async throws -> ExplorerICloudLocalCopyAssessment
    func executeTrash(nodeID: UInt64) async throws -> TrashPlatformResult
    func startSubtreeScan(nodeID: UInt64) async throws -> HomeScanStartDisposition
    func release() async
}

protocol DuxSnapshotDiffReviewSession: AnyObject, Sendable {
    var info: ExplorerSnapshotDiffInfo { get }
    func renew() async throws -> ExplorerSnapshotDiffInfo
    func rootNode() async throws -> ExplorerSnapshotDiffNode
    func childNodes(
        parentID: UInt64,
        sort: ExplorerSnapshotDiffSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotDiffNodePage
    func treemap(
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotDiffTreemap
    func release() async
}

protocol DuxRustTargetPlanReviewSession: AnyObject, Sendable {
    var scanID: String { get }
    var candidateID: String { get }
    func info() async throws -> ExplorerRustTargetPlanReviewRecord
    func startDryRun() async throws -> any DuxRustTargetDryRunTask
    func startCleanup() async throws -> any DuxRustTargetCleanupTask
    func release() async
}

extension DuxRustTargetPlanReviewSession {
    func startDryRun() async throws -> any DuxRustTargetDryRunTask {
        throw ExplorerRustTargetDryRunStartError.unavailable
    }

    func startCleanup() async throws -> any DuxRustTargetCleanupTask {
        throw ExplorerRustTargetCleanupStartError.unavailable
    }
}

extension DuxSnapshotReviewLease {
    func prepareSnapshotDiffReview() async throws -> any DuxSnapshotDiffReviewSession {
        throw ExplorerSnapshotDiffFailure.unavailable
    }

    func candidateSummaries(
        cursor _: UInt16,
        limit _: UInt16
    ) async throws -> ExplorerCandidateSummaryPage {
        throw ExplorerCandidateDetailError.unavailable
    }

    func reviewCandidate(
        candidateID _: String,
        command _: ExplorerCandidateReviewCommand
    ) async throws -> ExplorerCandidateReviewResult {
        throw ExplorerCandidateDetailError.unavailable
    }

    func candidatePaths(
        candidateID _: String,
        cursor _: UInt16,
        limit _: UInt16
    ) async throws -> ExplorerCandidatePathPage {
        throw ExplorerCandidateDetailError.unavailable
    }

    func candidateEvidence(
        candidateID _: String,
        cursor _: UInt16,
        limit _: UInt16
    ) async throws -> ExplorerCandidateEvidencePage {
        throw ExplorerCandidateDetailError.unavailable
    }

    func prepareRustTargetPlanReview(
        candidateID _: String
    ) async throws -> any DuxRustTargetPlanReviewSession {
        throw ExplorerRustTargetPlanReviewError.unavailable
    }

    func resolveLiveItem(
        nodeID _: UInt64,
        purpose _: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem {
        throw ExplorerSnapshotLivePathError.unavailable
    }

    func startSubtreeScan(nodeID _: UInt64) async throws -> HomeScanStartDisposition {
        throw ExplorerSnapshotSubtreeScanError.unavailable
    }

    func executeTrash(nodeID _: UInt64) async throws -> TrashPlatformResult {
        throw ExplorerTrashError.unavailable
    }

    func probeICloudLocalCopy(
        nodeID _: UInt64
    ) async throws -> ExplorerICloudLocalCopyAssessment {
        throw ExplorerICloudLocalCopyProbeError.unavailable
    }

    func icloudObservationSource(
        scopeNodeID _: UInt64,
        maxResults _: UInt16
    ) async throws -> ExplorerICloudObservationSource {
        throw ExplorerICloudObservationSourceError.unavailable
    }
}

struct EngineService: EngineServing, DuxMaintenanceServing, DuxSnapshotReviewServing,
    DuxSnapshotHistoryServing, DuxCleanupHistoryServing, DuxScanCoverageServing, HomeScanServing,
    Sendable
{
    fileprivate static let expectedFFIContractVersion: UInt32 = 53
    fileprivate static let expectedRecordVersion: UInt32 = 1
    private static let maximumTargetedProjectScanNodes: UInt32 = 50000
    private static let maximumTargetedProjectScanPassNodes: UInt32 = 200_000
    private static let maximumTargetedReclaimRootCount: UInt16 = 17
    private static let maximumKnownUserCacheScanNodes: UInt32 = 100_000
    private static let minimumTargetedProjectScanNodes: UInt32 = 10000
    private static let expectedKnownRootsPolicyRevision: UInt32 = 1

    private let state: EngineServiceState

    init(
        engine: DuxEngine? = nil,
        storageRoots: EngineStorageRoots? = nil,
        homeScanRoot: URL? = nil
    ) {
        precondition(engine == nil || storageRoots == nil)
        state = EngineServiceState(
            engine: engine,
            storageRoots: storageRoots,
            homeScanRoot: homeScanRoot
        )
    }

    func loadStatus() async throws -> EngineStatus {
        try await state.perform { state in
            let executedOffMainThread = !Thread.isMainThread
            precondition(executedOffMainThread, "Blocking FFI work reached the main thread")
            let engine = try state.resolveEngine()
            do {
                let version = try engine.libraryVersion()
                guard version.ffiContractVersion == Self.expectedFFIContractVersion else {
                    throw EngineServiceError.unexpected("incompatible FFI contract")
                }
                return EngineStatus(
                    libraryVersion: version.libraryVersion,
                    ffiContractVersion: version.ffiContractVersion,
                    databaseSchemaVersion: version.databaseSchemaVersion,
                    snapshotFormatVersion: version.snapshotFormatVersion,
                    executedOffMainThread: executedOffMainThread
                )
            } catch let error as EngineError {
                throw Self.serviceError(error)
            }
        }
    }

    func loadPersistentRecoveryDebt() async throws -> PersistentRecoveryDebt {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolvePersistentRecoveryDebtEngine(state)
            do {
                return try Self.persistentRecoveryDebt(
                    engine.runningScanDebtCensus()
                )
            } catch let error as RunningScanDebtCensusError {
                throw Self.persistentRecoveryDebtError(error)
            }
        }
    }

    func loadClaimedRunningScanProvenance() async throws
        -> ClaimedRunningScanProvenance
    {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveClaimedRunningScanProvenanceEngine(state)
            do {
                return try Self.claimedRunningScanProvenance(
                    engine.claimedRunningScanProvenanceCensus()
                )
            } catch let error as ClaimedRunningScanProvenanceCensusError {
                throw Self.claimedRunningScanProvenanceError(error)
            }
        }
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let milliseconds = snapshot.sampledAt.timeIntervalSince1970 * 1000
            guard
                milliseconds.isFinite,
                milliseconds >= 0,
                milliseconds <= Double(Int64.max)
            else {
                throw EngineServiceError.invalidCapacityObservation
            }
            let sampledAtUnixMS = Int64(milliseconds.rounded(.towardZero))
            let engine = try state.resolveEngine()
            do {
                let status = try engine.observeStartupVolume(
                    observation: StartupVolumeObservation(
                        recordVersion: Self.expectedRecordVersion,
                        stableVolumeId: snapshot.stableVolumeID,
                        displayName: snapshot.displayName,
                        filesystem: snapshot.filesystem,
                        isInternal: snapshot.isInternal,
                        isRemovable: snapshot.isRemovable,
                        sampledAtUnixMs: sampledAtUnixMS,
                        totalBytes: snapshot.totalBytes,
                        ordinaryAvailableBytes: snapshot.filesystemAvailableBytes,
                        importantAvailableBytes: snapshot.importantAvailableBytes
                    )
                )
                guard
                    status.recordVersion == Self.expectedRecordVersion,
                    status.sampledAtUnixMs == sampledAtUnixMS,
                    status.totalBytes == snapshot.totalBytes,
                    status.ordinaryAvailableBytes == snapshot.filesystemAvailableBytes,
                    status.importantAvailableBytes == snapshot.importantAvailableBytes,
                    status.headlineAvailableBytes <= status.totalBytes,
                    status.criticalBoundaryBytes <= status.warningBoundaryBytes,
                    status.warningBoundaryBytes <= status.totalBytes,
                    Self.hasConsistentHistoryEvidence(status, observation: snapshot)
                else {
                    throw EngineServiceError.unexpected("invalid volume status record")
                }
                let basis = Self.capacityBasis(status.headlineSource)
                guard
                    status.headlineAvailableBytes == snapshot.effectiveAvailableBytes,
                    basis == snapshot.availabilityBasis
                else {
                    throw EngineServiceError.unexpected("volume status changed observed capacity")
                }
                return snapshot.applying(
                    stableVolumeID: status.stableVolumeId,
                    pressure: Self.pressure(status.pressure),
                    previousDurablePressure: status.previousDurablePressure.map(Self.pressure),
                    criticalBoundaryBytes: status.criticalBoundaryBytes,
                    warningBoundaryBytes: status.warningBoundaryBytes,
                    historyDisposition: Self.historyDisposition(status.historyDisposition),
                    sampledAt: Date(
                        timeIntervalSince1970: Double(status.sampledAtUnixMs) / 1000
                    )
                )
            } catch let error as EngineError {
                throw Self.serviceError(error)
            }
        }
    }

    func loadCapacityTrend(stableVolumeID: String, at: Date) async throws -> VolumeCapacityTrend {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            guard let uuid = Self.macOSVolumeUUID(stableVolumeID) else {
                throw EngineServiceError.invalidCapacityObservation
            }
            let milliseconds = at.timeIntervalSince1970 * 1000
            guard milliseconds.isFinite, milliseconds >= 0, milliseconds <= Double(Int64.max) else {
                throw EngineServiceError.invalidCapacityObservation
            }
            let anchorAtUnixMS = Int64(milliseconds.rounded(.towardZero))
            let engine = try state.resolveEngine()
            do {
                let response = try engine.getCapacityTrend(
                    request: CapacityTrendRequest(
                        recordVersion: Self.expectedRecordVersion,
                        stableVolumeId: uuid.uuidString,
                        anchorAtUnixMs: anchorAtUnixMS
                    )
                )
                return try Self.capacityTrend(
                    response,
                    expectedStableVolumeID: "volume:macos:\(uuid.uuidString.lowercased())",
                    requestedAnchorAtUnixMS: anchorAtUnixMS
                )
            } catch let error as EngineError {
                throw Self.serviceError(error)
            }
        }
    }

    func loadPressureEpisodeHistory(
        stableVolumeID: String,
        at: Date,
        limit: UInt16
    ) async throws -> VolumePressureHistory {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            guard
                let uuid = Self.macOSVolumeUUID(stableVolumeID),
                (1 ... UInt16(64)).contains(limit)
            else {
                throw EngineServiceError.invalidCapacityObservation
            }
            let milliseconds = at.timeIntervalSince1970 * 1000
            guard milliseconds.isFinite, milliseconds >= 0, milliseconds <= Double(Int64.max) else {
                throw EngineServiceError.invalidCapacityObservation
            }
            let anchorAtUnixMS = Int64(milliseconds.rounded(.towardZero))
            let engine = try state.resolveEngine()
            do {
                let response = try engine.getPressureEpisodeHistory(
                    request: PressureEpisodeHistoryRequest(
                        recordVersion: Self.expectedRecordVersion,
                        stableVolumeId: uuid.uuidString,
                        anchorAtUnixMs: anchorAtUnixMS,
                        limit: limit
                    )
                )
                return try Self.pressureEpisodeHistory(
                    response,
                    expectedStableVolumeID: "volume:macos:\(uuid.uuidString.lowercased())",
                    expectedAnchorAtUnixMS: anchorAtUnixMS,
                    requestedLimit: limit
                )
            } catch let error as EngineError {
                throw Self.serviceError(error)
            }
        }
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolvePressureEngine(state)
            do {
                return try Self.policy(engine.getDiskPressurePolicy())
            } catch let error as PressurePolicyError {
                throw Self.pressurePolicyError(error)
            }
        }
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolvePressureEngine(state)
            do {
                let response = try engine.setDiskPressurePolicy(
                    input: PressurePolicyInput(
                        recordVersion: Self.expectedRecordVersion,
                        criticalAvailableBytes: configuration.criticalAvailableBytes,
                        criticalAvailableBasisPoints: configuration.criticalAvailableBasisPoints,
                        warningAvailableBytes: configuration.warningAvailableBytes,
                        warningAvailableBasisPoints: configuration.warningAvailableBasisPoints,
                        recoveryBytes: configuration.recoveryBytes,
                        recoveryBasisPoints: configuration.recoveryBasisPoints
                    )
                )
                let update = try Self.policyUpdate(response)
                guard
                    update.policy.source == .stored,
                    update.policy.configuration == configuration
                else {
                    throw DiskPressurePolicyServiceError.invalidResponse
                }
                return update
            } catch let error as PressurePolicyError {
                throw Self.pressurePolicyError(error)
            }
        }
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolvePressureEngine(state)
            do {
                let update = try Self.policyUpdate(engine.resetDiskPressurePolicy())
                guard update.policy.source == .default else {
                    throw DiskPressurePolicyServiceError.invalidResponse
                }
                return update
            } catch let error as PressurePolicyError {
                throw Self.pressurePolicyError(error)
            }
        }
    }

    func loadSnapshotRetentionCap() async throws -> SnapshotRetentionCapModel {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveSnapshotRetentionCapEngine(state)
            do {
                return try Self.snapshotRetentionCap(
                    engine.getSnapshotRetentionCap()
                )
            } catch let error as SnapshotRetentionCapError {
                throw Self.snapshotRetentionCapError(error)
            }
        }
    }

    func setSnapshotRetentionCap(
        _ capBytes: UInt64
    ) async throws -> SnapshotRetentionCapUpdateResultModel {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveSnapshotRetentionCapEngine(state)
            do {
                let update = try Self.snapshotRetentionCapUpdate(
                    engine.setSnapshotRetentionCap(
                        input: SnapshotRetentionCapInput(
                            recordVersion: Self.expectedRecordVersion,
                            capBytes: capBytes
                        )
                    )
                )
                guard
                    update.settings.source == .stored,
                    update.settings.capBytes == capBytes
                else {
                    throw SnapshotRetentionCapServiceError.invalidResponse
                }
                return update
            } catch let error as SnapshotRetentionCapError {
                throw Self.snapshotRetentionCapError(error)
            }
        }
    }

    func resetSnapshotRetentionCap() async throws
        -> SnapshotRetentionCapUpdateResultModel
    {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveSnapshotRetentionCapEngine(state)
            do {
                let update = try Self.snapshotRetentionCapUpdate(
                    engine.resetSnapshotRetentionCap()
                )
                guard update.settings.source == .default else {
                    throw SnapshotRetentionCapServiceError.invalidResponse
                }
                return update
            } catch let error as SnapshotRetentionCapError {
                throw Self.snapshotRetentionCapError(error)
            }
        }
    }

    func loadOwnedStorageFootprint() async throws
        -> DuxOwnedStorageFootprintModel
    {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveOwnedStorageFootprintEngine(state)
            do {
                return try Self.ownedStorageFootprint(
                    engine.getOwnedStorageFootprint()
                )
            } catch let error as OwnedStorageFootprintError {
                throw Self.ownedStorageFootprintError(error)
            }
        }
    }

    func prepareManagedScanCacheClear() async throws
        -> any DuxManagedScanCacheClearPreviewLease
    {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveManagedScanCacheClearEngine(state)
            do {
                let preview = try engine.prepareManagedScanCacheClear()
                do {
                    let model = try Self.managedScanCacheClearPreview(
                        preview.info()
                    )
                    return FFIManagedScanCacheClearPreviewLease(
                        ffiPreview: preview,
                        preview: model,
                        state: state
                    )
                } catch {
                    _ = try? preview.release()
                    throw error
                }
            } catch let error as ManagedScanCacheClearError {
                throw Self.managedScanCacheClearError(error)
            }
        }
    }

    func clearManagedScanCache(
        _ preview: any DuxManagedScanCacheClearPreviewLease
    ) async throws -> DuxManagedScanCacheClearResultModel {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            guard let preview = preview as? FFIManagedScanCacheClearPreviewLease else {
                throw DuxManagedScanCacheClearServiceError.wrongEngine
            }
            let engine = try Self.resolveManagedScanCacheClearEngine(state)
            let ffiPreview = try preview.take(for: state)
            let response: ManagedScanCacheClearResult
            do {
                response = try engine.clearManagedScanCache(preview: ffiPreview)
            } catch let error as ManagedScanCacheClearError {
                throw Self.managedScanCacheClearError(error)
            }
            let clearedUsage: DuxOwnedStorageUsageModel
            do {
                clearedUsage = try Self.ownedStorageUsage(response.clearedUsage)
            } catch {
                throw DuxManagedScanCacheClearServiceError.outcomeUnknown
            }
            let expected = preview.preview
            let expectedClearedCount = response.clearedEntryCount
                .addingReportingOverflow(response.clearedTemporaryCount)
            guard
                response.recordVersion == Self.expectedRecordVersion,
                !expectedClearedCount.overflow,
                response.clearedEntryCount == expected.entryCount,
                response.clearedTemporaryCount == expected.temporaryCount,
                response.clearedCount == expected.clearableCount,
                response.clearedCount == expectedClearedCount.partialValue,
                clearedUsage == expected.clearable
            else {
                // Rust may already have committed deletion. A malformed
                // success is uncertain and must never become retry authority.
                throw DuxManagedScanCacheClearServiceError.outcomeUnknown
            }
            return DuxManagedScanCacheClearResultModel(
                clearedEntryCount: response.clearedEntryCount,
                clearedTemporaryCount: response.clearedTemporaryCount,
                clearedCount: response.clearedCount,
                clearedUsage: clearedUsage
            )
        }
    }

    func loadPermanentCleanupPolicy() async throws -> PermanentCleanupPolicy {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolvePermanentCleanupEngine(state)
            do {
                return try Self.permanentCleanupPolicy(engine.getPermanentCleanupPolicy())
            } catch let error as PermanentCleanupPolicyError {
                throw Self.permanentCleanupPolicyError(error)
            }
        }
    }

    func setPermanentCleanupEnabled(
        _ enabled: Bool
    ) async throws -> PermanentCleanupPolicyUpdateResult {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolvePermanentCleanupEngine(state)
            do {
                let update = try Self.permanentCleanupPolicyUpdate(
                    engine.setPermanentCleanupEnabled(enabled: enabled)
                )
                guard update.policy.enabled == enabled else {
                    throw PermanentCleanupPolicyServiceError.invalidResponse
                }
                return update
            } catch let error as PermanentCleanupPolicyError {
                throw Self.permanentCleanupPolicyError(error)
            }
        }
    }

    func resetPermanentCleanup() async throws -> PermanentCleanupPolicyUpdateResult {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolvePermanentCleanupEngine(state)
            do {
                let update = try Self.permanentCleanupPolicyUpdate(
                    engine.resetPermanentCleanup()
                )
                guard update.policy.source == .default, !update.policy.enabled else {
                    throw PermanentCleanupPolicyServiceError.invalidResponse
                }
                return update
            } catch let error as PermanentCleanupPolicyError {
                throw Self.permanentCleanupPolicyError(error)
            }
        }
    }

    func loadCleanupExclusions() async throws -> CleanupExclusionsPolicy {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveCleanupExclusionsEngine(state)
            do {
                return try Self.cleanupExclusions(engine.getCleanupExclusions())
            } catch let error as CleanupExclusionsError {
                throw Self.cleanupExclusionsError(error)
            }
        }
    }

    func setCleanupExclusions(
        _ paths: [CleanupExclusionPathObservation]
    ) async throws -> CleanupExclusionsUpdateResult {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveCleanupExclusionsEngine(state)
            do {
                let input = CleanupExclusionsInput(
                    recordVersion: Self.expectedRecordVersion,
                    paths: paths.map(Self.cleanupExclusionPath)
                )
                let update = try Self.cleanupExclusionsUpdate(
                    engine.setCleanupExclusions(input: input)
                )
                guard update.exclusions.paths == Self.canonicalCleanupExclusionPaths(paths) else {
                    throw CleanupExclusionsServiceError.invalidResponse
                }
                return update
            } catch let error as CleanupExclusionsError {
                throw Self.cleanupExclusionsError(error)
            }
        }
    }

    func resetCleanupExclusions() async throws -> CleanupExclusionsUpdateResult {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveCleanupExclusionsEngine(state)
            do {
                let update = try Self.cleanupExclusionsUpdate(engine.resetCleanupExclusions())
                guard
                    update.exclusions.paths.isEmpty,
                    update.exclusions.source == .default
                else {
                    throw CleanupExclusionsServiceError.invalidResponse
                }
                return update
            } catch let error as CleanupExclusionsError {
                throw Self.cleanupExclusionsError(error)
            }
        }
    }

    func loadProjectDiscoveryRoots() async throws -> ProjectDiscoveryRoots {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveProjectDiscoveryRootsEngine(state)
            do {
                return try Self.projectDiscoveryRoots(engine.getConfiguredProjectRoots())
            } catch let error as ConfiguredProjectRootsError {
                throw Self.projectDiscoveryRootsError(error)
            }
        }
    }

    func setProjectDiscoveryRoots(
        _ roots: [ProjectDiscoveryRoot]
    ) async throws -> ProjectDiscoveryRootsUpdateResult {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveProjectDiscoveryRootsEngine(state)
            do {
                let input = ConfiguredProjectRootsInput(
                    recordVersion: Self.expectedRecordVersion,
                    roots: roots.map(Self.configuredProjectRootPath)
                )
                let update = try Self.projectDiscoveryRootsUpdate(
                    engine.setConfiguredProjectRoots(input: input)
                )
                guard update.roots.roots == Self.canonicalProjectDiscoveryRoots(roots) else {
                    throw ProjectDiscoveryRootsServiceError.invalidResponse
                }
                return update
            } catch let error as ConfiguredProjectRootsError {
                throw Self.projectDiscoveryRootsError(error)
            }
        }
    }

    func resetProjectDiscoveryRoots() async throws -> ProjectDiscoveryRootsUpdateResult {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveProjectDiscoveryRootsEngine(state)
            do {
                let update = try Self.projectDiscoveryRootsUpdate(
                    engine.resetConfiguredProjectRoots()
                )
                guard update.roots.roots.isEmpty, update.roots.source == .default else {
                    throw ProjectDiscoveryRootsServiceError.invalidResponse
                }
                return update
            } catch let error as ConfiguredProjectRootsError {
                throw Self.projectDiscoveryRootsError(error)
            }
        }
    }

    func loadDirectCargoEnrollmentStatus() async throws
        -> DirectCargoEnrollmentStatusModel
    {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveDirectCargoEnrollmentEngine(state)
            do {
                return try Self.directCargoEnrollmentStatus(
                    engine.directCargoEnrollmentStatus()
                )
            } catch let error as DirectCargoEnrollmentError {
                throw Self.directCargoEnrollmentError(error)
            }
        }
    }

    func inspectDirectCargoExecutable(
        _ selection: DirectCargoExecutableSelection
    ) async throws -> any DuxDirectCargoEnrollmentPreviewLease {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            guard DirectCargoExecutableSelection.isValidUnixCargoPath(
                selection.encodedPathBytes
            ) else {
                throw DirectCargoEnrollmentServiceError.invalidExecutablePath
            }
            let engine = try Self.resolveDirectCargoEnrollmentEngine(state)
            do {
                let preview = try engine.inspectDirectCargoEnrollment(
                    request: DirectCargoEnrollmentInspectionRequest(
                        recordVersion: Self.expectedRecordVersion,
                        pathEncoding: .unixBytes,
                        executablePathBytes: selection.encodedPathBytes
                    )
                )
                do {
                    let info = try preview.info()
                    let summary = try Self.directCargoEnrollmentPreview(info)
                    guard summary.executable == selection else {
                        throw DirectCargoEnrollmentServiceError.invalidResponse
                    }
                    return FFIDirectCargoEnrollmentPreviewLease(
                        preview: preview,
                        summary: summary,
                        state: state
                    )
                } catch {
                    _ = try? preview.release()
                    throw error
                }
            } catch let error as DirectCargoEnrollmentError {
                throw Self.directCargoEnrollmentError(error)
            }
        }
    }

    func enrollDirectCargo(
        _ preview: any DuxDirectCargoEnrollmentPreviewLease
    ) async throws -> DirectCargoEnrollmentUpdateModel {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            guard let preview = preview as? FFIDirectCargoEnrollmentPreviewLease else {
                throw DirectCargoEnrollmentServiceError.previewUnavailable
            }
            let engine = try Self.resolveDirectCargoEnrollmentEngine(state)
            let ffiPreview = try preview.take(for: state)
            let response: DirectCargoEnrollmentUpdate
            do {
                response = try engine.commitDirectCargoEnrollment(preview: ffiPreview)
            } catch let error as DirectCargoEnrollmentError {
                throw Self.directCargoEnrollmentError(error)
            }
            do {
                let update = try Self.directCargoEnrollmentUpdate(response)
                guard
                    case let .enrolled(identity) = update.status.disposition,
                    identity.executable == preview.preview.executable,
                    identity.executableSHA256 == preview.preview.executableSHA256,
                    identity.signature == preview.preview.signature
                else {
                    throw DirectCargoEnrollmentServiceError.outcomeUnknown
                }
                return update
            } catch {
                // The native commit returned, so any malformed or
                // non-correlating projection is an uncertain mutation
                // outcome rather than a safe-to-retry response error.
                throw DirectCargoEnrollmentServiceError.outcomeUnknown
            }
        }
    }

    func revokeDirectCargoEnrollment() async throws -> DirectCargoEnrollmentUpdateModel {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveDirectCargoEnrollmentEngine(state)
            let response: DirectCargoEnrollmentUpdate
            do {
                response = try engine.revokeDirectCargoEnrollment()
            } catch let error as DirectCargoEnrollmentError {
                throw Self.directCargoEnrollmentError(error)
            }
            do {
                let update = try Self.directCargoEnrollmentUpdate(response)
                guard update.status.disposition == .revoked else {
                    throw DirectCargoEnrollmentServiceError.outcomeUnknown
                }
                return update
            } catch {
                // A returned native revoke may already be durable even when
                // its projection cannot be trusted.
                throw DirectCargoEnrollmentServiceError.outcomeUnknown
            }
        }
    }

    func startHomeScan() async throws -> HomeScanStartDisposition {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let home = state.homeScanRoot ?? FileManager.default.homeDirectoryForCurrentUser
            guard home.isFileURL, home.path.hasPrefix("/") else {
                throw HomeScanServiceError.invalidRoot
            }
            let engine: DuxEngine
            do {
                engine = try state.resolveEngine()
            } catch let error as EngineServiceError {
                throw Self.homeScanServiceError(error)
            }
            do {
                let start = try engine.startScan(
                    request: ScanRequest(
                        recordVersion: Self.expectedRecordVersion,
                        root: home.path
                    )
                )
                guard start.recordVersion == Self.expectedRecordVersion else {
                    throw HomeScanServiceError.invalidResponse
                }
                let task = FFIHomeScanTask(task: start.task, state: state)
                return switch start.disposition {
                case .started: .started(task)
                case .alreadyActive: .alreadyActive(task)
                }
            } catch let error as ScanError {
                throw Self.homeScanServiceError(error)
            }
        }
    }

    func startTargetedReclaimScan(
        stableVolumeID: String,
        anchorAt: Date,
        ordinal: UInt16,
        expectedRootsRevision: UInt64?,
        expectedRootCatalogDigestSHA256: Data?
    ) async throws -> TargetedReclaimScanAdmission {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let canonicalVolumeID = try Self.targetedVolumeID(stableVolumeID)
            let anchorAtUnixMS = try Self.targetedTimestamp(anchorAt)
            let engine: DuxEngine
            do {
                engine = try state.resolveEngine()
            } catch let error as EngineServiceError {
                throw Self.targetedReclaimScanError(error)
            }

            let response: TargetedProjectScanAdmission
            do {
                response = try engine.startTargetedProjectScan(
                    request: TargetedProjectScanRequest(
                        recordVersion: Self.expectedRecordVersion,
                        stableVolumeId: canonicalVolumeID,
                        capacityAnchorUnixMs: anchorAtUnixMS,
                        selectedRootOrdinal: ordinal,
                        expectedConfiguredRootsRevision: expectedRootsRevision,
                        expectedRootCatalogDigestSha256: expectedRootCatalogDigestSHA256
                    )
                )
            } catch let error as TargetedProjectScanError {
                throw Self.targetedReclaimScanError(error)
            }

            let admission = try Self.targetedReclaimScanAdmission(
                response,
                expectedStableVolumeID: canonicalVolumeID,
                expectedAnchorAtUnixMS: anchorAtUnixMS,
                expectedOrdinal: ordinal,
                expectedRootsRevision: expectedRootsRevision,
                expectedRootCatalogDigestSHA256: expectedRootCatalogDigestSHA256,
                state: state
            )
            if let context = admission.context,
               context.rootCount > 0,
               let pressure = response.pressure
            {
                try state.rememberTargetedProjectScanProof(
                    context: context,
                    pressure: pressure
                )
            }
            return admission
        }
    }

    func validateTargetedReclaimScan(
        _ context: TargetedReclaimScanContext
    ) async throws -> TargetedReclaimScanContext {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            guard context.rootCount > 0 else {
                throw TargetedReclaimScanServiceError.invalidResponse
            }
            let proof = try state.targetedProjectScanProof(for: context)
            let engine: DuxEngine
            do {
                engine = try state.resolveEngine()
            } catch let error as EngineServiceError {
                throw Self.targetedReclaimScanError(error)
            }

            let response: TargetedProjectScanCheckpoint
            do {
                response = try engine.validateTargetedProjectScanContext(
                    request: TargetedProjectScanCheckpointRequest(
                        recordVersion: Self.expectedRecordVersion,
                        expectedPressure: proof.pressure,
                        expectedRootCatalog: Self.targetedReclaimRootCatalog(context)
                    )
                )
            } catch let error as TargetedProjectScanError {
                throw Self.targetedReclaimScanError(error)
            }
            let validated = try Self.targetedReclaimScanCheckpoint(
                response,
                expected: proof
            )
            state.consumeTargetedProjectScanProof(proof)
            return validated
        }
    }

    func finalizeEmergencyRecovery(
        _ context: TargetedReclaimScanContext
    ) async throws -> AppEmergencyRecoveryOrdering {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            guard
                context.rootCount > 0,
                context.pressure == .critical
            else {
                throw TargetedReclaimScanServiceError.pressureChanged
            }
            let proof = try state.targetedProjectScanProof(for: context)
            let engine: DuxEngine
            do {
                engine = try state.resolveEngine()
            } catch let error as EngineServiceError {
                throw Self.targetedReclaimScanError(error)
            }

            let response: EmergencyRecoveryOrdering
            do {
                response = try engine.finalizeEmergencyRecovery(
                    request: EmergencyRecoveryRequest(
                        recordVersion: Self.expectedRecordVersion,
                        expectedPressure: proof.pressure,
                        expectedRootCatalog: Self.targetedReclaimRootCatalog(context)
                    )
                )
            } catch let error as EmergencyRecoveryError {
                throw Self.emergencyRecoveryError(error)
            }
            let ordering = try Self.emergencyRecoveryOrdering(
                response,
                expected: proof
            )
            state.consumeTargetedProjectScanProof(proof)
            return ordering
        }
    }

    func startMaintenance(_ kind: DuxMaintenanceKind) async -> DuxMaintenanceStartDisposition {
        await state.performNonthrowing { state in
            do {
                let engine = try state.resolveEngine()
                let expectedKind = Self.ffiKind(kind)
                let start = try engine.startMaintenance(kind: expectedKind)
                guard start.recordVersion == 1 else {
                    return .failed(.blockedUntilRestart)
                }
                switch start.disposition {
                case .started:
                    guard let task = start.task else {
                        return .failed(.blockedUntilRestart)
                    }
                    return .started(
                        FFIDuxMaintenanceTask(
                            task: task,
                            state: state,
                            expectedKind: expectedKind
                        )
                    )
                case .alreadyActive:
                    guard let task = start.task else {
                        return .failed(.blockedUntilRestart)
                    }
                    return .alreadyActive(
                        FFIDuxMaintenanceTask(
                            task: task,
                            state: state,
                            expectedKind: expectedKind
                        )
                    )
                case .deferredBusy:
                    guard start.task == nil else {
                        return .failed(.blockedUntilRestart)
                    }
                    return .deferredBusy
                }
            } catch let error as EngineError {
                return .failed(Self.failureDisposition(error))
            } catch {
                return .failed(.blockedUntilRestart)
            }
        }
    }

    func acquireExplorerReview(scanID: String) async throws -> any DuxSnapshotReviewLease {
        let lease = try await state.perform { state in
            let engine = try state.resolveEngine()
            do {
                return try engine.acquireExplorerSnapshotReview(scanId: scanID)
            } catch let error as EngineError {
                throw Self.snapshotReviewAcquisitionError(error)
            }
        }
        return FFIDuxSnapshotReviewLease(lease: lease, state: state, scanID: scanID)
    }

    func acquireLatestExplorerReview() async throws -> any DuxSnapshotReviewLease {
        let (lease, info) = try await state.perform { state in
            let engine = try state.resolveEngine()
            do {
                let lease = try engine.acquireLatestExplorerSnapshotReview()
                do {
                    return try (lease, lease.info())
                } catch {
                    _ = try? lease.release()
                    throw error
                }
            } catch let error as EngineError {
                throw Self.snapshotReviewAcquisitionError(error)
            }
        }
        guard
            info.recordVersion == Self.expectedRecordVersion,
            !info.released,
            info.expiresAtUnixMs > 0,
            ExplorerSnapshotHistoryAdapter.validScanID(info.scanId)
        else {
            await state.performNonthrowing { _ in
                _ = try? lease.release()
            }
            throw EngineServiceError.unexpected("invalid latest review record")
        }
        return FFIDuxSnapshotReviewLease(lease: lease, state: state, scanID: info.scanId)
    }

    func loadRecentSnapshotHistory(limit: UInt16) async throws -> ExplorerSnapshotHistoryPage {
        guard (1 ... 200).contains(limit) else {
            throw ExplorerSnapshotHistoryError.invalidLimit
        }
        return try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try state.resolveEngine()
            do {
                return try ExplorerSnapshotHistoryAdapter.map(
                    engine.recentScanHistory(limit: limit)
                )
            } catch let error as EngineError {
                throw Self.serviceError(error)
            }
        }
    }

    func loadRecentCleanupHistory(
        cursor: CleanupHistoryCursorModel?,
        limit: UInt16
    ) async throws -> CleanupHistoryPageModel {
        guard (1 ... 64).contains(limit) else {
            throw CleanupHistoryServiceError.invalidLimit
        }
        return try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveCleanupHistoryEngine(state)
            let rawCursor = try cursor.map { observation in
                guard let milliseconds = Self.unixMilliseconds(observation.startedAt) else {
                    throw CleanupHistoryServiceError.invalidCursor
                }
                return CleanupHistoryCursor(
                    recordVersion: Self.expectedRecordVersion,
                    startedAtUnixMs: milliseconds,
                    sessionId: observation.sessionID
                )
            }
            do {
                return try CleanupHistoryAdapter.map(
                    engine.recentCleanupHistory(cursor: rawCursor, limit: limit)
                )
            } catch let error as CleanupHistoryError {
                throw Self.cleanupHistoryError(error)
            }
        }
    }

    func loadCleanupHistorySession(
        sessionID: String
    ) async throws -> CleanupHistorySessionDetailModel {
        guard CleanupHistoryAdapter.validStableToken(sessionID) else {
            throw CleanupHistoryServiceError.invalidSessionID
        }
        return try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveCleanupHistoryEngine(state)
            do {
                return try CleanupHistoryAdapter.mapSession(
                    engine.cleanupSessionHistory(
                        request: CleanupSessionHistoryRequest(
                            recordVersion: Self.expectedRecordVersion,
                            sessionId: sessionID
                        )
                    ),
                    requestedSessionID: sessionID
                )
            } catch let error as CleanupHistoryError {
                throw Self.cleanupHistoryError(error)
            }
        }
    }

    func loadCleanupHistoryRuleOutcomes(
        sessionID: String,
        detail: CleanupHistorySessionDetailModel
    ) async throws -> CleanupHistoryRuleOutcomeBatchModel {
        guard
            CleanupHistoryAdapter.validStableToken(sessionID),
            detail.summary.sessionID == sessionID
        else {
            throw CleanupHistoryServiceError.invalidSessionID
        }
        return try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveCleanupHistoryEngine(state)
            do {
                return try CleanupHistoryAdapter.mapRuleOutcomes(
                    engine.ruleOutcomesForCleanupSession(
                        request: CleanupSessionHistoryRequest(
                            recordVersion: Self.expectedRecordVersion,
                            sessionId: sessionID
                        )
                    ),
                    requestedSessionID: sessionID,
                    detail: detail
                )
            } catch let error as RuleOutcomeError {
                throw Self.ruleOutcomeError(error)
            }
        }
    }

    func loadRecurringStorageThieves() async throws
        -> CleanupHistoryStorageThiefRankingModel
    {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveCleanupHistoryEngine(state)
            do {
                return try CleanupHistoryAdapter.mapStorageThiefRanking(
                    engine.recurringStorageThieves()
                )
            } catch let error as StorageThiefError {
                throw Self.storageThiefError(error)
            }
        }
    }

    func prepareCleanupHistoryClear() async throws
        -> any DuxCleanupHistoryClearPreviewLease
    {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try Self.resolveCleanupHistoryClearEngine(state)
            do {
                let preview = try engine.prepareCleanupHistoryClear()
                do {
                    let model = try Self.cleanupHistoryClearPreview(
                        preview.info(),
                        observedAt: Date()
                    )
                    return FFICleanupHistoryClearPreviewLease(
                        ffiPreview: preview,
                        preview: model,
                        state: state
                    )
                } catch {
                    _ = try? preview.release()
                    throw error
                }
            } catch let error as CleanupHistoryClearError {
                throw Self.cleanupHistoryClearError(error)
            }
        }
    }

    func clearCleanupHistory(
        _ preview: any DuxCleanupHistoryClearPreviewLease
    ) async throws -> CleanupHistoryClearResultModel {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            guard let preview = preview as? FFICleanupHistoryClearPreviewLease else {
                throw CleanupHistoryClearServiceError.wrongEngine
            }
            let engine = try Self.resolveCleanupHistoryClearEngine(state)
            let ffiPreview = try preview.take(for: state)
            let response: CleanupHistoryClearResult
            do {
                response = try engine.clearCleanupHistory(preview: ffiPreview)
            } catch let error as CleanupHistoryClearError {
                throw Self.cleanupHistoryClearError(error)
            }
            guard
                response.recordVersion == Self.expectedRecordVersion,
                response.clearedSessionCount > 0,
                response.clearedSessionCount == preview.preview.sessionCount
            else {
                // Rust may already have committed the metadata deletion.
                // Malformed or non-correlating success is therefore unknown,
                // never an ordinary response error that may be retried.
                throw CleanupHistoryClearServiceError.outcomeUnknown
            }
            return CleanupHistoryClearResultModel(
                clearedSessionCount: response.clearedSessionCount
            )
        }
    }

    func loadScanCoverageDetails(scanID: String) async throws -> ExplorerScanCoverageDetails {
        guard ExplorerSnapshotHistoryAdapter.validScanID(scanID) else {
            throw ExplorerScanCoverageError.invalidRequest
        }
        return try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine = try state.resolveEngine()
            var pages: [ExplorerScanCoverageDetailsPage] = []
            var offset: UInt16 = 0
            do {
                repeat {
                    let raw = try engine.scanCoverageDetails(
                        scanId: scanID,
                        request: ScanCoverageDetailsRequest(
                            recordVersion: Self.expectedRecordVersion,
                            offset: offset,
                            limit: ExplorerScanCoverageDetailsAdapter.maximumPageLimit
                        )
                    )
                    let page = try ExplorerScanCoverageDetailsAdapter.map(
                        raw,
                        requestedScanID: scanID,
                        requestedOffset: offset,
                        requestedLimit: ExplorerScanCoverageDetailsAdapter.maximumPageLimit
                    )
                    pages.append(page)
                    guard page.hasMore else {
                        break
                    }
                    let next = Int(offset) + page.issues.count
                    guard next <= Int(UInt16.max), next > Int(offset) else {
                        throw ExplorerScanCoverageError.invalidResponse
                    }
                    offset = UInt16(next)
                } while pages.count <= 4
                guard pages.count <= 4 else {
                    throw ExplorerScanCoverageError.invalidResponse
                }
                return try ExplorerScanCoverageDetailsAdapter.assemble(
                    pages,
                    requestedScanID: scanID
                )
            } catch let error as EngineError {
                throw Self.scanCoverageError(error)
            }
        }
    }

    func close() async -> Bool {
        await state.close()
    }

    fileprivate static func defaultStorageRoots() throws -> EngineStorageRoots {
        if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil {
            let root = FileManager.default.temporaryDirectory.appending(
                path: "dux-app-test-host-\(ProcessInfo.processInfo.processIdentifier)",
                directoryHint: .isDirectory
            )
            return EngineStorageRoots(
                dataRoot: root.appending(path: "data", directoryHint: .isDirectory).path,
                cacheRoot: root
                    .appending(path: "cache", directoryHint: .isDirectory)
                    .appending(path: "Dux", directoryHint: .isDirectory)
                    .path
            )
        }
        let manager = FileManager.default
        guard
            let applicationSupport = manager.urls(
                for: .applicationSupportDirectory,
                in: .userDomainMask
            ).first,
            let caches = manager.urls(for: .cachesDirectory, in: .userDomainMask).first
        else {
            throw EngineServiceError.unexpected("platform storage directory unavailable")
        }
        return EngineStorageRoots(
            dataRoot: applicationSupport.appending(path: "Dux", directoryHint: .isDirectory).path,
            cacheRoot: caches.appending(path: "Dux", directoryHint: .isDirectory).path
        )
    }

    private static func ffiKind(_ kind: DuxMaintenanceKind) -> MaintenanceKind {
        switch kind {
        case .scanRecovery: .scanRecovery
        case .candidateEvaluationRecovery: .candidateEvaluationRecovery
        case .history: .history
        case .snapshotRetention: .snapshotRetention
        case .snapshotOrphan: .snapshotOrphan
        case .snapshotProvisioningStage: .snapshotProvisioningStage
        case .snapshotTerminalTemp: .snapshotTerminalTemp
        case .snapshotUnleasedTemp: .snapshotUnleasedTemp
        }
    }

    fileprivate static func serviceError(_ error: EngineError) -> EngineServiceError {
        switch error {
        case .Closed:
            .closed
        case .InvalidCapacityObservation:
            .invalidCapacityObservation
        case .ConflictingCapacityObservation:
            .conflictingCapacityObservation
        case .SupersededCapacityObservation:
            .supersededCapacityObservation
        case .Busy, .StorageUnavailable, .RegistryUnavailable:
            .retryable
        case .ReadOnlyStore, .IncompatibleSchema, .UnsafeStorage, .CorruptData,
             .OutcomeUnknown, .InternalState:
            .unavailable
        default:
            .unexpected(String(describing: error))
        }
    }

    private static func snapshotReviewAcquisitionError(
        _ error: EngineError
    ) -> ExplorerSnapshotReviewAcquisitionError {
        switch error {
        case .Closed:
            .closed
        case .ScanNotFound:
            .scanNotFound
        case .SnapshotUnavailable:
            .snapshotUnavailable
        case .Busy, .StorageUnavailable, .RegistryUnavailable:
            .retryable
        case .BudgetExceeded:
            .budgetExceeded
        case .ReadOnlyStore, .IncompatibleSchema, .UnsafeStorage, .CorruptData,
             .IncompatibleSnapshot, .OutcomeUnknown, .InternalState:
            .unavailable
        default:
            .invalidResponse
        }
    }

    private static func scanCoverageError(_ error: EngineError) -> ExplorerScanCoverageError {
        switch error {
        case .ScanNotFound:
            .scanNotFound
        case .Busy, .StorageUnavailable, .RegistryUnavailable, .BudgetExceeded:
            .retryable
        case .InvalidScanId, .InvalidScanCoverageDetailsRequest:
            .invalidRequest
        case .Closed, .ReadOnlyStore, .IncompatibleSchema, .UnsafeStorage, .CorruptData,
             .OutcomeUnknown, .InternalState:
            .unavailable
        default:
            .invalidResponse
        }
    }

    private static func cleanupHistoryError(
        _ error: CleanupHistoryError
    ) -> CleanupHistoryServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidResponse
        case .InvalidSessionId: .invalidSessionID
        case .InvalidLimit: .invalidLimit
        case .InvalidCursor: .invalidCursor
        case .SessionNotFound: .sessionNotFound
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .InternalState: .internalState
        }
    }

    private static func ruleOutcomeError(
        _ error: RuleOutcomeError
    ) -> CleanupHistoryServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidResponse
        case .InvalidSessionId: .invalidSessionID
        case .SessionNotFound: .sessionNotFound
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .InternalState: .internalState
        }
    }

    private static func storageThiefError(
        _ error: StorageThiefError
    ) -> CleanupHistoryServiceError {
        switch error {
        case .Closed: .closed
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .InternalState: .internalState
        }
    }

    static func persistentRecoveryDebt(
        _ census: RunningScanDebtCensus
    ) throws -> PersistentRecoveryDebt {
        let (classifiedCount, overflowed) =
            census.pristineUnclaimedCount.addingReportingOverflow(
                census.unexplainedUnclaimedCount
            )
        guard
            census.recordVersion == expectedRecordVersion,
            !overflowed,
            census.inspectedUnclaimedCount <= PersistentRecoveryDebt.maximumInspectedCount,
            census.pristineUnclaimedCount <= census.inspectedUnclaimedCount,
            census.unexplainedUnclaimedCount <= census.inspectedUnclaimedCount,
            classifiedCount == census.inspectedUnclaimedCount,
            !census.hasMore
            || census.inspectedUnclaimedCount
            == PersistentRecoveryDebt.maximumInspectedCount
        else {
            throw PersistentRecoveryDebtServiceError.invalidResponse
        }
        return PersistentRecoveryDebt(
            inspectedUnclaimedCount: census.inspectedUnclaimedCount,
            pristineUnclaimedCount: census.pristineUnclaimedCount,
            unexplainedUnclaimedCount: census.unexplainedUnclaimedCount,
            hasMore: census.hasMore
        )
    }

    static func claimedRunningScanProvenance(
        _ census: ClaimedRunningScanProvenanceCensus
    ) throws -> ClaimedRunningScanProvenance {
        let categoryCounts = [
            census.sameHostCurrentBootCount,
            census.sameHostPriorBootCount,
            census.foreignHostCount,
            census.storedUnprovenCount,
            census.currentContextUnavailableCount,
        ]
        var classifiedCount: UInt16 = 0
        for count in categoryCounts {
            let addition = classifiedCount.addingReportingOverflow(count)
            guard !addition.overflow else {
                throw ClaimedRunningScanProvenanceServiceError.invalidResponse
            }
            classifiedCount = addition.partialValue
        }
        guard
            census.recordVersion == expectedRecordVersion,
            census.inspectedClaimedCount
            <= ClaimedRunningScanProvenance.maximumInspectedCount,
            categoryCounts.allSatisfy({ $0 <= census.inspectedClaimedCount }),
            classifiedCount == census.inspectedClaimedCount,
            census.currentContextUnavailableCount == 0
            || (
                census.sameHostCurrentBootCount == 0
                    && census.sameHostPriorBootCount == 0
                    && census.foreignHostCount == 0
            ),
            !census.hasMore
            || census.inspectedClaimedCount
            == ClaimedRunningScanProvenance.maximumInspectedCount
        else {
            throw ClaimedRunningScanProvenanceServiceError.invalidResponse
        }
        return ClaimedRunningScanProvenance(
            inspectedClaimedCount: census.inspectedClaimedCount,
            sameHostCurrentBootCount: census.sameHostCurrentBootCount,
            sameHostPriorBootCount: census.sameHostPriorBootCount,
            foreignHostCount: census.foreignHostCount,
            storedUnprovenCount: census.storedUnprovenCount,
            currentContextUnavailableCount: census.currentContextUnavailableCount,
            hasMore: census.hasMore
        )
    }

    private static func persistentRecoveryDebtError(
        _ error: RunningScanDebtCensusError
    ) -> PersistentRecoveryDebtServiceError {
        switch error {
        case .Closed: .closed
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .InternalState: .internalState
        }
    }

    private static func claimedRunningScanProvenanceError(
        _ error: ClaimedRunningScanProvenanceCensusError
    ) -> ClaimedRunningScanProvenanceServiceError {
        switch error {
        case .Closed: .closed
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .InternalState: .internalState
        }
    }

    private static func cleanupHistoryClearError(
        _ error: CleanupHistoryClearError
    ) -> CleanupHistoryClearServiceError {
        switch error {
        case .Closed: .closed
        case .NothingToClear: .nothingToClear
        case .ActiveCleanup: .activeCleanup
        case .ChangedSincePreview: .changedSincePreview
        case .PreviewExpired: .previewExpired
        case .WrongEngine: .wrongEngine
        case .PreviewUnavailable: .previewUnavailable
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .OutcomeUnknown: .outcomeUnknown
        case .Unavailable: .unavailable
        case .InternalState: .internalState
        }
    }

    private static func cleanupHistoryClearPreview(
        _ info: CleanupHistoryClearPreviewInfo,
        observedAt: Date
    ) throws -> CleanupHistoryClearPreviewModel {
        let maximumUnixMilliseconds: Int64 = 253_402_300_799_999
        let observedAtMilliseconds = unixMilliseconds(observedAt)
        guard
            info.recordVersion == expectedRecordVersion,
            info.sessionCount > 0,
            let observedAtMilliseconds,
            (0 ... maximumUnixMilliseconds).contains(
                info.oldestStartedAtUnixMs
            ),
            (0 ... maximumUnixMilliseconds).contains(
                info.newestStartedAtUnixMs
            ),
            (0 ... maximumUnixMilliseconds).contains(info.preparedAtUnixMs),
            (0 ... maximumUnixMilliseconds).contains(info.expiresAtUnixMs),
            info.oldestStartedAtUnixMs <= info.newestStartedAtUnixMs,
            info.preparedAtUnixMs <= observedAtMilliseconds,
            observedAtMilliseconds < info.expiresAtUnixMs,
            info.expiresAtUnixMs - info.preparedAtUnixMs <= 120_000
        else {
            throw CleanupHistoryClearServiceError.invalidResponse
        }
        return CleanupHistoryClearPreviewModel(
            sessionCount: info.sessionCount,
            oldestStartedAt: Date(
                timeIntervalSince1970:
                Double(info.oldestStartedAtUnixMs) / 1000
            ),
            newestStartedAt: Date(
                timeIntervalSince1970:
                Double(info.newestStartedAtUnixMs) / 1000
            ),
            preparedAt: Date(
                timeIntervalSince1970: Double(info.preparedAtUnixMs) / 1000
            ),
            expiresAt: Date(
                timeIntervalSince1970: Double(info.expiresAtUnixMs) / 1000
            )
        )
    }

    private static func unixMilliseconds(_ date: Date) -> Int64? {
        let value = date.timeIntervalSince1970 * 1000
        guard value.isFinite, value >= 0, value <= Double(Int64.max) else {
            return nil
        }
        return Int64(value.rounded(.towardZero))
    }

    private static func policy(_ status: PressurePolicyStatus) throws -> DiskPressurePolicy {
        let configuration = DiskPressurePolicyConfiguration(
            criticalAvailableBytes: status.criticalAvailableBytes,
            criticalAvailableBasisPoints: status.criticalAvailableBasisPoints,
            warningAvailableBytes: status.warningAvailableBytes,
            warningAvailableBasisPoints: status.warningAvailableBasisPoints,
            recoveryBytes: status.recoveryBytes,
            recoveryBasisPoints: status.recoveryBasisPoints
        )
        let source: DiskPressurePolicySource = switch status.source {
        case .default: .default
        case .stored: .stored
        }
        guard
            status.recordVersion == expectedRecordVersion,
            status.revision <= UInt64(Int64.max),
            configuration.criticalAvailableBytes > 0,
            configuration.warningAvailableBytes >= configuration.criticalAvailableBytes,
            configuration.recoveryBytes > 0,
            (1 ... 10000).contains(configuration.criticalAvailableBasisPoints),
            configuration.warningAvailableBasisPoints
            >= configuration.criticalAvailableBasisPoints,
            (1 ... 10000).contains(configuration.warningAvailableBasisPoints),
            (1 ... 10000).contains(configuration.recoveryBasisPoints),
            configuration.warningAvailableBytes != configuration.criticalAvailableBytes
            || configuration.warningAvailableBasisPoints
            != configuration.criticalAvailableBasisPoints,
            source != .default || configuration == .defaults,
            (status.revision == 0
                && source == .default
                && status.updatedAtUnixMs == nil)
            || (status.revision > 0
                && status.updatedAtUnixMs.map { $0 >= 0 } == true)
        else {
            throw DiskPressurePolicyServiceError.invalidResponse
        }
        return DiskPressurePolicy(
            source: source,
            revision: status.revision,
            configuration: configuration,
            updatedAtUnixMilliseconds: status.updatedAtUnixMs
        )
    }

    private static func policyUpdate(
        _ response: PressurePolicyUpdate
    ) throws -> DiskPressurePolicyUpdateResult {
        guard response.recordVersion == expectedRecordVersion else {
            throw DiskPressurePolicyServiceError.invalidResponse
        }
        return try DiskPressurePolicyUpdateResult(
            policy: policy(response.policy),
            changed: response.changed
        )
    }

    private static func pressurePolicyError(
        _ error: PressurePolicyError
    ) -> DiskPressurePolicyServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidRecordVersion
        case .ThresholdBytesZero: .thresholdBytesZero
        case .ThresholdBasisPointsOutOfRange: .thresholdBasisPointsOutOfRange
        case .WarningBytesBelowCritical: .warningBytesBelowCritical
        case .WarningBasisPointsBelowCritical: .warningBasisPointsBelowCritical
        case .WarningThresholdMatchesCritical: .warningThresholdMatchesCritical
        case .RecoveryBytesZero: .recoveryBytesZero
        case .RecoveryBasisPointsOutOfRange: .recoveryBasisPointsOutOfRange
        case .RevisionExhausted: .revisionExhausted
        case .InvalidClock: .invalidClock
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy, .BudgetExceeded: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .OutcomeUnknown: .outcomeUnknown
        case .InternalState: .internalState
        }
    }

    private static func snapshotRetentionCap(
        _ status: SnapshotRetentionCapStatus
    ) throws -> SnapshotRetentionCapModel {
        let source: SnapshotRetentionCapSourceModel = switch status.source {
        case .default: .default
        case .stored: .stored
        }
        guard
            status.recordVersion == expectedRecordVersion,
            (source == .default && status.updatedAtUnixMs == nil)
                || (source == .stored
                    && status.updatedAtUnixMs.map { $0 >= 0 } == true)
        else {
            throw SnapshotRetentionCapServiceError.invalidResponse
        }
        return SnapshotRetentionCapModel(
            capBytes: status.capBytes,
            source: source,
            updatedAtUnixMilliseconds: status.updatedAtUnixMs
        )
    }

    private static func snapshotRetentionCapUpdate(
        _ response: SnapshotRetentionCapUpdate
    ) throws -> SnapshotRetentionCapUpdateResultModel {
        guard response.recordVersion == expectedRecordVersion else {
            throw SnapshotRetentionCapServiceError.invalidResponse
        }
        return SnapshotRetentionCapUpdateResultModel(
            settings: try snapshotRetentionCap(response.settings),
            changed: response.changed
        )
    }

    private static func snapshotRetentionCapError(
        _ error: SnapshotRetentionCapError
    ) -> SnapshotRetentionCapServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidRecordVersion
        case .InvalidClock: .invalidClock
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .OutcomeUnknown: .outcomeUnknown
        case .InternalState: .internalState
        }
    }

    static func ownedStorageFootprint(
        _ footprint: OwnedStorageFootprint
    ) throws -> DuxOwnedStorageFootprintModel {
        guard
            footprint.recordVersion == expectedRecordVersion,
            footprint.observedAtUnixMs >= 0
        else {
            throw DuxOwnedStorageFootprintServiceError.invalidResponse
        }

        let database = try ownedStorageUsage(footprint.database)
        let controls = try ownedStorageUsage(footprint.snapshots.controls)
        let available = try ownedStorageUsage(footprint.snapshots.available)
        let protected = try ownedStorageUsage(footprint.snapshots.protected)
        let retentionEligible = try ownedStorageUsage(
            footprint.snapshots.retentionEligible
        )
        let tombstonedResidual = try ownedStorageUsage(
            footprint.snapshots.tombstonedResidual
        )
        let orphan = try ownedStorageUsage(footprint.snapshots.orphan)
        let temporaryActive = try ownedStorageUsage(
            footprint.snapshots.temporaryActive
        )
        let temporaryQuiescent = try ownedStorageUsage(
            footprint.snapshots.temporaryQuiescent
        )
        let temporaryUnleased = try ownedStorageUsage(
            footprint.snapshots.temporaryUnleased
        )
        let snapshotTotal = try ownedStorageUsage(footprint.snapshots.total)
        let managedScanCacheControls = try ownedStorageUsage(
            footprint.managedScanCache.controls
        )
        let managedScanCacheEntries = try ownedStorageUsage(
            footprint.managedScanCache.entries
        )
        let managedScanCacheTemporary = try ownedStorageUsage(
            footprint.managedScanCache.temporary
        )
        let managedScanCacheTotal = try ownedStorageUsage(
            footprint.managedScanCache.total
        )
        let physicalTotal = try ownedStorageUsage(footprint.physicalTotal)

        guard
            footprint.snapshots.recordVersion == expectedRecordVersion,
            footprint.managedScanCache.recordVersion
                == expectedRecordVersion,
            footprint.embeddedAiCache.recordVersion == expectedRecordVersion
        else {
            throw DuxOwnedStorageFootprintServiceError.invalidResponse
        }

        let classifiedAvailable = try addOwnedStorageUsage(
            protected,
            retentionEligible
        )
        let classifiedSnapshotTotal = try [
            controls,
            available,
            tombstonedResidual,
            orphan,
            temporaryActive,
            temporaryQuiescent,
            temporaryUnleased,
        ]
        .reduce(
            DuxOwnedStorageUsageModel(
                logicalBytes: 0,
                allocatedBytes: 0,
                chargedBytes: 0
            ),
            addOwnedStorageUsage
        )
        let classifiedManagedScanCacheTotal = try [
            managedScanCacheControls,
            managedScanCacheEntries,
            managedScanCacheTemporary,
        ]
        .reduce(
            DuxOwnedStorageUsageModel.zero,
            addOwnedStorageUsage
        )
        let expectedPhysicalTotal = try addOwnedStorageUsage(
            try addOwnedStorageUsage(database, snapshotTotal),
            managedScanCacheTotal
        )
        let managedScanCacheObjectCount = try checkedAdd(
            footprint.managedScanCache.entryCount,
            footprint.managedScanCache.temporaryCount
        )
        let availableCount = try checkedAdd(
            footprint.snapshots.protectedCount,
            footprint.snapshots.retentionEligibleCount
        )
        let physicalSnapshotObjectCount = try [
            footprint.snapshots.availableCount,
            footprint.snapshots.tombstonedResidualCount,
            footprint.snapshots.orphanCount,
            footprint.snapshots.activeTemporaryCount,
            footprint.snapshots.quiescentTemporaryCount,
            footprint.snapshots.unleasedTemporaryCount,
        ]
        .reduce(UInt32(0), checkedAdd)
        let pinRowCount = try checkedAdd(
            footprint.snapshots.activePinRows,
            footprint.snapshots.expiredPinRows
        )
        let nonEvictableChargedBytes = try [
            controls,
            protected,
            tombstonedResidual,
            orphan,
            temporaryActive,
            temporaryQuiescent,
            temporaryUnleased,
        ]
        .reduce(UInt64(0)) { total, usage in
            try checkedAdd(total, usage.chargedBytes)
        }
        let expectedCapExcess = snapshotTotal.chargedBytes
            > footprint.snapshots.capBytes
            ? snapshotTotal.chargedBytes - footprint.snapshots.capBytes
            : 0
        let expectedAccountingUnstable =
            footprint.snapshots.activeTemporaryCount > 0
            || footprint.snapshots.unleasedTemporaryCount > 0
        let expectedNonEvictableOverCap =
            nonEvictableChargedBytes > footprint.snapshots.capBytes

        let ai = footprint.embeddedAiCache
        let aiShapeIsValid = try aiContentShapeIsValid(
            count: ai.recordCount,
            bytes: ai.logicalContentBytes
        )
        let expiredAiShapeIsValid = try aiContentShapeIsValid(
            count: ai.expiredRecordCount,
            bytes: ai.expiredLogicalContentBytes
        )
        let allAiExpiredShapeIsValid =
            (ai.expiredRecordCount == ai.recordCount)
            == (ai.expiredLogicalContentBytes == ai.logicalContentBytes)

        guard
            classifiedAvailable == available,
            classifiedSnapshotTotal == snapshotTotal,
            classifiedManagedScanCacheTotal == managedScanCacheTotal,
            expectedPhysicalTotal == physicalTotal,
            managedScanCacheObjectCount
                <= DuxManagedScanCacheFootprintModel.maximumObjectCount,
            footprint.managedScanCache.temporaryCount
                <= DuxManagedScanCacheFootprintModel
                    .maximumTemporaryObjectCount,
            footprint.managedScanCache.entryCount > 0
                || managedScanCacheEntries == .zero,
            footprint.managedScanCache.temporaryCount > 0
                || managedScanCacheTemporary == .zero,
            managedScanCacheControls != .zero
                || (managedScanCacheObjectCount == 0
                    && managedScanCacheTotal == .zero),
            availableCount == footprint.snapshots.availableCount,
            physicalSnapshotObjectCount
                <= DuxSnapshotStorageFootprintModel.maximumObjectCount,
            footprint.snapshots.residualTemporaryLeaseCount
                <= DuxSnapshotStorageFootprintModel
                    .maximumResidualTemporaryLeaseCount,
            pinRowCount <= DuxSnapshotStorageFootprintModel.maximumPinRowCount,
            footprint.snapshots.capExcessBytes == expectedCapExcess,
            footprint.snapshots.accountingUnstable
                == expectedAccountingUnstable,
            footprint.snapshots.nonEvictableOverCap
                == expectedNonEvictableOverCap,
            ai.expiredRecordCount <= ai.recordCount,
            ai.expiredLogicalContentBytes <= ai.logicalContentBytes,
            ai.logicalContentBytes <= database.logicalBytes,
            aiShapeIsValid,
            expiredAiShapeIsValid,
            allAiExpiredShapeIsValid
        else {
            throw DuxOwnedStorageFootprintServiceError.invalidResponse
        }

        return DuxOwnedStorageFootprintModel(
            observedAt: Date(
                timeIntervalSince1970:
                    Double(footprint.observedAtUnixMs) / 1_000
            ),
            database: database,
            snapshots: DuxSnapshotStorageFootprintModel(
                capBytes: footprint.snapshots.capBytes,
                capExcessBytes: footprint.snapshots.capExcessBytes,
                controls: controls,
                available: available,
                protected: protected,
                retentionEligible: retentionEligible,
                tombstonedResidual: tombstonedResidual,
                orphan: orphan,
                temporaryActive: temporaryActive,
                temporaryQuiescent: temporaryQuiescent,
                temporaryUnleased: temporaryUnleased,
                total: snapshotTotal,
                availableCount: footprint.snapshots.availableCount,
                protectedCount: footprint.snapshots.protectedCount,
                retentionEligibleCount:
                    footprint.snapshots.retentionEligibleCount,
                tombstonedResidualCount:
                    footprint.snapshots.tombstonedResidualCount,
                orphanCount: footprint.snapshots.orphanCount,
                activeTemporaryCount:
                    footprint.snapshots.activeTemporaryCount,
                quiescentTemporaryCount:
                    footprint.snapshots.quiescentTemporaryCount,
                unleasedTemporaryCount:
                    footprint.snapshots.unleasedTemporaryCount,
                residualTemporaryLeaseCount:
                    footprint.snapshots.residualTemporaryLeaseCount,
                activePinRows: footprint.snapshots.activePinRows,
                expiredPinRows: footprint.snapshots.expiredPinRows,
                nonEvictableOverCap:
                    footprint.snapshots.nonEvictableOverCap,
                accountingUnstable:
                    footprint.snapshots.accountingUnstable
            ),
            managedScanCache: DuxManagedScanCacheFootprintModel(
                controls: managedScanCacheControls,
                entries: managedScanCacheEntries,
                temporary: managedScanCacheTemporary,
                total: managedScanCacheTotal,
                entryCount: footprint.managedScanCache.entryCount,
                temporaryCount:
                    footprint.managedScanCache.temporaryCount
            ),
            embeddedAiCache: DuxEmbeddedAiCacheFootprintModel(
                recordCount: ai.recordCount,
                logicalContentBytes: ai.logicalContentBytes,
                expiredRecordCount: ai.expiredRecordCount,
                expiredLogicalContentBytes: ai.expiredLogicalContentBytes
            ),
            physicalTotal: physicalTotal
        )
    }

    private static func ownedStorageUsage(
        _ usage: OwnedStorageUsage
    ) throws -> DuxOwnedStorageUsageModel {
        guard
            usage.recordVersion == expectedRecordVersion,
            usage.chargedBytes >= usage.logicalBytes,
            usage.chargedBytes >= usage.allocatedBytes
        else {
            throw DuxOwnedStorageFootprintServiceError.invalidResponse
        }
        return DuxOwnedStorageUsageModel(
            logicalBytes: usage.logicalBytes,
            allocatedBytes: usage.allocatedBytes,
            chargedBytes: usage.chargedBytes
        )
    }

    private static func addOwnedStorageUsage(
        _ left: DuxOwnedStorageUsageModel,
        _ right: DuxOwnedStorageUsageModel
    ) throws -> DuxOwnedStorageUsageModel {
        DuxOwnedStorageUsageModel(
            logicalBytes: try checkedAdd(left.logicalBytes, right.logicalBytes),
            allocatedBytes: try checkedAdd(
                left.allocatedBytes,
                right.allocatedBytes
            ),
            chargedBytes: try checkedAdd(left.chargedBytes, right.chargedBytes)
        )
    }

    private static func checkedAdd(
        _ left: UInt64,
        _ right: UInt64
    ) throws -> UInt64 {
        let result = left.addingReportingOverflow(right)
        guard !result.overflow else {
            throw DuxOwnedStorageFootprintServiceError.invalidResponse
        }
        return result.partialValue
    }

    private static func checkedAdd(
        _ left: UInt32,
        _ right: UInt32
    ) throws -> UInt32 {
        let result = left.addingReportingOverflow(right)
        guard !result.overflow else {
            throw DuxOwnedStorageFootprintServiceError.invalidResponse
        }
        return result.partialValue
    }

    private static func aiContentShapeIsValid(
        count: UInt32,
        bytes: UInt64
    ) throws -> Bool {
        let count = UInt64(count)
        let minimum = try checkedMultiply(
            count,
            DuxEmbeddedAiCacheFootprintModel.minimumContentBytesPerRecord
        )
        let maximum = try checkedMultiply(
            count,
            DuxEmbeddedAiCacheFootprintModel.maximumContentBytesPerRecord
        )
        return (minimum ... maximum).contains(bytes)
    }

    private static func checkedMultiply(
        _ left: UInt64,
        _ right: UInt64
    ) throws -> UInt64 {
        let result = left.multipliedReportingOverflow(by: right)
        guard !result.overflow else {
            throw DuxOwnedStorageFootprintServiceError.invalidResponse
        }
        return result.partialValue
    }

    private static func managedScanCacheClearPreview(
        _ response: ManagedScanCacheClearPreviewInfo
    ) throws -> DuxManagedScanCacheClearPreviewModel {
        let clearable: DuxOwnedStorageUsageModel
        do {
            clearable = try ownedStorageUsage(response.clearable)
        } catch {
            throw DuxManagedScanCacheClearServiceError.invalidResponse
        }
        let expectedCount = response.entryCount.addingReportingOverflow(
            response.temporaryCount
        )
        guard
            response.recordVersion == expectedRecordVersion,
            !expectedCount.overflow,
            response.clearableCount == expectedCount.partialValue,
            (1 ... DuxManagedScanCacheFootprintModel.maximumObjectCount)
                .contains(response.clearableCount),
            response.temporaryCount
                <= DuxManagedScanCacheFootprintModel.maximumTemporaryObjectCount,
            response.preparedAtUnixMs >= 0,
            response.expiresAtUnixMs > response.preparedAtUnixMs
        else {
            throw DuxManagedScanCacheClearServiceError.invalidResponse
        }
        return DuxManagedScanCacheClearPreviewModel(
            entryCount: response.entryCount,
            temporaryCount: response.temporaryCount,
            clearableCount: response.clearableCount,
            clearable: clearable,
            preparedAt: Date(
                timeIntervalSince1970:
                    Double(response.preparedAtUnixMs) / 1_000
            ),
            expiresAt: Date(
                timeIntervalSince1970:
                    Double(response.expiresAtUnixMs) / 1_000
            )
        )
    }

    private static func managedScanCacheClearError(
        _ error: ManagedScanCacheClearError
    ) -> DuxManagedScanCacheClearServiceError {
        switch error {
        case .Closed: .closed
        case .NothingToClear: .nothingToClear
        case .ReadOnlyStore: .readOnlyStore
        case .ChangedSincePreview: .changedSincePreview
        case .PreviewExpired: .previewExpired
        case .WrongEngine: .wrongEngine
        case .PreviewUnavailable: .previewUnavailable
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .OutcomeUnknown: .outcomeUnknown
        case .Unavailable: .unavailable
        case .InternalState: .internalState
        }
    }

    private static func ownedStorageFootprintError(
        _ error: OwnedStorageFootprintError
    ) -> DuxOwnedStorageFootprintServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidClock: .invalidClock
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .InternalState: .internalState
        }
    }

    private static func permanentCleanupPolicy(
        _ status: PermanentCleanupPolicyStatus
    ) throws -> PermanentCleanupPolicy {
        let source: PermanentCleanupPolicyOrigin = switch status.source {
        case .default: .default
        case .stored: .stored
        }
        guard
            status.recordVersion == expectedRecordVersion,
            (status.revision == 0
                && !status.enabled
                && source == .default
                && status.updatedAtUnixMs == nil)
            || (status.revision > 0
                && status.updatedAtUnixMs.map { $0 >= 0 } == true
                && (source == .stored || (source == .default && !status.enabled)))
        else {
            throw PermanentCleanupPolicyServiceError.invalidResponse
        }
        return PermanentCleanupPolicy(
            enabled: status.enabled,
            source: source,
            revision: status.revision,
            updatedAtUnixMilliseconds: status.updatedAtUnixMs
        )
    }

    private static func permanentCleanupPolicyUpdate(
        _ response: PermanentCleanupPolicyUpdate
    ) throws -> PermanentCleanupPolicyUpdateResult {
        guard response.recordVersion == expectedRecordVersion else {
            throw PermanentCleanupPolicyServiceError.invalidResponse
        }
        return try PermanentCleanupPolicyUpdateResult(
            policy: permanentCleanupPolicy(response.policy),
            changed: response.changed
        )
    }

    private static func permanentCleanupPolicyError(
        _ error: PermanentCleanupPolicyError
    ) -> PermanentCleanupPolicyServiceError {
        switch error {
        case .Closed: .closed
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .OutcomeUnknown: .outcomeUnknown
        case .RevisionExhausted: .revisionExhausted
        case .InvalidClock: .invalidClock
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy, .BudgetExceeded: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .InternalState: .internalState
        }
    }

    private static func cleanupExclusions(
        _ status: CleanupExclusionsStatus
    ) throws -> CleanupExclusionsPolicy {
        let source: CleanupExclusionsOrigin = switch status.source {
        case .default: .default
        case .stored: .stored
        }
        let paths = try status.paths.map(cleanupExclusionPathObservation)
        guard
            status.recordVersion == expectedRecordVersion,
            paths.count <= 64,
            Set(paths).count == paths.count,
            paths == canonicalCleanupExclusionPaths(paths),
            (status.revision == 0
                && paths.isEmpty
                && source == .default
                && status.updatedAtUnixMs == nil)
            || (status.revision > 0
                && source == .stored
                && status.updatedAtUnixMs.map { $0 >= 0 } == true)
        else {
            throw CleanupExclusionsServiceError.invalidResponse
        }
        return CleanupExclusionsPolicy(
            paths: paths,
            source: source,
            revision: status.revision,
            updatedAtUnixMilliseconds: status.updatedAtUnixMs
        )
    }

    private static func cleanupExclusionsUpdate(
        _ response: CleanupExclusionsUpdate
    ) throws -> CleanupExclusionsUpdateResult {
        guard response.recordVersion == expectedRecordVersion else {
            throw CleanupExclusionsServiceError.invalidResponse
        }
        return try CleanupExclusionsUpdateResult(
            exclusions: cleanupExclusions(response.exclusions),
            changed: response.changed
        )
    }

    private static func cleanupExclusionPathObservation(
        _ path: CleanupExclusionPath
    ) throws -> CleanupExclusionPathObservation {
        guard
            path.encoding == .unixBytes,
            !path.encodedBytes.isEmpty,
            path.encodedBytes.count <= 32 * 1024,
            path.encodedBytes.first == UInt8(ascii: "/"),
            !path.encodedBytes.contains(0),
            hasNormalizedUnixPathComponents(path.encodedBytes)
        else {
            throw CleanupExclusionsServiceError.invalidResponse
        }
        return CleanupExclusionPathObservation(
            encoding: .unixBytes,
            encodedBytes: path.encodedBytes
        )
    }

    private static func cleanupExclusionPath(
        _ path: CleanupExclusionPathObservation
    ) -> CleanupExclusionPath {
        let encoding: SnapshotNameEncoding = switch path.encoding {
        case .unixBytes: .unixBytes
        case .windowsUTF16LittleEndian: .windowsUtf16LittleEndian
        }
        return CleanupExclusionPath(
            encoding: encoding,
            encodedBytes: path.encodedBytes
        )
    }

    private static func canonicalCleanupExclusionPaths(
        _ paths: [CleanupExclusionPathObservation]
    ) -> [CleanupExclusionPathObservation] {
        paths.sorted { left, right in
            let leftEncoding = left.encoding == .unixBytes ? UInt8(0) : UInt8(1)
            let rightEncoding = right.encoding == .unixBytes ? UInt8(0) : UInt8(1)
            if leftEncoding != rightEncoding {
                return leftEncoding < rightEncoding
            }
            return left.encodedBytes.lexicographicallyPrecedes(right.encodedBytes)
        }
    }

    private static func hasNormalizedUnixPathComponents(_ data: Data) -> Bool {
        let bytes = [UInt8](data)
        guard bytes.first == UInt8(ascii: "/") else {
            return false
        }
        if bytes.count == 1 {
            return true
        }
        guard bytes.last != UInt8(ascii: "/") else {
            return false
        }
        return bytes.dropFirst()
            .split(separator: UInt8(ascii: "/"), omittingEmptySubsequences: false)
            .allSatisfy { component in
                !component.isEmpty
                    && component != [UInt8(ascii: ".")]
                    && component != [UInt8(ascii: "."), UInt8(ascii: ".")]
            }
    }

    private static func cleanupExclusionsError(
        _ error: CleanupExclusionsError
    ) -> CleanupExclusionsServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidRecordVersion
        case .InvalidPath: .invalidPath
        case .TooManyPaths: .tooManyPaths
        case .RevisionExhausted: .revisionExhausted
        case .InvalidClock: .invalidClock
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .OutcomeUnknown: .outcomeUnknown
        case .InternalState: .internalState
        }
    }

    private static func projectDiscoveryRoots(
        _ status: ConfiguredProjectRootsStatus
    ) throws -> ProjectDiscoveryRoots {
        let source: ProjectDiscoveryRootsOrigin = switch status.source {
        case .default: .default
        case .stored: .stored
        }
        let roots = try status.roots.map(projectDiscoveryRoot)
        let hasOverlap = roots.indices.contains { index in
            roots.indices.dropFirst(index + 1).contains { otherIndex in
                roots[index].overlaps(roots[otherIndex])
            }
        }
        guard
            status.recordVersion == expectedRecordVersion,
            roots.count <= ProjectDiscoveryRoot.maximumCount,
            Set(roots).count == roots.count,
            roots == canonicalProjectDiscoveryRoots(roots),
            !hasOverlap,
            (status.revision == 0
                && roots.isEmpty
                && source == .default
                && status.updatedAtUnixMs == nil)
            || (status.revision > 0
                && source == .stored
                && status.updatedAtUnixMs.map { $0 >= 0 } == true)
        else {
            throw ProjectDiscoveryRootsServiceError.invalidResponse
        }
        return ProjectDiscoveryRoots(
            roots: roots,
            source: source,
            revision: status.revision,
            updatedAtUnixMilliseconds: status.updatedAtUnixMs
        )
    }

    private static func projectDiscoveryRootsUpdate(
        _ response: ConfiguredProjectRootsUpdate
    ) throws -> ProjectDiscoveryRootsUpdateResult {
        guard response.recordVersion == expectedRecordVersion else {
            throw ProjectDiscoveryRootsServiceError.invalidResponse
        }
        return try ProjectDiscoveryRootsUpdateResult(
            roots: projectDiscoveryRoots(response.roots),
            changed: response.changed
        )
    }

    private static func projectDiscoveryRoot(
        _ path: ConfiguredProjectRootPath
    ) throws -> ProjectDiscoveryRoot {
        let encoding: ProjectDiscoveryRoot.Encoding = switch path.encoding {
        case .unixBytes: .unixBytes
        case .windowsUtf16LittleEndian: .windowsUTF16LittleEndian
        }
        let root = ProjectDiscoveryRoot(
            encoding: encoding,
            encodedBytes: path.encodedBytes
        )
        guard
            path.encoding == .unixBytes,
            ProjectDiscoveryRoot.hasValidUnixShape(path.encodedBytes)
        else {
            throw ProjectDiscoveryRootsServiceError.invalidResponse
        }
        return root
    }

    private static func configuredProjectRootPath(
        _ root: ProjectDiscoveryRoot
    ) -> ConfiguredProjectRootPath {
        let encoding: SnapshotNameEncoding = switch root.encoding {
        case .unixBytes: .unixBytes
        case .windowsUTF16LittleEndian: .windowsUtf16LittleEndian
        }
        return ConfiguredProjectRootPath(
            encoding: encoding,
            encodedBytes: root.encodedBytes
        )
    }

    private static func canonicalProjectDiscoveryRoots(
        _ roots: [ProjectDiscoveryRoot]
    ) -> [ProjectDiscoveryRoot] {
        roots.sorted { left, right in
            let leftEncoding = left.encoding == .unixBytes ? UInt8(0) : UInt8(1)
            let rightEncoding = right.encoding == .unixBytes ? UInt8(0) : UInt8(1)
            if leftEncoding != rightEncoding {
                return leftEncoding < rightEncoding
            }
            return left.encodedBytes.lexicographicallyPrecedes(right.encodedBytes)
        }
    }

    private static func projectDiscoveryRootsError(
        _ error: ConfiguredProjectRootsError
    ) -> ProjectDiscoveryRootsServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidRecordVersion
        case .InvalidPath: .invalidPath
        case .TooManyPaths: .tooManyPaths
        case .OverlappingPaths: .overlappingPaths
        case .RevisionExhausted: .revisionExhausted
        case .InvalidClock: .invalidClock
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .OutcomeUnknown: .outcomeUnknown
        case .InternalState: .internalState
        }
    }

    private static func targetedVolumeID(
        _ stableVolumeID: String
    ) throws -> String {
        let prefix = "volume:macos:"
        guard
            stableVolumeID.hasPrefix(prefix),
            let uuid = macOSVolumeUUID(stableVolumeID)
        else {
            throw TargetedReclaimScanServiceError.invalidVolumeIdentity
        }
        let canonical = "\(prefix)\(uuid.uuidString.lowercased())"
        guard stableVolumeID == canonical else {
            throw TargetedReclaimScanServiceError.invalidVolumeIdentity
        }
        return canonical
    }

    private static func targetedTimestamp(_ date: Date) throws -> Int64 {
        let milliseconds = date.timeIntervalSince1970 * 1000
        guard
            milliseconds.isFinite,
            milliseconds >= 0,
            milliseconds <= Double(Int64.max)
        else {
            throw TargetedReclaimScanServiceError.invalidAnchor
        }
        return Int64(milliseconds.rounded(.towardZero))
    }

    private static func targetedReclaimScanAdmission(
        _ response: TargetedProjectScanAdmission,
        expectedStableVolumeID: String,
        expectedAnchorAtUnixMS: Int64,
        expectedOrdinal: UInt16,
        expectedRootsRevision: UInt64?,
        expectedRootCatalogDigestSHA256: Data?,
        state: EngineServiceState
    ) throws -> TargetedReclaimScanAdmission {
        let rootCatalog = try targetedReclaimRootCatalog(response.rootCatalog)
        guard
            response.recordVersion == expectedRecordVersion,
            response.rootCount <= maximumTargetedReclaimRootCount,
            response.rootCount == rootCatalog.rootCount,
            response.configuredRootsRevision == rootCatalog.configuredRootsRevision,
            expectedRootsRevision.map({ $0 == response.configuredRootsRevision }) ?? true,
            expectedRootCatalogDigestSHA256.map({ $0 == rootCatalog.digestSha256 }) ?? true
        else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }

        let selection = try response.selection.map {
            try targetedReclaimSelection(
                $0,
                expectedOrdinal: expectedOrdinal,
                catalog: rootCatalog
            )
        }
        let context = try response.pressure.map {
            try targetedReclaimContext(
                $0,
                expectedStableVolumeID: expectedStableVolumeID,
                expectedAnchorAtUnixMS: expectedAnchorAtUnixMS,
                catalog: rootCatalog
            )
        }
        let hasNoTaggedPayload =
            response.rootUnavailableReason == nil
                && response.currentResult == nil
                && response.task == nil
                && response.existingTaskObservedPhase == nil

        switch response.disposition {
        case .emptyRegistry:
            guard
                response.rootCount == 0,
                selection == nil,
                hasNoTaggedPayload
            else {
                throw TargetedReclaimScanServiceError.invalidResponse
            }
            return TargetedReclaimScanAdmission(
                context: context,
                ordinal: nil,
                root: nil,
                disposition: .noEligibleRoots
            )
        case .noPressure:
            guard
                response.rootCount > 0,
                selection != nil,
                context == nil,
                hasNoTaggedPayload
            else {
                throw TargetedReclaimScanServiceError.invalidResponse
            }
            return TargetedReclaimScanAdmission(
                context: nil,
                ordinal: nil,
                root: nil,
                disposition: .pressureNotActive
            )
        case .rootUnavailable:
            guard
                response.rootCount > 0,
                let selection,
                let context,
                let reason = response.rootUnavailableReason,
                response.currentResult == nil,
                response.task == nil,
                response.existingTaskObservedPhase == nil
            else {
                throw TargetedReclaimScanServiceError.invalidResponse
            }
            return TargetedReclaimScanAdmission(
                context: context,
                ordinal: selection.ordinal,
                root: selection,
                disposition: .unavailable(targetedRootFailure(reason))
            )
        case .current:
            guard
                response.rootCount > 0,
                let selection,
                let context,
                let rawResult = response.currentResult,
                response.rootUnavailableReason == nil,
                response.task == nil,
                response.existingTaskObservedPhase == nil
            else {
                throw TargetedReclaimScanServiceError.invalidResponse
            }
            let result: HomeScanTaskResult
            do {
                result = try FFIHomeScanTask.mapTargetedCurrentResult(
                    rawResult,
                    pressureEpisodeStartedAt: context.pressureEpisodeStartedAt
                )
            } catch {
                throw TargetedReclaimScanServiceError.invalidResponse
            }
            return TargetedReclaimScanAdmission(
                context: context,
                ordinal: selection.ordinal,
                root: selection,
                disposition: .current(result)
            )
        case .existingTask:
            guard
                response.rootCount > 0,
                let selection,
                let context,
                let task = response.task,
                let observedPhase = response.existingTaskObservedPhase,
                observedPhase == .queued || observedPhase == .running,
                response.rootUnavailableReason == nil,
                response.currentResult == nil
            else {
                throw TargetedReclaimScanServiceError.invalidResponse
            }
            return TargetedReclaimScanAdmission(
                context: context,
                ordinal: selection.ordinal,
                root: selection,
                disposition: .observing(
                    FFIHomeScanTask(
                        task: task,
                        state: state,
                        targetedPressureEpisodeStartedAt: context.pressureEpisodeStartedAt
                    )
                )
            )
        case .started:
            guard
                response.rootCount > 0,
                let selection,
                let context,
                let task = response.task,
                response.rootUnavailableReason == nil,
                response.currentResult == nil,
                response.existingTaskObservedPhase == nil
            else {
                throw TargetedReclaimScanServiceError.invalidResponse
            }
            return TargetedReclaimScanAdmission(
                context: context,
                ordinal: selection.ordinal,
                root: selection,
                disposition: .started(
                    FFIHomeScanTask(
                        task: task,
                        state: state,
                        targetedPressureEpisodeStartedAt: context.pressureEpisodeStartedAt
                    )
                )
            )
        }
    }

    private static func targetedReclaimRootCatalog(
        _ catalog: TargetedReclaimRootCatalog
    ) throws -> TargetedReclaimRootCatalog {
        let knownCount: UInt16 = catalog.knownUserLibraryCachesIncluded ? 1 : 0
        guard
            catalog.recordVersion == expectedRecordVersion,
            catalog.knownRootsPolicyRevision == expectedKnownRootsPolicyRevision,
            catalog.rootCount <= maximumTargetedReclaimRootCount,
            catalog.rootCount >= knownCount,
            catalog.rootCount - knownCount <= UInt16(ProjectDiscoveryRoot.maximumCount),
            catalog.digestSha256.count == 32
        else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        return catalog
    }

    private static func targetedReclaimRootCatalog(
        _ context: TargetedReclaimScanContext
    ) -> TargetedReclaimRootCatalog {
        TargetedReclaimRootCatalog(
            recordVersion: expectedRecordVersion,
            knownRootsPolicyRevision: context.knownRootsPolicyRevision,
            configuredRootsRevision: context.rootsRevision,
            knownUserLibraryCachesIncluded: context.knownUserLibraryCachesIncluded,
            rootCount: context.rootCount,
            digestSha256: context.rootCatalogDigestSHA256
        )
    }

    private static func targetedReclaimSelection(
        _ selection: TargetedProjectScanSelection,
        expectedOrdinal: UInt16,
        catalog: TargetedReclaimRootCatalog
    ) throws -> TargetedReclaimScanRoot {
        guard catalog.rootCount > 0 else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        let knownCount: UInt16 = catalog.knownUserLibraryCachesIncluded ? 1 : 0
        guard catalog.rootCount >= knownCount else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        let configuredCount = catalog.rootCount - knownCount
        let configuredReservation =
            UInt32(configuredCount) * minimumTargetedProjectScanNodes
        guard configuredReservation <= maximumTargetedProjectScanPassNodes else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        let knownMaxNodes: UInt32 = catalog.knownUserLibraryCachesIncluded
            ? min(
                maximumKnownUserCacheScanNodes,
                maximumTargetedProjectScanPassNodes - configuredReservation
            )
            : 0
        let configuredMaxNodes: UInt32 = configuredCount == 0
            ? 0
            : min(
                maximumTargetedProjectScanNodes,
                max(
                    1,
                    (maximumTargetedProjectScanPassNodes - knownMaxNodes)
                        / UInt32(configuredCount)
                )
            )
        let expectedKind: TargetedReclaimScanRootKind
        let expectedMaxNodes: UInt32
        if catalog.knownUserLibraryCachesIncluded, expectedOrdinal == 0 {
            expectedKind = .knownUserLibraryCaches
            expectedMaxNodes = knownMaxNodes
        } else {
            expectedKind = .configuredProject
            expectedMaxNodes = configuredMaxNodes
        }
        let kind: TargetedReclaimScanRootKind = switch selection.kind {
        case .knownUserLibraryCaches: .knownUserLibraryCaches
        case .configuredProject: .configuredProject
        }
        let root: ProjectDiscoveryRoot
        do {
            root = try projectDiscoveryRoot(selection.root)
        } catch {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        guard
            selection.recordVersion == expectedRecordVersion,
            selection.ordinal == expectedOrdinal,
            selection.ordinal < catalog.rootCount,
            kind == expectedKind,
            selection.maxNodes == expectedMaxNodes,
            UInt64(knownMaxNodes)
            + UInt64(configuredMaxNodes) * UInt64(configuredCount)
            <= UInt64(maximumTargetedProjectScanPassNodes)
        else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        return TargetedReclaimScanRoot(
            ordinal: selection.ordinal,
            kind: kind,
            path: root
        )
    }

    private static func targetedReclaimContext(
        _ pressure: TargetedProjectScanPressureContext,
        expectedStableVolumeID: String,
        expectedAnchorAtUnixMS: Int64,
        catalog: TargetedReclaimRootCatalog
    ) throws -> TargetedReclaimScanContext {
        guard
            pressure.recordVersion == expectedRecordVersion,
            pressure.stableVolumeId == expectedStableVolumeID,
            pressure.capacityAnchorUnixMs == expectedAnchorAtUnixMS,
            pressure.pressureStartedAtUnixMs >= 0,
            pressure.currentEpisodeStartedAtUnixMs >= pressure.pressureStartedAtUnixMs,
            pressure.capacityAnchorUnixMs >= pressure.currentEpisodeStartedAtUnixMs,
            catalog.rootCount <= maximumTargetedReclaimRootCount
        else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        let mappedPressure: TargetedReclaimPressure = switch pressure.pressure {
        case .warning: .warning
        case .critical: .critical
        }
        return TargetedReclaimScanContext(
            stableVolumeID: pressure.stableVolumeId,
            capacityAnchorAt: Date(
                timeIntervalSince1970: Double(pressure.capacityAnchorUnixMs) / 1000
            ),
            pressure: mappedPressure,
            pressureEpisodeStartedAt: Date(
                timeIntervalSince1970:
                Double(pressure.currentEpisodeStartedAtUnixMs) / 1000
            ),
            lowPressureSequenceStartedAt: Date(
                timeIntervalSince1970: Double(pressure.pressureStartedAtUnixMs) / 1000
            ),
            policyRevision: pressure.policyRevision,
            rootsRevision: catalog.configuredRootsRevision,
            knownRootsPolicyRevision: catalog.knownRootsPolicyRevision,
            knownUserLibraryCachesIncluded: catalog.knownUserLibraryCachesIncluded,
            rootCatalogDigestSHA256: catalog.digestSha256,
            rootCount: catalog.rootCount
        )
    }

    private static func targetedRootFailure(
        _ reason: TargetedProjectScanRootUnavailableReason
    ) -> TargetedReclaimRootFailure {
        switch reason {
        case .invalidPath, .notDirectory, .symlink:
            .invalidRoot
        case .missing:
            .rootMissing
        case .accessDenied:
            .accessDenied
        case .changedDuringValidation, .identityUnavailable:
            .rootChanged
        case .volumeMismatch:
            .differentVolume
        case .volumeUnproven:
            .volumeUnproven
        case .unsupportedPlatform, .unavailable:
            .storageUnavailable
        }
    }

    private static func targetedReclaimScanCheckpoint(
        _ response: TargetedProjectScanCheckpoint,
        expected proof: TargetedProjectScanCheckpointProof
    ) throws -> TargetedReclaimScanContext {
        let catalog = try targetedReclaimRootCatalog(response.rootCatalog)
        guard
            response.recordVersion == expectedRecordVersion,
            response.configuredRootsRevision == proof.context.rootsRevision,
            response.rootCount == proof.context.rootCount,
            response.rootCount > 0,
            response.rootCount <= maximumTargetedReclaimRootCount,
            catalog == targetedReclaimRootCatalog(proof.context),
            response.pressure == proof.pressure
        else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        let context = try targetedReclaimContext(
            response.pressure,
            expectedStableVolumeID: proof.context.stableVolumeID,
            expectedAnchorAtUnixMS: proof.pressure.capacityAnchorUnixMs,
            catalog: catalog
        )
        guard context == proof.context else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        return context
    }

    private static func emergencyRecoveryOrdering(
        _ response: EmergencyRecoveryOrdering,
        expected proof: TargetedProjectScanCheckpointProof
    ) throws -> AppEmergencyRecoveryOrdering {
        let catalog = try targetedReclaimRootCatalog(response.rootCatalog)
        guard
            response.recordVersion == expectedRecordVersion,
            response.policyRevision == AppEmergencyRecoveryOrdering.supportedPolicyRevision,
            response.pressure == proof.pressure,
            catalog == targetedReclaimRootCatalog(proof.context),
            response.groups.count <= AppEmergencyRecoveryOrdering.maximumGroupCount,
            UInt32(response.observedRootCount) + UInt32(response.unavailableRootCount)
            == UInt32(catalog.rootCount),
            response.candidateEvaluatedRootCount <= response.observedRootCount
        else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        let context = try targetedReclaimContext(
            response.pressure,
            expectedStableVolumeID: proof.context.stableVolumeID,
            expectedAnchorAtUnixMS: proof.pressure.capacityAnchorUnixMs,
            catalog: catalog
        )
        guard context == proof.context else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        let groups = try response.groups.enumerated().map { index, group in
            try emergencyRecoveryGroup(
                group,
                expectedRank: UInt16(index),
                context: context
            )
        }
        let ordering = AppEmergencyRecoveryOrdering(
            policyRevision: response.policyRevision,
            context: context,
            observedRootCount: response.observedRootCount,
            candidateEvaluatedRootCount: response.candidateEvaluatedRootCount,
            unavailableRootCount: response.unavailableRootCount,
            groups: groups
        )
        guard ordering.hasValidPresentationShape else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        return ordering
    }

    private static func emergencyRecoveryGroup(
        _ group: EmergencyRecoveryGroup,
        expectedRank: UInt16,
        context: TargetedReclaimScanContext
    ) throws -> AppEmergencyRecoveryGroup {
        let permitsUnavailableOnlyGroup =
            group.lane == .permissionGap && group.unavailableRootCount > 0
        guard
            group.recordVersion == expectedRecordVersion,
            group.rank == expectedRank,
            !group.sources.isEmpty || permitsUnavailableOnlyGroup,
            group.sources.count <= Int(context.rootCount)
        else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        let lane: AppEmergencyRecoveryLane = switch group.lane {
        case .evictableCloud: .evictableCloud
        case .staleSafeRegenerable: .staleSafeRegenerable
        case .trashInformation: .trashInformation
        case .reviewableInstallerArchive: .reviewableInstallerArchive
        case .largeFile: .largeFile
        case .guidedExploration: .guidedExploration
        case .permissionGap: .permissionGap
        }
        let category = group.category.map(emergencyRecoveryCategory)
        let sources = try group.sources.map {
            try emergencyRecoverySource($0, context: context)
        }
        return AppEmergencyRecoveryGroup(
            rank: group.rank,
            lane: lane,
            ruleID: group.ruleId,
            ruleRevision: group.ruleRevision,
            category: category,
            unavailableRootCount: group.unavailableRootCount,
            sources: sources
        )
    }

    private static func emergencyRecoverySource(
        _ source: EmergencyRecoverySource,
        context: TargetedReclaimScanContext
    ) throws -> AppEmergencyRecoverySource {
        let episodeStartedAtUnixMS = try targetedTimestamp(
            context.pressureEpisodeStartedAt
        )
        let latestPlausibleObservationUnixMS = try targetedTimestamp(
            Date().addingTimeInterval(5 * 60)
        )
        guard
            source.recordVersion == expectedRecordVersion,
            source.rootOrdinal < context.rootCount,
            source.scanId.hasPrefix("scan:targeted:"),
            !source.scanId.isEmpty,
            source.scanId.utf8.count <= 4096,
            !source.scanId.unicodeScalars.contains(where: {
                CharacterSet.controlCharacters.contains($0)
            }),
            source.observedAtUnixMs >= 0,
            source.observedAtUnixMs >= episodeStartedAtUnixMS,
            source.observedAtUnixMs <= latestPlausibleObservationUnixMS
        else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        return AppEmergencyRecoverySource(
            rootOrdinal: source.rootOrdinal,
            scanID: source.scanId,
            observedAt: Date(
                timeIntervalSince1970: Double(source.observedAtUnixMs) / 1000
            ),
            candidateCount: source.candidateCount,
            blockedCandidateCount: source.blockedCandidateCount,
            permissionIssueCount: source.permissionIssueCount
        )
    }

    private static func emergencyRecoveryCategory(
        _ category: CandidateCategory
    ) -> ExplorerCandidateCategory {
        switch category {
        case .developerArtifact: .developerArtifact
        case .applicationCache: .applicationCache
        case .browserCache: .browserCache
        case .logAndDiagnostic: .logAndDiagnostic
        case .installerAndDownload: .installerAndDownload
        case .deviceAndSimulatorData: .deviceAndSimulatorData
        case .cloudFile: .cloudFile
        case .largeReviewItem: .largeReviewItem
        case .protectedSystemData: .protectedSystemData
        case .unknownStorage: .unknownStorage
        }
    }

    private static func emergencyRecoveryError(
        _ error: EmergencyRecoveryError
    ) -> TargetedReclaimScanServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidRecordVersion
        case .InvalidPressureProof, .NotCritical, .PressureChanged:
            .pressureChanged
        case .InvalidCatalog:
            .invalidResponse
        case .RegistryChanged, .CatalogChanged:
            .configuredRootsChanged
        case .ReadOnlyStore:
            .readOnlyStore
        case .IncompatibleSchema:
            .incompatibleSchema
        case .Busy:
            .busy
        case .UnsafeStorage:
            .unsafeStorage
        case .BudgetExceeded:
            .budgetExceeded
        case .CorruptData:
            .corruptData
        case .Unavailable:
            .storageUnavailable
        case .OutcomeUnknown:
            .outcomeUnknown
        case .InternalState:
            .internalState
        }
    }

    private static func targetedReclaimScanError(
        _ error: TargetedProjectScanError
    ) -> TargetedReclaimScanServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidRecordVersion
        case .InvalidVolumeIdentity: .invalidVolumeIdentity
        case .InvalidAnchor: .invalidAnchor
        case .InvalidOrdinal: .invalidOrdinal
        case .InvalidCatalog: .invalidResponse
        case .RegistryChanged: .configuredRootsChanged
        case .CatalogChanged: .configuredRootsChanged
        case .PressureChanged: .pressureChanged
        case .ReadOnlyStore: .readOnlyStore
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .busy
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .storageUnavailable
        case .OutcomeUnknown: .outcomeUnknown
        case .QueueFull: .queueFull
        case .TaskIdExhausted, .InternalState: .internalState
        }
    }

    private static func targetedReclaimScanError(
        _ error: EngineServiceError
    ) -> TargetedReclaimScanServiceError {
        switch error {
        case .closed: .closed
        case .retryable: .busy
        case .unavailable: .storageUnavailable
        case .invalidCapacityObservation, .conflictingCapacityObservation,
             .supersededCapacityObservation, .unexpected:
            .internalState
        }
    }

    private static func directCargoEnrollmentPreview(
        _ info: DirectCargoEnrollmentPreviewInfo
    ) throws -> DirectCargoEnrollmentPreviewModel {
        guard info.recordVersion == expectedRecordVersion else {
            throw DirectCargoEnrollmentServiceError.invalidResponse
        }
        return try DirectCargoEnrollmentPreviewModel(
            executable: directCargoExecutable(info.executablePath),
            executableSHA256: directCargoSHA256(info.executableSha256),
            signature: directCargoSignature(info.codeSignature)
        )
    }

    private static func directCargoEnrollmentStatus(
        _ status: DirectCargoEnrollmentStatus
    ) throws -> DirectCargoEnrollmentStatusModel {
        guard status.recordVersion == expectedRecordVersion else {
            throw DirectCargoEnrollmentServiceError.invalidResponse
        }
        let disposition: DirectCargoEnrollmentDisposition
        switch status.state {
        case .notEnrolled:
            guard
                status.revision == 0,
                status.identity == nil,
                status.updatedAtUnixMs == nil
            else {
                throw DirectCargoEnrollmentServiceError.invalidResponse
            }
            disposition = .notEnrolled
        case .revoked:
            guard
                status.revision > 0,
                status.identity == nil,
                status.updatedAtUnixMs.map({ $0 >= 0 }) == true
            else {
                throw DirectCargoEnrollmentServiceError.invalidResponse
            }
            disposition = .revoked
        case .enrolled:
            guard
                status.revision > 0,
                let identity = status.identity,
                status.updatedAtUnixMs.map({ $0 >= 0 }) == true
            else {
                throw DirectCargoEnrollmentServiceError.invalidResponse
            }
            disposition = try .enrolled(directCargoIdentity(identity))
        }
        return DirectCargoEnrollmentStatusModel(
            revision: status.revision,
            disposition: disposition,
            updatedAtUnixMilliseconds: status.updatedAtUnixMs
        )
    }

    private static func directCargoEnrollmentUpdate(
        _ update: DirectCargoEnrollmentUpdate
    ) throws -> DirectCargoEnrollmentUpdateModel {
        guard update.recordVersion == expectedRecordVersion else {
            throw DirectCargoEnrollmentServiceError.invalidResponse
        }
        return try DirectCargoEnrollmentUpdateModel(
            status: directCargoEnrollmentStatus(update.status),
            changed: update.changed
        )
    }

    private static func directCargoIdentity(
        _ identity: DirectCargoEnrollmentIdentity
    ) throws -> DirectCargoEnrollmentIdentityModel {
        guard
            identity.recordVersion == expectedRecordVersion,
            identity.cargoMajor == 1,
            identity.cargoMinor == 96,
            identity.cargoPatch == 0
        else {
            throw DirectCargoEnrollmentServiceError.invalidResponse
        }
        return try DirectCargoEnrollmentIdentityModel(
            executable: directCargoExecutable(identity.executablePath),
            executableSHA256: directCargoSHA256(identity.executableSha256),
            versionSHA256: directCargoSHA256(identity.versionSha256),
            version: DirectCargoVersion(
                major: identity.cargoMajor,
                minor: identity.cargoMinor,
                patch: identity.cargoPatch
            ),
            signature: directCargoSignature(identity.codeSignature)
        )
    }

    private static func directCargoExecutable(
        _ executable: DirectCargoExecutablePath
    ) throws -> DirectCargoExecutableSelection {
        guard
            executable.encoding == .unixBytes,
            DirectCargoExecutableSelection.isValidUnixCargoPath(executable.encodedBytes)
        else {
            throw DirectCargoEnrollmentServiceError.invalidResponse
        }
        return DirectCargoExecutableSelection(encodedPathBytes: executable.encodedBytes)
    }

    private static func directCargoSHA256(_ bytes: Data) throws -> Data {
        guard bytes.count == 32 else {
            throw DirectCargoEnrollmentServiceError.invalidResponse
        }
        return bytes
    }

    private static func directCargoSignature(
        _ signature: DirectCargoCodeSignature
    ) throws -> DirectCargoSignatureEvidence {
        guard
            signature.recordVersion == expectedRecordVersion,
            (1 ... 16).contains(signature.codeDirectoryHashes.count),
            DirectCargoSignatureEvidence.hasSafeBoundedIdentifier(
                signature.signingIdentifier,
                maximumUTF8Bytes: 512
            ),
            signature.teamIdentifier.map({
                DirectCargoSignatureEvidence.hasSafeBoundedIdentifier(
                    $0,
                    maximumUTF8Bytes: 128
                )
            }) ?? true,
            signature.designatedRequirementSha256.map({ $0.count == 32 }) ?? true
        else {
            throw DirectCargoEnrollmentServiceError.invalidResponse
        }
        let hashes = try signature.codeDirectoryHashes.map { hash -> Data in
            guard
                hash.recordVersion == expectedRecordVersion,
                (20 ... 64).contains(hash.bytes.count)
            else {
                throw DirectCargoEnrollmentServiceError.invalidResponse
            }
            return hash.bytes
        }
        guard zip(hashes, hashes.dropFirst()).allSatisfy({
            $0.0.lexicographicallyPrecedes($0.1)
        }) else {
            throw DirectCargoEnrollmentServiceError.invalidResponse
        }
        let kind: DirectCargoSignatureKind
        switch signature.class {
        case .adHoc:
            guard signature.flags & 0x2 != 0, signature.teamIdentifier == nil else {
                throw DirectCargoEnrollmentServiceError.invalidResponse
            }
            kind = .adHoc
        case .cms:
            guard signature.flags & 0x2 == 0 else {
                throw DirectCargoEnrollmentServiceError.invalidResponse
            }
            kind = .cms
        }
        return DirectCargoSignatureEvidence(
            kind: kind,
            flags: signature.flags,
            codeDirectoryHashes: hashes,
            signingIdentifier: signature.signingIdentifier,
            teamIdentifier: signature.teamIdentifier,
            designatedRequirementSHA256: signature.designatedRequirementSha256
        )
    }

    private static func directCargoEnrollmentError(
        _ error: DirectCargoEnrollmentError
    ) -> DirectCargoEnrollmentServiceError {
        switch error {
        case .Closed: .closed
        case .UnsupportedPlatform: .unsupportedPlatform
        case .InvalidRecordVersion: .invalidRecordVersion
        case .InvalidExecutablePath: .invalidExecutablePath
        case .ExecutableNotRegular: .executableNotRegular
        case .ChangedDuringInspection: .changedDuringInspection
        case .InspectionUnavailable: .inspectionUnavailable
        case .InspectionLimitExceeded: .inspectionLimitExceeded
        case .InvalidResolutionEnvironment: .invalidResolutionEnvironment
        case .InvalidCargoVersion: .invalidCargoVersion
        case .InvalidCodeSignature: .invalidCodeSignature
        case .WrongEngine: .wrongEngine
        case .PreviewUnavailable: .previewUnavailable
        case .RevisionExhausted: .revisionExhausted
        case .InvalidClock: .invalidClock
        case .IncompatibleSchema: .incompatibleSchema
        case .Busy: .retryable
        case .UnsafeStorage: .unsafeStorage
        case .BudgetExceeded: .budgetExceeded
        case .CorruptData: .corruptData
        case .Unavailable: .unavailable
        case .OutcomeUnknown: .outcomeUnknown
        case .InternalState: .internalState
        }
    }

    fileprivate static func homeScanServiceError(_ error: ScanError) -> HomeScanServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidResponse
        case .ForeignReview: .invalidResponse
        case .ReviewExpired, .ReviewUnavailable, .SnapshotNodeNotFound,
             .SnapshotNodeNotDirectory:
            .rootUnavailable
        case .InvalidRoot: .invalidRoot
        case .RootMissing: .rootMissing
        case .RootAccessDenied: .rootAccessDenied
        case .RootNotDirectory: .rootNotDirectory
        case .RootSymlink: .rootSymlink
        case .RootChanged: .rootChanged
        case .RootIdentityUnavailable: .rootIdentityUnavailable
        case .UnsupportedPlatform: .unsupportedPlatform
        case .RootUnavailable: .rootUnavailable
        case .QueueFull: .queueFull
        case .Busy: .busy
        case .InputTooLarge: .inputTooLarge
        case .ReadOnlyStore: .readOnlyStore
        case .StorageUnavailable: .persistenceUnavailable
        case .RegistryUnavailable: .internalState
        case .TaskUnavailable: .taskExpired
        case .EventHistoryUnavailable: .outcomeUnknown
        case .WrongTaskKind: .wrongTaskKind
        case .InternalState: .internalState
        }
    }

    fileprivate static func homeScanServiceError(
        _ error: EngineServiceError
    ) -> HomeScanServiceError {
        switch error {
        case .closed: .closed
        case .retryable: .busy
        case .unavailable: .persistenceUnavailable
        case .invalidCapacityObservation, .conflictingCapacityObservation,
             .supersededCapacityObservation, .unexpected:
            .internalState
        }
    }

    private static func resolvePressureEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: DiskPressurePolicyServiceError.closed
            case .retryable: DiskPressurePolicyServiceError.retryable
            case .unavailable: DiskPressurePolicyServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                DiskPressurePolicyServiceError.invalidResponse
            }
        }
    }

    private static func resolveSnapshotRetentionCapEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: SnapshotRetentionCapServiceError.closed
            case .retryable: SnapshotRetentionCapServiceError.retryable
            case .unavailable: SnapshotRetentionCapServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                SnapshotRetentionCapServiceError.invalidResponse
            }
        }
    }

    private static func resolveOwnedStorageFootprintEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: DuxOwnedStorageFootprintServiceError.closed
            case .retryable: DuxOwnedStorageFootprintServiceError.retryable
            case .unavailable: DuxOwnedStorageFootprintServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                DuxOwnedStorageFootprintServiceError.invalidResponse
            }
        }
    }

    private static func resolveManagedScanCacheClearEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: DuxManagedScanCacheClearServiceError.closed
            case .retryable: DuxManagedScanCacheClearServiceError.retryable
            case .unavailable: DuxManagedScanCacheClearServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                DuxManagedScanCacheClearServiceError.invalidResponse
            }
        }
    }

    private static func resolvePermanentCleanupEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: PermanentCleanupPolicyServiceError.closed
            case .retryable: PermanentCleanupPolicyServiceError.retryable
            case .unavailable: PermanentCleanupPolicyServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                PermanentCleanupPolicyServiceError.invalidResponse
            }
        }
    }

    private static func resolveCleanupExclusionsEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: CleanupExclusionsServiceError.closed
            case .retryable: CleanupExclusionsServiceError.retryable
            case .unavailable: CleanupExclusionsServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                CleanupExclusionsServiceError.invalidResponse
            }
        }
    }

    private static func resolveProjectDiscoveryRootsEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: ProjectDiscoveryRootsServiceError.closed
            case .retryable: ProjectDiscoveryRootsServiceError.retryable
            case .unavailable: ProjectDiscoveryRootsServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                ProjectDiscoveryRootsServiceError.invalidResponse
            }
        }
    }

    private static func resolveDirectCargoEnrollmentEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: DirectCargoEnrollmentServiceError.closed
            case .retryable: DirectCargoEnrollmentServiceError.retryable
            case .unavailable: DirectCargoEnrollmentServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                DirectCargoEnrollmentServiceError.invalidResponse
            }
        }
    }

    private static func resolveCleanupHistoryEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: CleanupHistoryServiceError.closed
            case .retryable: CleanupHistoryServiceError.retryable
            case .unavailable: CleanupHistoryServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                CleanupHistoryServiceError.internalState
            }
        }
    }

    private static func resolvePersistentRecoveryDebtEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: PersistentRecoveryDebtServiceError.closed
            case .retryable: PersistentRecoveryDebtServiceError.retryable
            case .unavailable: PersistentRecoveryDebtServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                PersistentRecoveryDebtServiceError.internalState
            }
        }
    }

    private static func resolveClaimedRunningScanProvenanceEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: ClaimedRunningScanProvenanceServiceError.closed
            case .retryable: ClaimedRunningScanProvenanceServiceError.retryable
            case .unavailable: ClaimedRunningScanProvenanceServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                ClaimedRunningScanProvenanceServiceError.internalState
            }
        }
    }

    private static func resolveCleanupHistoryClearEngine(
        _ state: EngineServiceState
    ) throws -> DuxEngine {
        do {
            return try state.resolveEngine()
        } catch let error as EngineServiceError {
            throw switch error {
            case .closed: CleanupHistoryClearServiceError.closed
            case .retryable: CleanupHistoryClearServiceError.retryable
            case .unavailable: CleanupHistoryClearServiceError.unavailable
            case .invalidCapacityObservation, .conflictingCapacityObservation,
                 .supersededCapacityObservation, .unexpected:
                CleanupHistoryClearServiceError.internalState
            }
        }
    }

    private static func capacityBasis(_ source: VolumeCapacitySource) -> VolumeCapacityBasis {
        switch source {
        case .importantUsage: .importantUsage
        case .ordinary: .filesystemAvailable
        }
    }

    private static func pressure(_ pressure: VolumePressure) -> DiskPressureLevel {
        switch pressure {
        case .healthy: .healthy
        case .warning: .warning
        case .critical: .critical
        case .unknown: .unknown
        }
    }

    private static func capacityTrend(
        _ response: CapacityTrendStatus,
        expectedStableVolumeID: String,
        requestedAnchorAtUnixMS: Int64
    ) throws -> VolumeCapacityTrend {
        guard
            response.recordVersion == expectedRecordVersion,
            response.stableVolumeId == expectedStableVolumeID,
            response.sampledAtUnixMs >= 0,
            response.sampledAtUnixMs <= requestedAnchorAtUnixMS,
            response.totalBytes > 0,
            response.availableBytes <= response.totalBytes,
            response.importantAvailableBytes.map({ $0 <= response.totalBytes }) ?? true,
            response.points.count <= 31
        else {
            throw EngineServiceError.unexpected("invalid capacity trend record")
        }
        let points = try response.points.map { point in
            guard
                point.recordVersion == expectedRecordVersion,
                point.totalBytes > 0,
                point.availableBytes <= point.totalBytes,
                point.importantAvailableBytes.map({ $0 <= point.totalBytes }) ?? true
            else {
                throw EngineServiceError.unexpected("invalid capacity trend point")
            }
            return VolumeCapacityTrendPoint(
                sampledAt: Date(timeIntervalSince1970: Double(point.sampledAtUnixMs) / 1000),
                totalBytes: point.totalBytes,
                availableBytes: point.availableBytes,
                importantAvailableBytes: point.importantAvailableBytes,
                pressure: pressure(point.pressure),
                source: point.source == .raw ? .raw : .dailyRollup
            )
        }
        guard zip(points, points.dropFirst()).allSatisfy({ $0.0.sampledAt < $0.1.sampledAt }) else {
            throw EngineServiceError.unexpected("capacity trend points are not ordered")
        }
        return try VolumeCapacityTrend(
            stableVolumeID: response.stableVolumeId,
            sampledAt: Date(timeIntervalSince1970: Double(response.sampledAtUnixMs) / 1000),
            totalBytes: response.totalBytes,
            availableBytes: response.availableBytes,
            importantAvailableBytes: response.importantAvailableBytes,
            pressure: pressure(response.pressure),
            change24h: response.change24h.map(capacityTrendChange),
            change7d: response.change7d.map(capacityTrendChange),
            points: points
        )
    }

    private static func pressureEpisodeHistory(
        _ response: PressureEpisodeHistoryStatus,
        expectedStableVolumeID: String,
        expectedAnchorAtUnixMS: Int64,
        requestedLimit: UInt16
    ) throws -> VolumePressureHistory {
        let maximumCount = Int(requestedLimit)
        guard
            response.recordVersion == expectedRecordVersion,
            response.stableVolumeId == expectedStableVolumeID,
            response.anchorAtUnixMs == expectedAnchorAtUnixMS,
            response.anchorAtUnixMs >= 0,
            response.episodes.count <= maximumCount,
            !response.hasMore || response.episodes.count == maximumCount
        else {
            throw EngineServiceError.unexpected("invalid pressure episode history record")
        }
        let episodes = try response.episodes.enumerated().map { index, episode in
            guard
                episode.recordVersion == expectedRecordVersion,
                episode.enteredAtUnixMs >= 0,
                episode.enteredAtUnixMs <= response.anchorAtUnixMs,
                episode.exitedAtUnixMs.map({
                    $0 >= episode.enteredAtUnixMs && $0 <= response.anchorAtUnixMs
                }) ?? true,
                index == 0 || episode.exitedAtUnixMs != nil
            else {
                throw EngineServiceError.unexpected("invalid pressure episode record")
            }
            return VolumePressureEpisode(
                level: episode.level == .warning ? .warning : .critical,
                enteredAt: Date(
                    timeIntervalSince1970: Double(episode.enteredAtUnixMs) / 1000
                ),
                exitedAt: episode.exitedAtUnixMs.map {
                    Date(timeIntervalSince1970: Double($0) / 1000)
                },
                policyRevision: episode.policyRevision
            )
        }
        guard zip(episodes, episodes.dropFirst()).allSatisfy({
            let newer = $0.0
            let older = $0.1
            return newer.enteredAt > older.enteredAt
                && older.exitedAt.map { $0 <= newer.enteredAt } == true
        }) else {
            throw EngineServiceError.unexpected("pressure episode records are not ordered")
        }
        return VolumePressureHistory(
            stableVolumeID: response.stableVolumeId,
            anchorAt: Date(
                timeIntervalSince1970: Double(response.anchorAtUnixMs) / 1000
            ),
            episodes: episodes,
            hasMore: response.hasMore
        )
    }

    private static func macOSVolumeUUID(_ stableVolumeID: String) -> UUID? {
        let prefix = "volume:macos:"
        let uuidText: Substring
        if stableVolumeID.hasPrefix(prefix) {
            uuidText = stableVolumeID.dropFirst(prefix.count)
        } else {
            uuidText = stableVolumeID[...]
        }
        guard let uuid = UUID(uuidString: String(uuidText)) else {
            return nil
        }
        if stableVolumeID.hasPrefix(prefix),
           stableVolumeID != "\(prefix)\(uuid.uuidString.lowercased())"
        {
            return nil
        }
        return uuid
    }

    private static func capacityTrendChange(
        _ change: CapacityTrendChange
    ) throws -> VolumeCapacityTrendChange {
        guard
            change.recordVersion == expectedRecordVersion,
            change.fromUnixMs >= 0,
            change.toUnixMs >= change.fromUnixMs
        else {
            throw EngineServiceError.unexpected("invalid capacity trend change")
        }
        return VolumeCapacityTrendChange(
            from: Date(timeIntervalSince1970: Double(change.fromUnixMs) / 1000),
            to: Date(timeIntervalSince1970: Double(change.toUnixMs) / 1000),
            totalBytes: change.totalBytes,
            availableBytes: change.availableBytes,
            importantAvailableBytes: change.importantAvailableBytes
        )
    }

    private static func historyDisposition(
        _ disposition: VolumeHistoryDisposition
    ) -> VolumeCapacityHistoryDisposition {
        switch disposition {
        case .stored: .stored
        case .existingExact: .existingExact
        case .suppressedByHourlyCadence: .suppressedByHourlyCadence
        case .notStoredMissingOrdinaryAvailability:
            .notStoredMissingOrdinaryAvailability
        case .notStoredMissingStableIdentity:
            .notStoredMissingStableIdentity
        case .notStoredIncompleteMetadata:
            .notStoredIncompleteMetadata
        }
    }

    static func hasConsistentHistoryEvidence(
        _ status: StartupVolumeStatus,
        observation: VolumeCapacitySnapshot
    ) -> Bool {
        let expectedStableID = observation.stableVolumeID
            .flatMap { UUID(uuidString: $0) }
            .map { "volume:macos:\($0.uuidString.lowercased())" }
        guard status.stableVolumeId == expectedStableID else {
            return false
        }
        let hasCompleteMetadata = observation.displayName != nil
            && observation.filesystem != nil
            && observation.isInternal != nil
            && observation.isRemovable != nil

        switch status.historyDisposition {
        case .stored, .existingExact, .suppressedByHourlyCadence:
            return expectedStableID != nil
                && observation.filesystemAvailableBytes != nil
                && hasCompleteMetadata
        case .notStoredMissingOrdinaryAvailability:
            return expectedStableID != nil
                && observation.filesystemAvailableBytes == nil
        case .notStoredMissingStableIdentity:
            return expectedStableID == nil
        case .notStoredIncompleteMetadata:
            return expectedStableID != nil
                && observation.filesystemAvailableBytes != nil
                && !hasCompleteMetadata
        }
    }

    private static func failureDisposition(
        _ error: EngineError
    ) -> DuxMaintenanceFailureDisposition {
        switch error {
        case .Busy, .StorageUnavailable, .RegistryUnavailable, .BudgetExceeded:
            .retryable
        case .Closed, .InvalidStorage, .InvalidScanId, .InvalidCapacityObservation,
             .InvalidPressureEpisodeRequest,
             .ConflictingCapacityObservation, .SupersededCapacityObservation, .ScanNotFound,
             .SnapshotUnavailable, .ComparableSnapshotUnavailable, .WrongParentReview,
             .ReviewExpired, .SnapshotNodeNotFound,
             .SnapshotNodeNotDirectory, .InvalidSnapshotNodePage,
             .InvalidSnapshotTreemapBudget, .InvalidSnapshotLargeFileRequest,
             .InvalidSnapshotICloudObservationSourceRequest,
             .InvalidSnapshotLiveTargetRequest, .SnapshotLiveTargetUnsupported,
             .SnapshotLivePathUnavailable, .SnapshotLivePathMissing,
             .SnapshotLivePathSymlink, .SnapshotLivePathCrossVolume,
             .SnapshotLivePathChanged, .SnapshotLivePathAccessDenied,
             .InvalidScanCoverageDetailsRequest, .InvalidCandidateDetailRequest,
             .CandidateEvaluationNotSucceeded, .CandidateNotFound,
             .CandidateReviewNotReviewable,
             .CandidateCursorOutOfRange, .ReadOnlyStore,
             .IncompatibleSchema, .UnsafeStorage, .CorruptData,
             .IncompatibleSnapshot, .OutcomeUnknown, .InternalState:
            .blockedUntilRestart
        }
    }
}

private struct TargetedProjectScanCheckpointProof {
    let context: TargetedReclaimScanContext
    let pressure: TargetedProjectScanPressureContext
}

private final class EngineServiceState: @unchecked Sendable {
    private static let maximumTargetedProjectScanProofs = 8

    fileprivate let queue = DispatchQueue(label: "se.mjukis.dux.engine", qos: .utility)
    private let planReviewQueue = DispatchQueue(
        label: "se.mjukis.dux.plan-review",
        qos: .utility
    )

    private var engine: DuxEngine?
    private let storageRoots: EngineStorageRoots?
    fileprivate let homeScanRoot: URL?
    private var closeResult: Bool?
    private var targetedProjectScanProofs: [TargetedProjectScanCheckpointProof] = []

    init(engine: DuxEngine?, storageRoots: EngineStorageRoots?, homeScanRoot: URL?) {
        self.engine = engine
        self.storageRoots = storageRoots
        self.homeScanRoot = homeScanRoot
    }

    fileprivate func resolveEngine() throws -> DuxEngine {
        dispatchPrecondition(condition: .onQueue(queue))
        if closeResult != nil {
            throw EngineServiceError.closed
        }
        if let engine {
            return engine
        }

        do {
            guard libraryVersion().ffiContractVersion == EngineService.expectedFFIContractVersion else {
                throw EngineServiceError.unexpected("incompatible FFI contract")
            }
            let roots = try storageRoots ?? EngineService.defaultStorageRoots()
            let opened = try DuxEngine(storage: roots)
            engine = opened
            return opened
        } catch let error as EngineError {
            throw EngineService.serviceError(error)
        } catch let error as EngineServiceError {
            throw error
        } catch {
            throw EngineServiceError.unexpected("engine initialization failed")
        }
    }

    fileprivate func rememberTargetedProjectScanProof(
        context: TargetedReclaimScanContext,
        pressure: TargetedProjectScanPressureContext
    ) throws {
        dispatchPrecondition(condition: .onQueue(queue))
        // AppModel serializes pressure runners. Replacing an unconsumed proof
        // lets a later accepted capacity anchor recover after cancellation;
        // the final checkpoint still replays only the latest exact raw proof.
        targetedProjectScanProofs.removeAll { $0.context == context }
        targetedProjectScanProofs.append(
            TargetedProjectScanCheckpointProof(context: context, pressure: pressure)
        )
        if targetedProjectScanProofs.count > Self.maximumTargetedProjectScanProofs {
            targetedProjectScanProofs.removeFirst(
                targetedProjectScanProofs.count - Self.maximumTargetedProjectScanProofs
            )
        }
    }

    fileprivate func targetedProjectScanProof(
        for context: TargetedReclaimScanContext
    ) throws -> TargetedProjectScanCheckpointProof {
        dispatchPrecondition(condition: .onQueue(queue))
        let matches = targetedProjectScanProofs.filter { $0.context == context }
        guard matches.count == 1, let proof = matches.first else {
            throw TargetedReclaimScanServiceError.invalidResponse
        }
        return proof
    }

    fileprivate func consumeTargetedProjectScanProof(
        _ proof: TargetedProjectScanCheckpointProof
    ) {
        dispatchPrecondition(condition: .onQueue(queue))
        targetedProjectScanProofs.removeAll {
            $0.context == proof.context && $0.pressure == proof.pressure
        }
    }

    fileprivate func perform<T: Sendable>(
        _ operation: @escaping @Sendable (EngineServiceState) throws -> T
    ) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [self] in
                do {
                    try continuation.resume(returning: operation(self))
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    fileprivate func performNonthrowing<T: Sendable>(
        _ operation: @escaping @Sendable (EngineServiceState) -> T
    ) async -> T {
        await withCheckedContinuation { continuation in
            queue.async { [self] in
                continuation.resume(returning: operation(self))
            }
        }
    }

    fileprivate func performPlanReview<T: Sendable>(
        _ operation: @escaping @Sendable () throws -> T
    ) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            planReviewQueue.async {
                do {
                    try continuation.resume(returning: operation())
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    fileprivate func close() async -> Bool {
        await performNonthrowing { state in
            if let closeResult = state.closeResult {
                return closeResult
            }
            let result = state.engine?.close() ?? true
            state.closeResult = result
            return result
        }
    }
}

private final class FFIDirectCargoEnrollmentPreviewLease:
    DuxDirectCargoEnrollmentPreviewLease, @unchecked Sendable
{
    let preview: DirectCargoEnrollmentPreviewModel

    private let ffiPreview: DirectCargoEnrollmentPreviewSession
    private let state: EngineServiceState
    private var isAvailable = true

    init(
        preview: DirectCargoEnrollmentPreviewSession,
        summary: DirectCargoEnrollmentPreviewModel,
        state: EngineServiceState
    ) {
        ffiPreview = preview
        self.preview = summary
        self.state = state
    }

    fileprivate func take(
        for expectedState: EngineServiceState
    ) throws -> DirectCargoEnrollmentPreviewSession {
        dispatchPrecondition(condition: .onQueue(state.queue))
        guard state === expectedState, isAvailable else {
            throw DirectCargoEnrollmentServiceError.previewUnavailable
        }
        isAvailable = false
        return ffiPreview
    }

    func release() async {
        let ffiPreview = ffiPreview
        await state.performNonthrowing { [self] _ in
            guard isAvailable else {
                return
            }
            isAvailable = false
            _ = try? ffiPreview.release()
        }
    }
}

private final class FFICleanupHistoryClearPreviewLease:
    DuxCleanupHistoryClearPreviewLease, @unchecked Sendable
{
    let preview: CleanupHistoryClearPreviewModel

    private let ffiPreview: CleanupHistoryClearPreviewSession
    private let state: EngineServiceState
    private var isAvailable = true

    init(
        ffiPreview: CleanupHistoryClearPreviewSession,
        preview: CleanupHistoryClearPreviewModel,
        state: EngineServiceState
    ) {
        self.ffiPreview = ffiPreview
        self.preview = preview
        self.state = state
    }

    fileprivate func take(
        for expectedState: EngineServiceState
    ) throws -> CleanupHistoryClearPreviewSession {
        // Identity must be checked before asserting the owner queue. A lease
        // handed to another EngineService is a typed rejection, not a crash.
        guard state === expectedState else {
            throw CleanupHistoryClearServiceError.wrongEngine
        }
        dispatchPrecondition(condition: .onQueue(state.queue))
        guard isAvailable else {
            throw CleanupHistoryClearServiceError.previewUnavailable
        }
        isAvailable = false
        return ffiPreview
    }

    func release() async {
        let ffiPreview = ffiPreview
        await state.performNonthrowing { [self] _ in
            guard isAvailable else {
                return
            }
            isAvailable = false
            _ = try? ffiPreview.release()
        }
    }
}

private final class FFIManagedScanCacheClearPreviewLease:
    DuxManagedScanCacheClearPreviewLease, @unchecked Sendable
{
    let preview: DuxManagedScanCacheClearPreviewModel

    private let ffiPreview: ManagedScanCacheClearPreviewSession
    private let state: EngineServiceState
    private var isAvailable = true

    init(
        ffiPreview: ManagedScanCacheClearPreviewSession,
        preview: DuxManagedScanCacheClearPreviewModel,
        state: EngineServiceState
    ) {
        self.ffiPreview = ffiPreview
        self.preview = preview
        self.state = state
    }

    fileprivate func take(
        for expectedState: EngineServiceState
    ) throws -> ManagedScanCacheClearPreviewSession {
        guard state === expectedState else {
            throw DuxManagedScanCacheClearServiceError.wrongEngine
        }
        dispatchPrecondition(condition: .onQueue(state.queue))
        guard isAvailable else {
            throw DuxManagedScanCacheClearServiceError.previewUnavailable
        }
        isAvailable = false
        return ffiPreview
    }

    func release() async {
        let ffiPreview = ffiPreview
        await state.performNonthrowing { [self] _ in
            guard isAvailable else {
                return
            }
            isAvailable = false
            _ = try? ffiPreview.release()
        }
    }
}

private enum HomeScanResponseViolation: String, Error {
    case envelope
    case progressRecord
    case resultRecord
    case terminalStatus
    case phaseShape
    case terminalAggregates
    case transition
    case events
}

private final class FFIHomeScanTask: HomeScanTask, @unchecked Sendable {
    private static let logger = Logger(
        subsystem: Bundle.main.bundleIdentifier ?? "se.mjukis.dux",
        category: "scan-contract"
    )
    private let task: ScanTask
    private let state: EngineServiceState
    private let targetedPressureEpisodeStartedAt: Date?
    private var lastPoll: HomeScanTaskPoll?

    init(
        task: ScanTask,
        state: EngineServiceState,
        targetedPressureEpisodeStartedAt: Date? = nil
    ) {
        self.task = task
        self.state = state
        self.targetedPressureEpisodeStartedAt = targetedPressureEpisodeStartedAt
    }

    func poll() async throws -> HomeScanTaskPoll {
        try await state.perform { _ in
            do {
                let poll = try Self.map(self.task.poll(), after: self.lastPoll)
                if let pressureEpisodeStartedAt = self.targetedPressureEpisodeStartedAt,
                   let result = poll.result,
                   !Self.validTargetedResult(
                       result,
                       pressureEpisodeStartedAt: pressureEpisodeStartedAt
                   )
                {
                    throw HomeScanResponseViolation.resultRecord
                }
                self.lastPoll = poll
                return poll
            } catch let violation as HomeScanResponseViolation {
                Self.logger.error(
                    "Rejected path-free scan poll: \(violation.rawValue, privacy: .public)"
                )
                throw HomeScanServiceError.invalidResponse
            } catch let error as ScanError {
                Self.logger.error(
                    "Scan FFI poll failed: \(Self.diagnosticCode(error), privacy: .public)"
                )
                throw EngineService.homeScanServiceError(error)
            }
        }
    }

    fileprivate static func mapTargetedCurrentResult(
        _ raw: ScanTaskResult,
        pressureEpisodeStartedAt: Date
    ) throws -> HomeScanTaskResult {
        let result = try mapResult(raw)
        guard
            result.succeeded,
            validTargetedResult(
                result,
                pressureEpisodeStartedAt: pressureEpisodeStartedAt
            )
        else {
            throw HomeScanResponseViolation.resultRecord
        }
        return result
    }

    func requestCancellation() async throws -> HomeScanCancelOutcome {
        try await state.perform { _ in
            do {
                return switch try self.task.cancel() {
                case .cancelledBeforeStart: .cancelledBeforeStart
                case .requested: .requested
                case .alreadyRequested: .alreadyRequested
                case .alreadyTerminal: .alreadyTerminal
                }
            } catch let error as ScanError {
                throw EngineService.homeScanServiceError(error)
            }
        }
    }

    private static func diagnosticCode(_ error: ScanError) -> String {
        switch error {
        case .InternalState: "internal-state"
        case .RegistryUnavailable: "registry-unavailable"
        case .TaskUnavailable: "task-unavailable"
        case .EventHistoryUnavailable: "event-history-unavailable"
        case .WrongTaskKind: "wrong-task-kind"
        case .InvalidRecordVersion: "invalid-record-version"
        default: "ffi-error"
        }
    }

    private static func map(
        _ raw: ScanPoll,
        after previous: HomeScanTaskPoll?
    ) throws -> HomeScanTaskPoll {
        guard raw.recordVersion == EngineService.expectedRecordVersion, raw.revision > 0 else {
            throw HomeScanResponseViolation.envelope
        }
        let progress = try raw.progress.map(mapProgress)
        let result = try raw.result.map(mapResult)
        let failure = raw.failure.map(mapFailure)
        let events = try raw.events.map(mapEvent)
        let phase: HomeScanTaskPhase = switch raw.phase {
        case .queued: .queued
        case .running: .running
        case .succeeded: .succeeded
        case .failed: .failed
        case .cancelled: .cancelled
        }
        let stage: HomeScanTaskStage = switch raw.stage {
        case .queued: .queued
        case .scanning: .scanning
        case .finalizing: .finalizing
        case .evaluating: .evaluating
        case .terminal: .terminal
        }

        guard validShape(
            phase: raw.phase,
            stage: raw.stage,
            failure: raw.failure,
            result: raw.result
        ) else {
            throw HomeScanResponseViolation.phaseShape
        }

        let mapped = HomeScanTaskPoll(
            phase: phase,
            stage: stage,
            cancellationRequested: raw.cancellationRequested,
            revision: raw.revision,
            progress: progress,
            eventsTruncated: raw.eventsTruncated,
            failure: failure,
            result: result,
            events: events,
            eventCursor: raw.nextEventSequence,
            oldestAvailableEventSequence: raw.oldestAvailableEventSequence
        )
        guard validTerminalResult(progress: mapped.progress, result: mapped.result) else {
            throw HomeScanResponseViolation.terminalAggregates
        }
        if let previous {
            guard
                mapped.revision >= previous.revision,
                !previous.cancellationRequested || mapped.cancellationRequested,
                !previous.eventsTruncated || mapped.eventsTruncated,
                validEventTransition(from: previous, to: mapped),
                validPhaseTransition(from: previous.phase, to: mapped.phase),
                validStageTransition(from: previous.stage, to: mapped.stage),
                validProgressTransition(from: previous.progress, to: mapped.progress),
                validTerminalResult(progress: previous.progress, result: mapped.result)
            else {
                throw HomeScanResponseViolation.transition
            }
        }
        return mapped
    }

    private static func mapEvent(_ raw: ScanEvent) throws -> HomeScanEvent {
        guard raw.recordVersion == EngineService.expectedRecordVersion, raw.sequence > 0 else {
            throw HomeScanResponseViolation.events
        }
        let kind: HomeScanEventKind = switch raw.kind {
        case .queued: .queued
        case .started: .started
        case let .progress(completed, total):
            try mapProgressEvent(completed: completed, total: total)
        case let .scanProgress(filesScanned, directoriesScanned, knownAllocatedBytes, errorCount):
            .scanning(
                files: filesScanned,
                directories: directoriesScanned,
                knownAllocatedBytes: knownAllocatedBytes,
                errors: errorCount
            )
        case .scanFinalizing: .finalizing
        case .candidateEvaluationStarted: .candidateEvaluationStarted
        case let .candidateEvaluationFinished(status, candidateCount, failure):
            try mapCandidateEvaluationEvent(
                status: status,
                candidateCount: candidateCount,
                failure: failure
            )
        case .cancellationRequested: .cancellationRequested
        case let .terminal(phase):
            .terminal(mapTaskPhase(phase))
        case .maintenance: .maintenance
        }
        return HomeScanEvent(sequence: raw.sequence, kind: kind)
    }

    private static func mapProgressEvent(
        completed: UInt64,
        total: UInt64
    ) throws -> HomeScanEventKind {
        guard completed <= total else { throw HomeScanResponseViolation.events }
        return .progress(completed: completed, total: total)
    }

    private static func mapCandidateEvaluationEvent(
        status: ScanCandidateEvaluationStatus,
        candidateCount: UInt32,
        failure: ScanCandidateEvaluationFailure?
    ) throws -> HomeScanEventKind {
        let mappedStatus: HomeScanCandidateEvaluation = switch status {
        case .notRun: .notRun
        case .succeeded: .succeeded(candidateCount: candidateCount)
        case .failed: .failed
        }
        guard candidateCount <= 4096 else { throw HomeScanResponseViolation.events }
        if mappedStatus == .notRun || mappedStatus == .succeeded(candidateCount: candidateCount) {
            guard failure == nil else { throw HomeScanResponseViolation.events }
        } else {
            guard failure != nil else { throw HomeScanResponseViolation.events }
        }
        return .candidateEvaluationFinished(
            status: mappedStatus,
            candidateCount: candidateCount,
            failure: failure.map(mapCandidateEvaluationFailure)
        )
    }

    private static func mapTaskPhase(_ phase: TaskPhase) -> HomeScanTaskPhase {
        switch phase {
        case .queued: .queued
        case .running: .running
        case .succeeded: .succeeded
        case .failed: .failed
        case .cancelled: .cancelled
        }
    }

    private static func mapCandidateEvaluationFailure(
        _ failure: ScanCandidateEvaluationFailure
    ) -> HomeScanCandidateEvaluationFailure {
        switch failure {
        case .cancelled: .cancelled
        case .catalogInvalid: .catalogInvalid
        case .contextInvalid: .contextInvalid
        case .evaluationFailed: .evaluationFailed
        case .candidateInvalid: .candidateInvalid
        case .limitExceeded: .limitExceeded
        case .internalState: .internalState
        }
    }

    private static func validEventTransition(
        from previous: HomeScanTaskPoll,
        to current: HomeScanTaskPoll
    ) -> Bool {
        guard
            current.eventCursor >= previous.eventCursor,
            current.oldestAvailableEventSequence >= previous.oldestAvailableEventSequence,
            !previous.eventsTruncated || current.eventsTruncated,
            current.oldestAvailableEventSequence <= current.eventCursor || current.eventCursor == 0
        else { return false }

        var last = previous.eventCursor
        for event in current.events {
            guard event.sequence > last, event.sequence <= current.eventCursor else { return false }
            if case let .terminal(eventPhase) = event.kind {
                guard eventPhase == current.phase || !current.phase.isTerminal else { return false }
            }
            last = event.sequence
        }
        if current.events.isEmpty {
            guard current.eventCursor == previous.eventCursor else { return false }
        } else if last != current.eventCursor {
            return false
        }
        if !current.eventsTruncated, current.oldestAvailableEventSequence > previous.eventCursor + 1 {
            return false
        }
        return true
    }

    private static func mapProgress(_ raw: ScanProgress) throws -> ScanProgressFacts {
        guard raw.recordVersion == EngineService.expectedRecordVersion else {
            throw HomeScanResponseViolation.progressRecord
        }
        return ScanProgressFacts(
            files: raw.filesScanned,
            directories: raw.directoriesScanned,
            knownAllocatedBytes: raw.knownAllocatedBytes,
            issueCount: raw.errorCount
        )
    }

    private static func mapResult(_ raw: ScanTaskResult) throws -> HomeScanTaskResult {
        guard
            raw.recordVersion == EngineService.expectedRecordVersion,
            validStableID(raw.scanId),
            raw.startedAtUnixMs >= 0,
            raw.completedAtUnixMs >= raw.startedAtUnixMs,
            validCoverage(raw.coverage),
            validCandidateEvaluation(raw.candidateEvaluation)
        else {
            throw HomeScanResponseViolation.resultRecord
        }

        let succeeded = raw.status == .succeeded
        switch raw.status {
        case .succeeded:
            guard
                raw.snapshotAvailable,
                raw.coverage.status != .unknown,
                raw.candidateEvaluation.status != .notRun
            else {
                throw HomeScanResponseViolation.terminalStatus
            }
        case .failed, .cancelled, .interrupted:
            guard
                !raw.snapshotAvailable,
                raw.directoryCount == 0,
                raw.fileCount == 0,
                raw.logicalBytes == 0,
                raw.allocatedBytes == nil,
                raw.candidateEvaluation.status == .notRun
            else {
                throw HomeScanResponseViolation.terminalStatus
            }
        }

        let candidate: HomeScanCandidateEvaluation = switch raw.candidateEvaluation.status {
        case .notRun: .notRun
        case .succeeded:
            .succeeded(candidateCount: raw.candidateEvaluation.candidateCount)
        case .failed: .failed
        }
        return HomeScanTaskResult(
            scanID: raw.scanId,
            startedAt: Date(timeIntervalSince1970: Double(raw.startedAtUnixMs) / 1000),
            completedAt: Date(timeIntervalSince1970: Double(raw.completedAtUnixMs) / 1000),
            succeeded: succeeded,
            directoryCount: raw.directoryCount,
            fileCount: raw.fileCount,
            logicalBytes: raw.logicalBytes,
            allocatedBytes: raw.allocatedBytes,
            coverage: mapCoverage(raw.coverage.status),
            coveragePermille: raw.coverage.measuredPermille,
            issueCount: raw.coverage.issueOccurrenceCount,
            snapshotAvailable: raw.snapshotAvailable,
            candidateEvaluation: candidate
        )
    }

    private static func mapFailure(_ failure: ScanTaskFailure) -> HomeScanTaskFailure {
        switch failure {
        case .rootChanged: .rootChanged
        case .scanFailed: .scanFailed
        case .snapshotRejected: .snapshotRejected
        case .persistenceUnavailable: .persistenceUnavailable
        case .persistenceOutcomeUnknown: .persistenceOutcomeUnknown
        case .internalState: .internalFailure
        }
    }

    private static func mapCoverage(_ status: ScanCoverageStatus) -> AppScanCoverage {
        switch status {
        case .unknown: .unknown
        case .complete: .complete
        case .limitedAccess: .limitedAccess
        case .partial: .partial
        }
    }

    private static func validShape(
        phase: TaskPhase,
        stage: ScanStage,
        failure: ScanTaskFailure?,
        result: ScanTaskResult?
    ) -> Bool {
        switch phase {
        case .queued:
            return stage == .queued && failure == nil && result == nil
        case .running:
            return matchesActiveStage(stage) && failure == nil && result == nil
        case .succeeded:
            return stage == .terminal && failure == nil && result?.status == .succeeded
        case .failed:
            return stage == .terminal
                && failure != nil
                && (result.map { $0.status == .failed || $0.status == .interrupted } ?? true)
        case .cancelled:
            return stage == .terminal
                && failure == nil
                && (result.map { $0.status == .cancelled } ?? true)
        }
    }

    private static func matchesActiveStage(_ stage: ScanStage) -> Bool {
        switch stage {
        case .scanning, .finalizing, .evaluating: true
        case .queued, .terminal: false
        }
    }

    private static func validStableID(_ value: String) -> Bool {
        !value.isEmpty
            && value.utf8.count <= 128
            && value.unicodeScalars.allSatisfy { scalar in
                scalar.isASCII
                    && (CharacterSet.alphanumerics.contains(scalar)
                        || "._-:".unicodeScalars.contains(scalar))
            }
    }

    private static func validTargetedResult(
        _ result: HomeScanTaskResult,
        pressureEpisodeStartedAt: Date
    ) -> Bool {
        result.scanID.hasPrefix("scan:targeted:")
            && result.startedAt >= pressureEpisodeStartedAt
    }

    private static func validCoverage(_ coverage: ScanCoverageSummary) -> Bool {
        guard
            coverage.recordVersion == EngineService.expectedRecordVersion,
            coverage.issueRecordCount <= 256,
            coverage.issueOccurrenceCount >= coverage.issueRecordCount,
            coverage.measuredPermille.map({ $0 <= 1000 }) ?? true
        else {
            return false
        }
        switch coverage.status {
        case .unknown:
            return coverage.measuredPermille == nil
                && coverage.issueRecordCount == 0
                && coverage.issueOccurrenceCount == 0
        case .complete:
            return coverage.measuredPermille == 1000
                && coverage.issueRecordCount == 0
                && coverage.issueOccurrenceCount == 0
        case .limitedAccess, .partial:
            return coverage.issueRecordCount > 0 && coverage.measuredPermille != 1000
        }
    }

    private static func validCandidateEvaluation(
        _ evaluation: ScanCandidateEvaluationSummary
    ) -> Bool {
        guard
            evaluation.recordVersion == EngineService.expectedRecordVersion,
            evaluation.candidateCount <= 4096
        else {
            return false
        }
        switch evaluation.status {
        case .notRun:
            return evaluation.candidateCount == 0 && evaluation.failure == nil
        case .succeeded:
            return evaluation.failure == nil
        case .failed:
            return evaluation.candidateCount == 0 && evaluation.failure != nil
        }
    }

    private static func validPhaseTransition(
        from previous: HomeScanTaskPhase,
        to current: HomeScanTaskPhase
    ) -> Bool {
        switch previous {
        case .queued:
            return true
        case .running:
            return current != .queued
        case .succeeded, .failed, .cancelled:
            return current == previous
        }
    }

    private static func validStageTransition(
        from previous: HomeScanTaskStage,
        to current: HomeScanTaskStage
    ) -> Bool {
        func rank(_ stage: HomeScanTaskStage) -> Int {
            switch stage {
            case .queued: 0
            case .scanning: 1
            case .finalizing: 2
            case .evaluating: 3
            case .terminal: 4
            }
        }
        return rank(current) >= rank(previous)
    }

    private static func validProgressTransition(
        from previous: ScanProgressFacts?,
        to current: ScanProgressFacts?
    ) -> Bool {
        guard let previous else {
            return true
        }
        guard let current else {
            return false
        }
        return current.files >= previous.files
            && current.directories >= previous.directories
            && current.issueCount >= previous.issueCount
    }

    private static func validTerminalResult(
        progress: ScanProgressFacts?,
        result: HomeScanTaskResult?
    ) -> Bool {
        guard let progress, let result, result.succeeded else {
            return true
        }
        return result.fileCount >= progress.files
            && result.directoryCount >= progress.directories
            && result.issueCount >= progress.issueCount
    }
}

private final class FFIDuxMaintenanceTask: DuxMaintenanceTask, @unchecked Sendable {
    private let task: MaintenanceTask
    private let state: EngineServiceState
    private let expectedKind: MaintenanceKind

    init(
        task: MaintenanceTask,
        state: EngineServiceState,
        expectedKind: MaintenanceKind
    ) {
        self.task = task
        self.state = state
        self.expectedKind = expectedKind
    }

    func poll() async -> DuxMaintenanceTaskPoll {
        await state.performNonthrowing { _ in
            do {
                return try Self.map(
                    self.task.poll(),
                    expectedKind: self.expectedKind
                )
            } catch let error as EngineError {
                return .failed(Self.failureDisposition(error))
            } catch {
                return .failed(.blockedUntilRestart)
            }
        }
    }

    func requestCancellation() async {
        await state.performNonthrowing { _ in
            _ = try? self.task.cancel()
        }
    }

    private static func map(
        _ poll: MaintenancePoll,
        expectedKind: MaintenanceKind
    ) -> DuxMaintenanceTaskPoll {
        guard
            poll.recordVersion == EngineService.expectedRecordVersion,
            poll.kind == expectedKind
        else {
            return .failed(.blockedUntilRestart)
        }
        switch poll.phase {
        case .queued, .running:
            guard poll.failure == nil, poll.result == nil else {
                return .failed(.blockedUntilRestart)
            }
            return .running
        case .cancelled:
            guard poll.failure == nil, poll.result == nil else {
                return .failed(.blockedUntilRestart)
            }
            return .cancelled
        case .failed:
            guard let failure = poll.failure, poll.result == nil else {
                return .failed(.blockedUntilRestart)
            }
            return .failed(failureDisposition(failure))
        case .succeeded:
            guard
                poll.failure == nil,
                let result = poll.result,
                valid(result, expectedKind: expectedKind)
            else {
                return .failed(.blockedUntilRestart)
            }
            return .finished(
                DuxMaintenanceBatchSignal(
                    hasMore: result.hasMore,
                    deferral: deferral(result.outcome)
                )
            )
        }
    }

    private static func valid(
        _ result: MaintenanceResult,
        expectedKind: MaintenanceKind
    ) -> Bool {
        guard
            result.recordVersion == EngineService.expectedRecordVersion,
            result.kind == expectedKind,
            result.observedAtUnixMs >= 0
        else {
            return false
        }

        switch expectedKind {
        case .scanRecovery:
            guard
                result.secondaryCountAfter == 0,
                result.tertiaryCountAfter == 0,
                result.quaternaryCountAfter == 0,
                zeroByteFields(result)
            else {
                return false
            }
            switch result.outcome {
            case .scanRecoveryNone, .scanRecoveryDeferredUnproven,
                 .scanRecoveryInterrupted, .scanRecoveryChangedConcurrently:
                return true
            default:
                return false
            }
        case .candidateEvaluationRecovery:
            guard
                result.primaryCountBefore == 0,
                result.secondaryCountBefore == 0,
                result.secondaryCountAfter == 0,
                result.tertiaryCountBefore == 0,
                result.tertiaryCountAfter == 0,
                result.quaternaryCountBefore == 0,
                result.quaternaryCountAfter == 0,
                zeroByteFields(result)
            else {
                return false
            }
            switch result.outcome {
            case .candidateEvaluationRecoveryNone:
                return result.primaryCountAfter == 0 && !result.hasMore
            case .candidateEvaluationRecoveryRecovered:
                return result.primaryCountAfter <= 4096
            case .candidateEvaluationRecoveryIncompatible:
                return result.primaryCountAfter == 0
            default:
                return false
            }
        case .history:
            guard
                result.primaryCountBefore == 0,
                result.secondaryCountBefore == 0,
                result.tertiaryCountBefore == 0,
                result.quaternaryCountBefore == 0,
                zeroByteFields(result)
            else {
                return false
            }
            return result.outcome == .historyApplied
        case .snapshotRetention:
            guard zeroCountFields(result) else {
                return false
            }
            switch result.outcome {
            case .retentionUnderCap, .retentionDeferredUnstable,
                 .retentionDeferredNoEligibleSnapshot,
                 .retentionRemovedTombstonedResidual,
                 .retentionTombstonedAndRemoved:
                return true
            default:
                return false
            }
        case .snapshotOrphan:
            guard
                result.secondaryCountBefore == 0,
                result.secondaryCountAfter == 0,
                result.tertiaryCountBefore == 0,
                result.tertiaryCountAfter == 0,
                result.quaternaryCountBefore == 0,
                result.quaternaryCountAfter == 0,
                result.capBytes == 0
            else {
                return false
            }
            switch result.outcome {
            case .orphanNone, .orphanRemoved:
                return true
            default:
                return false
            }
        case .snapshotProvisioningStage:
            guard
                result.quaternaryCountBefore == 0,
                result.quaternaryCountAfter == 0,
                result.capBytes == 0
            else {
                return false
            }
            switch result.outcome {
            case .stageNone, .stageDeferredUnproven,
                 .stageRemovedMarkerOnly, .stageRemovedMarkerComplete:
                return true
            default:
                return false
            }
        case .snapshotTerminalTemp:
            guard
                result.tertiaryCountBefore == 0,
                result.tertiaryCountAfter == 0,
                result.quaternaryCountBefore == 0,
                result.quaternaryCountAfter == 0,
                result.capBytes == 0
            else {
                return false
            }
            switch result.outcome {
            case .terminalTempNone, .terminalTempDeferredActive,
                 .terminalTempReconciledRowOnly, .terminalTempRemoved:
                return true
            default:
                return false
            }
        case .snapshotUnleasedTemp:
            guard
                result.tertiaryCountBefore == 0,
                result.tertiaryCountAfter == 0,
                result.quaternaryCountBefore == 0,
                result.quaternaryCountAfter == 0,
                result.capBytes == 0
            else {
                return false
            }
            switch result.outcome {
            case .unleasedTempNone, .unleasedTempDeferredActive,
                 .unleasedTempRemoved:
                return true
            default:
                return false
            }
        }
    }

    private static func zeroCountFields(_ result: MaintenanceResult) -> Bool {
        result.primaryCountBefore == 0
            && result.primaryCountAfter == 0
            && result.secondaryCountBefore == 0
            && result.secondaryCountAfter == 0
            && result.tertiaryCountBefore == 0
            && result.tertiaryCountAfter == 0
            && result.quaternaryCountBefore == 0
            && result.quaternaryCountAfter == 0
    }

    private static func zeroByteFields(_ result: MaintenanceResult) -> Bool {
        result.chargedBytesBefore == 0
            && result.chargedBytesAfter == 0
            && result.removedBytes == 0
            && result.capBytes == 0
    }

    private static func deferral(_ outcome: MaintenanceOutcome) -> DuxMaintenanceDeferral? {
        switch outcome {
        case .scanRecoveryDeferredUnproven:
            .unproven
        case .terminalTempDeferredActive, .unleasedTempDeferredActive:
            .active
        case .retentionDeferredUnstable:
            .unstable
        case .stageDeferredUnproven:
            .unproven
        case .retentionDeferredNoEligibleSnapshot:
            .noEligibleWork
        default:
            nil
        }
    }

    private static func failureDisposition(
        _ failure: MaintenanceFailure
    ) -> DuxMaintenanceFailureDisposition {
        switch failure {
        case .busy, .unavailable, .budgetExceeded:
            .retryable
        case .invalidClock, .incompatibleSchema, .unsafeStorage, .corruptData,
             .incompatibleSnapshot, .outcomeUnknown, .internalState:
            .blockedUntilRestart
        }
    }

    private static func failureDisposition(
        _ error: EngineError
    ) -> DuxMaintenanceFailureDisposition {
        switch error {
        case .Busy, .StorageUnavailable, .RegistryUnavailable, .BudgetExceeded:
            .retryable
        default:
            .blockedUntilRestart
        }
    }
}

private final class FFIDuxSnapshotReviewLease: DuxSnapshotReviewLease, @unchecked Sendable {
    let scanID: String

    private let lease: SnapshotReviewSession
    private let state: EngineServiceState

    init(lease: SnapshotReviewSession, state: EngineServiceState, scanID: String) {
        self.lease = lease
        self.state = state
        self.scanID = scanID
    }

    func renew() async throws -> Int64 {
        try await state.perform { _ in
            do {
                return try self.lease.renew().expiresAtUnixMs
            } catch let error as EngineError {
                throw EngineService.serviceError(error)
            }
        }
    }

    func rootNode() async throws -> ExplorerSnapshotNode {
        try await state.perform { _ in
            do {
                return try ExplorerSnapshotNodeAdapter.mapRoot(self.lease.rootNode())
            } catch let error as EngineError {
                throw Self.navigationError(error)
            }
        }
    }

    func childNodes(
        parentID: UInt64,
        sort: ExplorerSnapshotNodeSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotNodePage {
        guard (1 ... 200).contains(limit) else {
            throw ExplorerSnapshotNodeError.invalidLimit
        }
        return try await state.perform { _ in
            do {
                let raw = try self.lease.childNodes(
                    parentId: parentID,
                    sort: ExplorerSnapshotNodeAdapter.ffiSort(sort),
                    offset: offset,
                    limit: limit
                )
                return try ExplorerSnapshotNodeAdapter.mapPage(
                    raw,
                    expectedParentID: parentID,
                    expectedOffset: offset,
                    requestedLimit: limit
                )
            } catch let error as EngineError {
                throw Self.navigationError(error)
            }
        }
    }

    func treemap(
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotTreemap {
        guard (1 ... ExplorerSnapshotNodeAdapter.maximumTreemapCells).contains(maxCells) else {
            throw ExplorerSnapshotTreemapError.invalidBudget
        }
        return try await state.perform { _ in
            do {
                let raw = try self.lease.treemap(parentId: parentID, maxCells: maxCells)
                return try ExplorerSnapshotNodeAdapter.mapTreemap(
                    raw,
                    expectedParentID: parentID,
                    requestedMaxCells: maxCells
                )
            } catch let error as EngineError {
                throw Self.treemapError(error)
            }
        }
    }

    func largeFiles(
        minimumLogicalBytes: UInt64,
        modifiedBefore: ExplorerSnapshotTimestamp?,
        maxResults: UInt16
    ) async throws -> ExplorerSnapshotLargeFilesPage {
        let request = try ExplorerSnapshotLargeFilesAdapter.request(
            minimumLogicalBytes: minimumLogicalBytes,
            modifiedBefore: modifiedBefore,
            maxResults: maxResults
        )
        return try await state.perform { _ in
            do {
                let raw = try self.lease.largeFiles(request: request)
                return try ExplorerSnapshotLargeFilesAdapter.map(
                    raw,
                    minimumLogicalBytes: minimumLogicalBytes,
                    modifiedBefore: modifiedBefore,
                    requestedMaxResults: maxResults
                )
            } catch let error as EngineError {
                throw Self.largeFilesError(error)
            }
        }
    }

    func icloudObservationSource(
        scopeNodeID: UInt64,
        maxResults: UInt16
    ) async throws -> ExplorerICloudObservationSource {
        let request = try ExplorerICloudObservationSourceAdapter.request(
            scopeNodeID: scopeNodeID,
            maxResults: maxResults
        )
        do {
            return try await state.perform { _ in
                do {
                    let raw = try self.lease.icloudObservationSource(request: request)
                    return try ExplorerICloudObservationSourceAdapter.map(
                        raw,
                        expectedScanID: self.scanID,
                        expectedScopeNodeID: scopeNodeID,
                        requestedMaxResults: maxResults
                    )
                } catch let error as EngineError {
                    throw Self.iCloudObservationSourceError(error)
                } catch let error as ExplorerICloudObservationSourceError {
                    throw error
                } catch {
                    throw ExplorerICloudObservationSourceError.invalidResponse
                }
            }
        } catch let error as ExplorerICloudObservationSourceError {
            throw error
        } catch is EngineServiceError {
            throw ExplorerICloudObservationSourceError.unavailable
        } catch {
            throw ExplorerICloudObservationSourceError.invalidResponse
        }
    }

    func candidatePaths(
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidatePathPage {
        guard (1 ... ExplorerCandidateDetailAdapter.maximumPageLimit).contains(limit) else {
            throw ExplorerCandidateDetailError.invalidLimit
        }
        return try await state.perform { _ in
            do {
                let raw = try self.lease.candidatePaths(
                    candidateId: candidateID,
                    cursor: cursor,
                    limit: limit
                )
                return try ExplorerCandidateDetailAdapter.mapPaths(
                    raw,
                    expectedScanID: self.scanID,
                    expectedCandidateID: candidateID,
                    expectedCursor: cursor,
                    requestedLimit: limit
                )
            } catch let error as EngineError {
                throw Self.candidateDetailError(error)
            }
        }
    }

    func candidateSummaries(
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateSummaryPage {
        guard (1 ... ExplorerCandidateDetailAdapter.maximumPageLimit).contains(limit) else {
            throw ExplorerCandidateDetailError.invalidLimit
        }
        return try await state.perform { _ in
            do {
                let raw = try self.lease.candidateSummaries(cursor: cursor, limit: limit)
                return try ExplorerCandidateDetailAdapter.mapSummaries(
                    raw,
                    expectedScanID: self.scanID,
                    expectedCursor: cursor,
                    requestedLimit: limit
                )
            } catch let error as EngineError {
                throw Self.candidateDetailError(error)
            }
        }
    }

    func reviewCandidate(
        candidateID: String,
        command: ExplorerCandidateReviewCommand
    ) async throws -> ExplorerCandidateReviewResult {
        return try await state.perform { _ in
            do {
                let raw = try self.lease.reviewCandidate(
                    candidateId: candidateID,
                    command: ExplorerCandidateDetailAdapter.ffiCommand(command)
                )
                return try ExplorerCandidateDetailAdapter.mapReviewResult(
                    raw,
                    expectedScanID: self.scanID,
                    expectedCandidateID: candidateID
                )
            } catch let error as EngineError {
                throw Self.candidateDetailError(error)
            }
        }
    }

    func candidateEvidence(
        candidateID: String,
        cursor: UInt16,
        limit: UInt16
    ) async throws -> ExplorerCandidateEvidencePage {
        guard (1 ... ExplorerCandidateDetailAdapter.maximumPageLimit).contains(limit) else {
            throw ExplorerCandidateDetailError.invalidLimit
        }
        return try await state.perform { _ in
            do {
                let raw = try self.lease.candidateEvidence(
                    candidateId: candidateID,
                    cursor: cursor,
                    limit: limit
                )
                return try ExplorerCandidateDetailAdapter.mapEvidence(
                    raw,
                    expectedScanID: self.scanID,
                    expectedCandidateID: candidateID,
                    expectedCursor: cursor,
                    requestedLimit: limit
                )
            } catch let error as EngineError {
                throw Self.candidateDetailError(error)
            }
        }
    }

    func prepareRustTargetPlanReview(
        candidateID: String
    ) async throws -> any DuxRustTargetPlanReviewSession {
        let engine = try await state.perform { state in
            try state.resolveEngine()
        }
        let lease = lease
        let session = try await state.performPlanReview {
            do {
                return try engine.prepareRustTargetPlanReview(
                    parentReview: lease,
                    request: RustTargetPlanReviewRequest(
                        recordVersion: EngineService.expectedRecordVersion,
                        candidateId: candidateID
                    )
                )
            } catch let error as RustTargetPlanReviewError {
                throw EngineRustTargetPlanReviewAdapter.map(error)
            }
        }
        return FFIDuxRustTargetPlanReviewSession(
            session: session,
            state: state,
            scanID: scanID,
            candidateID: candidateID
        )
    }

    func prepareSnapshotDiffReview() async throws -> any DuxSnapshotDiffReviewSession {
        let (session, info) = try await state.perform { state in
            let engine = try state.resolveEngine()
            do {
                let session = try engine.prepareExplorerSnapshotDiffReview(
                    parent: self.lease
                )
                do {
                    let rawInfo = try session.info()
                    let info = try ExplorerSnapshotDiffAdapter.mapInfo(
                        rawInfo,
                        expectedCurrentScanID: self.scanID
                    )
                    return (session, info)
                } catch {
                    _ = try? session.release()
                    throw error
                }
            } catch let error as EngineError {
                throw Self.snapshotDiffError(error)
            }
        }
        return FFIDuxSnapshotDiffReviewSession(
            session: session,
            parent: lease,
            state: state,
            info: info
        )
    }

    func resolveLiveItem(
        nodeID: UInt64,
        purpose: ExplorerSnapshotLivePathPurpose
    ) async throws -> ExplorerResolvedLiveItem {
        let request = ExplorerSnapshotLivePathAdapter.request(nodeID: nodeID, purpose: purpose)
        return try await state.perform { _ in
            do {
                let raw = try self.lease.resolveLiveTarget(request: request)
                return try ExplorerSnapshotLivePathAdapter.map(
                    raw,
                    requestedNodeID: nodeID,
                    requestedPurpose: purpose
                )
            } catch let error as EngineError {
                throw Self.livePathError(error)
            }
        }
    }

    func executeTrash(nodeID: UInt64) async throws -> TrashPlatformResult {
        try await state.perform { _ in
            do {
                let engine = try self.state.resolveEngine()
                return try engine.executeExplorerTrash(
                    review: self.lease,
                    nodeId: nodeID,
                    driver: MacOSTrashPlatformDriver()
                )
            } catch let error as TrashExecutionError {
                throw ExplorerTrashError(error)
            } catch let error as EngineError {
                throw Self.livePathError(error)
            }
        }
    }

    func probeICloudLocalCopy(
        nodeID: UInt64
    ) async throws -> ExplorerICloudLocalCopyAssessment {
        try await state.perform { _ in
            precondition(!Thread.isMainThread, "Blocking iCloud metadata read reached main thread")
            do {
                let engine = try self.state.resolveEngine()
                let raw = try engine.probeExplorerIcloudLocalCopy(
                    review: self.lease,
                    selection: ICloudLocalCopyProbeSelection(
                        recordVersion: EngineService.expectedRecordVersion,
                        nodeId: nodeID
                    ),
                    driver: MacOSICloudLocalCopyMetadataDriver()
                )
                return try ExplorerICloudLocalCopyAssessmentAdapter.map(raw)
            } catch let error as ICloudLocalCopyProbeError {
                throw Self.iCloudProbeError(error)
            } catch let error as ExplorerICloudLocalCopyProbeError {
                throw error
            } catch {
                throw ExplorerICloudLocalCopyProbeError.unavailable
            }
        }
    }

    func startSubtreeScan(nodeID: UInt64) async throws -> HomeScanStartDisposition {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let engine: DuxEngine
            do {
                engine = try state.resolveEngine()
            } catch let error as EngineServiceError {
                throw Self.subtreeScanError(error)
            }
            do {
                let start = try engine.startSubtreeScan(
                    review: self.lease,
                    request: SubtreeScanRequest(
                        recordVersion: EngineService.expectedRecordVersion,
                        nodeId: nodeID
                    )
                )
                guard
                    start.recordVersion == EngineService.expectedRecordVersion,
                    start.disposition == .started
                else {
                    throw ExplorerSnapshotSubtreeScanError.invalidResponse
                }
                return .started(FFIHomeScanTask(task: start.task, state: state))
            } catch let error as ScanError {
                throw Self.subtreeScanError(error)
            }
        }
    }

    func release() async {
        await state.performNonthrowing { _ in
            _ = try? self.lease.release()
        }
    }

    private static func iCloudProbeError(
        _ error: ICloudLocalCopyProbeError
    ) -> ExplorerICloudLocalCopyProbeError {
        switch error {
        case .InvalidTarget:
            .invalidTarget
        case .ChangedSinceSnapshot:
            .changedSinceSnapshot
        case .PlatformUnsupported:
            .unsupported
        case .PlatformFailed:
            .failed
        case .InvalidRecordVersion:
            .invalidResponse
        case .Closed, .WrongReview, .ReviewUnavailable, .InternalState:
            .unavailable
        }
    }

    private static func iCloudObservationSourceError(
        _ error: EngineError
    ) -> ExplorerICloudObservationSourceError {
        switch error {
        case .ReviewExpired:
            .expired
        case .SnapshotNodeNotFound, .SnapshotNodeNotDirectory,
             .InvalidSnapshotICloudObservationSourceRequest:
            .invalidRequest
        case .BudgetExceeded:
            .budgetExceeded
        case .Closed, .StorageUnavailable, .RegistryUnavailable,
             .SnapshotUnavailable, .Busy:
            .unavailable
        default:
            .invalidResponse
        }
    }

    private static func navigationError(_ error: EngineError) -> Error {
        switch error {
        case .ReviewExpired: ExplorerSnapshotNodeError.reviewExpired
        case .SnapshotNodeNotFound: ExplorerSnapshotNodeError.nodeNotFound
        case .SnapshotNodeNotDirectory: ExplorerSnapshotNodeError.nodeNotDirectory
        case .InvalidSnapshotNodePage: ExplorerSnapshotNodeError.invalidPage
        case .BudgetExceeded: ExplorerSnapshotNodeError.budgetExceeded
        default: EngineService.serviceError(error)
        }
    }

    fileprivate static func snapshotDiffError(
        _ error: EngineError
    ) -> ExplorerSnapshotDiffFailure {
        switch error {
        case .ComparableSnapshotUnavailable, .SnapshotUnavailable, .ScanNotFound:
            .notAvailable
        case .ReviewExpired, .WrongParentReview:
            .expired
        case .SnapshotNodeNotFound, .SnapshotNodeNotDirectory,
             .InvalidSnapshotNodePage, .InvalidSnapshotTreemapBudget:
            .invalidRequest
        case .BudgetExceeded:
            .budgetExceeded
        case .Closed:
            .closed
        case .Busy, .StorageUnavailable, .RegistryUnavailable:
            .unavailable
        default:
            .invalidResponse
        }
    }

    private static func treemapError(_ error: EngineError) -> Error {
        switch error {
        case .ReviewExpired: ExplorerSnapshotTreemapError.reviewExpired
        case .SnapshotNodeNotFound: ExplorerSnapshotTreemapError.nodeNotFound
        case .SnapshotNodeNotDirectory: ExplorerSnapshotTreemapError.nodeNotDirectory
        case .InvalidSnapshotTreemapBudget: ExplorerSnapshotTreemapError.invalidBudget
        case .BudgetExceeded: ExplorerSnapshotTreemapError.budgetExceeded
        default: EngineService.serviceError(error)
        }
    }

    private static func largeFilesError(_ error: EngineError) -> Error {
        switch error {
        case .ReviewExpired: ExplorerSnapshotLargeFilesError.reviewExpired
        case .InvalidSnapshotLargeFileRequest: ExplorerSnapshotLargeFilesError.invalidRequest
        case .BudgetExceeded: ExplorerSnapshotLargeFilesError.budgetExceeded
        default: EngineService.serviceError(error)
        }
    }

    private static func candidateDetailError(_ error: EngineError) -> Error {
        switch error {
        case .ReviewExpired: ExplorerCandidateDetailError.reviewExpired
        case .InvalidCandidateDetailRequest: ExplorerCandidateDetailError.invalidRequest
        case .CandidateEvaluationNotSucceeded: ExplorerCandidateDetailError.evaluationUnavailable
        case .CandidateNotFound: ExplorerCandidateDetailError.candidateNotFound
        case .CandidateReviewNotReviewable: ExplorerCandidateDetailError.notReviewable
        case .BudgetExceeded: ExplorerCandidateDetailError.budgetExceeded
        default: EngineService.serviceError(error)
        }
    }

    private static func livePathError(_ error: EngineError) -> Error {
        switch error {
        case .ReviewExpired: ExplorerSnapshotLivePathError.reviewExpired
        case .SnapshotNodeNotFound: ExplorerSnapshotLivePathError.nodeNotFound
        case .InvalidSnapshotLiveTargetRequest: ExplorerSnapshotLivePathError.invalidResponse
        case .SnapshotLiveTargetUnsupported: ExplorerSnapshotLivePathError.unsupportedItem
        case .SnapshotLivePathUnavailable: ExplorerSnapshotLivePathError.identityUnavailable
        case .SnapshotLivePathMissing: ExplorerSnapshotLivePathError.unavailable
        case .SnapshotLivePathSymlink, .SnapshotLivePathCrossVolume,
             .SnapshotLivePathChanged:
            ExplorerSnapshotLivePathError.changedSinceScan
        case .SnapshotLivePathAccessDenied: ExplorerSnapshotLivePathError.accessDenied
        default: EngineService.serviceError(error)
        }
    }

    private static func subtreeScanError(_ error: ScanError) -> Error {
        switch error {
        case .ReviewExpired: ExplorerSnapshotSubtreeScanError.reviewExpired
        case .ForeignReview: ExplorerSnapshotSubtreeScanError.foreignReview
        case .SnapshotNodeNotFound: ExplorerSnapshotSubtreeScanError.nodeNotFound
        case .SnapshotNodeNotDirectory: ExplorerSnapshotSubtreeScanError.nodeNotDirectory
        case .RootMissing, .RootAccessDenied, .RootNotDirectory, .RootSymlink,
             .RootChanged, .RootIdentityUnavailable, .UnsupportedPlatform,
             .RootUnavailable, .InvalidRoot:
            ExplorerSnapshotSubtreeScanError.rootUnavailable
        case .QueueFull, .Busy:
            ExplorerSnapshotSubtreeScanError.busy
        case .Closed, .ReviewUnavailable, .ReadOnlyStore, .StorageUnavailable,
             .RegistryUnavailable, .TaskUnavailable, .EventHistoryUnavailable:
            ExplorerSnapshotSubtreeScanError.unavailable
        case .InvalidRecordVersion, .InputTooLarge, .WrongTaskKind, .InternalState:
            ExplorerSnapshotSubtreeScanError.invalidResponse
        }
    }

    private static func subtreeScanError(_ error: EngineServiceError) -> Error {
        switch error {
        case .closed, .unavailable:
            ExplorerSnapshotSubtreeScanError.unavailable
        case .retryable:
            ExplorerSnapshotSubtreeScanError.busy
        case .invalidCapacityObservation, .conflictingCapacityObservation,
             .supersededCapacityObservation, .unexpected:
            ExplorerSnapshotSubtreeScanError.invalidResponse
        }
    }
}

private final class FFIDuxSnapshotDiffReviewSession:
    DuxSnapshotDiffReviewSession, @unchecked Sendable
{
    let info: ExplorerSnapshotDiffInfo

    private let session: SnapshotDiffReviewSession
    // The Rust transport intentionally retains the parent weakly. Native
    // ownership keeps this exact generated parent alive until child release.
    private let parent: SnapshotReviewSession
    private let state: EngineServiceState

    init(
        session: SnapshotDiffReviewSession,
        parent: SnapshotReviewSession,
        state: EngineServiceState,
        info: ExplorerSnapshotDiffInfo
    ) {
        self.session = session
        self.parent = parent
        self.state = state
        self.info = info
    }

    func renew() async throws -> ExplorerSnapshotDiffInfo {
        try await state.perform { _ in
            do {
                return try ExplorerSnapshotDiffAdapter.mapInfo(
                    self.session.renew(),
                    expectedCurrentScanID: self.info.currentScanID
                )
            } catch let error as EngineError {
                throw FFIDuxSnapshotReviewLease.snapshotDiffError(error)
            }
        }
    }

    func rootNode() async throws -> ExplorerSnapshotDiffNode {
        try await state.perform { _ in
            do {
                return try ExplorerSnapshotDiffAdapter.mapRoot(
                    self.session.rootNode()
                )
            } catch let error as EngineError {
                throw FFIDuxSnapshotReviewLease.snapshotDiffError(error)
            }
        }
    }

    func childNodes(
        parentID: UInt64,
        sort: ExplorerSnapshotDiffSort,
        offset: UInt64,
        limit: UInt16
    ) async throws -> ExplorerSnapshotDiffNodePage {
        guard (1 ... ExplorerSnapshotDiffAdapter.maximumPageLimit).contains(limit) else {
            throw ExplorerSnapshotDiffFailure.invalidRequest
        }
        return try await state.perform { _ in
            do {
                let raw = try self.session.childNodes(
                    parentId: parentID,
                    sort: ExplorerSnapshotDiffAdapter.ffiSort(sort),
                    offset: offset,
                    limit: limit
                )
                return try ExplorerSnapshotDiffAdapter.mapPage(
                    raw,
                    expectedParentID: parentID,
                    expectedOffset: offset,
                    requestedLimit: limit,
                    sort: sort
                )
            } catch let error as EngineError {
                throw FFIDuxSnapshotReviewLease.snapshotDiffError(error)
            }
        }
    }

    func treemap(
        parentID: UInt64,
        maxCells: UInt16
    ) async throws -> ExplorerSnapshotDiffTreemap {
        guard
            (1 ... ExplorerSnapshotDiffAdapter.maximumTreemapCells).contains(maxCells)
        else {
            throw ExplorerSnapshotDiffFailure.invalidRequest
        }
        return try await state.perform { _ in
            do {
                let raw = try self.session.treemap(
                    parentId: parentID,
                    maxCells: maxCells
                )
                return try ExplorerSnapshotDiffAdapter.mapTreemap(
                    raw,
                    expectedParentID: parentID,
                    requestedMaxCells: maxCells
                )
            } catch let error as EngineError {
                throw FFIDuxSnapshotReviewLease.snapshotDiffError(error)
            }
        }
    }

    func release() async {
        _ = parent
        await state.performNonthrowing { _ in
            _ = try? self.session.release()
        }
    }
}

enum EngineRustTargetPlanReviewAdapter {
    static func map(
        _ raw: RustTargetPlanReviewInfo
    ) throws -> ExplorerRustTargetPlanReviewRecord {
        guard
            let createdAt = timestamp(raw.createdAt),
            let newestMtime = timestamp(raw.newestMtime),
            let effectiveExpiresAt = timestamp(raw.effectiveExpiresAt)
        else {
            throw ExplorerRustTargetPlanReviewError.invalidResponse
        }
        return try ExplorerRustTargetPlanReviewRecord(
            recordVersion: raw.recordVersion,
            planID: raw.planId,
            sourceScanID: raw.sourceScanId,
            candidateID: raw.candidateId,
            ruleID: raw.ruleId,
            ruleRevision: raw.ruleRevision,
            category: map(raw.category),
            mode: map(raw.mode),
            safety: map(raw.safety),
            action: map(raw.action),
            estimatedBytes: raw.estimatedBytes,
            newestMtime: newestMtime,
            minimumAgeSeconds: raw.minimumAgeSeconds,
            minimumAgeNanoseconds: raw.minimumAgeNanoseconds,
            itemCount: raw.itemCount,
            pathCount: raw.pathCount,
            warnings: raw.warnings.map(map),
            createdAt: createdAt,
            effectiveExpiresAt: effectiveExpiresAt,
            scheduleEligible: raw.scheduleEligible,
            target: ExplorerRustTargetPlanReviewPath(
                encoding: map(raw.path.encoding),
                encodedBytes: raw.path.encodedBytes,
                display: raw.path.display
            )
        )
    }

    static func map(
        _ error: RustTargetPlanReviewError
    ) -> ExplorerRustTargetPlanReviewError {
        switch error {
        case .Closed:
            .closed
        case .WrongEngine, .InvalidRecordVersion, .InternalState:
            .invalidResponse
        case .ParentReviewUnavailable:
            .parentReviewUnavailable
        case .ReviewExpired:
            .reviewExpired
        case .CandidateUnavailable:
            .candidateUnavailable
        case .CargoNotEnrolled:
            .cargoNotEnrolled
        case .ActiveProcesses:
            .activeProcesses
        case .ChangedDuringReview:
            .changedDuringReview
        case .UnsupportedPlatform:
            .unsupportedPlatform
        case .BudgetExceeded:
            .budgetExceeded
        case .Busy, .ReviewBusy:
            .busy
        case .UnsafeStorage:
            .unsafeStorage
        case .CorruptData:
            .corruptData
        case .Unavailable, .ReviewUnavailable:
            .unavailable
        }
    }

    private static func timestamp(
        _ raw: SnapshotNodeTimestamp
    ) -> ExplorerSnapshotTimestamp? {
        guard raw.nanoseconds < 1_000_000_000 else {
            return nil
        }
        return ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: raw.secondsSinceUnixEpoch,
            nanoseconds: raw.nanoseconds
        )
    }

    private static func map(
        _ value: CandidateCategory
    ) -> ExplorerCandidateCategory {
        switch value {
        case .developerArtifact: .developerArtifact
        case .applicationCache: .applicationCache
        case .browserCache: .browserCache
        case .logAndDiagnostic: .logAndDiagnostic
        case .installerAndDownload: .installerAndDownload
        case .deviceAndSimulatorData: .deviceAndSimulatorData
        case .cloudFile: .cloudFile
        case .largeReviewItem: .largeReviewItem
        case .protectedSystemData: .protectedSystemData
        case .unknownStorage: .unknownStorage
        }
    }

    private static func map(
        _ value: CleanupMode
    ) throws -> ExplorerRustTargetPlanReviewMode {
        switch value {
        case .permanentSafe: .permanentSafe
        case .dryRun, .trash, .evictLocalCopy:
            throw ExplorerRustTargetPlanReviewError.invalidResponse
        }
    }

    private static func map(
        _ value: CandidateSafety
    ) -> ExplorerCandidateSafety {
        switch value {
        case .safeRegenerable: .safeRegenerable
        case .safeEvictable: .safeEvictable
        case .reviewRequired: .reviewRequired
        case .informational: .informational
        case .protected: .protected
        }
    }

    private static func map(
        _ value: CandidateAction
    ) -> ExplorerCandidateAction {
        switch value {
        case .removeKnownRegenerableContents: .removeKnownRegenerableContents
        case .evictLocalCopy: .evictLocalCopy
        case .moveToTrash: .moveToTrash
        case .revealOnly: .revealOnly
        case .noAction: .noAction
        }
    }

    private static func map(
        _ value: CleanupWarning
    ) throws -> ExplorerRustTargetPlanReviewWarning {
        switch value {
        case .estimatedBytesUnverified: .estimatedBytesUnverified
        case .permanentRemovalCannotBeUndone: .permanentRemovalCannotBeUndone
        case .dryRunDoesNotMutate, .trashDoesNotFreeSpaceImmediately,
             .cloudEvictionRequiresNetworkToRedownload:
            throw ExplorerRustTargetPlanReviewError.invalidResponse
        }
    }

    private static func map(
        _ value: SnapshotNameEncoding
    ) -> ExplorerRustTargetPlanReviewPathEncoding {
        switch value {
        case .unixBytes: .unixBytes
        case .windowsUtf16LittleEndian: .windowsUTF16LittleEndian
        }
    }
}

private enum EngineRustTargetCleanupResponseViolation: Error {
    case envelope
    case phaseShape
    case result
    case transition
}

enum EngineRustTargetCleanupAdapter {
    static func map(
        _ raw: RustTargetCleanupPoll,
        after previous: ExplorerRustTargetCleanupPoll?
    ) throws -> ExplorerRustTargetCleanupPoll {
        guard raw.recordVersion == EngineService.expectedRecordVersion, raw.revision > 0 else {
            throw EngineRustTargetCleanupResponseViolation.envelope
        }
        let result = try raw.result.map(map)
        let mapped = ExplorerRustTargetCleanupPoll(
            phase: map(raw.phase),
            cancellationRequested: raw.cancellationRequested,
            revision: raw.revision,
            failure: raw.failure.map(map),
            result: result
        )
        guard validShape(mapped) else {
            throw EngineRustTargetCleanupResponseViolation.phaseShape
        }
        if let previous {
            guard validTransition(from: previous, to: mapped) else {
                throw EngineRustTargetCleanupResponseViolation.transition
            }
        }
        return mapped
    }

    static func map(
        _ error: RustTargetCleanupStartError
    ) -> ExplorerRustTargetCleanupStartError {
        switch error {
        case .Closed:
            .closed
        case .WrongEngine, .InternalState:
            .invalidResponse
        case .ReviewUnavailable:
            .reviewUnavailable
        case .ParentReviewUnavailable:
            .parentReviewUnavailable
        case .ReviewExpired:
            .reviewExpired
        case .ChangedDuringReview:
            .changedSincePlan
        case .CancelledBeforeStart:
            .cancelledBeforeStart
        case .BudgetExceeded:
            .budgetExceeded
        case .QueueFull:
            .queueFull
        case .Busy:
            .busy
        case .UnsafeStorage:
            .unsafeStorage
        case .IncompatibleSchema:
            .incompatibleSchema
        case .CorruptData:
            .corruptData
        case .OutcomeUnknown:
            .outcomeUnknown
        case .Unavailable:
            .unavailable
        }
    }

    static func map(
        _ error: RustTargetCleanupTaskError
    ) -> ExplorerRustTargetCleanupTaskError {
        switch error {
        case .Closed:
            .closed
        case .TaskUnavailable:
            .taskUnavailable
        case .WrongTaskKind, .InternalState:
            .invalidResponse
        }
    }

    private static func map(_ phase: TaskPhase) -> ExplorerRustTargetCleanupPhase {
        switch phase {
        case .queued: .queued
        case .running: .running
        case .succeeded: .succeeded
        case .failed: .failed
        case .cancelled: .cancelled
        }
    }

    private static func map(
        _ failure: RustTargetCleanupTaskFailure
    ) -> ExplorerRustTargetCleanupFailure {
        switch failure {
        case .parentReviewUnavailable: .parentReviewUnavailable
        case .reviewExpired: .reviewExpired
        case .changedDuringReview: .changedSincePlan
        case .budgetExceeded: .budgetExceeded
        case .busy: .busy
        case .unsafeStorage: .unsafeStorage
        case .incompatibleSchema: .incompatibleSchema
        case .corruptData: .corruptData
        case .outcomeUnknown: .outcomeUnknown
        case .unavailable: .unavailable
        case .internalState: .internalState
        }
    }

    private static func map(
        _ raw: RustTargetCleanupResult
    ) throws -> ExplorerRustTargetCleanupResult {
        let status: CleanupHistorySessionStatus = switch raw.status {
        case .planned: .planned
        case .running: .running
        case .recovering: .recovering
        case .completed: .completed
        case .partiallyCompleted: .partiallyCompleted
        case .failed: .failed
        case .cancelled: .cancelled
        case .interrupted: .interrupted
        case .rejected: .rejected
        case .dryRun: .dryRun
        }
        guard
            raw.recordVersion == EngineService.expectedRecordVersion,
            validSessionID(raw.sessionId),
            status != .recovering
            || (raw.removedEntries == 0
                && raw.removedLogicalBytes == 0
                && raw.verifiedCapacityDeltaBytes == nil)
        else {
            throw EngineRustTargetCleanupResponseViolation.result
        }
        return ExplorerRustTargetCleanupResult(
            sessionID: raw.sessionId,
            status: status,
            removedEntries: raw.removedEntries,
            removedLogicalBytes: raw.removedLogicalBytes,
            verifiedCapacityDeltaBytes: raw.verifiedCapacityDeltaBytes
        )
    }

    private static func validShape(_ poll: ExplorerRustTargetCleanupPoll) -> Bool {
        switch poll.phase {
        case .queued, .running:
            poll.failure == nil && poll.result == nil
        case .succeeded:
            poll.failure == nil
                && poll.result.map {
                    switch $0.status {
                    case .completed, .partiallyCompleted, .failed, .interrupted, .rejected:
                        true
                    case .planned, .running, .recovering, .cancelled, .dryRun:
                        false
                    }
                } == true
        case .cancelled:
            poll.failure == nil
                && poll.result.map { $0.status == .cancelled } ?? true
        case .failed:
            switch poll.failure {
            case .outcomeUnknown:
                poll.result.map { $0.status == .recovering } ?? true
            case .some:
                poll.result == nil
            case nil:
                false
            }
        }
    }

    private static func validTransition(
        from previous: ExplorerRustTargetCleanupPoll,
        to current: ExplorerRustTargetCleanupPoll
    ) -> Bool {
        guard
            current.revision >= previous.revision,
            !previous.cancellationRequested || current.cancellationRequested
        else {
            return false
        }
        if current.revision == previous.revision {
            return current == previous
        }
        switch previous.phase {
        case .queued:
            return true
        case .running:
            return current.phase != .queued
        case .succeeded, .failed, .cancelled:
            return current == previous
        }
    }

    private static func validSessionID(_ value: String) -> Bool {
        let prefix = "cleanup:rust-target:"
        guard
            value.utf8.count <= 128,
            value.hasPrefix(prefix)
        else {
            return false
        }
        let suffix = value.dropFirst(prefix.count)
        return suffix.utf8.count == 32
            && suffix.utf8.allSatisfy {
                (0x30 ... 0x39).contains($0) || (0x61 ... 0x66).contains($0)
            }
    }
}

private enum EngineRustTargetDryRunResponseViolation: Error {
    case envelope
    case phaseShape
    case result
    case transition
}

enum EngineRustTargetDryRunAdapter {
    static func map(
        _ raw: RustTargetDryRunPoll,
        after previous: ExplorerRustTargetDryRunPoll?
    ) throws -> ExplorerRustTargetDryRunPoll {
        guard raw.recordVersion == EngineService.expectedRecordVersion, raw.revision > 0 else {
            throw EngineRustTargetDryRunResponseViolation.envelope
        }
        let mapped = try ExplorerRustTargetDryRunPoll(
            phase: map(raw.phase),
            cancellationRequested: raw.cancellationRequested,
            revision: raw.revision,
            failure: raw.failure.map(map),
            result: raw.result.map(map)
        )
        guard validShape(mapped) else {
            throw EngineRustTargetDryRunResponseViolation.phaseShape
        }
        if let previous, !validTransition(from: previous, to: mapped) {
            throw EngineRustTargetDryRunResponseViolation.transition
        }
        return mapped
    }

    static func map(
        _ error: RustTargetDryRunStartError
    ) -> ExplorerRustTargetDryRunStartError {
        switch error {
        case .Closed:
            .closed
        case .WrongEngine, .InternalState:
            .invalidResponse
        case .ReviewUnavailable:
            .reviewUnavailable
        case .ParentReviewUnavailable:
            .parentReviewUnavailable
        case .ReviewExpired:
            .reviewExpired
        case .ChangedDuringReview:
            .changedSincePlan
        case .CancelledBeforeStart:
            .cancelledBeforeStart
        case .BudgetExceeded:
            .budgetExceeded
        case .QueueFull:
            .queueFull
        case .Busy:
            .busy
        case .UnsafeStorage:
            .unsafeStorage
        case .IncompatibleSchema:
            .incompatibleSchema
        case .CorruptData:
            .corruptData
        case .HistoryUnresolved:
            .historyUnresolved
        case .Unavailable:
            .unavailable
        }
    }

    static func map(
        _ error: RustTargetDryRunTaskError
    ) -> ExplorerRustTargetDryRunTaskError {
        switch error {
        case .Closed:
            .closed
        case .TaskUnavailable:
            .taskUnavailable
        case .WrongTaskKind, .InternalState:
            .invalidResponse
        }
    }

    private static func map(_ phase: TaskPhase) -> ExplorerRustTargetDryRunPhase {
        switch phase {
        case .queued: .queued
        case .running: .running
        case .succeeded: .succeeded
        case .failed: .failed
        case .cancelled: .cancelled
        }
    }

    private static func map(
        _ failure: RustTargetDryRunTaskFailure
    ) -> ExplorerRustTargetDryRunFailure {
        switch failure {
        case .parentReviewUnavailable: .parentReviewUnavailable
        case .reviewExpired: .reviewExpired
        case .changedDuringReview: .changedSincePlan
        case .budgetExceeded: .budgetExceeded
        case .busy: .busy
        case .unsafeStorage: .unsafeStorage
        case .incompatibleSchema: .incompatibleSchema
        case .corruptData: .corruptData
        case .historyUnresolved: .historyUnresolved
        case .unavailable: .unavailable
        case .internalState: .internalState
        }
    }

    private static func map(
        _ raw: RustTargetDryRunResult
    ) throws -> ExplorerRustTargetDryRunResult {
        let status: CleanupHistorySessionStatus = switch raw.status {
        case .planned: .planned
        case .running: .running
        case .recovering: .recovering
        case .completed: .completed
        case .partiallyCompleted: .partiallyCompleted
        case .failed: .failed
        case .cancelled: .cancelled
        case .interrupted: .interrupted
        case .rejected: .rejected
        case .dryRun: .dryRun
        }
        let isAllowedStatus = switch status {
        case .dryRun, .failed, .rejected, .interrupted, .cancelled:
            true
        case .planned, .running, .recovering, .completed, .partiallyCompleted:
            false
        }
        guard
            raw.recordVersion == EngineService.expectedRecordVersion,
            validSessionID(raw.sessionId),
            isAllowedStatus
        else {
            throw EngineRustTargetDryRunResponseViolation.result
        }
        return ExplorerRustTargetDryRunResult(
            sessionID: raw.sessionId,
            status: status
        )
    }

    private static func validShape(_ poll: ExplorerRustTargetDryRunPoll) -> Bool {
        switch poll.phase {
        case .queued, .running:
            poll.failure == nil && poll.result == nil
        case .succeeded:
            poll.failure == nil
                && poll.result.map {
                    switch $0.status {
                    case .dryRun, .failed, .rejected, .interrupted:
                        true
                    case .planned, .running, .recovering, .completed,
                         .partiallyCompleted, .cancelled:
                        false
                    }
                } == true
        case .cancelled:
            poll.failure == nil
                && poll.result.map { $0.status == .cancelled } ?? true
        case .failed:
            poll.failure != nil && poll.result == nil
        }
    }

    private static func validTransition(
        from previous: ExplorerRustTargetDryRunPoll,
        to current: ExplorerRustTargetDryRunPoll
    ) -> Bool {
        guard
            current.revision >= previous.revision,
            !previous.cancellationRequested || current.cancellationRequested
        else {
            return false
        }
        if current.revision == previous.revision {
            return current == previous
        }
        switch previous.phase {
        case .queued:
            return true
        case .running:
            return current.phase != .queued
        case .succeeded, .failed, .cancelled:
            return current == previous
        }
    }

    private static func validSessionID(_ value: String) -> Bool {
        let prefix = "cleanup:rust-target-dry-run:"
        guard value.utf8.count <= 128, value.hasPrefix(prefix) else {
            return false
        }
        let suffix = value.dropFirst(prefix.count)
        return suffix.utf8.count == 32
            && suffix.utf8.allSatisfy {
                (0x30 ... 0x39).contains($0) || (0x61 ... 0x66).contains($0)
            }
    }
}

private final class FFIDuxRustTargetPlanReviewSession:
    DuxRustTargetPlanReviewSession,
    @unchecked Sendable
{
    let scanID: String
    let candidateID: String

    private let session: RustTargetPlanReviewSession
    private let state: EngineServiceState
    private var isAvailable = true

    init(
        session: RustTargetPlanReviewSession,
        state: EngineServiceState,
        scanID: String,
        candidateID: String
    ) {
        self.session = session
        self.state = state
        self.scanID = scanID
        self.candidateID = candidateID
    }

    func info() async throws -> ExplorerRustTargetPlanReviewRecord {
        try await state.performPlanReview {
            guard self.isAvailable else {
                throw ExplorerRustTargetPlanReviewError.reviewNotAcquired
            }
            do {
                return try EngineRustTargetPlanReviewAdapter.map(
                    self.session.info()
                )
            } catch let error as RustTargetPlanReviewError {
                throw EngineRustTargetPlanReviewAdapter.map(error)
            }
        }
    }

    func startCleanup() async throws -> any DuxRustTargetCleanupTask {
        let engine = try await state.perform { state in
            try state.resolveEngine()
        }
        let task = try await state.performPlanReview {
            guard self.isAvailable else {
                throw ExplorerRustTargetCleanupStartError.reviewUnavailable
            }
            self.isAvailable = false
            do {
                return try engine.startPermanentSafeCleanup(review: self.session)
            } catch let error as RustTargetCleanupStartError {
                throw EngineRustTargetCleanupAdapter.map(error)
            }
        }
        return FFIDuxRustTargetCleanupTask(task: task, state: state)
    }

    func startDryRun() async throws -> any DuxRustTargetDryRunTask {
        let engine = try await state.perform { state in
            try state.resolveEngine()
        }
        let task = try await state.performPlanReview {
            guard self.isAvailable else {
                throw ExplorerRustTargetDryRunStartError.reviewUnavailable
            }
            self.isAvailable = false
            do {
                return try engine.startRustTargetDryRun(review: self.session)
            } catch let error as RustTargetDryRunStartError {
                throw EngineRustTargetDryRunAdapter.map(error)
            }
        }
        return FFIDuxRustTargetDryRunTask(task: task, state: state)
    }

    func release() async {
        _ = try? await state.performPlanReview {
            guard self.isAvailable else {
                return
            }
            self.isAvailable = false
            do {
                switch try self.session.release() {
                case .released, .alreadyUnavailable:
                    break
                }
            } catch {
                // Release is consuming, idempotent best effort during teardown.
            }
        }
    }
}

private final class FFIDuxRustTargetDryRunTask:
    DuxRustTargetDryRunTask,
    @unchecked Sendable
{
    private let task: RustTargetDryRunTask
    private let state: EngineServiceState
    private var lastPoll: ExplorerRustTargetDryRunPoll?

    init(task: RustTargetDryRunTask, state: EngineServiceState) {
        self.task = task
        self.state = state
    }

    func poll() async throws -> ExplorerRustTargetDryRunPoll {
        try await state.perform { _ in
            do {
                let poll = try EngineRustTargetDryRunAdapter.map(
                    self.task.poll(),
                    after: self.lastPoll
                )
                self.lastPoll = poll
                return poll
            } catch is EngineRustTargetDryRunResponseViolation {
                throw ExplorerRustTargetDryRunTaskError.invalidResponse
            } catch let error as RustTargetDryRunTaskError {
                throw EngineRustTargetDryRunAdapter.map(error)
            }
        }
    }

    func requestCancellation() async throws -> ExplorerRustTargetDryRunCancelOutcome {
        try await state.perform { _ in
            do {
                return switch try self.task.cancel() {
                case .cancelledBeforeStart: .cancelledBeforeStart
                case .requested: .requested
                case .alreadyRequested: .alreadyRequested
                case .alreadyTerminal: .alreadyTerminal
                }
            } catch let error as RustTargetDryRunTaskError {
                throw EngineRustTargetDryRunAdapter.map(error)
            }
        }
    }
}

private final class FFIDuxRustTargetCleanupTask:
    DuxRustTargetCleanupTask,
    @unchecked Sendable
{
    private let task: RustTargetCleanupTask
    private let state: EngineServiceState
    private var lastPoll: ExplorerRustTargetCleanupPoll?

    init(task: RustTargetCleanupTask, state: EngineServiceState) {
        self.task = task
        self.state = state
    }

    func poll() async throws -> ExplorerRustTargetCleanupPoll {
        try await state.perform { _ in
            do {
                let poll = try EngineRustTargetCleanupAdapter.map(
                    self.task.poll(),
                    after: self.lastPoll
                )
                self.lastPoll = poll
                return poll
            } catch is EngineRustTargetCleanupResponseViolation {
                throw ExplorerRustTargetCleanupTaskError.invalidResponse
            } catch let error as RustTargetCleanupTaskError {
                throw EngineRustTargetCleanupAdapter.map(error)
            }
        }
    }

    func requestCancellation() async throws -> ExplorerRustTargetCleanupCancelOutcome {
        try await state.perform { _ in
            do {
                return switch try self.task.cancel() {
                case .cancelledBeforeStart: .cancelledBeforeStart
                case .requested: .requested
                case .alreadyRequested: .alreadyRequested
                case .alreadyTerminal: .alreadyTerminal
                }
            } catch let error as RustTargetCleanupTaskError {
                throw EngineRustTargetCleanupAdapter.map(error)
            }
        }
    }
}
