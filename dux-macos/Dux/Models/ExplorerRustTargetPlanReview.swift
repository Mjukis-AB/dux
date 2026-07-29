import Foundation

enum ExplorerRustTargetPlanReviewMode: Equatable, Sendable {
    case permanentSafe

    var displayName: String {
        "Permanent-safe"
    }
}

enum ExplorerRustTargetPlanReviewWarning: Equatable, Sendable {
    case estimatedBytesUnverified
    case permanentRemovalCannotBeUndone

    var displayName: String {
        switch self {
        case .estimatedBytesUnverified:
            "The reclaimable-space estimate is not guaranteed."
        case .permanentRemovalCannotBeUndone:
            "Permanent removal cannot be undone."
        }
    }
}

enum ExplorerRustTargetPlanReviewPathEncoding: Equatable, Sendable {
    case unixBytes
    case windowsUTF16LittleEndian
}

struct ExplorerRustTargetPlanReviewPath: Equatable, Sendable {
    let encoding: ExplorerRustTargetPlanReviewPathEncoding
    let encodedBytes: Data
    let display: String
}

/// Transport-neutral contract-v34 record. Generated-FFI values are projected into this
/// app-owned shape at the EngineService boundary before strict validation.
struct ExplorerRustTargetPlanReviewRecord: Equatable, Sendable {
    let recordVersion: UInt32
    let planID: String
    let sourceScanID: String
    let candidateID: String
    let ruleID: String
    let ruleRevision: UInt32
    let category: ExplorerCandidateCategory
    let mode: ExplorerRustTargetPlanReviewMode
    let safety: ExplorerCandidateSafety
    let action: ExplorerCandidateAction
    let estimatedBytes: UInt64
    let newestMtime: ExplorerSnapshotTimestamp
    let minimumAgeSeconds: UInt64
    let minimumAgeNanoseconds: UInt32
    let itemCount: UInt16
    let pathCount: UInt16
    let warnings: [ExplorerRustTargetPlanReviewWarning]
    let createdAt: ExplorerSnapshotTimestamp
    let effectiveExpiresAt: ExplorerSnapshotTimestamp
    let scheduleEligible: Bool
    let target: ExplorerRustTargetPlanReviewPath
}

/// Display-only observation of a core-owned reviewed plan. None of these
/// values can be supplied back to an approval, journal, or executor boundary.
struct ExplorerRustTargetPlanReviewInfo: Equatable, Sendable {
    let planID: String
    let sourceScanID: String
    let candidateID: String
    let ruleID: String
    let ruleRevision: UInt32
    let category: ExplorerCandidateCategory
    let mode: ExplorerRustTargetPlanReviewMode
    let safety: ExplorerCandidateSafety
    let action: ExplorerCandidateAction
    let estimatedBytes: UInt64
    let newestMtime: ExplorerSnapshotTimestamp
    let minimumAgeSeconds: UInt64
    let minimumAgeNanoseconds: UInt32
    let itemCount: UInt16
    let pathCount: UInt16
    let warnings: [ExplorerRustTargetPlanReviewWarning]
    let createdAt: ExplorerSnapshotTimestamp
    let effectiveExpiresAt: ExplorerSnapshotTimestamp
    let scheduleEligible: Bool
    let target: ExplorerRustTargetPlanReviewPath
}

struct ExplorerRustTargetPlanReviewHandle: Equatable, Sendable {
    let id: UUID
    let info: ExplorerRustTargetPlanReviewInfo

    init(id: UUID, info: ExplorerRustTargetPlanReviewInfo) {
        self.id = id
        self.info = info
    }
}

enum ExplorerRustTargetPlanReviewState: Equatable, Sendable {
    case idle
    case preparing
    case ready(ExplorerRustTargetPlanReviewInfo)
    case failed(ExplorerRustTargetPlanReviewError)
    case expired
}

