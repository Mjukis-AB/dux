@testable import DUX
import XCTest

final class AutomationScheduleTests: XCTestCase {
    func testFrozenDefaultsMatchTheReviewedM8Policy() {
        XCTAssertEqual(AutomationScheduleDefaults.cadence, .monthly)
        XCTAssertEqual(
            AutomationScheduleDefaults.minimumAgeSeconds,
            30 * 24 * 60 * 60
        )
        XCTAssertEqual(AutomationScheduleDefaults.maximumBytesPerRun, 25 * 1_073_741_824)
        XCTAssertEqual(AutomationScheduleDefaults.notificationRuns, 3)
        XCTAssertTrue(AutomationScheduleDefaults.notifyBeforeRun)
        XCTAssertEqual(
            AutomationScheduleDefaults.confirmationMode,
            .requireConfirmation
        )
    }

    func testRuleReferencesMirrorCoreDottedIDGrammar() throws {
        let valid = try DuxAutomationScheduleRuleReference(
            ruleID: "developer.rust-target_v2",
            ruleRevision: 3
        )
        XCTAssertEqual(valid.ruleID, "developer.rust-target_v2")

        for invalid in [
            "", ".developer", "developer.", "developer..rust", "Developer.rust",
            "developer.-rust", "developer.rust_", "developer/rust", "developer rust",
        ] {
            XCTAssertThrowsError(
                try DuxAutomationScheduleRuleReference(
                    ruleID: invalid,
                    ruleRevision: 1
                )
            )
        }
        XCTAssertThrowsError(
            try DuxAutomationScheduleRuleReference(
                ruleID: String(repeating: "a", count: 129),
                ruleRevision: 1
            )
        )
        XCTAssertThrowsError(
            try DuxAutomationScheduleRuleReference(
                ruleID: "developer.rust.target",
                ruleRevision: 0
            )
        )
    }

    func testValidDisabledDraftPreservesEveryPathFreeField() throws {
        let exclusion = try rule("developer.python.pycache", revision: 2)
        let draft = try makeDraft(
            scheduleID: "schedule:01HZ-123",
            scope: .category(.developerArtifact),
            exclusions: [exclusion],
            notifyBeforeRun: true,
            notificationsRemaining: 2
        )

        XCTAssertEqual(draft.id, "schedule:01HZ-123")
        XCTAssertEqual(draft.scope, .category(.developerArtifact))
        XCTAssertEqual(draft.cadence, .monthly)
        XCTAssertEqual(draft.minimumAgeSeconds, 2_592_000)
        XCTAssertEqual(draft.minimumReclaimableBytes, 0)
        XCTAssertEqual(draft.maximumBytesPerRun, 25 * 1_073_741_824)
        XCTAssertEqual(draft.exclusions, [exclusion])
        XCTAssertTrue(draft.notifyBeforeRun)
        XCTAssertEqual(draft.notifyBeforeRunsRemaining, 2)
        XCTAssertEqual(draft.confirmationMode, .requireConfirmation)
        XCTAssertFalse(draft.enabled)
        XCTAssertEqual(draft.revision, 1)
        XCTAssertEqual(draft.createdAtUnixMilliseconds, 100)
        XCTAssertEqual(draft.updatedAtUnixMilliseconds, 200)
    }

    func testDraftRejectsEnabledOrNonCanonicalAuthorityShapes() throws {
        XCTAssertThrowsError(try makeDraft(enabled: true)) {
            XCTAssertEqual(
                $0 as? AutomationScheduleModelError,
                .enabledDraftRejected
            )
        }
        XCTAssertThrowsError(
            try makeDraft(scheduleID: "schedule/one")
        )
        XCTAssertThrowsError(
            try makeDraft(scheduleID: String(repeating: "a", count: 129))
        )

        let a = try rule("developer.a", revision: 1)
        let z = try rule("developer.z", revision: 1)
        XCTAssertThrowsError(try makeDraft(exclusions: [z, a]))
        XCTAssertThrowsError(try makeDraft(exclusions: [a, a]))
        XCTAssertThrowsError(
            try makeDraft(
                scope: .rule(rule("developer.a", revision: 1)),
                exclusions: [z]
            )
        ) {
            XCTAssertEqual(
                $0 as? AutomationScheduleModelError,
                .exclusionsRequireCategoryScope
            )
        }
    }

