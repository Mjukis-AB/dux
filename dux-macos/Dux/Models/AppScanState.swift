import Foundation

struct ScanProgressFacts: Equatable, Sendable {
    let files: UInt64
    let directories: UInt64
    let knownAllocatedBytes: UInt64?
    let issueCount: UInt64

    var itemCount: UInt64 {
        let (sum, overflow) = files.addingReportingOverflow(directories)
        return overflow ? UInt64.max : sum
    }
}

enum AppScanCoverage: Equatable, Sendable {
    case complete
    case limitedAccess
    case partial
    case unknown
}

struct AppScanSummary: Equatable, Sendable {
    let scanID: String
    let startedAt: Date
    let completedAt: Date
    let progress: ScanProgressFacts
    let logicalBytes: UInt64
    let coverage: AppScanCoverage
    let coveragePermille: UInt16?
    let snapshotAvailable: Bool
}

enum AppScanScope: Equatable, Sendable {
    case home
    case startupVolume(displayName: String)
    case subtree(displayName: String)

    var isHome: Bool {
        self == .home
    }
}

enum AppScanFailure: Equatable, Sendable {
    case closed
    case busy
    case rootUnavailable
    case storageUnavailable
    case incompatibleStorage
    case taskExpired
    case scanFailed
    case snapshotRejected
    case outcomeUnknown
    case invalidResponse
    case unexpected
}

enum AppScanPhase: Equatable, Sendable {
    case idle
    case queued
    case scanning(ScanProgressFacts?)
    case finalizing(ScanProgressFacts?)
    case evaluating(ScanProgressFacts?)
    case cancellationRequested(ScanProgressFacts?)
    case succeeded(AppScanSummary)
    case cancelled
    case failed(AppScanFailure)

    var isActive: Bool {
        switch self {
        case .queued, .scanning, .finalizing, .evaluating, .cancellationRequested:
            true
        case .idle, .succeeded, .cancelled, .failed:
            false
        }
    }
}

struct AppScanState: Equatable, Sendable {
    var phase: AppScanPhase
    var lastSuccessful: AppScanSummary?
    var scope: AppScanScope?

    init(
        phase: AppScanPhase,
        lastSuccessful: AppScanSummary?,
        scope: AppScanScope? = nil
    ) {
        self.phase = phase
        self.lastSuccessful = lastSuccessful
        self.scope = scope
    }

    static let idle = Self(phase: .idle, lastSuccessful: nil)
}
