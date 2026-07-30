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