    func testDraftRejectsOutOfRangeLimitsNotificationsAndTimestamps() throws {
        XCTAssertThrowsError(
            try makeDraft(
                minimumAgeSeconds:
                AutomationScheduleDraftModel.maximumMinimumAgeSeconds + 1
            )
        )
        XCTAssertThrowsError(
            try makeDraft(
                minimumReclaimableBytes:
                AutomationScheduleDraftModel.maximumStoredBytes + 1,
                maximumBytesPerRun:
                AutomationScheduleDraftModel.maximumStoredBytes
            )
        )
        XCTAssertNoThrow(
            try makeDraft(minimumReclaimableBytes: 2, maximumBytesPerRun: 1)
        )
        XCTAssertThrowsError(try makeDraft(maximumBytesPerRun: 0))
        XCTAssertThrowsError(
            try makeDraft(notifyBeforeRun: false, notificationsRemaining: 1)
        )
        XCTAssertThrowsError(
            try makeDraft(notifyBeforeRun: true, notificationsRemaining: 4)
        )
        XCTAssertNoThrow(
            try makeDraft(notifyBeforeRun: true, notificationsRemaining: 0)
        )
        XCTAssertThrowsError(
            try makeDraft(createdAt: 200, updatedAt: 100)
        )
        XCTAssertThrowsError(
            try makeDraft(createdAt: -1, updatedAt: 100)
        )
    }

