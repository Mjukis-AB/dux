import Foundation

/// The origin of the path-free global permanent-cleanup kill switch.
enum PermanentCleanupPolicyOrigin: Equatable, Sendable {
    case `default`
    case stored
}

/// A validated, path-free snapshot of the global permanent-cleanup switch.
/// This setting never selects a target or grants execution authority.
struct PermanentCleanupPolicy: Equatable, Sendable {
    let enabled: Bool
    let source: PermanentCleanupPolicyOrigin
    let revision: UInt64
    let updatedAtUnixMilliseconds: Int64?
}

struct PermanentCleanupPolicyUpdateResult: Equatable, Sendable {
    let policy: PermanentCleanupPolicy
    let changed: Bool
}

enum PermanentCleanupPolicyServiceError: Error, Equatable, Sendable {
    case closed
    case corruptData
    case unavailable
    case outcomeUnknown
    case revisionExhausted
    case invalidClock
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case budgetExceeded
    case internalState
    case invalidResponse
}

enum PermanentCleanupPolicyState: Equatable {
    case idle
    case loading
    case ready
    case disabling
    case enabling
    case resetting
    case failed(PermanentCleanupPolicyFailure)

    var isBusy: Bool {
        switch self {
        case .loading, .disabling, .enabling, .resetting:
            true
        case .idle, .ready, .failed:
            false
        }
    }
}

enum PermanentCleanupPolicyFailure: Equatable {
    case confirmationRequired
    case service(PermanentCleanupPolicyServiceError)
    case unexpected
}