enum ExplorerRustTargetPlanReviewError: Error, Equatable, Sendable {
    case closed
    case reviewNotAcquired
    case parentReviewUnavailable
    case reviewExpired
    case candidateUnavailable
    case cargoNotEnrolled
    case activeProcesses
    case changedDuringReview
    case unsupportedPlatform
    case budgetExceeded
    case busy
    case unsafeStorage
    case corruptData
    case unavailable
    case invalidResponse

    var title: String {
        switch self {
        case .cargoNotEnrolled:
            "Cargo enrollment required"
        case .activeProcesses:
            "Rust tools are active"
        case .changedDuringReview:
            "Storage changed during review"
        case .parentReviewUnavailable, .reviewNotAcquired:
            "Snapshot review expired"
        case .reviewExpired:
            "Plan preview expired"
        case .unsupportedPlatform:
            "Plan preview unsupported"
        case .budgetExceeded, .busy:
            "Plan preview is busy"
        case .unsafeStorage, .corruptData:
            "Plan preview blocked"
        case .closed, .candidateUnavailable, .unavailable:
            "Plan preview unavailable"
        case .invalidResponse:
            "Plan preview was rejected"
        }
    }

    var detail: String {
        switch self {
        case .cargoNotEnrolled:
            "Enroll the direct Cargo executable in Settings, then prepare a new preview."
        case .activeProcesses:
            "Quit active Cargo or rustc processes, then explicitly check again."
        case .changedDuringReview:
            "The target or its Cargo metadata changed. Run a new scan before preparing another preview."
        case .parentReviewUnavailable, .reviewNotAcquired:
            "The retained snapshot review is no longer current. Reload a snapshot before trying again."
        case .reviewExpired:
            "This short-lived plan preview expired. Check again to prepare a fresh observation."
        case .unsupportedPlatform:
            "This deterministic Rust-target review is only available on supported macOS systems."
        case .budgetExceeded:
            "DUX refused to exceed its bounded review budget. Close another review and try again."
        case .busy:
            "Another bounded storage operation is active. Nothing was changed."
        case .unsafeStorage:
            "DUX cannot trust its review store. No plan preview was accepted."
        case .corruptData:
            "DUX rejected inconsistent durable review data. No plan preview was accepted."
        case .closed:
            "The storage engine is closing. No plan preview was accepted."
        case .candidateUnavailable:
            "This candidate is no longer available in its exact discovered state. Run a new scan."
        case .unavailable:
            "The deterministic plan preview is unavailable right now. Nothing was changed."
        case .invalidResponse:
            "DUX rejected inconsistent plan-review information instead of displaying it."
        }
    }
}

enum ExplorerRustTargetPlanReviewAdapter {
    static let recordVersion: UInt32 = 1
    static let maximumEncodedPathBytes = 65_536
    static let maximumDisplayPathBytes = maximumEncodedPathBytes * 4
    static let maximumValiditySeconds: UInt64 = 10 * 60

    private static let maximumIdentifierBytes = 128
    private static let maximumUnixSeconds: UInt64 = 253_402_300_799
    private static let expectedRuleID = "developer.rust.target"
    private static let expectedRuleRevision: UInt32 = 3
    private static let expectedMinimumAgeSeconds: UInt64 = 7 * 24 * 60 * 60
    private static let expectedMinimumAgeNanoseconds: UInt32 = 0
    private static let expectedWarnings: [ExplorerRustTargetPlanReviewWarning] = [
        .estimatedBytesUnverified,
        .permanentRemovalCannotBeUndone,
    ]

