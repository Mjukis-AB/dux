@testable import DUX
import XCTest

final class AutomationScheduleTests: XCTestCase {
    func testRuleReferenceAcceptsCanonicalGrammarAndRejectsMalformedValues() throws {
        XCTAssertEqual(
            try DuxAutomationScheduleRuleReference(
                ruleID: "developer.rust-cache_2",
                ruleRevision: 7
            ).ruleRevision,
            7
        )
        for id in ["", ".developer", "developer.", "developer..cache", "Developer.cache"] {
            XCTAssertThrowsError(
                try DuxAutomationScheduleRuleReference(ruleID: id, ruleRevision: 1)
            )
        }
        XCTAssertThrowsError(
            try DuxAutomationScheduleRuleReference(
                ruleID: "developer.valid",
                ruleRevision: 0
            )
        )
    }

    func testGlobalControlAcceptsAbsentAndDurableDefaultWithoutRevisionABA() throws {
        let absent = try AutomationGlobalControlModel(
            enabled: false,
            source: .default,
            revision: 0,
            updatedAtUnixMilliseconds: nil
        )
        let reset = try AutomationGlobalControlModel(
            enabled: false,
            source: .default,
            revision: 3,
            updatedAtUnixMilliseconds: 300
        )
        let stored = try AutomationGlobalControlModel(
            enabled: true,
            source: .stored,
            revision: 4,
            updatedAtUnixMilliseconds: 400
        )

        XCTAssertFalse(absent.enabled)
        XCTAssertEqual(reset.revision, 3)
        XCTAssertTrue(stored.enabled)
    }

    func testGlobalControlRejectsImpossibleSourceRevisionTimestampShapes() {
        XCTAssertThrowsError(
            try AutomationGlobalControlModel(
                enabled: true,
                source: .default,
                revision: 0,
                updatedAtUnixMilliseconds: nil
            )
        )
        XCTAssertThrowsError(
            try AutomationGlobalControlModel(
                enabled: false,
                source: .stored,
                revision: 0,
                updatedAtUnixMilliseconds: nil
            )
        )
        XCTAssertThrowsError(
            try AutomationGlobalControlModel(
                enabled: false,
                source: .default,
                revision: 1,
                updatedAtUnixMilliseconds: nil
            )
        )
    }

    func testPeriodicRecurrencePreservesCoreValuesAndRejectsMalformedShapes() throws {
        let recurrence = try makeRecurrence()
        XCTAssertEqual(recurrence.cursorRevision, 2)
        XCTAssertEqual(recurrence.recurrencePolicyRevision, 1)
        XCTAssertEqual(recurrence.nextOccurrenceOrdinal, 3)

        for candidate in [
            (UInt32(2), UInt64(2), UInt32(1), Int64(100), UInt64(3), Int64(200)),
            (UInt32(1), UInt64(0), UInt32(1), Int64(100), UInt64(3), Int64(200)),
            (UInt32(1), UInt64(2), UInt32(2), Int64(100), UInt64(3), Int64(200)),
            (UInt32(1), UInt64(2), UInt32(1), Int64(100), UInt64(0), Int64(200)),
            (UInt32(1), UInt64(2), UInt32(1), Int64(200), UInt64(3), Int64(200)),
        ] {
            XCTAssertThrowsError(
                try AutomationScheduleRecurrenceModel(
                    recordVersion: candidate.0,
                    cursorRevision: candidate.1,
                    recurrencePolicyRevision: candidate.2,
                    anchorAtUnixMilliseconds: candidate.3,
                    nextOccurrenceOrdinal: candidate.4,
                    nextRunAtUnixMilliseconds: candidate.5
                )
            )
        }
    }

