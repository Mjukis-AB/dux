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
    case invalidGlobalControl
    case invalidScheduleState
    case invalidRecurrence
    case duplicateScheduleID
    case nonCanonicalScheduleOrder
    case invalidEligibilityAssessment
    case duplicateEligibilityReason
    case nonCanonicalEligibilityReasons
    case eligibilityAssessmentMismatch
    case invalidHistorySuggestion
    case invalidHistorySuggestionFeed
    case duplicateHistorySuggestionRule
    case nonCanonicalHistorySuggestionOrder
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

/// A bounded, history-only idea for an exact shipped rule revision. It carries
/// no schedule configuration or execution authority.
struct AutomationScheduleHistorySuggestionModel: Equatable, Identifiable, Sendable {
    static let recordVersion: UInt32 = 1

    var id: DuxAutomationScheduleRuleReference { rule }

    let rank: UInt16
    let rule: DuxAutomationScheduleRuleReference
    let successfulManualRunCount: UInt16
    let manualRegrowthCycleCount: UInt16
    let latestManualAttemptAtUnixMilliseconds: Int64
    let latestRegrowthAtUnixMilliseconds: Int64

    init(
        recordVersion: UInt32,
        rank: UInt16,
        rule: DuxAutomationScheduleRuleReference,
        successfulManualRunCount: UInt16,
        manualRegrowthCycleCount: UInt16,
        latestManualAttemptAtUnixMilliseconds: Int64,
        latestRegrowthAtUnixMilliseconds: Int64
    ) throws {
        guard
            recordVersion == Self.recordVersion,
            rank > 0,
            rank <= AutomationScheduleHistorySuggestionFeedModel.maximumSuggestions,
            successfulManualRunCount >= 2,
            successfulManualRunCount
            <= AutomationScheduleHistorySuggestionFeedModel.maximumSourceSessions,
            manualRegrowthCycleCount > 0,
            manualRegrowthCycleCount <= successfulManualRunCount,
            latestManualAttemptAtUnixMilliseconds >= 0,
            latestRegrowthAtUnixMilliseconds >= 0
        else {
            throw AutomationScheduleModelError.invalidHistorySuggestion
        }

        self.rank = rank
        self.rule = rule
        self.successfulManualRunCount = successfulManualRunCount
        self.manualRegrowthCycleCount = manualRegrowthCycleCount
        self.latestManualAttemptAtUnixMilliseconds = latestManualAttemptAtUnixMilliseconds
        self.latestRegrowthAtUnixMilliseconds = latestRegrowthAtUnixMilliseconds
    }
}

/// Bounded, read-only projection of already-stored manual cleanup history.
/// Loading this feed never scans storage and cannot create or run a schedule.
struct AutomationScheduleHistorySuggestionFeedModel: Equatable, Sendable {
    static let recordVersion: UInt32 = 1
    static let derivationRevision: UInt32 = 1
    static let maximumSourceSessions: UInt16 = 32
    static let maximumSuggestions: UInt16 = 12
    static let maximumQualifyingRules: UInt16 = 256

    let sourceSessionCount: UInt16
    let hasOlderSourceSessions: Bool
    let qualifyingRuleCount: UInt16
    let suggestions: [AutomationScheduleHistorySuggestionModel]

