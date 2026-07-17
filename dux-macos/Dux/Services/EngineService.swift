import Dispatch
import Foundation

protocol DuxVolumeStatusServing: Sendable {
    func observeVolumeCapacity(_ snapshot: VolumeCapacitySnapshot) async throws
        -> VolumeCapacitySnapshot
}

protocol DuxPressurePolicyServing: Sendable {
    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy
    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult
    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult
}

protocol EngineServing: DuxVolumeStatusServing, DuxPressurePolicyServing, Sendable {
    func loadStatus() async throws -> EngineStatus
}

protocol DuxSnapshotReviewServing: Sendable {
    func acquireExplorerReview(scanID: String) async throws -> any DuxSnapshotReviewLease
}

protocol DuxSnapshotReviewLease: AnyObject, Sendable {
    var scanID: String { get }
    func renew() async throws -> Int64
    func release() async
}

struct EngineService: EngineServing, DuxMaintenanceServing, DuxSnapshotReviewServing,
    HomeScanServing, Sendable
{
    fileprivate static let expectedFFIContractVersion: UInt32 = 7
    fileprivate static let expectedRecordVersion: UInt32 = 1

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
                    executedOffMainThread: executedOffMainThread
                )
            } catch let error as EngineError {
                throw Self.serviceError(error)
            }
        }
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        try await state.perform { state in
            precondition(!Thread.isMainThread, "Blocking FFI work reached the main thread")
            let milliseconds = snapshot.sampledAt.timeIntervalSince1970 * 1_000
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
                    criticalBoundaryBytes: status.criticalBoundaryBytes,
                    warningBoundaryBytes: status.warningBoundaryBytes,
                    historyDisposition: Self.historyDisposition(status.historyDisposition),
                    sampledAt: Date(
                        timeIntervalSince1970: Double(status.sampledAtUnixMs) / 1_000
                    )
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

    func startMaintenance(_ kind: DuxMaintenanceKind) async -> DuxMaintenanceStartDisposition {
        await state.performNonthrowing { state in
            do {
                let engine = try state.resolveEngine()
                let start = try engine.startMaintenance(kind: Self.ffiKind(kind))
                guard start.recordVersion == 1 else {
                    return .failed(.blockedUntilRestart)
                }
                switch start.disposition {
                case .started:
                    guard let task = start.task else {
                        return .failed(.blockedUntilRestart)
                    }
                    return .started(FFIDuxMaintenanceTask(task: task, state: state))
                case .alreadyActive:
                    guard let task = start.task else {
                        return .failed(.blockedUntilRestart)
                    }
                    return .alreadyActive(FFIDuxMaintenanceTask(task: task, state: state))
                case .deferredBusy:
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
                throw Self.serviceError(error)
            }
        }
        return FFIDuxSnapshotReviewLease(lease: lease, state: state, scanID: scanID)
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
                cacheRoot: root.appending(path: "cache", directoryHint: .isDirectory).path
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
        case .Busy, .StorageUnavailable, .RegistryUnavailable, .BudgetExceeded:
            .retryable
        case .ReadOnlyStore, .IncompatibleSchema, .UnsafeStorage, .CorruptData,
             .OutcomeUnknown, .InternalState:
            .unavailable
        default:
            .unexpected(String(describing: error))
        }
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
            (1 ... 10_000).contains(configuration.criticalAvailableBasisPoints),
            configuration.warningAvailableBasisPoints
                >= configuration.criticalAvailableBasisPoints,
            (1 ... 10_000).contains(configuration.warningAvailableBasisPoints),
            (1 ... 10_000).contains(configuration.recoveryBasisPoints),
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
        return DiskPressurePolicyUpdateResult(
            policy: try policy(response.policy),
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

    fileprivate static func homeScanServiceError(_ error: ScanError) -> HomeScanServiceError {
        switch error {
        case .Closed: .closed
        case .InvalidRecordVersion: .invalidResponse
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
             .ConflictingCapacityObservation, .SupersededCapacityObservation, .ScanNotFound,
             .SnapshotUnavailable, .ReviewExpired, .ReadOnlyStore,
             .IncompatibleSchema, .UnsafeStorage, .CorruptData,
             .IncompatibleSnapshot, .OutcomeUnknown, .InternalState:
            .blockedUntilRestart
        }
    }
}

private final class EngineServiceState: @unchecked Sendable {
    fileprivate let queue = DispatchQueue(label: "se.mjukis.dux.engine", qos: .utility)

    private var engine: DuxEngine?
    private let storageRoots: EngineStorageRoots?
    fileprivate let homeScanRoot: URL?
    private var closeResult: Bool?

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

    fileprivate func perform<T: Sendable>(
        _ operation: @escaping @Sendable (EngineServiceState) throws -> T
    ) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [self] in
                do {
                    continuation.resume(returning: try operation(self))
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

private final class FFIHomeScanTask: HomeScanTask, @unchecked Sendable {
    private let task: ScanTask
    private let state: EngineServiceState
    private var lastPoll: HomeScanTaskPoll?

    init(task: ScanTask, state: EngineServiceState) {
        self.task = task
        self.state = state
    }

    func poll() async throws -> HomeScanTaskPoll {
        try await state.perform { _ in
            do {
                let poll = try Self.map(self.task.poll(), after: self.lastPoll)
                self.lastPoll = poll
                return poll
            } catch let error as ScanError {
                throw EngineService.homeScanServiceError(error)
            }
        }
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

    private static func map(
        _ raw: ScanPoll,
        after previous: HomeScanTaskPoll?
    ) throws -> HomeScanTaskPoll {
        guard raw.recordVersion == EngineService.expectedRecordVersion, raw.revision > 0 else {
            throw HomeScanServiceError.invalidResponse
        }
        let progress = try raw.progress.map(mapProgress)
        let result = try raw.result.map(mapResult)
        let failure = raw.failure.map(mapFailure)
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
            throw HomeScanServiceError.invalidResponse
        }

        let mapped = HomeScanTaskPoll(
            phase: phase,
            stage: stage,
            cancellationRequested: raw.cancellationRequested,
            revision: raw.revision,
            progress: progress,
            eventsTruncated: raw.eventsTruncated,
            failure: failure,
            result: result
        )
        guard validTerminalResult(progress: mapped.progress, result: mapped.result) else {
            throw HomeScanServiceError.invalidResponse
        }
        if let previous {
            guard
                mapped.revision >= previous.revision,
                !previous.cancellationRequested || mapped.cancellationRequested,
                !previous.eventsTruncated || mapped.eventsTruncated,
                validPhaseTransition(from: previous.phase, to: mapped.phase),
                validStageTransition(from: previous.stage, to: mapped.stage),
                validProgressTransition(from: previous.progress, to: mapped.progress),
                validTerminalResult(progress: previous.progress, result: mapped.result)
            else {
                throw HomeScanServiceError.invalidResponse
            }
        }
        return mapped
    }

    private static func mapProgress(_ raw: ScanProgress) throws -> ScanProgressFacts {
        guard raw.recordVersion == EngineService.expectedRecordVersion else {
            throw HomeScanServiceError.invalidResponse
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
            throw HomeScanServiceError.invalidResponse
        }

        let succeeded = raw.status == .succeeded
        switch raw.status {
        case .succeeded:
            guard
                raw.snapshotAvailable,
                raw.coverage.status != .unknown,
                raw.candidateEvaluation.status != .notRun
            else {
                throw HomeScanServiceError.invalidResponse
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
                throw HomeScanServiceError.invalidResponse
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
            startedAt: Date(timeIntervalSince1970: Double(raw.startedAtUnixMs) / 1_000),
            completedAt: Date(timeIntervalSince1970: Double(raw.completedAtUnixMs) / 1_000),
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

    private static func validCoverage(_ coverage: ScanCoverageSummary) -> Bool {
        guard
            coverage.recordVersion == EngineService.expectedRecordVersion,
            coverage.issueRecordCount <= 256,
            coverage.issueOccurrenceCount >= coverage.issueRecordCount,
            coverage.measuredPermille.map({ $0 <= 1_000 }) ?? true
        else {
            return false
        }
        switch coverage.status {
        case .unknown:
            return coverage.measuredPermille == nil
                && coverage.issueRecordCount == 0
                && coverage.issueOccurrenceCount == 0
        case .complete:
            return coverage.measuredPermille == 1_000
                && coverage.issueRecordCount == 0
                && coverage.issueOccurrenceCount == 0
        case .limitedAccess, .partial:
            return coverage.issueRecordCount > 0 && coverage.measuredPermille != 1_000
        }
    }

    private static func validCandidateEvaluation(
        _ evaluation: ScanCandidateEvaluationSummary
    ) -> Bool {
        guard
            evaluation.recordVersion == EngineService.expectedRecordVersion,
            evaluation.candidateCount <= 4_096
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

    init(task: MaintenanceTask, state: EngineServiceState) {
        self.task = task
        self.state = state
    }

    func poll() async -> DuxMaintenanceTaskPoll {
        await state.performNonthrowing { _ in
            do {
                return Self.map(try self.task.poll())
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

    private static func map(_ poll: MaintenancePoll) -> DuxMaintenanceTaskPoll {
        guard poll.recordVersion == 1 else {
            return .failed(.blockedUntilRestart)
        }
        switch poll.phase {
        case .queued, .running:
            return .running
        case .cancelled:
            return .cancelled
        case .failed:
            return .failed(poll.failure.map(failureDisposition) ?? .blockedUntilRestart)
        case .succeeded:
            guard let result = poll.result, result.recordVersion == 1 else {
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

    func release() async {
        await state.performNonthrowing { _ in
            _ = try? self.lease.release()
        }
    }
}