    func testScheduleStateAndRecurrenceFormAClosedFailClosedShape() throws {
        let disabled = try makeSchedule(state: .disabled, recurrence: nil)
        let enabled = try makeSchedule(state: .enabled, recurrence: makeRecurrence())
        let paused = try makeSchedule(
            state: .paused(.failure),
            recurrence: makeRecurrence()
        )

        XCTAssertEqual(disabled.state, .disabled)
        XCTAssertEqual(enabled.state, .enabled)
        XCTAssertEqual(paused.state, .paused(.failure))

        XCTAssertThrowsError(
            try makeSchedule(state: .disabled, recurrence: makeRecurrence())
        )
        XCTAssertThrowsError(
            try makeSchedule(state: .enabled, recurrence: nil)
        )
        XCTAssertThrowsError(
            try makeSchedule(
                cadence: .lowDiskOnly,
                state: .enabled,
                recurrence: makeRecurrence()
            )
        )
    }

    func testScheduleStillValidatesLimitsOrderingAndTimestamps() throws {
        let exclusions = try [
            DuxAutomationScheduleRuleReference(ruleID: "developer.a", ruleRevision: 1),
            DuxAutomationScheduleRuleReference(ruleID: "developer.z", ruleRevision: 2),
        ]
        let valid = try makeSchedule(exclusions: exclusions)
        XCTAssertEqual(valid.exclusions, exclusions)

        XCTAssertThrowsError(try makeSchedule(exclusions: Array(exclusions.reversed())))
        XCTAssertThrowsError(
            try makeSchedule(
                minimumAgeSeconds: AutomationScheduleModel.maximumMinimumAgeSeconds + 1
            )
        )
        XCTAssertThrowsError(
            try makeSchedule(createdAtUnixMilliseconds: 201, updatedAtUnixMilliseconds: 200)
        )
    }