    init(
        recordVersion: UInt32,
        derivationRevision: UInt32,
        sourceSessionCount: UInt16,
        hasOlderSourceSessions: Bool,
        qualifyingRuleCount: UInt16,
        suggestions: [AutomationScheduleHistorySuggestionModel]
    ) throws {
        guard
            recordVersion == Self.recordVersion,
            derivationRevision == Self.derivationRevision,
            sourceSessionCount <= Self.maximumSourceSessions,
            suggestions.count <= Int(Self.maximumSuggestions),
            qualifyingRuleCount <= Self.maximumQualifyingRules,
            suggestions.count
            == min(Int(qualifyingRuleCount), Int(Self.maximumSuggestions)),
            !hasOlderSourceSessions || sourceSessionCount == Self.maximumSourceSessions,
            sourceSessionCount >= 2 || suggestions.isEmpty,
            suggestions.allSatisfy({ suggestion in
                suggestion.successfulManualRunCount <= sourceSessionCount
            })
        else {
            throw AutomationScheduleModelError.invalidHistorySuggestionFeed
        }

        let rules = suggestions.map(\.rule)
        guard Set(rules).count == rules.count else {
            throw AutomationScheduleModelError.duplicateHistorySuggestionRule
        }
        let expectedRanks = suggestions.indices.map { UInt16($0 + 1) }
        guard suggestions.map(\.rank) == expectedRanks else {
            throw AutomationScheduleModelError.nonCanonicalHistorySuggestionOrder
        }
        guard suggestions == suggestions.sorted(by: Self.isOrderedBefore) else {
            throw AutomationScheduleModelError.nonCanonicalHistorySuggestionOrder
        }

        self.sourceSessionCount = sourceSessionCount
        self.hasOlderSourceSessions = hasOlderSourceSessions
        self.qualifyingRuleCount = qualifyingRuleCount
        self.suggestions = suggestions
    }

    private static func isOrderedBefore(
        _ lhs: AutomationScheduleHistorySuggestionModel,
        _ rhs: AutomationScheduleHistorySuggestionModel
    ) -> Bool {
        if lhs.latestRegrowthAtUnixMilliseconds != rhs.latestRegrowthAtUnixMilliseconds {
            return lhs.latestRegrowthAtUnixMilliseconds > rhs.latestRegrowthAtUnixMilliseconds
        }
        if lhs.successfulManualRunCount != rhs.successfulManualRunCount {
            return lhs.successfulManualRunCount > rhs.successfulManualRunCount
        }
        if lhs.manualRegrowthCycleCount != rhs.manualRegrowthCycleCount {
            return lhs.manualRegrowthCycleCount > rhs.manualRegrowthCycleCount
        }
        return lhs.rule < rhs.rule
    }

    private init(unavailable _: Void) {
        sourceSessionCount = 0
        hasOlderSourceSessions = false
        qualifyingRuleCount = 0
        suggestions = []
    }

    static let unavailable = Self(unavailable: ())
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

enum DuxAutomationGlobalControlSource: Equatable, Sendable {
    case `default`
    case stored
}

/// Dedicated automation master control. This is persisted consent state only;
/// it is never evidence that cleanup is currently executable.
struct AutomationGlobalControlModel: Equatable, Sendable {
    static let recordVersion: UInt32 = 1

    let enabled: Bool
    let source: DuxAutomationGlobalControlSource
    let revision: UInt64
    let updatedAtUnixMilliseconds: Int64?

    init(
        enabled: Bool,
        source: DuxAutomationGlobalControlSource,
        revision: UInt64,
        updatedAtUnixMilliseconds: Int64?
    ) throws {
        switch source {
        case .default:
            guard !enabled else {
                throw AutomationScheduleModelError.invalidGlobalControl
            }
            if revision == 0 {
                guard updatedAtUnixMilliseconds == nil else {
                    throw AutomationScheduleModelError.invalidGlobalControl
                }
            } else {
                guard Self.isValidTimestamp(updatedAtUnixMilliseconds) else {
                    throw AutomationScheduleModelError.invalidGlobalControl
                }
            }
        case .stored:
            guard revision > 0, Self.isValidTimestamp(updatedAtUnixMilliseconds) else {
                throw AutomationScheduleModelError.invalidGlobalControl
            }
        }

        self.enabled = enabled
        self.source = source
        self.revision = revision
        self.updatedAtUnixMilliseconds = updatedAtUnixMilliseconds
    }

    private static func isValidTimestamp(_ value: Int64?) -> Bool {
        value.map { (0 ... AutomationScheduleModel.maximumUnixMilliseconds).contains($0) }
            == true
    }
}

enum DuxAutomationSchedulePauseReason: Equatable, Hashable, Sendable {
    case user
    case failure

