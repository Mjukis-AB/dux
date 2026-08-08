import Foundation
import Observation

/// A bounded, path-free observation of unclaimed durable running-scan rows.
/// These counts describe DUX bookkeeping only. They are not disk usage,
/// liveness evidence, recovery authority, or permission to remove anything.
struct PersistentRecoveryDebt: Equatable, Sendable {
    static let maximumInspectedCount: UInt16 = 64

    let inspectedUnclaimedCount: UInt16
    let pristineUnclaimedCount: UInt16
    let unexplainedUnclaimedCount: UInt16
    let hasMore: Bool

    var displayedCount: String {
        hasMore ? "\(inspectedUnclaimedCount)+" : String(inspectedUnclaimedCount)
    }

    var accessibilityCount: String {
        if hasMore {
            return "\(inspectedUnclaimedCount) or more retained recovery records; incomplete bounded census"
        }
        switch inspectedUnclaimedCount {
        case 1:
            return "1 retained recovery record"
        default:
            return "\(inspectedUnclaimedCount) retained recovery records"
        }
    }
}

enum PersistentRecoveryDebtLoadState: Equatable, Sendable {
    case idle
    case loading
    case loaded
    case failed(PersistentRecoveryDebtServiceError)

    var isLoading: Bool {
        self == .loading
    }
}

enum PersistentRecoveryDebtServiceError: Error, Equatable, Sendable {
    case closed
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case budgetExceeded
    case corruptData
    case unavailable
    case internalState
    case invalidResponse
}

/// Immutable aggregate confirmation facts. No scan identity, root, path,
/// timestamp of the legacy row, or filesystem authority is present.
struct LegacyRunningScanDismissalPreviewModel: Equatable, Sendable {
    let eligibleCount: UInt16
    let hasMore: Bool
    let preparedAt: Date
    let expiresAt: Date
}

struct LegacyRunningScanDismissalConfirmation: Equatable, Sendable {
    let generation: UInt64
    let preview: LegacyRunningScanDismissalPreviewModel
}

struct LegacyRunningScanDismissalResultModel: Equatable, Sendable {
    let dismissedCount: UInt16
    let hasMore: Bool
}

enum LegacyRunningScanDismissalServiceError: Error, Equatable, Sendable {
    case closed
    case nothingEligible
    case changedSincePreview
    case previewExpired
    case wrongEngine
    case previewUnavailable
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case budgetExceeded
    case corruptData
    case outcomeUnknown
    case unavailable
    case internalState
    case invalidResponse
}

enum LegacyRunningScanDismissalState: Equatable, Sendable {
    case idle
    case preparing
    case awaitingConfirmation(LegacyRunningScanDismissalConfirmation)
    case dismissing(LegacyRunningScanDismissalPreviewModel)
    case completed(LegacyRunningScanDismissalResultModel)
    case failed(LegacyRunningScanDismissalServiceError)
    case outcomeUnknown

    var isBusy: Bool {
        switch self {
        case .preparing, .dismissing:
            true
        case .idle, .awaitingConfirmation, .completed, .failed, .outcomeUnknown:
            false
        }
    }
}

/// A bounded, path-free classification of claimed running-scan provenance.
/// The categories compare stored execution evidence with the current host and
/// boot context. They do not establish process liveness, recovery authority,
/// disk usage, or permission to remove anything.
struct ClaimedRunningScanProvenance: Equatable, Sendable {
    static let maximumInspectedCount: UInt16 = 64

    let inspectedClaimedCount: UInt16
    let sameHostCurrentBootCount: UInt16
    let sameHostPriorBootCount: UInt16
    let foreignHostCount: UInt16
    let storedUnprovenCount: UInt16
    let currentContextUnavailableCount: UInt16
    let hasMore: Bool

    var displayedCount: String {
        hasMore ? "\(inspectedClaimedCount)+" : String(inspectedClaimedCount)
    }

    var accessibilityCount: String {
        if hasMore {
            return "\(inspectedClaimedCount) or more claimed running scan records; "
                + "incomplete bounded census"
        }
        switch inspectedClaimedCount {
        case 1:
            return "1 claimed running scan record"
        default:
            return "\(inspectedClaimedCount) claimed running scan records"
        }
    }

    var accessibilityDistribution: String {
        "Current startup session \(sameHostCurrentBootCount); "
            + "earlier startup session on this Mac \(sameHostPriorBootCount); "
            + "different host \(foreignHostCount); identity not stored \(storedUnprovenCount); "
            + "current identity unavailable \(currentContextUnavailableCount)"
    }
}

enum ClaimedRunningScanProvenanceLoadState: Equatable, Sendable {
    case idle
    case loading
    case loaded
    case failed(ClaimedRunningScanProvenanceServiceError)

    var isLoading: Bool {
        self == .loading
    }
}

enum ClaimedRunningScanProvenanceServiceError: Error, Equatable, Sendable {
    case closed
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case budgetExceeded
    case corruptData
    case unavailable
    case internalState
    case invalidResponse
}