    static func map(
        _ raw: ExplorerRustTargetPlanReviewRecord,
        expectedScanID: String,
        expectedCandidateID: String,
        now: ExplorerSnapshotTimestamp
    ) throws -> ExplorerRustTargetPlanReviewInfo {
        guard
            raw.recordVersion == recordVersion,
            validIdentifier(raw.planID, prefix: "plan:"),
            validIdentifier(raw.sourceScanID, prefix: "scan:"),
            raw.sourceScanID == expectedScanID,
            validIdentifier(raw.candidateID, prefix: "candidate:"),
            raw.candidateID == expectedCandidateID,
            raw.ruleID == expectedRuleID,
            raw.ruleRevision == expectedRuleRevision,
            raw.category == .developerArtifact,
            raw.mode == .permanentSafe,
            raw.safety == .safeRegenerable,
            raw.action == .removeKnownRegenerableContents,
            validTimestamp(raw.newestMtime),
            raw.minimumAgeSeconds == expectedMinimumAgeSeconds,
            raw.minimumAgeNanoseconds == expectedMinimumAgeNanoseconds,
            satisfiesMinimumAge(
                newestMtime: raw.newestMtime,
                observedAt: raw.createdAt,
                minimumAgeSeconds: raw.minimumAgeSeconds,
                minimumAgeNanoseconds: raw.minimumAgeNanoseconds
            ),
            !raw.scheduleEligible,
            raw.itemCount == 1,
            raw.pathCount == 1,
            raw.warnings == expectedWarnings,
            validTimestamp(raw.createdAt),
            validTimestamp(raw.effectiveExpiresAt),
            compare(raw.createdAt, raw.effectiveExpiresAt) == .orderedAscending,
            compare(raw.createdAt, now) != .orderedDescending,
            compare(now, raw.effectiveExpiresAt) == .orderedAscending,
            isWithinMaximumValidity(
                start: raw.createdAt,
                end: raw.effectiveExpiresAt
            ),
            validPath(raw.target)
        else {
            throw ExplorerRustTargetPlanReviewError.invalidResponse
        }
        return ExplorerRustTargetPlanReviewInfo(
            planID: raw.planID,
            sourceScanID: raw.sourceScanID,
            candidateID: raw.candidateID,
            ruleID: raw.ruleID,
            ruleRevision: raw.ruleRevision,
            category: raw.category,
            mode: raw.mode,
            safety: raw.safety,
            action: raw.action,
            estimatedBytes: raw.estimatedBytes,
            newestMtime: raw.newestMtime,
            minimumAgeSeconds: raw.minimumAgeSeconds,
            minimumAgeNanoseconds: raw.minimumAgeNanoseconds,
            itemCount: raw.itemCount,
            pathCount: raw.pathCount,
            warnings: raw.warnings,
            createdAt: raw.createdAt,
            effectiveExpiresAt: raw.effectiveExpiresAt,
            scheduleEligible: raw.scheduleEligible,
            target: raw.target
        )
    }

    static func matches(
        _ info: ExplorerRustTargetPlanReviewInfo,
        candidate: ExplorerCandidateSummary
    ) -> Bool {
        info.candidateID == candidate.candidateID
            && info.ruleID == candidate.ruleID
            && info.ruleRevision == candidate.ruleRevision
            && info.category == candidate.category
            && info.safety == candidate.safety
            && info.action == candidate.action
            && info.estimatedBytes == candidate.estimatedBytes
            && candidate.newestMtime == info.newestMtime
            && info.itemCount == 1
            && info.pathCount == candidate.pathCount
            && info.scheduleEligible == candidate.ruleScheduleEligible
            && candidate.category == .developerArtifact
            && candidate.pathCount == 1
            && candidate.evidenceKinds
                == [.matchedPath, .requiredMarker, .requiredMarker, .minimumAge]
            && candidate.blockers == [.protectedPath]
            && candidate.status == .discovered
    }

