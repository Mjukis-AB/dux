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

    func testEditorDefaultsProduceCompleteSuggestionScopedConfiguration() throws {
        let rule = try DuxAutomationScheduleRuleReference(
            ruleID: "developer.cache",
            ruleRevision: 4
        )
        let draft = AutomationScheduleEditorDraft(scope: .rule(rule))

        let configuration = try draft.configuration(decimalSeparator: ".")

        XCTAssertEqual(configuration.scope, .rule(rule))
        XCTAssertEqual(configuration.cadence, .monthly)
        XCTAssertEqual(
            configuration.minimumAgeSeconds,
            AutomationScheduleDefaults.minimumAgeSeconds
        )
        XCTAssertEqual(configuration.minimumReclaimableBytes, 0)
        XCTAssertEqual(
            configuration.maximumBytesPerRun,
            AutomationScheduleDefaults.maximumBytesPerRun
        )
        XCTAssertEqual(configuration.exclusions, [])
        XCTAssertTrue(configuration.notifyBeforeRun)
        XCTAssertEqual(configuration.confirmationMode, .requireConfirmation)
    }

    func testEditorAgeUnitsRoundTripEverySecondAndRejectInvalidOrOverflowingValues() throws {
        let secondsSchedule = try makeSchedule(minimumAgeSeconds: 3601)
        var secondsDraft = AutomationScheduleEditorDraft(schedule: secondsSchedule)
        XCTAssertEqual(secondsDraft.minimumAgeValue, "3601")
        XCTAssertEqual(secondsDraft.minimumAgeUnit, .seconds)
        XCTAssertEqual(
            try secondsDraft.configuration(decimalSeparator: ".").minimumAgeSeconds,
            3601
        )

        secondsDraft.minimumAgeValue = "36500"
        secondsDraft.minimumAgeUnit = .days
        XCTAssertEqual(
            try secondsDraft.configuration(decimalSeparator: ".").minimumAgeSeconds,
            AutomationScheduleModel.maximumMinimumAgeSeconds
        )

        for value in ["36501", String(UInt64.max)] {
            secondsDraft.minimumAgeValue = value
            XCTAssertThrowsError(
                try secondsDraft.configuration(decimalSeparator: ".")
            ) { error in
                XCTAssertEqual(
                    error as? AutomationScheduleEditorDraftError,
                    .outOfRange(.minimumAge)
                )
            }
        }

        secondsDraft.minimumAgeValue = "1.5"
        XCTAssertThrowsError(
            try secondsDraft.configuration(decimalSeparator: ".")
        ) { error in
            XCTAssertEqual(
                error as? AutomationScheduleEditorDraftError,
                .invalidNumber(.minimumAge)
            )
        }
    }

    func testEditorByteFieldsAreExactBoundedAndDoNotInventCrossFieldOrdering() throws {
        var draft = AutomationScheduleEditorDraft(scope: .category(.developerArtifact))
        draft.minimumReclaimableGiB = ExactPolicyDecimal.formatGiB(
            AutomationScheduleModel.maximumStoredBytes
        )
        draft.maximumBytesPerRunGiB = ExactPolicyDecimal.formatGiB(1)

        let configuration = try draft.configuration(decimalSeparator: ".")
        XCTAssertEqual(
            configuration.minimumReclaimableBytes,
            AutomationScheduleModel.maximumStoredBytes
        )
        XCTAssertEqual(configuration.maximumBytesPerRun, 1)

        draft.maximumBytesPerRunGiB = "0"
        XCTAssertThrowsError(try draft.configuration(decimalSeparator: ".")) { error in
            XCTAssertEqual(
                error as? AutomationScheduleEditorDraftError,
                .zero(.maximumBytesPerRun)
            )
        }

        draft.maximumBytesPerRunGiB = ExactPolicyDecimal.formatGiB(
            AutomationScheduleModel.maximumStoredBytes + 1
        )
        XCTAssertThrowsError(try draft.configuration(decimalSeparator: ".")) { error in
            XCTAssertEqual(
                error as? AutomationScheduleEditorDraftError,
                .outOfRange(.maximumBytesPerRun)
            )
        }

        draft.maximumBytesPerRunGiB = "1,000"
        XCTAssertThrowsError(try draft.configuration(decimalSeparator: ".")) { error in
            XCTAssertEqual(
                error as? AutomationScheduleEditorDraftError,
                .invalidNumber(.maximumBytesPerRun)
            )
        }
    }

    func testEditorPreservesEveryCompleteStoredConfigurationField() throws {
        let exclusions = try [
            DuxAutomationScheduleRuleReference(ruleID: "developer.a", ruleRevision: 1),
            DuxAutomationScheduleRuleReference(ruleID: "developer.z", ruleRevision: 2),
        ]
        let schedule = try makeSchedule(
            scope: .category(.applicationCache),
            cadence: .weekly,
            minimumAgeSeconds: 3601,
            minimumReclaimableBytes: 53_687_091_200,
            maximumBytesPerRun: 26_843_545_601,
            exclusions: exclusions,
            notifyBeforeRun: false,
            confirmationMode: .fullyAutomatic
        )

        let configuration = try AutomationScheduleEditorDraft(schedule: schedule)
            .configuration(decimalSeparator: ".")

        XCTAssertEqual(configuration.scope, schedule.scope)
        XCTAssertEqual(configuration.cadence, schedule.cadence)
        XCTAssertEqual(configuration.minimumAgeSeconds, schedule.minimumAgeSeconds)
        XCTAssertEqual(
            configuration.minimumReclaimableBytes,
            schedule.minimumReclaimableBytes
        )
        XCTAssertEqual(configuration.maximumBytesPerRun, schedule.maximumBytesPerRun)
        XCTAssertEqual(configuration.exclusions, schedule.exclusions)
        XCTAssertEqual(configuration.notifyBeforeRun, schedule.notifyBeforeRun)
        XCTAssertEqual(configuration.confirmationMode, schedule.confirmationMode)
    }

    func testEditorSessionRejectsEachReviewedImmutableFieldSubstitution() throws {
        let baseline = try AutomationScheduleEditorDraft(schedule: makeSchedule())
        let session = AutomationScheduleEditorSession(
            mode: .edit(scheduleID: "automation:test", expectedRevision: 1),
            draft: baseline,
            requiresReReview: false
        )
        let exclusion = try DuxAutomationScheduleRuleReference(
            ruleID: "developer.excluded",
            ruleRevision: 1
        )
        let candidates = try [
            AutomationScheduleEditorDraft(
                schedule: makeSchedule(scope: .category(.applicationCache))
            ),
            AutomationScheduleEditorDraft(
                schedule: makeSchedule(exclusions: [exclusion])
            ),
            AutomationScheduleEditorDraft(
                schedule: makeSchedule(notifyBeforeRun: false)
            ),
            AutomationScheduleEditorDraft(
                schedule: makeSchedule(confirmationMode: .fullyAutomatic)
            ),
        ]

        XCTAssertTrue(session.preservesReviewedImmutableFields(in: baseline))
        for candidate in candidates {
            XCTAssertFalse(session.preservesReviewedImmutableFields(in: candidate))
        }
    }

    func testDraftConfigurationRejectsExactRuleExclusions() throws {
        let rule = try DuxAutomationScheduleRuleReference(
            ruleID: "developer.cache",
            ruleRevision: 1
        )
        XCTAssertThrowsError(
            try AutomationScheduleDraftConfigurationModel(
                scope: .rule(rule),
                cadence: .monthly,
                minimumAgeSeconds: 0,
                minimumReclaimableBytes: 0,
                maximumBytesPerRun: 1,
                exclusions: [rule],
                notifyBeforeRun: true,
                confirmationMode: .requireConfirmation
            )
        ) { error in
            XCTAssertEqual(
                error as? AutomationScheduleEditorDraftError,
                .exclusionsRequireCategoryScope
            )
        }
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

    func testAuthoringCatalogAcceptsEmptyAndStrictCanonicalCategoryMembership() throws {
        let empty = try AutomationScheduleAuthoringCatalogModel(
            recordVersion: 1,
            authoringPolicyRevision: 1,
            maximumSelectedExclusions: 32,
            staticallySelectableRuleCount: 0,
            categories: []
        )
        XCTAssertTrue(empty.categories.isEmpty)

        let category = try authoringCategory()
        let catalog = try AutomationScheduleAuthoringCatalogModel(
            recordVersion: 1,
            authoringPolicyRevision: 1,
            maximumSelectedExclusions: 32,
            staticallySelectableRuleCount: 2,
            categories: [category]
        )
        XCTAssertEqual(catalog.categories, [category])
        XCTAssertEqual(
            category.scopeMembershipDigestSHA256,
            "db48a78219fafe428339627a0bc308727726902d73369db438dba51488894aa8"
        )

        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCatalogModel(
                recordVersion: 1,
                authoringPolicyRevision: 1,
                maximumSelectedExclusions: 31,
                staticallySelectableRuleCount: 2,
                categories: [category]
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCatalogModel(
                recordVersion: 1,
                authoringPolicyRevision: 1,
                maximumSelectedExclusions: 32,
                staticallySelectableRuleCount: 1,
                categories: [category]
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCategoryModel(
                recordVersion: 1,
                category: .developerArtifact,
                scopeMembershipDigestSHA256: String(repeating: "A", count: 64),
                rules: category.rules
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCategoryModel(
                recordVersion: 1,
                category: .developerArtifact,
                scopeMembershipDigestSHA256: String(repeating: "a", count: 64),
                rules: category.rules
            )
        )

        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCatalogModel(
                recordVersion: 2,
                authoringPolicyRevision: 1,
                maximumSelectedExclusions: 32,
                staticallySelectableRuleCount: 0,
                categories: []
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCatalogModel(
                recordVersion: 1,
                authoringPolicyRevision: 2,
                maximumSelectedExclusions: 32,
                staticallySelectableRuleCount: 0,
                categories: []
            )
        )
        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCategoryModel(
                recordVersion: 1,
                category: .developerArtifact,
                scopeMembershipDigestSHA256: String(repeating: "a", count: 64),
                rules: []
            )
        )

        let laterCategory = try authoringCategory(
            category: .applicationCache,
            ruleIDPrefix: "application"
        )
        XCTAssertEqual(
            laterCategory.scopeMembershipDigestSHA256,
            "901cbb37f39566ca4d73309cc3d094be9b63eedbb4090c58f9b0094894fa0abc"
        )
        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCatalogModel(
                recordVersion: 1,
                authoringPolicyRevision: 1,
                maximumSelectedExclusions: 32,
                staticallySelectableRuleCount: 4,
                categories: [laterCategory, category]
            )
        )
        let duplicateRules = try AutomationScheduleAuthoringCategoryModel(
            recordVersion: 1,
            category: .applicationCache,
            scopeMembershipDigestSHA256: AutomationScheduleAuthoringCatalogModel
                .membershipDigestSHA256(
                    category: .applicationCache,
                    rules: category.rules
                ),
            rules: category.rules
        )
        XCTAssertThrowsError(
            try AutomationScheduleAuthoringCatalogModel(
                recordVersion: 1,
                authoringPolicyRevision: 1,
                maximumSelectedExclusions: 32,
                staticallySelectableRuleCount: 4,
                categories: [category, duplicateRules]
            )
        )
    }

    func testCatalogCategorySessionAllowsOnlyExactCanonicalExclusions() throws {
        let category = try authoringCategory()
        let catalog = try AutomationScheduleAuthoringCatalogModel(
            recordVersion: 1,
            authoringPolicyRevision: 1,
            maximumSelectedExclusions: 32,
            staticallySelectableRuleCount: 2,
            categories: [category]
        )
        let selection = try AutomationScheduleCategoryAuthoringSelection(
            catalog: catalog,
            category: category
        )
        var draft = AutomationScheduleEditorDraft(scope: .category(.developerArtifact))
        let session = AutomationScheduleEditorSession(
            mode: .createCategory(selection: selection),
            draft: draft,
            requiresReReview: false
        )

        draft.exclusions = [category.rules[0].rule]
        XCTAssertTrue(session.preservesReviewedImmutableFields(in: draft))
        let configuration = try AutomationScheduleEditorSession(
            mode: .createCategory(selection: selection),
            draft: draft,
            requiresReReview: false
        ).categoryConfiguration(decimalSeparator: ".")
        XCTAssertEqual(configuration.configuration.exclusions, draft.exclusions)
        XCTAssertEqual(
            configuration.selection.category.scopeMembershipDigestSHA256,
            "db48a78219fafe428339627a0bc308727726902d73369db438dba51488894aa8"
        )

        draft.exclusions = category.rules.map(\.rule)
        XCTAssertThrowsError(
            try AutomationScheduleEditorSession(
                mode: .createCategory(selection: selection),
                draft: draft,
                requiresReReview: false
            ).categoryConfiguration(decimalSeparator: ".")
        ) { error in
            XCTAssertEqual(
                error as? AutomationScheduleEditorDraftError,
                .allRulesExcluded
            )
        }

        draft.exclusions = try [
            DuxAutomationScheduleRuleReference(
                ruleID: "developer.not-projected",
                ruleRevision: 1
            ),
        ]
        XCTAssertFalse(session.preservesReviewedImmutableFields(in: draft))
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
            policyRevision: 2,
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
        scope: DuxAutomationScheduleScope = .category(.developerArtifact),
        cadence: DuxAutomationScheduleCadence = .monthly,
        minimumAgeSeconds: UInt64 = AutomationScheduleDefaults.minimumAgeSeconds,
        minimumReclaimableBytes: UInt64 = 0,
        maximumBytesPerRun: UInt64 = AutomationScheduleDefaults.maximumBytesPerRun,
        exclusions: [DuxAutomationScheduleRuleReference] = [],
        notifyBeforeRun: Bool = true,
        confirmationMode: DuxAutomationScheduleConfirmationMode = .requireConfirmation,
        state: DuxAutomationScheduleState = .disabled,
        recurrence: AutomationScheduleRecurrenceModel? = nil,
        revision: UInt64 = 1,
        createdAtUnixMilliseconds: Int64 = 100,
        updatedAtUnixMilliseconds: Int64 = 200
    ) throws -> AutomationScheduleModel {
        try AutomationScheduleModel(
            scheduleID: scheduleID,
            scope: scope,
            cadence: cadence,
            minimumAgeSeconds: minimumAgeSeconds,
            minimumReclaimableBytes: minimumReclaimableBytes,
            maximumBytesPerRun: maximumBytesPerRun,
            exclusions: exclusions,
            notifyBeforeRun: notifyBeforeRun,
            notifyBeforeRunsRemaining: notifyBeforeRun ? 3 : 0,
            confirmationMode: confirmationMode,
            state: state,
            recurrence: recurrence,
            revision: revision,
            createdAtUnixMilliseconds: createdAtUnixMilliseconds,
            updatedAtUnixMilliseconds: updatedAtUnixMilliseconds
        )
    }

    private func authoringCategory(
        category: ExplorerCandidateCategory = .developerArtifact,
        ruleIDPrefix: String = "developer"
    ) throws -> AutomationScheduleAuthoringCategoryModel {
        let rules = try [
            AutomationScheduleAuthoringRuleModel(
                recordVersion: 1,
                rule: DuxAutomationScheduleRuleReference(
                    ruleID: "\(ruleIDPrefix).a-cache",
                    ruleRevision: 1
                ),
                titleKey: "rule.\(ruleIDPrefix).a-cache.title"
            ),
            AutomationScheduleAuthoringRuleModel(
                recordVersion: 1,
                rule: DuxAutomationScheduleRuleReference(
                    ruleID: "\(ruleIDPrefix).z-cache",
                    ruleRevision: 2
                ),
                titleKey: "rule.\(ruleIDPrefix).z-cache.title"
            ),
        ]
        return try AutomationScheduleAuthoringCategoryModel(
            recordVersion: 1,
            category: category,
            scopeMembershipDigestSHA256: AutomationScheduleAuthoringCatalogModel
                .membershipDigestSHA256(category: category, rules: rules),
            rules: rules
        )
    }
}