    func testOverviewAcceptsActivationStateButKeepsExecutionUnavailable() throws {
        let enabled = try makeSchedule(
            scheduleID: "automation:enabled",
            state: .enabled,
            recurrence: makeRecurrence(),
            revision: 2,
            updatedAtUnixMilliseconds: 300
        )
        let disabled = try makeSchedule(
            scheduleID: "automation:disabled",
            updatedAtUnixMilliseconds: 200
        )
        let overview = try AutomationScheduleOverviewModel(
            recordVersion: 3,
            globalControl: global(enabled: true),
            executionAvailable: false,
            eligibleRuleCount: 2,
            schedules: [enabled, disabled],
            scheduleEligibility: [
                assessment(for: enabled, status: .awaitingRuntimeEvidence),
                assessment(for: disabled, status: .blockedByStaticPolicy),
            ]
        )

        XCTAssertTrue(overview.globalControl.enabled)
        XCTAssertFalse(overview.executionAvailable)
        XCTAssertEqual(overview.schedules.map(\.scheduleID), [enabled.scheduleID, disabled.scheduleID])

        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 3,
                globalControl: global(enabled: true),
                executionAvailable: true,
                eligibleRuleCount: 2,
                schedules: [enabled, disabled],
                scheduleEligibility: [
                    assessment(for: enabled, status: .awaitingRuntimeEvidence),
                    assessment(for: disabled, status: .blockedByStaticPolicy),
                ]
            )
        )
    }

    func testOverviewRejectsDuplicateUnorderedAndMismatchedSchedules() throws {
        let newer = try makeSchedule(
            scheduleID: "automation:newer",
            revision: 2,
            updatedAtUnixMilliseconds: 300
        )
        let older = try makeSchedule(
            scheduleID: "automation:older",
            updatedAtUnixMilliseconds: 200
        )
        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 3,
                globalControl: global(),
                executionAvailable: false,
                eligibleRuleCount: 1,
                schedules: [older, newer],
                scheduleEligibility: [
                    assessment(for: older),
                    assessment(for: newer),
                ]
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 3,
                globalControl: global(),
                executionAvailable: false,
                eligibleRuleCount: 1,
                schedules: [newer, newer],
                scheduleEligibility: [
                    assessment(for: newer),
                    assessment(for: newer),
                ]
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleOverviewModel(
                recordVersion: 3,
                globalControl: global(),
                executionAvailable: false,
                eligibleRuleCount: 1,
                schedules: [newer, older],
                scheduleEligibility: [
                    assessment(for: older),
                    assessment(for: newer),
                ]
            )
        )
    }

    func testEligibilityAwaitingRuntimeEvidenceIsNotNamedRunnable() throws {
        let schedule = try makeSchedule()
        let value = try assessment(for: schedule, status: .awaitingRuntimeEvidence)
        XCTAssertTrue(value.reasons.isEmpty)
        XCTAssertFalse(value.statusLabel.localizedCaseInsensitiveContains("runnable"))
        XCTAssertFalse(value.statusLabel.localizedCaseInsensitiveContains("eligible"))
    }

    func testHistorySuggestionFeedRetainsBoundedCanonicalValidation() throws {
        let suggestion = try AutomationScheduleHistorySuggestionModel(
            recordVersion: 1,
            rank: 1,
            rule: DuxAutomationScheduleRuleReference(
                ruleID: "developer.cache",
                ruleRevision: 1
            ),
            successfulManualRunCount: 2,
            manualRegrowthCycleCount: 1,
            latestManualAttemptAtUnixMilliseconds: 200,
            latestRegrowthAtUnixMilliseconds: 100
        )
        let feed = try AutomationScheduleHistorySuggestionFeedModel(
            recordVersion: 1,
            derivationRevision: 1,
            sourceSessionCount: 2,
            hasOlderSourceSessions: false,
            qualifyingRuleCount: 1,
            suggestions: [suggestion]
        )
        XCTAssertEqual(feed.suggestions, [suggestion])
    }

    private func global(enabled: Bool = false) throws -> AutomationGlobalControlModel {
        try AutomationGlobalControlModel(
            enabled: enabled,
            source: enabled ? .stored : .default,
            revision: enabled ? 1 : 0,
            updatedAtUnixMilliseconds: enabled ? 100 : nil
        )
    }

    private func makeRecurrence() throws -> AutomationScheduleRecurrenceModel {
        try AutomationScheduleRecurrenceModel(
            recordVersion: 1,
            cursorRevision: 2,
            recurrencePolicyRevision: 1,
            anchorAtUnixMilliseconds: 100,
            nextOccurrenceOrdinal: 3,
            nextRunAtUnixMilliseconds: 200
        )
    }

    private func assessment(
        for schedule: AutomationScheduleModel,
        status: DuxAutomationScheduleEligibilityStatus = .blockedByStaticPolicy
    ) throws -> AutomationScheduleEligibilityModel {
        try AutomationScheduleEligibilityModel(
            recordVersion: 1,
            policyRevision: 1,
            scheduleID: schedule.scheduleID,
            scheduleRevision: schedule.revision,
            status: status,
            includedStaticallyEligibleRuleCount: status == .awaitingRuntimeEvidence ? 1 : 0,
            reasons: status == .awaitingRuntimeEvidence
                ? [] : [.scopeRuleNotMarkedScheduleEligible]
        )
    }

    private func makeSchedule(
        scheduleID: String = "automation:test",
        cadence: DuxAutomationScheduleCadence = .monthly,
        minimumAgeSeconds: UInt64 = AutomationScheduleDefaults.minimumAgeSeconds,
        exclusions: [DuxAutomationScheduleRuleReference] = [],
        state: DuxAutomationScheduleState = .disabled,
        recurrence: AutomationScheduleRecurrenceModel? = nil,
        revision: UInt64 = 1,
        createdAtUnixMilliseconds: Int64 = 100,
        updatedAtUnixMilliseconds: Int64 = 200
    ) throws -> AutomationScheduleModel {
        try AutomationScheduleModel(
            scheduleID: scheduleID,
            scope: .category(.developerArtifact),
            cadence: cadence,
            minimumAgeSeconds: minimumAgeSeconds,
            minimumReclaimableBytes: 0,
            maximumBytesPerRun: AutomationScheduleDefaults.maximumBytesPerRun,
            exclusions: exclusions,
            notifyBeforeRun: true,
            notifyBeforeRunsRemaining: 3,
            confirmationMode: .requireConfirmation,
            state: state,
            recurrence: recurrence,
            revision: revision,
            createdAtUnixMilliseconds: createdAtUnixMilliseconds,
            updatedAtUnixMilliseconds: updatedAtUnixMilliseconds
        )
    }
}
