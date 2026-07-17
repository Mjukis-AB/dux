import Dispatch
import Foundation

protocol EngineServing: Sendable {
    func loadSmokeResult(bytes: UInt64) async throws -> EngineSmokeResult
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
    fileprivate static let expectedFFIContractVersion: UInt32 = 3

    private let state: EngineServiceState

    init(
        engine: DuxEngine? = nil,
        storageRoots: EngineStorageRoots? = nil
    ) {
        precondition(engine == nil || storageRoots == nil)
        state = EngineServiceState(engine: engine, storageRoots: storageRoots)
    }

    func loadSmokeResult(bytes: UInt64) async throws -> EngineSmokeResult {
        try await state.perform { state in
            let executedOffMainThread = !Thread.isMainThread
            precondition(executedOffMainThread, "Blocking FFI work reached the main thread")
            let engine = try state.resolveEngine()
            do {
                let version = try engine.libraryVersion()
                guard version.ffiContractVersion == Self.expectedFFIContractVersion else {
                    throw EngineServiceError.unexpected("incompatible FFI contract")
                }
                let size = try engine.formatSize(bytes: bytes)
                return EngineSmokeResult(
                    libraryVersion: version.libraryVersion,
                    ffiContractVersion: version.ffiContractVersion,
                    bytes: size.bytes,
                    displaySize: size.display,
                    executedOffMainThread: executedOffMainThread
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
        default:
            .unexpected(String(describing: error))
        }
    }

    private static func failureDisposition(
        _ error: EngineError
    ) -> DuxMaintenanceFailureDisposition {
        switch error {
        case .Busy, .StorageUnavailable, .RegistryUnavailable, .BudgetExceeded:
            .retryable
        case .Closed, .InvalidStorage, .InvalidScanId, .ScanNotFound,
             .SnapshotUnavailable, .ReviewExpired, .ReadOnlyStore,
             .IncompatibleSchema, .UnsafeStorage, .CorruptData,
             .IncompatibleSnapshot, .OutcomeUnknown, .InternalState:
            .blockedUntilRestart
        }
    }
}

private final class EngineServiceState: @unchecked Sendable {
    fileprivate let queue = DispatchQueue(label: "se.mjukis.dux.engine", qos: .utility)

    private var engine: Result<DuxEngine, EngineServiceError>?
    private let storageRoots: EngineStorageRoots?
    private var closeResult: Bool?

    init(engine: DuxEngine?, storageRoots: EngineStorageRoots?) {
        self.engine = engine.map(Result.success)
        self.storageRoots = storageRoots
    }

    fileprivate func resolveEngine() throws -> DuxEngine {
        dispatchPrecondition(condition: .onQueue(queue))
        if closeResult != nil {
            throw EngineServiceError.closed
        }
        if let engine {
            return try engine.get()
        }

        let opened: Result<DuxEngine, EngineServiceError>
        do {
            guard libraryVersion().ffiContractVersion == EngineService.expectedFFIContractVersion else {
                throw EngineServiceError.unexpected("incompatible FFI contract")
            }
            let roots = try storageRoots ?? EngineService.defaultStorageRoots()
            opened = .success(try DuxEngine(storage: roots))
        } catch let error as EngineError {
            opened = .failure(EngineService.serviceError(error))
        } catch let error as EngineServiceError {
            opened = .failure(error)
        } catch {
            opened = .failure(.unexpected("engine initialization failed"))
        }
        engine = opened
        return try opened.get()
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
            let result: Bool
            switch state.engine {
            case let .success(engine):
                result = engine.close()
            case .failure, .none:
                result = true
            }
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