@MainActor
@Observable
final class LegacyRunningScanDismissalSettingsModel {
    private(set) var state = LegacyRunningScanDismissalState.idle
    private(set) var confirmation: LegacyRunningScanDismissalConfirmation?

    private let service: any DuxLegacyRunningScanDismissalServing

    @ObservationIgnored
    private var operationTask: Task<Void, Never>?
    @ObservationIgnored
    private var expiryTask: Task<Void, Never>?
    @ObservationIgnored
    private var lease: (any DuxLegacyRunningScanDismissalPreviewLease)?
    @ObservationIgnored
    private var generation: UInt64 = 0
    @ObservationIgnored
    private var shuttingDown = false

    init(service: any DuxLegacyRunningScanDismissalServing) {
        self.service = service
    }

    func prepare() async {
        guard
            !shuttingDown,
            operationTask == nil,
            confirmation == nil,
            !state.isBusy
        else {
            return
        }
        generation &+= 1
        let requestGeneration = generation
        state = .preparing
        let service = service
        let task = Task { @MainActor [weak self] in
            let result: Result<any DuxLegacyRunningScanDismissalPreviewLease, Error>
            do {
                result = try .success(
                    await service.prepareLegacyRunningScanDismissal()
                )
            } catch {
                result = .failure(error)
            }
            guard
                let self,
                !self.shuttingDown,
                self.generation == requestGeneration
            else {
                if case .success(let lease) = result {
                    await lease.release()
                }
                return
            }
            self.operationTask = nil
            switch result {
            case .success(let lease):
                guard lease.preview.expiresAt > Date() else {
                    self.state = .failed(.previewExpired)
                    await lease.release()
                    return
                }
                let confirmation = LegacyRunningScanDismissalConfirmation(
                    generation: requestGeneration,
                    preview: lease.preview
                )
                self.lease = lease
                self.confirmation = confirmation
                self.state = .awaitingConfirmation(confirmation)
                self.scheduleExpiration(confirmation)
            case .failure(let error):
                self.state = .failed(Self.failure(for: error))
            }
        }
        operationTask = task
        await task.value
    }

    func confirm(_ requested: LegacyRunningScanDismissalConfirmation) async {
        guard
            !shuttingDown,
            operationTask == nil,
            confirmation == requested,
            requested.generation == generation,
            let lease
        else {
            return
        }
        guard requested.preview.expiresAt > Date() else {
            await cancel(requested)
            state = .failed(.previewExpired)
            return
        }

        generation &+= 1
        let operationGeneration = generation
        expiryTask?.cancel()
        expiryTask = nil
        confirmation = nil
        self.lease = nil
        state = .dismissing(requested.preview)
        let service = service
        let task = Task { @MainActor [weak self] in
            let result: Result<LegacyRunningScanDismissalResultModel, Error>
            do {
                result = try .success(
                    await service.dismissLegacyRunningScans(lease)
                )
            } catch {
                result = .failure(error)
            }
            await lease.release()
            guard
                let self,
                !self.shuttingDown,
                self.generation == operationGeneration
            else {
                return
            }
            self.operationTask = nil
            switch result {
            case .success(let result):
                self.state = .completed(result)
            case .failure(let error):
                let failure = Self.failure(for: error)
                self.state = failure == .outcomeUnknown ? .outcomeUnknown : .failed(failure)
            }
        }
        operationTask = task
        await task.value
    }

    func cancel(_ requested: LegacyRunningScanDismissalConfirmation? = nil) async {
        guard
            let current = confirmation,
            requested == nil || requested == current
        else {
            return
        }
        generation &+= 1
        expiryTask?.cancel()
        expiryTask = nil
        let lease = lease
        self.lease = nil
        confirmation = nil
        state = .idle
        await lease?.release()
    }

    func dismissResult() {
        guard !state.isBusy, confirmation == nil else {
            return
        }
        state = .idle
    }

    func shutdown() async {
        guard !shuttingDown else {
            if let operationTask {
                await operationTask.value
            }
            return
        }
        shuttingDown = true
        generation &+= 1
        expiryTask?.cancel()
        expiryTask = nil
        confirmation = nil
        let lease = lease
        self.lease = nil
        await lease?.release()
        if let operationTask {
            await operationTask.value
        }
        operationTask = nil
        state = .idle
    }

    private func scheduleExpiration(
        _ confirmation: LegacyRunningScanDismissalConfirmation
    ) {
        expiryTask?.cancel()
        let delay = max(0, confirmation.preview.expiresAt.timeIntervalSinceNow)
        expiryTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled, let self else {
                return
            }
            guard self.confirmation == confirmation else {
                return
            }
            await self.cancel(confirmation)
            self.state = .failed(.previewExpired)
        }
    }

    private static func failure(for error: Error) -> LegacyRunningScanDismissalServiceError {
        if let error = error as? LegacyRunningScanDismissalServiceError {
            return error
        }
        return .internalState
    }
}
