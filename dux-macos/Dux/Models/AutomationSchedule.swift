import Foundation

enum AutomationScheduleModelError: Error, Equatable, Sendable {
    case invalidRecordVersion
    case invalidScheduleID
    case invalidRuleReference
    case invalidRevision
    case invalidLimits
    case invalidNotificationCount
    case invalidTimestamp
    case invalidExclusions
    case exclusionsRequireCategoryScope
    case enabledDraftRejected
    case activeAutomationRejected
    case duplicateScheduleID
    case nonCanonicalDraftOrder
}

struct DuxAutomationScheduleRuleReference: Equatable, Hashable, Sendable, Comparable {
    static let maximumRuleIDBytes = 128

    let ruleID: String
    let ruleRevision: UInt32

    init(ruleID: String, ruleRevision: UInt32) throws {
        guard
            !ruleID.isEmpty,
            ruleID.utf8.count <= Self.maximumRuleIDBytes,
            Self.isValidRuleID(ruleID),
            ruleRevision > 0
        else {
            throw AutomationScheduleModelError.invalidRuleReference
        }
        self.ruleID = ruleID
        self.ruleRevision = ruleRevision
    }

    static func < (
        lhs: DuxAutomationScheduleRuleReference,
        rhs: DuxAutomationScheduleRuleReference
    ) -> Bool {
        if lhs.ruleID != rhs.ruleID {
            return lhs.ruleID < rhs.ruleID
        }
        return lhs.ruleRevision < rhs.ruleRevision
    }

    private static func isValidRuleID(_ value: String) -> Bool {
        for segment in value.split(separator: ".", omittingEmptySubsequences: false) {
            guard !segment.isEmpty else {
                return false
            }
            for (index, byte) in segment.utf8.enumerated() {
                let isBoundary = index == 0 || index == segment.utf8.count - 1
                let isLowercaseOrDigit = (0x61 ... 0x7A).contains(byte)
                    || (0x30 ... 0x39).contains(byte)
                if isBoundary {
                    guard isLowercaseOrDigit else {
                        return false
                    }
                } else {
                    guard isLowercaseOrDigit || byte == 0x5F || byte == 0x2D else {
                        return false
                    }
                }
            }
        }
        return true
    }
}

enum DuxAutomationScheduleScope: Equatable, Hashable, Sendable {
    case rule(DuxAutomationScheduleRuleReference)
    case category(ExplorerCandidateCategory)

    var displayName: String {
        switch self {
        case let .rule(rule):
            "\(rule.ruleID) r\(rule.ruleRevision)"
        case let .category(category):
            category.displayName
        }
    }
}

enum DuxAutomationScheduleCadence: String, CaseIterable, Identifiable, Sendable {
    case weekly
    case monthly
    case lowDiskOnly

    var id: Self { self }

    var displayName: String {
        switch self {
        case .weekly: "Weekly"
        case .monthly: "Monthly"
        case .lowDiskOnly: "Low disk only"
        }
    }
}

enum DuxAutomationScheduleConfirmationMode: String, CaseIterable, Sendable {
    case requireConfirmation
    case fullyAutomatic

    var displayName: String {
        switch self {
        case .requireConfirmation: "Confirmation required"
        case .fullyAutomatic: "Fully automatic"
        }
    }
}

enum AutomationScheduleDefaults {
    static let minimumAgeSeconds: UInt64 = 30 * 24 * 60 * 60
    static let maximumBytesPerRun: UInt64 =
        25 * DiskPressurePolicyConfiguration.bytesPerGiB
    static let notificationRuns: UInt8 = 3
    static let notifyBeforeRun = true
    static let cadence = DuxAutomationScheduleCadence.monthly
    static let confirmationMode =
        DuxAutomationScheduleConfirmationMode.requireConfirmation
}

/// Path-free, read-only native representation of a stored schedule draft.
/// This type deliberately cannot authorize or execute cleanup.
struct AutomationScheduleDraftModel: Equatable, Identifiable, Sendable {
    static let maximumScheduleIDBytes = 128
    static let maximumExclusions = 32
    static let maximumMinimumAgeSeconds: UInt64 = 3_153_600_000
    static let maximumStoredBytes = UInt64(Int64.max)

    var id: String { scheduleID }

    let scheduleID: String
    let scope: DuxAutomationScheduleScope
    let cadence: DuxAutomationScheduleCadence
    let minimumAgeSeconds: UInt64
    let minimumReclaimableBytes: UInt64
    let maximumBytesPerRun: UInt64
    let exclusions: [DuxAutomationScheduleRuleReference]
    let notifyBeforeRun: Bool
    let notifyBeforeRunsRemaining: UInt8
    let confirmationMode: DuxAutomationScheduleConfirmationMode
    let enabled: Bool
    let revision: UInt64
    let createdAtUnixMilliseconds: Int64
    let updatedAtUnixMilliseconds: Int64