    static func timestamp(for date: Date) -> ExplorerSnapshotTimestamp {
        let value = max(0, date.timeIntervalSince1970)
        let seconds = UInt64(value.rounded(.down))
        let fractional = value - Double(seconds)
        return ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: seconds,
            nanoseconds: UInt32(min(fractional * 1_000_000_000, 999_999_999))
        )
    }

    static func date(for timestamp: ExplorerSnapshotTimestamp) -> Date {
        Date(
            timeIntervalSince1970: Double(timestamp.secondsSinceUnixEpoch)
                + Double(timestamp.nanoseconds) / 1_000_000_000
        )
    }

    private static func validIdentifier(_ value: String, prefix: String) -> Bool {
        value.hasPrefix(prefix)
            && value.utf8.count > prefix.utf8.count
            && value.utf8.count <= maximumIdentifierBytes
            && value.unicodeScalars.allSatisfy {
                $0.isASCII
                    && (CharacterSet.alphanumerics.contains($0)
                        || "._-:".unicodeScalars.contains($0))
            }
    }

    private static func validTimestamp(_ value: ExplorerSnapshotTimestamp) -> Bool {
        value.secondsSinceUnixEpoch <= maximumUnixSeconds
            && value.nanoseconds < 1_000_000_000
    }

    private static func compare(
        _ lhs: ExplorerSnapshotTimestamp,
        _ rhs: ExplorerSnapshotTimestamp
    ) -> ComparisonResult {
        if lhs.secondsSinceUnixEpoch != rhs.secondsSinceUnixEpoch {
            return lhs.secondsSinceUnixEpoch < rhs.secondsSinceUnixEpoch
                ? .orderedAscending : .orderedDescending
        }
        if lhs.nanoseconds == rhs.nanoseconds {
            return .orderedSame
        }
        return lhs.nanoseconds < rhs.nanoseconds ? .orderedAscending : .orderedDescending
    }

    private static func isWithinMaximumValidity(
        start: ExplorerSnapshotTimestamp,
        end: ExplorerSnapshotTimestamp
    ) -> Bool {
        let (maximumEndSeconds, overflow) =
            start.secondsSinceUnixEpoch.addingReportingOverflow(maximumValiditySeconds)
        guard !overflow else {
            return false
        }
        let maximumEnd = ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: maximumEndSeconds,
            nanoseconds: start.nanoseconds
        )
        return compare(end, maximumEnd) != .orderedDescending
    }

    private static func satisfiesMinimumAge(
        newestMtime: ExplorerSnapshotTimestamp,
        observedAt: ExplorerSnapshotTimestamp,
        minimumAgeSeconds: UInt64,
        minimumAgeNanoseconds: UInt32
    ) -> Bool {
        let secondsFromNanoseconds = UInt64(minimumAgeNanoseconds / 1_000_000_000)
        let remainderNanoseconds = minimumAgeNanoseconds % 1_000_000_000
        let (baseSeconds, firstOverflow) =
            newestMtime.secondsSinceUnixEpoch.addingReportingOverflow(minimumAgeSeconds)
        let (secondsWithCarry, secondOverflow) =
            baseSeconds.addingReportingOverflow(secondsFromNanoseconds)
        let nanosecondTotal = UInt64(newestMtime.nanoseconds)
            + UInt64(remainderNanoseconds)
        let carry = nanosecondTotal / 1_000_000_000
        let (qualifiedSeconds, carryOverflow) =
            secondsWithCarry.addingReportingOverflow(carry)
        guard !firstOverflow, !secondOverflow, !carryOverflow else {
            return false
        }
        let qualifiedAt = ExplorerSnapshotTimestamp(
            secondsSinceUnixEpoch: qualifiedSeconds,
            nanoseconds: UInt32(nanosecondTotal % 1_000_000_000)
        )
        return compare(qualifiedAt, observedAt) != .orderedDescending
    }

    private static func validPath(_ path: ExplorerRustTargetPlanReviewPath) -> Bool {
        guard
            path.encoding == .unixBytes,
            path.encodedBytes.first == 0x2f,
            !path.encodedBytes.contains(0),
            path.encodedBytes.count <= maximumEncodedPathBytes,
            validRustTargetComponents(path.encodedBytes),
            !path.display.isEmpty,
            path.display.utf8.count <= maximumDisplayPathBytes,
            !path.display.unicodeScalars.contains(where: {
                unsafeDisplayScalar($0)
            })
        else {
            return false
        }
        return Data(path.display.utf8)
            == Data(expectedUnixDisplay(for: path.encodedBytes).utf8)
    }

    private static func validRustTargetComponents(_ bytes: Data) -> Bool {
        let components = bytes.split(
            separator: 0x2f,
            omittingEmptySubsequences: false
        )
        guard
            components.count >= 3,
            components.first?.isEmpty == true,
            components.dropFirst().allSatisfy({ !$0.isEmpty }),
            components.last == Data("target".utf8)
        else {
            return false
        }
        return components.dropFirst().allSatisfy { component in
            component != Data(".".utf8) && component != Data("..".utf8)
        }
    }

    private static func expectedUnixDisplay(for bytes: Data) -> String {
        if
            let exact = String(data: bytes, encoding: .utf8),
            !exact.unicodeScalars.contains(where: {
                unsafeDisplayScalar($0)
            })
        {
            return exact
        }
        return "unix-bytes:" + bytes.map { byte in
            switch byte {
            case 0x20 ... 0x7e where byte != 0x5c:
                String(UnicodeScalar(byte))
            case 0x5c:
                "\\\\"
            default:
                String(format: "\\x%02x", byte)
            }
        }.joined()
    }

    private static func unsafeDisplayScalar(_ scalar: UnicodeScalar) -> Bool {
        if scalar.properties.generalCategory == .control {
            return true
        }
        return switch scalar.value {
        case 0x00ad, 0x034f,
             0x0600 ... 0x0605, 0x061c, 0x06dd, 0x070f,
             0x0890 ... 0x0891, 0x08e2,
             0x115f ... 0x1160,
             0x17b4 ... 0x17b5,
             0x180b ... 0x180f,
             0x200b ... 0x200f,
             0x202a ... 0x202e,
             0x2060 ... 0x206f,
             0x3164,
             0xfe00 ... 0xfe0f, 0xfeff, 0xffa0,
             0xfff0 ... 0xfffb,
             0x110bd, 0x110cd,
             0x13430 ... 0x1343f,
             0x1bca0 ... 0x1bca3,
             0x1d173 ... 0x1d17a,
             0xe0000 ... 0xe0fff:
            true
        default:
            false
        }
    }
}

