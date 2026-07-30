import Foundation

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