    init(
        scheduleID: String,
        scope: DuxAutomationScheduleScope,
        cadence: DuxAutomationScheduleCadence,
        minimumAgeSeconds: UInt64,
        minimumReclaimableBytes: UInt64,
        maximumBytesPerRun: UInt64,
        exclusions: [DuxAutomationScheduleRuleReference],
        notifyBeforeRun: Bool,
        notifyBeforeRunsRemaining: UInt8,
        confirmationMode: DuxAutomationScheduleConfirmationMode,
        enabled: Bool,
        revision: UInt64,
        createdAtUnixMilliseconds: Int64,
        updatedAtUnixMilliseconds: Int64
    ) throws {
        guard
            !scheduleID.isEmpty,
            scheduleID.utf8.count <= Self.maximumScheduleIDBytes,
            scheduleID.utf8.allSatisfy(Self.isScheduleIDByte)
        else {
            throw AutomationScheduleModelError.invalidScheduleID
        }
        guard !enabled else {
            throw AutomationScheduleModelError.enabledDraftRejected
        }
        guard revision > 0 else {
            throw AutomationScheduleModelError.invalidRevision
        }
        guard
            minimumAgeSeconds <= Self.maximumMinimumAgeSeconds,
            minimumReclaimableBytes <= Self.maximumStoredBytes,
            maximumBytesPerRun > 0,
            maximumBytesPerRun <= Self.maximumStoredBytes
        else {
            throw AutomationScheduleModelError.invalidLimits
        }
        guard
            notifyBeforeRunsRemaining <= AutomationScheduleDefaults.notificationRuns,
            notifyBeforeRun || notifyBeforeRunsRemaining == 0
        else {
            throw AutomationScheduleModelError.invalidNotificationCount
        }
        guard
            createdAtUnixMilliseconds >= 0,
            updatedAtUnixMilliseconds >= createdAtUnixMilliseconds
        else {
            throw AutomationScheduleModelError.invalidTimestamp
        }
        guard
            exclusions.count <= Self.maximumExclusions,
            exclusions == exclusions.sorted(),
            Set(exclusions).count == exclusions.count
        else {
            throw AutomationScheduleModelError.invalidExclusions
        }
        guard !scope.isExactRule || exclusions.isEmpty else {
            throw AutomationScheduleModelError.exclusionsRequireCategoryScope
        }

        self.scheduleID = scheduleID
        self.scope = scope
        self.cadence = cadence
        self.minimumAgeSeconds = minimumAgeSeconds
        self.minimumReclaimableBytes = minimumReclaimableBytes
        self.maximumBytesPerRun = maximumBytesPerRun
        self.exclusions = exclusions
        self.notifyBeforeRun = notifyBeforeRun
        self.notifyBeforeRunsRemaining = notifyBeforeRunsRemaining
        self.confirmationMode = confirmationMode
        self.enabled = enabled
        self.revision = revision
        self.createdAtUnixMilliseconds = createdAtUnixMilliseconds
        self.updatedAtUnixMilliseconds = updatedAtUnixMilliseconds
    }

    private static func isScheduleIDByte(_ byte: UInt8) -> Bool {
        (0x41 ... 0x5A).contains(byte)
            || (0x61 ... 0x7A).contains(byte)
            || (0x30 ... 0x39).contains(byte)
            || byte == 0x2E
            || byte == 0x5F
            || byte == 0x2D
            || byte == 0x3A
    }
}

private extension DuxAutomationScheduleScope {
    var isExactRule: Bool {
        if case .rule = self {
            true
        } else {
            false
        }
    }
}

/// The first M8 client accepts only the non-executing automation state. A newer
/// backend cannot silently activate behavior through this older Settings UI.
struct AutomationScheduleOverviewModel: Equatable, Sendable {
    static let recordVersion: UInt32 = 1
    static let maximumDraftCount = 64
    static let maximumEligibleRuleCount: UInt16 = 256

    let recordVersion: UInt32
    let globalEnabled: Bool
    let executionAvailable: Bool
    let eligibleRuleCount: UInt16
    let disabledDrafts: [AutomationScheduleDraftModel]

    init(
        recordVersion: UInt32,
        globalEnabled: Bool,
        executionAvailable: Bool,
        eligibleRuleCount: UInt16,
        disabledDrafts: [AutomationScheduleDraftModel]
    ) throws {
        guard recordVersion == Self.recordVersion else {
            throw AutomationScheduleModelError.invalidRecordVersion
        }
        guard !globalEnabled, !executionAvailable else {
            throw AutomationScheduleModelError.activeAutomationRejected
        }
        guard
            eligibleRuleCount <= Self.maximumEligibleRuleCount,
            disabledDrafts.count <= Self.maximumDraftCount
        else {
            throw AutomationScheduleModelError.invalidLimits
        }
        let ids = disabledDrafts.map(\.scheduleID)
        guard Set(ids).count == ids.count else {
            throw AutomationScheduleModelError.duplicateScheduleID
        }
        let canonicalDrafts = disabledDrafts.sorted { lhs, rhs in
            if lhs.updatedAtUnixMilliseconds != rhs.updatedAtUnixMilliseconds {
                return lhs.updatedAtUnixMilliseconds > rhs.updatedAtUnixMilliseconds
            }
            return lhs.scheduleID < rhs.scheduleID
        }
        guard disabledDrafts == canonicalDrafts else {
            throw AutomationScheduleModelError.nonCanonicalDraftOrder
        }

        self.recordVersion = recordVersion
        self.globalEnabled = globalEnabled
        self.executionAvailable = executionAvailable
        self.eligibleRuleCount = eligibleRuleCount
        self.disabledDrafts = disabledDrafts
    }

    private init(unavailable _: Void) {
        recordVersion = Self.recordVersion
        globalEnabled = false
        executionAvailable = false
        eligibleRuleCount = 0
        disabledDrafts = []
    }

    static let unavailable = Self(unavailable: ())
}