/// App-owned confirmation evidence for one exact opaque plan-review handle.
/// The displayed fields remain observations; only the controller-retained
/// handle can cross the core's consume-once execution edge.
struct ExplorerRustTargetCleanupConfirmation: Equatable, Sendable {
    let generation: UInt64
    let reviewHandleID: UUID
    let info: ExplorerRustTargetPlanReviewInfo
}

enum ExplorerRustTargetCleanupPhase: Equatable, Sendable {
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

enum ExplorerRustTargetCleanupFailure: Equatable, Sendable {
    case parentReviewUnavailable
    case reviewExpired
    case changedSincePlan
    case budgetExceeded
    case busy
    case unsafeStorage
    case incompatibleSchema
    case corruptData
    case outcomeUnknown
    case unavailable
    case internalState

    var title: String {
        switch self {
        case .changedSincePlan:
            "Storage changed since review"
        case .reviewExpired, .parentReviewUnavailable:
            "Review expired"
        case .budgetExceeded, .busy:
            "Cleanup is busy"
        case .outcomeUnknown:
            "Cleanup outcome is unknown"
        case .unsafeStorage, .incompatibleSchema, .corruptData:
            "Cleanup is blocked"
        case .unavailable, .internalState:
            "Cleanup is unavailable"
        }
    }

