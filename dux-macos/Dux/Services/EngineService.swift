import Dispatch
import Foundation

protocol DuxVolumeStatusServing: Sendable {
    func observeVolumeCapacity(_ snapshot: VolumeCapacitySnapshot) async throws
        -> VolumeCapacitySnapshot
}

protocol EngineServing: DuxVolumeStatusServing, Sendable {
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

struct EngineService: EngineServing, DuxMaintenanceServing, DuxSnapshotReviewServing, Sendable {
    fileprivate static let expectedFFIContractVersion: UInt32 = 5
    fileprivate static let expectedRecordVersion: UInt32 = 1

    private let state: EngineServiceState

    init(
        engine: DuxEngine? = nil,
        storageRoots: EngineStorageRoots? = nil
    ) {
        precondition(engine == nil || storageRoots == nil)
        state = EngineServiceState(engine: engine, storageRoots: storageRoots)
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
    private var closeResult: Bool?

    init(engine: DuxEngine?, storageRoots: EngineStorageRoots?) {
        self.engine = engine
        self.storageRoots = storageRoots
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