    var displayName: String {
        switch self {
        case .user: "Paused by you"
        case .failure: "Paused after a failure"
        }
    }
}

enum DuxAutomationScheduleState: Equatable, Hashable, Sendable {
    case disabled
    case enabled
    case paused(DuxAutomationSchedulePauseReason)

    var displayName: String {
        switch self {
        case .disabled: "Disabled"
        case .enabled: "Enabled"
        case let .paused(reason): reason.displayName
        }
    }
}

/// Core-computed UTC recurrence. Native code may present these instants but
/// must never derive, advance, or persist a replacement recurrence.
struct AutomationScheduleRecurrenceModel: Equatable, Sendable {
    static let recordVersion: UInt32 = 1
    static let policyRevision: UInt32 = 1

    let cursorRevision: UInt64
    let recurrencePolicyRevision: UInt32
    let anchorAtUnixMilliseconds: Int64
    let nextOccurrenceOrdinal: UInt64
    let nextRunAtUnixMilliseconds: Int64

    init(
        recordVersion: UInt32,
        cursorRevision: UInt64,
        recurrencePolicyRevision: UInt32,
        anchorAtUnixMilliseconds: Int64,
        nextOccurrenceOrdinal: UInt64,
        nextRunAtUnixMilliseconds: Int64
    ) throws {
        guard
            recordVersion == Self.recordVersion,
            cursorRevision > 0,
            recurrencePolicyRevision == Self.policyRevision,
            nextOccurrenceOrdinal > 0,
            (0 ... AutomationScheduleModel.maximumUnixMilliseconds)
            .contains(anchorAtUnixMilliseconds),
            (0 ... AutomationScheduleModel.maximumUnixMilliseconds)
            .contains(nextRunAtUnixMilliseconds),
            nextRunAtUnixMilliseconds > anchorAtUnixMilliseconds
        else {
            throw AutomationScheduleModelError.invalidRecurrence
        }

        self.cursorRevision = cursorRevision
        self.recurrencePolicyRevision = recurrencePolicyRevision
        self.anchorAtUnixMilliseconds = anchorAtUnixMilliseconds
        self.nextOccurrenceOrdinal = nextOccurrenceOrdinal
        self.nextRunAtUnixMilliseconds = nextRunAtUnixMilliseconds
    }
}

/// Path-free native representation of stored automation configuration and
/// activation state. This type deliberately cannot authorize or execute cleanup.
struct AutomationScheduleModel: Equatable, Identifiable, Sendable {
    static let maximumScheduleIDBytes = 128
    static let maximumExclusions = 32
    static let maximumMinimumAgeSeconds: UInt64 = 3_153_600_000
    static let maximumStoredBytes = UInt64(Int64.max)
    static let maximumUnixMilliseconds: Int64 = 253_402_300_799_999

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
    let state: DuxAutomationScheduleState
    let recurrence: AutomationScheduleRecurrenceModel?
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
        state: DuxAutomationScheduleState,
        recurrence: AutomationScheduleRecurrenceModel?,
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
            (0 ... Self.maximumUnixMilliseconds).contains(createdAtUnixMilliseconds),
            (createdAtUnixMilliseconds ... Self.maximumUnixMilliseconds)
            .contains(updatedAtUnixMilliseconds)
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
        switch state {
        case .disabled:
            guard recurrence == nil else {
                throw AutomationScheduleModelError.invalidScheduleState
            }
        case .enabled, .paused:
            guard cadence != .lowDiskOnly, recurrence != nil else {
                throw AutomationScheduleModelError.invalidScheduleState
            }
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
        self.state = state
        self.recurrence = recurrence
        self.revision = revision
        self.createdAtUnixMilliseconds = createdAtUnixMilliseconds
        self.updatedAtUnixMilliseconds = updatedAtUnixMilliseconds
    }