    var detail: String {
        switch self {
        case .changedSincePlan:
            "DUX refused the changed target. Run a new scan and review a fresh plan before trying again."
        case .reviewExpired, .parentReviewUnavailable:
            "The exact reviewed authority ended before cleanup could start. Prepare and confirm a fresh plan."
        case .budgetExceeded:
            "DUX refused to exceed its bounded cleanup budget. Nothing should be retried automatically."
        case .busy:
            "Another cleanup operation is active. Check Cleanup History before preparing a new plan."
        case .outcomeUnknown:
            "DUX cannot prove whether the filesystem effect completed. Do not retry; inspect Cleanup History after restarting DUX."
        case .unsafeStorage:
            "DUX cannot trust its cleanup store. No new cleanup should be attempted."
        case .incompatibleSchema:
            "Cleanup history uses an incompatible schema. Update DUX before attempting cleanup."
        case .corruptData:
            "DUX rejected inconsistent cleanup state. No retry was attempted."
        case .unavailable:
            "The cleanup engine is unavailable. No retry was attempted."
        case .internalState:
            "DUX rejected an inconsistent cleanup response. No retry was attempted."
        }
    }
}

enum ExplorerRustTargetCleanupStartError: Error, Equatable, Sendable {
    case closed
    case reviewUnavailable
    case parentReviewUnavailable
    case reviewExpired
    case changedSincePlan
    case cancelledBeforeStart
    case budgetExceeded
    case queueFull
    case busy
    case unsafeStorage
    case incompatibleSchema
    case corruptData
    case outcomeUnknown
    case unavailable
    case invalidResponse

    var failure: ExplorerRustTargetCleanupFailure {
        switch self {
        case .parentReviewUnavailable:
            .parentReviewUnavailable
        case .reviewExpired, .reviewUnavailable:
            .reviewExpired
        case .changedSincePlan:
            .changedSincePlan
        case .budgetExceeded:
            .budgetExceeded
        case .queueFull, .busy:
            .busy
        case .unsafeStorage:
            .unsafeStorage
        case .incompatibleSchema:
            .incompatibleSchema
        case .corruptData:
            .corruptData
        case .outcomeUnknown:
            .outcomeUnknown
        case .closed, .cancelledBeforeStart, .unavailable:
            .unavailable
        case .invalidResponse:
            .internalState
        }
    }
}

enum ExplorerRustTargetCleanupTaskError: Error, Equatable, Sendable {
    case closed
    case taskUnavailable
    case invalidResponse
}

enum ExplorerRustTargetCleanupCancelOutcome: Equatable, Sendable {
    case cancelledBeforeStart
    case requested
    case alreadyRequested
    case alreadyTerminal
}

struct ExplorerRustTargetCleanupResult: Equatable, Sendable {
    let sessionID: String
    let status: CleanupHistorySessionStatus
    let removedEntries: UInt64
    let removedLogicalBytes: UInt64
    let verifiedCapacityDeltaBytes: Int64?
}

struct ExplorerRustTargetCleanupPoll: Equatable, Sendable {
    let phase: ExplorerRustTargetCleanupPhase
    let cancellationRequested: Bool
    let revision: UInt64
    let failure: ExplorerRustTargetCleanupFailure?
    let result: ExplorerRustTargetCleanupResult?
}

enum ExplorerRustTargetCleanupState: Equatable, Sendable {
    case idle
    case starting(ExplorerRustTargetPlanReviewInfo)
    case observing(ExplorerRustTargetPlanReviewInfo, ExplorerRustTargetCleanupPoll)
    case startFailed(ExplorerRustTargetPlanReviewInfo, ExplorerRustTargetCleanupFailure)
    case observationFailed(ExplorerRustTargetPlanReviewInfo)

    var isActive: Bool {
        switch self {
        case .starting:
            true
        case let .observing(_, poll):
            !poll.phase.isTerminal
        case .idle, .startFailed, .observationFailed:
            false
        }
    }
}

protocol DuxRustTargetCleanupTask: AnyObject, Sendable {
    func poll() async throws -> ExplorerRustTargetCleanupPoll
    func requestCancellation() async throws -> ExplorerRustTargetCleanupCancelOutcome
}

protocol ExplorerRustTargetCleanupPollingClock: Sendable {
    func sleepUntilNextPoll() async throws
}

struct ContinuousExplorerRustTargetCleanupPollingClock:
    ExplorerRustTargetCleanupPollingClock
{
    func sleepUntilNextPoll() async throws {
        try await Task.sleep(for: .milliseconds(250))
    }
}
