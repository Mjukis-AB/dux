import Foundation

enum HomeScanServiceError: Error, Equatable, Sendable {
    case closed
    case queueFull
    case inputTooLarge
    case invalidRoot
    case rootMissing
    case rootAccessDenied
    case rootNotDirectory
    case rootSymlink
    case rootChanged
    case rootIdentityUnavailable
    case unsupportedPlatform
    case rootUnavailable
    case readOnlyStore
    case persistenceUnavailable
    case incompatibleSchema
    case busy
    case unsafeStorage
    case budgetExceeded
    case corruptData
    case taskExpired
    case wrongTaskKind
    case outcomeUnknown
    case internalState
    case invalidResponse
}

/// Exact failures while converting a historical review node into a new,
/// standalone scan root. This boundary remains path-free: Swift receives no
/// live path and cannot substitute a display name for Rust's identity check.
enum ExplorerSnapshotSubtreeScanError: Error, Equatable, Sendable {
    case reviewExpired
    case foreignReview
    case nodeNotFound
    case nodeNotDirectory
    case rootUnavailable
    case busy
    case unavailable
    case invalidResponse
}

enum HomeScanTaskPhase: Equatable, Sendable {
    case queued
    case running
    case succeeded
    case failed
    case cancelled

    var isTerminal: Bool {
        switch self {
        case .succeeded, .failed, .cancelled:
            true
        case .queued, .running:
            false
        }
    }
}

enum HomeScanTaskStage: Equatable, Sendable {
    case queued
    case scanning
    case finalizing
    case evaluating
    case terminal
}

enum HomeScanTaskFailure: Equatable, Sendable {
    case rootChanged
    case scanFailed
    case snapshotRejected
    case persistenceUnavailable
    case persistenceOutcomeUnknown
    case internalFailure
}

enum HomeScanCandidateEvaluation: Equatable, Sendable {
    case notRun
    case succeeded(candidateCount: UInt32)
    case failed
}

struct HomeScanTaskResult: Equatable, Sendable {
    let scanID: String
    let startedAt: Date
    let completedAt: Date
    let succeeded: Bool
    let directoryCount: UInt64
    let fileCount: UInt64
    let logicalBytes: UInt64
    let allocatedBytes: UInt64?
    let coverage: AppScanCoverage
    let coveragePermille: UInt16?
    let issueCount: UInt64
    let snapshotAvailable: Bool
    let candidateEvaluation: HomeScanCandidateEvaluation

    var successfulSummary: AppScanSummary? {
        guard succeeded else {
            return nil
        }
        return AppScanSummary(
            scanID: scanID,
            startedAt: startedAt,
            completedAt: completedAt,
            progress: ScanProgressFacts(
                files: fileCount,
                directories: directoryCount,
                knownAllocatedBytes: allocatedBytes,
                issueCount: issueCount
            ),
            logicalBytes: logicalBytes,
            coverage: coverage,
            coveragePermille: coveragePermille,
            snapshotAvailable: snapshotAvailable
        )
    }
}

struct HomeScanTaskPoll: Equatable, Sendable {
    let phase: HomeScanTaskPhase
    let stage: HomeScanTaskStage
    let cancellationRequested: Bool
    let revision: UInt64
    let progress: ScanProgressFacts?
    let eventsTruncated: Bool
    let failure: HomeScanTaskFailure?
    let result: HomeScanTaskResult?
}

enum HomeScanCancelOutcome: Equatable, Sendable {
    case cancelledBeforeStart
    case requested
    case alreadyRequested
    case alreadyTerminal
}

protocol HomeScanTask: AnyObject, Sendable {
    func poll() async throws -> HomeScanTaskPoll
    func requestCancellation() async throws -> HomeScanCancelOutcome
}

enum HomeScanStartDisposition: Sendable {
    case started(any HomeScanTask)
    case alreadyActive(any HomeScanTask)

    var task: any HomeScanTask {
        switch self {
        case let .started(task), let .alreadyActive(task):
            task
        }
    }
}

enum AppScanRunOutcome: Equatable, Sendable {
    case succeeded(AppScanSummary)
    case cancelled
    case failed(AppScanFailure)
    case superseded
}

protocol HomeScanServing: Sendable {
    func startHomeScan() async throws -> HomeScanStartDisposition
}

/// Starts a scan from one exact retained snapshot directory. Implementations
/// must keep the live path inside Rust and return only the ordinary opaque scan
/// task; a historical display name is never a scan root.
protocol DuxSnapshotSubtreeScanServing: Sendable {
    func startSubtreeScan(
        sourceScanID: String,
        nodeID: UInt64
    ) async throws -> HomeScanStartDisposition
}

protocol HomeScanPollingClock: Sendable {
    func sleepUntilNextPoll() async throws
}

struct ContinuousHomeScanPollingClock: HomeScanPollingClock {
    func sleepUntilNextPoll() async throws {
        try await Task.sleep(for: .milliseconds(500))
    }
}

private struct UnavailableHomeScanService: HomeScanServing {
    func startHomeScan() async throws -> HomeScanStartDisposition {
        throw HomeScanServiceError.internalState
    }
}

func resolvedHomeScanService(
    engineService: any EngineServing,
    override: (any HomeScanServing)?
) -> any HomeScanServing {
    if let override {
        return override
    }
    return (engineService as? any HomeScanServing) ?? UnavailableHomeScanService()
}
