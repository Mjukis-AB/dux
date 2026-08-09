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