    static func isScheduleIDByte(_ byte: UInt8) -> Bool {
        (0x41 ... 0x5A).contains(byte)
            || (0x61 ... 0x7A).contains(byte)
            || (0x30 ... 0x39).contains(byte)
            || byte == 0x2E
            || byte == 0x5F
            || byte == 0x2D
            || byte == 0x3A
    }
}

enum DuxAutomationScheduleEligibilityStatus: Equatable, Sendable {
    case blockedByStaticPolicy
    case awaitingRuntimeEvidence
}

enum DuxAutomationScheduleEligibilityReason: Int, CaseIterable, Sendable, Comparable, Hashable {
    case scopeRuleNotShipped
    case scopeRuleRevisionNotCurrent
    case scopeRuleNotSafeRegenerable
    case scopeRuleActionNotPermanentSafe
    case scopeRuleNotMarkedScheduleEligible
    case categoryHasNoScheduleEligibleRules
    case allScheduleEligibleRulesExcluded
    case exclusionRuleNotShipped
    case exclusionRuleRevisionNotCurrent

    static func < (
        lhs: DuxAutomationScheduleEligibilityReason,
        rhs: DuxAutomationScheduleEligibilityReason
    ) -> Bool {
        lhs.rawValue < rhs.rawValue
    }

    var explanation: String {
        switch self {
        case .scopeRuleNotShipped:
            "The selected rule is not shipped by this version of DUX."
        case .scopeRuleRevisionNotCurrent:
            "The selected rule revision is no longer current."
        case .scopeRuleNotSafeRegenerable:
            "The selected rule is not classified as safely regenerable."
        case .scopeRuleActionNotPermanentSafe:
            "The selected rule does not use the permanent-safe cleanup action."
        case .scopeRuleNotMarkedScheduleEligible:
            "The selected rule is not approved by shipped policy for scheduling."
        case .categoryHasNoScheduleEligibleRules:
            "This category has no rules approved by shipped policy for scheduling."
        case .allScheduleEligibleRulesExcluded:
            "Every schedule-eligible rule in this category is excluded."
        case .exclusionRuleNotShipped:
            "An excluded rule is not shipped by this version of DUX."
        case .exclusionRuleRevisionNotCurrent:
            "An excluded rule revision is no longer current."
        }
    }
}

/// Static policy-only assessment. It contains no runtime evidence and cannot
/// represent an eligible, runnable, enabled, or approved schedule.
struct AutomationScheduleEligibilityModel: Equatable, Sendable {
    static let recordVersion: UInt32 = 1
    static let policyRevision: UInt32 = 1
    static let maximumReasons = 16

    let scheduleID: String
    let scheduleRevision: UInt64
    let status: DuxAutomationScheduleEligibilityStatus
    let includedStaticallyEligibleRuleCount: UInt16
    let reasons: [DuxAutomationScheduleEligibilityReason]

    init(
        recordVersion: UInt32,
        policyRevision: UInt32,
        scheduleID: String,
        scheduleRevision: UInt64,
        status: DuxAutomationScheduleEligibilityStatus,
        includedStaticallyEligibleRuleCount: UInt16,
        reasons: [DuxAutomationScheduleEligibilityReason]
    ) throws {
        guard
            recordVersion == Self.recordVersion,
            policyRevision == Self.policyRevision,
            !scheduleID.isEmpty,
            scheduleID.utf8.count <= AutomationScheduleModel.maximumScheduleIDBytes,
            scheduleID.utf8.allSatisfy(AutomationScheduleModel.isScheduleIDByte),
            scheduleRevision > 0,
            includedStaticallyEligibleRuleCount
            <= AutomationScheduleOverviewModel.maximumEligibleRuleCount,
            reasons.count <= Self.maximumReasons
        else {
            throw AutomationScheduleModelError.invalidEligibilityAssessment
        }
        guard Set(reasons).count == reasons.count else {
            throw AutomationScheduleModelError.duplicateEligibilityReason
        }
        guard reasons == reasons.sorted() else {
            throw AutomationScheduleModelError.nonCanonicalEligibilityReasons
        }
        switch status {
        case .blockedByStaticPolicy:
            guard !reasons.isEmpty else {
                throw AutomationScheduleModelError.invalidEligibilityAssessment
            }
        case .awaitingRuntimeEvidence:
            guard includedStaticallyEligibleRuleCount > 0, reasons.isEmpty else {
                throw AutomationScheduleModelError.invalidEligibilityAssessment
            }
        }
        self.scheduleID = scheduleID
        self.scheduleRevision = scheduleRevision
        self.status = status
        self.includedStaticallyEligibleRuleCount = includedStaticallyEligibleRuleCount
        self.reasons = reasons
    }

