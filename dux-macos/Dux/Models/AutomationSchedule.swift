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

enum DuxAutomationScheduleDraftEligibilityStatus: Equatable, Sendable {
    case blockedByStaticPolicy
    case awaitingRuntimeEvidence
}

enum DuxAutomationScheduleDraftEligibilityReason: Int, CaseIterable, Sendable, Comparable, Hashable {
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
        lhs: DuxAutomationScheduleDraftEligibilityReason,
        rhs: DuxAutomationScheduleDraftEligibilityReason
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
struct AutomationScheduleDraftEligibilityModel: Equatable, Sendable {
    static let recordVersion: UInt32 = 1
    static let policyRevision: UInt32 = 1
    static let maximumReasons = 16

    let scheduleID: String
    let draftRevision: UInt64
    let status: DuxAutomationScheduleDraftEligibilityStatus
    let includedStaticallyEligibleRuleCount: UInt16
    let reasons: [DuxAutomationScheduleDraftEligibilityReason]

    init(
        recordVersion: UInt32,
        policyRevision: UInt32,
        scheduleID: String,
        draftRevision: UInt64,
        status: DuxAutomationScheduleDraftEligibilityStatus,
        includedStaticallyEligibleRuleCount: UInt16,
        reasons: [DuxAutomationScheduleDraftEligibilityReason]
    ) throws {
        guard
            recordVersion == Self.recordVersion,
            policyRevision == Self.policyRevision,
            !scheduleID.isEmpty,
            scheduleID.utf8.count <= AutomationScheduleDraftModel.maximumScheduleIDBytes,
            scheduleID.utf8.allSatisfy(AutomationScheduleDraftModel.isScheduleIDByte),
            draftRevision > 0,
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
        self.draftRevision = draftRevision
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

/// The first M8 client accepts only the non-executing automation state. A newer
/// backend cannot silently activate behavior through this older Settings UI.
struct AutomationScheduleOverviewModel: Equatable, Sendable {
    static let recordVersion: UInt32 = 2
    static let maximumDraftCount = 64
    static let maximumEligibleRuleCount: UInt16 = 256

    let recordVersion: UInt32
    let globalEnabled: Bool
    let executionAvailable: Bool
    let eligibleRuleCount: UInt16
    let disabledDrafts: [AutomationScheduleDraftModel]
    let draftEligibility: [AutomationScheduleDraftEligibilityModel]

    init(
        recordVersion: UInt32,
        globalEnabled: Bool,
        executionAvailable: Bool,
        eligibleRuleCount: UInt16,
        disabledDrafts: [AutomationScheduleDraftModel],
        draftEligibility: [AutomationScheduleDraftEligibilityModel] = []
    ) throws {
        guard recordVersion == Self.recordVersion else {
            throw AutomationScheduleModelError.invalidRecordVersion
        }
        guard !globalEnabled, !executionAvailable else {
            throw AutomationScheduleModelError.activeAutomationRejected
        }
        guard
            eligibleRuleCount <= Self.maximumEligibleRuleCount,
            disabledDrafts.count <= Self.maximumDraftCount,
            draftEligibility.count == disabledDrafts.count
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
        for (draft, assessment) in zip(disabledDrafts, draftEligibility) {
            guard
                assessment.scheduleID == draft.scheduleID,
                assessment.draftRevision == draft.revision,
                assessment.includedStaticallyEligibleRuleCount <= eligibleRuleCount
            else {
                throw AutomationScheduleModelError.eligibilityAssessmentMismatch
            }
        }

        self.recordVersion = recordVersion
        self.globalEnabled = globalEnabled
        self.executionAvailable = executionAvailable
        self.eligibleRuleCount = eligibleRuleCount
        self.disabledDrafts = disabledDrafts
        self.draftEligibility = draftEligibility
    }

    private init(unavailable _: Void) {
        recordVersion = Self.recordVersion
        globalEnabled = false
        executionAvailable = false
        eligibleRuleCount = 0
        disabledDrafts = []
        draftEligibility = []
    }

    static let unavailable = Self(unavailable: ())
}