    func testOverviewRejectsActiveDuplicateOrNonCanonicalDrafts() throws {
        let a = try makeDraft(scheduleID: "a")
        let z = try makeDraft(scheduleID: "z")
        let overview = try AutomationScheduleOverviewModel(
            recordVersion: 2,
            globalEnabled: false,
            executionAvailable: false,
            eligibleRuleCount: 0,
            disabledDrafts: [a, z],
            draftEligibility: [
                try blockedAssessment(for: a),
                try blockedAssessment(for: z),
            ]
        )
        XCTAssertEqual(overview.disabledDrafts.map(\.id), ["a", "z"])

        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 3,
                globalEnabled: false,
                executionAvailable: false,
                eligibleRuleCount: 0,
                disabledDrafts: []
            )
        )
        for active in [(true, false), (false, true), (true, true)] {
            XCTAssertThrowsError(
                try AutomationScheduleOverviewModel(
                    recordVersion: 2,
                    globalEnabled: active.0,
                    executionAvailable: active.1,
                    eligibleRuleCount: 0,
                    disabledDrafts: []
                )
            )
        }
        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 2,
                globalEnabled: false,
                executionAvailable: false,
                eligibleRuleCount:
                AutomationScheduleOverviewModel.maximumEligibleRuleCount + 1,
                disabledDrafts: []
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 2,
                globalEnabled: false,
                executionAvailable: false,
                eligibleRuleCount: 0,
                disabledDrafts: [z, a]
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 2,
                globalEnabled: false,
                executionAvailable: false,
                eligibleRuleCount: 0,
                disabledDrafts: [a, a]
            )
        )

        let newerZ = try makeDraft(
            scheduleID: "z",
            createdAt: 100,
            updatedAt: 300
        )
        XCTAssertNoThrow(
            try AutomationScheduleOverviewModel(
                recordVersion: 2,
                globalEnabled: false,
                executionAvailable: false,
                eligibleRuleCount: 0,
                disabledDrafts: [newerZ, a],
                draftEligibility: [
                    try blockedAssessment(for: newerZ),
                    try blockedAssessment(for: a),
                ]
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 2,
                globalEnabled: false,
                executionAvailable: false,
                eligibleRuleCount: 0,
                disabledDrafts: [a, newerZ]
            )
        )
    }

    func testAwaitingRuntimeEvidenceIsValidButNeverNamedRunnable() throws {
        let draft = try makeDraft()
        let assessment = try AutomationScheduleDraftEligibilityModel(
            recordVersion: 1,
            policyRevision: 1,
            scheduleID: draft.scheduleID,
            draftRevision: draft.revision,
            status: .awaitingRuntimeEvidence,
            includedStaticallyEligibleRuleCount: 1,
            reasons: []
        )
        let overview = try AutomationScheduleOverviewModel(
            recordVersion: 2,
            globalEnabled: false,
            executionAvailable: false,
            eligibleRuleCount: 1,
            disabledDrafts: [draft],
            draftEligibility: [assessment]
        )

        XCTAssertEqual(overview.draftEligibility, [assessment])
        XCTAssertEqual(
            assessment.statusLabel,
            "Static checks passed; runtime checks not evaluated"
        )
        XCTAssertFalse(assessment.statusLabel.localizedCaseInsensitiveContains("runnable"))
        XCTAssertFalse(assessment.statusLabel.localizedCaseInsensitiveContains("eligible"))
    }

    func testHistorySuggestionFeedPreservesBoundedPathFreeEvidence() throws {
        let newest = try makeHistorySuggestion(
            rank: 1,
            ruleID: "developer.rust.target",
            successfulRuns: 3,
            regrowthCycles: 2,
            latestAttempt: 900,
            latestRegrowth: 1000
        )
        let older = try makeHistorySuggestion(
            rank: 2,
            ruleID: "developer.python.pycache",
            latestAttempt: 1100,
            latestRegrowth: 800
        )
        let feed = try AutomationScheduleHistorySuggestionFeedModel(
            recordVersion: 1,
            derivationRevision: 1,
            sourceSessionCount: 32,
            hasOlderSourceSessions: true,
            qualifyingRuleCount: 2,
            suggestions: [newest, older]
        )

        XCTAssertEqual(feed.sourceSessionCount, 32)
        XCTAssertTrue(feed.hasOlderSourceSessions)
        XCTAssertEqual(feed.qualifyingRuleCount, 2)
        XCTAssertEqual(feed.suggestions.map(\.rule.ruleID), [
            "developer.rust.target", "developer.python.pycache",
        ])
        XCTAssertEqual(feed.suggestions.map(\.rank), [1, 2])
    }

    func testHistorySuggestionRejectsMalformedCountsRanksAndTimes() throws {
        let cases: [(UInt32, UInt16, UInt16, UInt16, Int64, Int64)] = [
            (2, 1, 2, 1, 100, 100),
            (1, 0, 2, 1, 100, 100),
            (1, 13, 2, 1, 100, 100),
            (1, 1, 1, 1, 100, 100),
            (1, 1, 33, 1, 100, 100),
            (1, 1, 2, 0, 100, 100),
            (1, 1, 2, 3, 100, 100),
            (1, 1, 2, 1, -1, 100),
            (1, 1, 2, 1, 100, -1),
        ]

        for value in cases {
            XCTAssertThrowsError(
                try AutomationScheduleHistorySuggestionModel(
                    recordVersion: value.0,
                    rank: value.1,
                    rule: rule("developer.rust.target", revision: 1),
                    successfulManualRunCount: value.2,
                    manualRegrowthCycleCount: value.3,
                    latestManualAttemptAtUnixMilliseconds: value.4,
                    latestRegrowthAtUnixMilliseconds: value.5
                )
            ) {
                XCTAssertEqual(
                    $0 as? AutomationScheduleModelError,
                    .invalidHistorySuggestion
                )
            }
        }
    }

    func testHistorySuggestionFeedRejectsMalformedCoverageAndCardinality() throws {
        let suggestion = try makeHistorySuggestion()
        let malformed: [
            (UInt32, UInt32, UInt16, Bool, UInt16,
             [AutomationScheduleHistorySuggestionModel])
        ] = [
            (2, 1, 2, false, 1, [suggestion]),
            (1, 2, 2, false, 1, [suggestion]),
            (1, 1, 33, false, 1, [suggestion]),
            (1, 1, 31, true, 1, [suggestion]),
            (1, 1, 2, false, 257, [suggestion]),
            (1, 1, 2, false, 0, [suggestion]),
            (1, 1, 2, false, 2, [suggestion]),
        ]

        for value in malformed {
            XCTAssertThrowsError(
                try AutomationScheduleHistorySuggestionFeedModel(
                    recordVersion: value.0,
                    derivationRevision: value.1,
                    sourceSessionCount: value.2,
                    hasOlderSourceSessions: value.3,
                    qualifyingRuleCount: value.4,
                    suggestions: value.5
                )
            ) {
                XCTAssertEqual(
                    $0 as? AutomationScheduleModelError,
                    .invalidHistorySuggestionFeed
                )
            }
        }

        let tooManySuccessfulRuns = try makeHistorySuggestion(successfulRuns: 3)
        XCTAssertThrowsError(
            try AutomationScheduleHistorySuggestionFeedModel(
                recordVersion: 1,
                derivationRevision: 1,
                sourceSessionCount: 2,
                hasOlderSourceSessions: false,
                qualifyingRuleCount: 1,
                suggestions: [tooManySuccessfulRuns]
            )
        )
    }

    func testHistorySuggestionFeedRejectsDuplicateNonContiguousOrNonCanonicalRows() throws {
        let first = try makeHistorySuggestion(
            rank: 1,
            ruleID: "developer.a",
            latestRegrowth: 200
        )
        let duplicate = try makeHistorySuggestion(
            rank: 2,
            ruleID: "developer.a",
            latestRegrowth: 100
        )
        XCTAssertThrowsError(
            try makeHistoryFeed([first, duplicate])
        ) {
            XCTAssertEqual(
                $0 as? AutomationScheduleModelError,
                .duplicateHistorySuggestionRule
            )
        }

        let skippedRank = try makeHistorySuggestion(
            rank: 3,
            ruleID: "developer.b",
            latestRegrowth: 100
        )
        XCTAssertThrowsError(try makeHistoryFeed([first, skippedRank])) {
            XCTAssertEqual(
                $0 as? AutomationScheduleModelError,
                .nonCanonicalHistorySuggestionOrder
            )
        }

        let newerSecond = try makeHistorySuggestion(
            rank: 2,
            ruleID: "developer.b",
            latestRegrowth: 300
        )
        XCTAssertThrowsError(try makeHistoryFeed([first, newerSecond])) {
            XCTAssertEqual(
                $0 as? AutomationScheduleModelError,
                .nonCanonicalHistorySuggestionOrder
            )
        }
    }

    private func rule(
        _ ruleID: String,
        revision: UInt32
    ) throws -> DuxAutomationScheduleRuleReference {
        try DuxAutomationScheduleRuleReference(
            ruleID: ruleID,
            ruleRevision: revision
        )
    }

    private func blockedAssessment(
        for draft: AutomationScheduleDraftModel
    ) throws -> AutomationScheduleDraftEligibilityModel {
        try AutomationScheduleDraftEligibilityModel(
            recordVersion: 1,
            policyRevision: 1,
            scheduleID: draft.scheduleID,
            draftRevision: draft.revision,
            status: .blockedByStaticPolicy,
            includedStaticallyEligibleRuleCount: 0,
            reasons: [.categoryHasNoScheduleEligibleRules]
        )
    }

    private func makeHistorySuggestion(
        rank: UInt16 = 1,
        ruleID: String = "developer.rust.target",
        successfulRuns: UInt16 = 2,
        regrowthCycles: UInt16 = 1,
        latestAttempt: Int64 = 200,
        latestRegrowth: Int64 = 100
    ) throws -> AutomationScheduleHistorySuggestionModel {
        try AutomationScheduleHistorySuggestionModel(
            recordVersion: 1,
            rank: rank,
            rule: rule(ruleID, revision: 1),
            successfulManualRunCount: successfulRuns,
            manualRegrowthCycleCount: regrowthCycles,
            latestManualAttemptAtUnixMilliseconds: latestAttempt,
            latestRegrowthAtUnixMilliseconds: latestRegrowth
        )
    }

    private func makeHistoryFeed(
        _ suggestions: [AutomationScheduleHistorySuggestionModel]
    ) throws -> AutomationScheduleHistorySuggestionFeedModel {
        try AutomationScheduleHistorySuggestionFeedModel(
            recordVersion: 1,
            derivationRevision: 1,
            sourceSessionCount: 3,
            hasOlderSourceSessions: false,
            qualifyingRuleCount: UInt16(suggestions.count),
            suggestions: suggestions
        )
    }

    private func makeDraft(
        scheduleID: String = "schedule:01HZ",
        scope: DuxAutomationScheduleScope = .category(.developerArtifact),
        minimumAgeSeconds: UInt64 = 2_592_000,
        minimumReclaimableBytes: UInt64 = 0,
        maximumBytesPerRun: UInt64 = 25 * 1_073_741_824,
        exclusions: [DuxAutomationScheduleRuleReference] = [],
        notifyBeforeRun: Bool = true,
        notificationsRemaining: UInt8 = 3,
        enabled: Bool = false,
        createdAt: Int64 = 100,
        updatedAt: Int64 = 200
    ) throws -> AutomationScheduleDraftModel {
        try AutomationScheduleDraftModel(
            scheduleID: scheduleID,
            scope: scope,
            cadence: .monthly,
            minimumAgeSeconds: minimumAgeSeconds,
            minimumReclaimableBytes: minimumReclaimableBytes,
            maximumBytesPerRun: maximumBytesPerRun,
            exclusions: exclusions,
            notifyBeforeRun: notifyBeforeRun,
            notifyBeforeRunsRemaining: notificationsRemaining,
            confirmationMode: .requireConfirmation,
            enabled: enabled,
            revision: 1,
            createdAtUnixMilliseconds: createdAt,
            updatedAtUnixMilliseconds: updatedAt
        )
    }
}