    var statusLabel: String {
        switch status {
        case .blockedByStaticPolicy:
            "Blocked by shipped safety policy"
        case .awaitingRuntimeEvidence:
            "Static checks passed; runtime checks not evaluated"
        }
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

/// Full, bounded activation-management graph. `executionAvailable` is a
/// separate hard gate and remains false for this checkpoint.
struct AutomationScheduleOverviewModel: Equatable, Sendable {
    static let recordVersion: UInt32 = 3
    static let maximumScheduleCount = 64
    static let maximumEligibleRuleCount: UInt16 = 256

    let recordVersion: UInt32
    let globalControl: AutomationGlobalControlModel
    let executionAvailable: Bool
    let eligibleRuleCount: UInt16
    let schedules: [AutomationScheduleModel]
    let scheduleEligibility: [AutomationScheduleEligibilityModel]

    init(
        recordVersion: UInt32,
        globalControl: AutomationGlobalControlModel,
        executionAvailable: Bool,
        eligibleRuleCount: UInt16,
        schedules: [AutomationScheduleModel],
        scheduleEligibility: [AutomationScheduleEligibilityModel] = []
    ) throws {
        guard recordVersion == Self.recordVersion else {
            throw AutomationScheduleModelError.invalidRecordVersion
        }
        guard !executionAvailable else {
            throw AutomationScheduleModelError.invalidScheduleState
        }
        guard
            eligibleRuleCount <= Self.maximumEligibleRuleCount,
            schedules.count <= Self.maximumScheduleCount,
            scheduleEligibility.count == schedules.count
        else {
            throw AutomationScheduleModelError.invalidLimits
        }
        let ids = schedules.map(\.scheduleID)
        guard Set(ids).count == ids.count else {
            throw AutomationScheduleModelError.duplicateScheduleID
        }
        let canonicalSchedules = schedules.sorted { lhs, rhs in
            if lhs.updatedAtUnixMilliseconds != rhs.updatedAtUnixMilliseconds {
                return lhs.updatedAtUnixMilliseconds > rhs.updatedAtUnixMilliseconds
            }
            return lhs.scheduleID < rhs.scheduleID
        }
        guard schedules == canonicalSchedules else {
            throw AutomationScheduleModelError.nonCanonicalScheduleOrder
        }
        for (schedule, assessment) in zip(schedules, scheduleEligibility) {
            guard
                assessment.scheduleID == schedule.scheduleID,
                assessment.scheduleRevision == schedule.revision,
                assessment.includedStaticallyEligibleRuleCount <= eligibleRuleCount
            else {
                throw AutomationScheduleModelError.eligibilityAssessmentMismatch
            }
        }

        self.recordVersion = recordVersion
        self.globalControl = globalControl
        self.executionAvailable = executionAvailable
        self.eligibleRuleCount = eligibleRuleCount
        self.schedules = schedules
        self.scheduleEligibility = scheduleEligibility
    }

    private init(unavailable _: Void) {
        recordVersion = Self.recordVersion
        globalControl = try! AutomationGlobalControlModel(
            enabled: false,
            source: .default,
            revision: 0,
            updatedAtUnixMilliseconds: nil
        )
        executionAvailable = false
        eligibleRuleCount = 0
        schedules = []
        scheduleEligibility = []
    }

    static let unavailable = Self(unavailable: ())
}

struct AutomationScheduleOverviewUpdateModel: Equatable, Sendable {
    let overview: AutomationScheduleOverviewModel
    let changed: Bool
}
