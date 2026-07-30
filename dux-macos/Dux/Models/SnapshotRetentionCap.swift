import Foundation

enum SnapshotRetentionCapSourceModel: Equatable, Sendable {
    case `default`
    case stored
}

/// Path-free snapshot retention policy. Changing it never runs retention.
struct SnapshotRetentionCapModel: Equatable, Sendable {
    let capBytes: UInt64
    let source: SnapshotRetentionCapSourceModel
    let updatedAtUnixMilliseconds: Int64?
}

struct SnapshotRetentionCapUpdateResultModel: Equatable, Sendable {
    let settings: SnapshotRetentionCapModel
    let changed: Bool
}

enum SnapshotRetentionCapServiceError: Error, Equatable, Sendable {
    case closed
    case invalidRecordVersion
    case invalidClock
    case incompatibleSchema
    case retryable
    case unsafeStorage
    case budgetExceeded
    case corruptData
    case unavailable
    case outcomeUnknown
    case internalState
    case invalidResponse
}

enum SnapshotRetentionCapDraftError: Error, Equatable, Sendable {
    case invalidNumber
}

struct SnapshotRetentionCapDraft: Equatable, Sendable {
    var gib: String

    static let defaults = Self(
        capBytes: 2 * DiskPressurePolicyConfiguration.bytesPerGiB
    )

    init(capBytes: UInt64) {
        gib = ExactPolicyDecimal.formatGiB(capBytes)
    }

    init(gib: String) {
        self.gib = gib
    }

    func capBytes(
        decimalSeparator: String? = Locale.current.decimalSeparator
    ) throws -> UInt64 {
        guard let bytes = ExactPolicyDecimal.parseGiB(
            gib,
            decimalSeparator: decimalSeparator
        ) else {
            throw SnapshotRetentionCapDraftError.invalidNumber
        }
        return bytes
    }
}

enum SnapshotRetentionCapState: Equatable {
    case idle
    case loading
    case ready
    case saving
    case resetting
    case failed(SnapshotRetentionCapFailure)

    var isBusy: Bool {
        switch self {
        case .loading, .saving, .resetting: true
        case .idle, .ready, .failed: false
        }
    }

    var isFailed: Bool {
        if case .failed = self {
            true
        } else {
            false
        }
    }
}

enum SnapshotRetentionCapFailure: Equatable {
    case draft(SnapshotRetentionCapDraftError)
    case service(SnapshotRetentionCapServiceError)
    case unexpected
}
