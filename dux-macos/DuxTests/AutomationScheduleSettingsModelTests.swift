@testable import DUX
import XCTest

@MainActor
final class AutomationScheduleSettingsModelTests: XCTestCase {
    func testUnavailableSeamPublishesClosedDefaultOverview() async {
        let model = AutomationScheduleSettingsModel(
            service: UnavailableDuxAutomationScheduleService()
        )

        await model.load()

        XCTAssertEqual(model.state, .ready)
        XCTAssertEqual(model.overview?.recordVersion, 3)
        XCTAssertEqual(model.overview?.globalControl.enabled, false)
        XCTAssertEqual(model.overview?.globalControl.revision, 0)
        XCTAssertEqual(model.overview?.executionAvailable, false)
        XCTAssertEqual(model.overview?.schedules, [])
        await model.shutdown()
    }

    func testLoadCachesAndForceRefreshesWhileHistoryCacheRemainsIndependent() async throws {
        let spy = try AutomationScheduleServiceSpy(response: makeOverview())
        let model = AutomationScheduleSettingsModel(service: spy)

        await model.load()
        await model.load()
        var loadCount = await spy.loadRequestCount()
        XCTAssertEqual(loadCount, 1)

        await model.load(force: true)
        loadCount = await spy.loadRequestCount()
        var historyLoadCount = await spy.historyLoadRequestCount()
        XCTAssertEqual(loadCount, 2)
        XCTAssertEqual(historyLoadCount, 0)

        await model.loadHistorySuggestions()
        await model.loadHistorySuggestions()
        historyLoadCount = await spy.historyLoadRequestCount()
        XCTAssertEqual(historyLoadCount, 1)
        await model.shutdown()
    }

    func testSuggestionSeededCreateUsesCompleteValidatedInputAndPublishesOverview() async throws {
        let initial = try makeOverview(includeSchedule: false)
        let created = try makeOverview(scheduleRevision: 1)
        let suggestion = try makeHistorySuggestion()
        let spy = AutomationScheduleServiceSpy(response: initial)
        try await spy.setHistoryResponse(makeHistoryFeed(suggestion: suggestion))
        await spy.setMutationResponse(created)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()
        await model.loadHistorySuggestions()

        model.beginCreatingSchedule(from: suggestion)
        var draft = try XCTUnwrap(model.editor?.draft)
        draft.cadence = .weekly
        draft.minimumAgeValue = "48"
        draft.minimumAgeUnit = .hours
        draft.minimumReclaimableGiB = "50"
        draft.maximumBytesPerRunGiB = "25"
        model.setEditorDraft(draft)

        await model.saveEditor(decimalSeparator: ".")

        let calls = await spy.mutationCalls()
        guard case let .create(configuration) = try XCTUnwrap(calls.first) else {
            return XCTFail("Expected create call")
        }
        XCTAssertEqual(configuration.scope, .rule(suggestion.rule))
        XCTAssertEqual(configuration.cadence, .weekly)
        XCTAssertEqual(configuration.minimumAgeSeconds, 172_800)
        XCTAssertEqual(configuration.minimumReclaimableBytes, 53_687_091_200)
        XCTAssertEqual(configuration.maximumBytesPerRun, 26_843_545_600)
        XCTAssertTrue(configuration.notifyBeforeRun)
        XCTAssertEqual(configuration.confirmationMode, .requireConfirmation)
        XCTAssertEqual(model.overview, created)
        XCTAssertNil(model.editor)
        XCTAssertEqual(model.state, .ready)
        await model.shutdown()
    }

    func testEditorRejectsSubstitutionOfSuggestionReviewedImmutableFields() async throws {
        let initial = try makeOverview(includeSchedule: false)
        let suggestion = try makeHistorySuggestion()
        let spy = AutomationScheduleServiceSpy(response: initial)
        try await spy.setHistoryResponse(makeHistoryFeed(suggestion: suggestion))
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()
        await model.loadHistorySuggestions()
        model.beginCreatingSchedule(from: suggestion)
        let reviewedDraft = try XCTUnwrap(model.editor?.draft)

        let substitutedDraft = AutomationScheduleEditorDraft(
            scope: .category(.developerArtifact)
        )
        model.setEditorDraft(substitutedDraft)

        XCTAssertEqual(model.state, .failed(.draft(.immutableFieldsChanged)))
        XCTAssertEqual(model.editor?.draft, reviewedDraft)
        let calls = await spy.mutationCalls()
        XCTAssertTrue(calls.isEmpty)
        await model.shutdown()
    }

    func testEditRequiresExactReviewedDisabledRevisionAndPreservesCompleteInput() async throws {
        let initial = try makeOverview(scheduleRevision: 7)
        let updated = try makeOverview(scheduleRevision: 8)
        let spy = AutomationScheduleServiceSpy(response: initial)
        await spy.setMutationResponse(updated)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        model.beginEditingSchedule(id: "automation:test")
        var draft = try XCTUnwrap(model.editor?.draft)
        draft.minimumAgeValue = "31"
        draft.minimumAgeUnit = .days
        model.setEditorDraft(draft)
        await model.saveEditor(decimalSeparator: ".")

        let calls = await spy.mutationCalls()
        guard case let .replace(id, revision, configuration) = try XCTUnwrap(calls.first) else {
            return XCTFail("Expected replace call")
        }
        XCTAssertEqual(id, "automation:test")
        XCTAssertEqual(revision, 7)
        XCTAssertEqual(configuration.scope, .category(.developerArtifact))
        XCTAssertEqual(configuration.cadence, .monthly)
        XCTAssertEqual(configuration.minimumAgeSeconds, 31 * 24 * 60 * 60)
        XCTAssertEqual(configuration.minimumReclaimableBytes, 0)
        XCTAssertEqual(
            configuration.maximumBytesPerRun,
            AutomationScheduleDefaults.maximumBytesPerRun
        )
        XCTAssertTrue(configuration.notifyBeforeRun)
        XCTAssertEqual(configuration.confirmationMode, .requireConfirmation)
        XCTAssertEqual(model.overview, updated)
        XCTAssertNil(model.editor)
        await model.shutdown()
    }

    func testEditorValidationRetainsDraftAndDoesNotCallService() async throws {
        let spy = try AutomationScheduleServiceSpy(response: makeOverview())
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()
        model.beginEditingSchedule(id: "automation:test")
        var draft = try XCTUnwrap(model.editor?.draft)
        draft.maximumBytesPerRunGiB = "0"
        model.setEditorDraft(draft)

        await model.saveEditor(decimalSeparator: ".")

        XCTAssertEqual(
            model.state,
            .failed(.draft(.zero(.maximumBytesPerRun)))
        )
        XCTAssertEqual(model.editor?.draft.maximumBytesPerRunGiB, "0")
        let calls = await spy.mutationCalls()
        XCTAssertTrue(calls.isEmpty)
        await model.shutdown()
    }

    func testEditorConflictRetainsProposalAndRequiresRefreshThenExactReReview() async throws {
        let original = try makeOverview(scheduleRevision: 3)
        let spy = AutomationScheduleServiceSpy(response: original)
        await spy.failNextMutation(.revisionConflict)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()
        model.beginEditingSchedule(id: "automation:test")
        var draft = try XCTUnwrap(model.editor?.draft)
        draft.minimumAgeValue = "45"
        model.setEditorDraft(draft)

        await model.saveEditor(decimalSeparator: ".")

        XCTAssertTrue(model.requiresRefresh)
        XCTAssertTrue(model.editor?.requiresReReview == true)
        XCTAssertEqual(model.editor?.draft.minimumAgeValue, "45")
        await model.saveEditor(decimalSeparator: ".")
        var calls = await spy.mutationCalls()
        XCTAssertEqual(calls.count, 1)

        let refreshed = try makeOverview(scheduleRevision: 4)
        await spy.setResponse(refreshed)
        await model.load(force: true)
        XCTAssertFalse(model.requiresRefresh)
        XCTAssertTrue(model.editor?.requiresReReview == true)

        model.reviewEditedScheduleAgain()
        XCTAssertFalse(model.editor?.requiresReReview == true)
        XCTAssertEqual(model.editor?.draft.minimumAgeValue, "30")
        guard case let .edit(_, revision) = model.editor?.mode else {
            return XCTFail("Expected refreshed edit session")
        }
        XCTAssertEqual(revision, 4)
        calls = await spy.mutationCalls()
        XCTAssertEqual(calls.count, 1)
        await model.shutdown()
    }

    func testUnknownCreateIsNeverRetriedAndMustCloseAfterAuthoritativeRefresh() async throws {
        let initial = try makeOverview(includeSchedule: false)
        let suggestion = try makeHistorySuggestion()
        let spy = AutomationScheduleServiceSpy(response: initial)
        try await spy.setHistoryResponse(makeHistoryFeed(suggestion: suggestion))
        await spy.failNextMutation(.outcomeUnknown)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()
        await model.loadHistorySuggestions()
        model.beginCreatingSchedule(from: suggestion)

        await model.saveEditor(decimalSeparator: ".")
        await model.saveEditor(decimalSeparator: ".")

        var calls = await spy.mutationCalls()
        XCTAssertEqual(calls.count, 1)
        XCTAssertTrue(model.requiresRefresh)
        XCTAssertTrue(model.editor?.requiresReReview == true)

        await model.load(force: true)
        XCTAssertFalse(model.requiresRefresh)
        XCTAssertTrue(model.editor?.requiresReReview == true)
        model.cancelEditor()
        XCTAssertNil(model.editor)
        calls = await spy.mutationCalls()
        XCTAssertEqual(calls.count, 1)
        await model.shutdown()
    }

    func testStaleEditOutcomesFenceRetryAndRefreshToReviewOrClose() async throws {
        let cases: [(
            failure: AutomationScheduleServiceError,
            refreshed: AutomationScheduleOverviewModel,
            canReview: Bool
        )] = try [
            (.notFound, makeOverview(includeSchedule: false), false),
            (
                .invalidStateTransition,
                makeOverview(scheduleState: .enabled, scheduleRevision: 4),
                false
            ),
            (.outcomeUnknown, makeOverview(scheduleRevision: 4), true),
        ]

        for testCase in cases {
            let spy = try AutomationScheduleServiceSpy(
                response: makeOverview(scheduleRevision: 3)
            )
            await spy.failNextMutation(testCase.failure)
            let model = AutomationScheduleSettingsModel(service: spy)
            await model.load()
            model.beginEditingSchedule(id: "automation:test")

            await model.saveEditor(decimalSeparator: ".")
            await model.saveEditor(decimalSeparator: ".")

            var calls = await spy.mutationCalls()
            XCTAssertEqual(calls.count, 1, "Unexpected retry for \(testCase.failure)")
            XCTAssertTrue(model.requiresRefresh)
            XCTAssertTrue(model.editor?.requiresReReview == true)

            await spy.setResponse(testCase.refreshed)
            await model.load(force: true)

            XCTAssertFalse(model.requiresRefresh)
            XCTAssertEqual(model.canReviewEditedScheduleAgain, testCase.canReview)
            if testCase.canReview {
                model.reviewEditedScheduleAgain()
                XCTAssertFalse(model.editor?.requiresReReview == true)
                guard case let .edit(_, revision) = model.editor?.mode else {
                    return XCTFail("Expected refreshed edit session")
                }
                XCTAssertEqual(revision, 4)
            } else {
                model.cancelEditor()
                XCTAssertNil(model.editor)
            }
            calls = await spy.mutationCalls()
            XCTAssertEqual(calls.count, 1)
            await model.shutdown()
        }
    }

    func testSuggestionCreateRefusesTheBoundedSixtyFourScheduleLimit() async throws {
        let suggestion = try makeHistorySuggestion()
        let spy = try AutomationScheduleServiceSpy(response: makeFullOverview())
        try await spy.setHistoryResponse(makeHistoryFeed(suggestion: suggestion))
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()
        await model.loadHistorySuggestions()

        model.beginCreatingSchedule(from: suggestion)

        XCTAssertNil(model.editor)
        XCTAssertEqual(model.state, .failed(.service(.draftLimitExceeded)))
        let calls = await spy.mutationCalls()
        XCTAssertTrue(calls.isEmpty)
        await model.shutdown()
    }

    func testAuthoringMutationIsSerializedAndShutdownJoinsWithoutLatePublication() async throws {
        let original = try makeOverview(scheduleRevision: 3)
        let updated = try makeOverview(scheduleRevision: 4)
        let spy = AutomationScheduleServiceSpy(response: original)
        await spy.setMutationResponse(updated)
        await spy.suspendNextMutation()
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()
        model.beginEditingSchedule(id: "automation:test")

        let save = Task { @MainActor in
            await model.saveEditor(decimalSeparator: ".")
        }
        await spy.waitForMutationRequest()
        XCTAssertEqual(model.state, .editingSchedule("automation:test"))

        await model.saveEditor(decimalSeparator: ".")
        var calls = await spy.mutationCalls()
        XCTAssertEqual(calls.count, 1)

        let shutdown = Task { @MainActor in
            await model.shutdown()
        }
        await Task.yield()
        await spy.completeSuspendedMutation()
        await shutdown.value
        await save.value

        XCTAssertEqual(model.overview, original)
        XCTAssertEqual(model.state, .ready)
        XCTAssertNil(model.editor)
        calls = await spy.mutationCalls()
        XCTAssertEqual(calls.count, 1)
    }

    func testGlobalEnableRequiresExactConfirmationAndUsesReviewedRevision() async throws {
        let spy = try AutomationScheduleServiceSpy(response: makeOverview())
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        await model.setGlobalEnabled(true, confirmation: "enable automations")

        XCTAssertEqual(model.state, .failed(.confirmationRequired))
        var calls = await spy.mutationCalls()
        XCTAssertTrue(calls.isEmpty)

        await model.setGlobalEnabled(
            true,
            confirmation: AutomationScheduleSettingsModel.globalEnableConfirmation
        )

        calls = await spy.mutationCalls()
        XCTAssertEqual(
            calls,
            [.setGlobal(expectedRevision: 0, enabled: true)]
        )
        XCTAssertEqual(model.state, .ready)
        await model.shutdown()
    }

    func testGlobalDisableAndResetAreImmediateExactRevisionMutations() async throws {
        let enabled = try makeOverview(globalEnabled: true, globalRevision: 7)
        let spy = AutomationScheduleServiceSpy(response: enabled)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        await model.setGlobalEnabled(false)
        try await spy.setResponse(makeOverview(globalRevision: 8))
        await model.load(force: true)
        await model.resetGlobalControl()

        let calls = await spy.mutationCalls()
        XCTAssertEqual(
            calls,
            [
                .setGlobal(expectedRevision: 7, enabled: false),
                .resetGlobal(expectedRevision: 8),
            ]
        )
        await model.shutdown()
    }

    func testEnableRequiresDisabledScheduleAndAwaitingStaticPreflight() async throws {
        let blocked = try makeOverview(eligibility: .blockedByStaticPolicy)
        let spy = AutomationScheduleServiceSpy(response: blocked)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        await model.enableSchedule(id: "automation:test")
        XCTAssertEqual(model.state, .failed(.service(.staticPolicyBlocked)))
        let blockedCalls = await spy.mutationCalls()
        XCTAssertTrue(blockedCalls.isEmpty)

        try await spy.setResponse(makeOverview(eligibility: .awaitingRuntimeEvidence))
        await model.load(force: true)
        await model.enableSchedule(id: "automation:test")

        let calls = await spy.mutationCalls()
        XCTAssertEqual(
            calls,
            [.enable(id: "automation:test", expectedRevision: 1)]
        )
        await model.shutdown()
    }

    func testEveryScheduleTransitionUsesReviewedIdentityAndPublishesFullOverview() async throws {
        let initial = try makeOverview(
            scheduleState: .enabled,
            scheduleRevision: 4
        )
        let paused = try makeOverview(
            scheduleState: .paused(.user),
            scheduleRevision: 5
        )
        let spy = AutomationScheduleServiceSpy(response: initial)
        await spy.setMutationResponse(paused)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        await model.pauseSchedule(id: "automation:test")

        XCTAssertEqual(model.overview, paused)
        var calls = await spy.mutationCalls()
        XCTAssertEqual(
            calls,
            [.pause(id: "automation:test", expectedRevision: 4)]
        )

        let resumed = try makeOverview(scheduleState: .enabled, scheduleRevision: 6)
        await spy.setMutationResponse(resumed)
        await model.resumeSchedule(id: "automation:test")
        XCTAssertEqual(model.overview, resumed)

        let disabled = try makeOverview(scheduleState: .disabled, scheduleRevision: 7)
        await spy.setMutationResponse(disabled)
        await model.disableSchedule(id: "automation:test")
        XCTAssertEqual(model.overview, disabled)

        let empty = try makeOverview(includeSchedule: false)
        await spy.setMutationResponse(empty)
        await model.deleteSchedule(id: "automation:test", confirmed: true)
        XCTAssertEqual(model.overview, empty)

        calls = await spy.mutationCalls()
        XCTAssertEqual(
            calls,
            [
                .pause(id: "automation:test", expectedRevision: 4),
                .resume(id: "automation:test", expectedRevision: 5),
                .disable(id: "automation:test", expectedRevision: 6),
                .delete(id: "automation:test", expectedRevision: 7),
            ]
        )
        await model.shutdown()
    }

    func testFailurePauseCannotBeResumedByUser() async throws {
        let spy = try AutomationScheduleServiceSpy(
            response: makeOverview(
                scheduleState: .paused(.failure),
                scheduleRevision: 2
            )
        )
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        await model.resumeSchedule(id: "automation:test")

        XCTAssertEqual(model.state, .failed(.service(.invalidStateTransition)))
        let calls = await spy.mutationCalls()
        XCTAssertTrue(calls.isEmpty)
        await model.shutdown()
    }

    func testDeleteRequiresConfirmation() async throws {
        let spy = try AutomationScheduleServiceSpy(response: makeOverview())
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        await model.deleteSchedule(id: "automation:test", confirmed: false)

        XCTAssertEqual(model.state, .failed(.deletionConfirmationRequired))
        let calls = await spy.mutationCalls()
        XCTAssertTrue(calls.isEmpty)
        await model.shutdown()
    }

    func testOneMutationIsSerializedAndLateSecondActionIsIgnored() async throws {
        let spy = try AutomationScheduleServiceSpy(
            response: makeOverview(
                scheduleState: .enabled,
                scheduleRevision: 3
            )
        )
        await spy.suspendNextMutation()
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        let pause = Task { @MainActor in
            await model.pauseSchedule(id: "automation:test")
        }
        await spy.waitForMutationRequest()
        XCTAssertEqual(model.state, .pausingSchedule("automation:test"))

        await model.disableSchedule(id: "automation:test")
        let calls = await spy.mutationCalls()
        XCTAssertEqual(calls.count, 1)

        await spy.completeSuspendedMutation()
        await pause.value
        XCTAssertEqual(model.state, .ready)
        await model.shutdown()
    }

    func testRevisionConflictPreservesConfirmedOverviewAndRequiresRefresh() async throws {
        let original = try makeOverview(scheduleState: .enabled, scheduleRevision: 3)
        let spy = AutomationScheduleServiceSpy(response: original)
        await spy.failNextMutation(.revisionConflict)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        await model.pauseSchedule(id: "automation:test")

        XCTAssertEqual(model.overview, original)
        XCTAssertEqual(model.state, .failed(.service(.revisionConflict)))
        XCTAssertTrue(model.requiresRefresh)

        await model.disableSchedule(id: "automation:test")
        let calls = await spy.mutationCalls()
        XCTAssertEqual(calls.count, 1)

        let refreshed = try makeOverview(scheduleState: .paused(.user), scheduleRevision: 4)
        await spy.setResponse(refreshed)
        await model.load(force: true)
        XCTAssertEqual(model.overview, refreshed)
        XCTAssertFalse(model.requiresRefresh)
        await model.shutdown()
    }

    func testOutcomeUnknownIsDistinctAndFencesFurtherMutation() async throws {
        let original = try makeOverview(scheduleState: .enabled, scheduleRevision: 3)
        let spy = AutomationScheduleServiceSpy(response: original)
        await spy.failNextMutation(.outcomeUnknown)
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        await model.pauseSchedule(id: "automation:test")

        XCTAssertEqual(model.overview, original)
        XCTAssertEqual(model.state, .failed(.service(.outcomeUnknown)))
        XCTAssertTrue(model.requiresRefresh)
        await model.shutdown()
    }

    func testShutdownCancelsAndJoinsMutationWithoutLatePublication() async throws {
        let original = try makeOverview(scheduleState: .enabled, scheduleRevision: 3)
        let paused = try makeOverview(scheduleState: .paused(.user), scheduleRevision: 4)
        let spy = AutomationScheduleServiceSpy(response: original)
        await spy.setMutationResponse(paused)
        await spy.suspendNextMutation()
        let model = AutomationScheduleSettingsModel(service: spy)
        await model.load()

        let mutation = Task { @MainActor in
            await model.pauseSchedule(id: "automation:test")
        }
        await spy.waitForMutationRequest()
        let shutdown = Task { @MainActor in
            await model.shutdown()
        }
        await Task.yield()
        await spy.completeSuspendedMutation()
        await shutdown.value
        await mutation.value

        XCTAssertEqual(model.overview, original)
        XCTAssertEqual(model.state, .ready)
    }

    func testAccessibilityIdentifiersRemainUniqueAndDisjoint() {
        let staticIdentifiers = AutomationScheduleAccessibility.allStaticIdentifiers
        XCTAssertEqual(Set(staticIdentifiers).count, staticIdentifiers.count)

        let dynamic = [
            AutomationScheduleAccessibility.scheduleRow(0),
            AutomationScheduleAccessibility.scheduleEligibility(0),
            AutomationScheduleAccessibility.scheduleState(0),
            AutomationScheduleAccessibility.scheduleRecurrence(0),
            AutomationScheduleAccessibility.scheduleEnable(0),
            AutomationScheduleAccessibility.schedulePause(0),
            AutomationScheduleAccessibility.scheduleResume(0),
            AutomationScheduleAccessibility.scheduleDisable(0),
            AutomationScheduleAccessibility.scheduleDelete(0),
            AutomationScheduleAccessibility.scheduleEdit(0),
            AutomationScheduleAccessibility.historySuggestionRow(rank: 1),
            AutomationScheduleAccessibility.historySuggestionCreate(rank: 1),
        ]
        XCTAssertEqual(Set(dynamic).count, dynamic.count)
        XCTAssertTrue(Set(staticIdentifiers).isDisjoint(with: dynamic))
        XCTAssertTrue(
            Set(staticIdentifiers + dynamic).isDisjoint(
                with: PermanentCleanupPolicyAccessibility.allControlIdentifiers
            )
        )
    }

    func testFailureMessagesDistinguishConflictUnknownAndPolicyRefusal() {
        let conflict = AutomationScheduleSettingsView.message(
            for: .service(.revisionConflict)
        )
        let unknown = AutomationScheduleSettingsView.message(
            for: .service(.outcomeUnknown)
        )
        let policy = AutomationScheduleSettingsView.message(
            for: .service(.staticPolicyBlocked)
        )
        XCTAssertNotEqual(conflict, unknown)
        XCTAssertNotEqual(unknown, policy)
        XCTAssertTrue(conflict.localizedCaseInsensitiveContains("refresh"))
        XCTAssertTrue(unknown.localizedCaseInsensitiveContains("complete state"))
    }

    func testAuthoringMessagesDistinguishDraftLimitValidationAndReviewFence() {
        let limit = AutomationScheduleSettingsView.message(
            for: .service(.draftLimitExceeded)
        )
        let validation = AutomationScheduleSettingsView.message(
            for: .draft(.zero(.maximumBytesPerRun))
        )
        let review = AutomationScheduleSettingsView.message(
            for: .editorReviewRequired
        )

        XCTAssertTrue(limit.localizedCaseInsensitiveContains("64"))
        XCTAssertTrue(validation.localizedCaseInsensitiveContains("greater than zero"))
        XCTAssertTrue(review.localizedCaseInsensitiveContains("refresh"))
        XCTAssertNotEqual(limit, validation)
        XCTAssertNotEqual(validation, review)
    }

    func testNotificationSummaryDisclosesPreferenceAndRemainingCount() throws {
        let enabled = try makeOverview().schedules[0]
        XCTAssertTrue(
            AutomationScheduleSettingsView.notificationSummary(enabled)
                .localizedCaseInsensitiveContains("3")
        )
    }
}

private enum AutomationScheduleMutationCall: Equatable, Sendable {
    case create(configuration: AutomationScheduleDraftConfigurationModel)
    case replace(
        id: String,
        expectedRevision: UInt64,
        configuration: AutomationScheduleDraftConfigurationModel
    )
    case setGlobal(expectedRevision: UInt64, enabled: Bool)
    case resetGlobal(expectedRevision: UInt64)
    case enable(id: String, expectedRevision: UInt64)
    case pause(id: String, expectedRevision: UInt64)
    case resume(id: String, expectedRevision: UInt64)
    case disable(id: String, expectedRevision: UInt64)
    case delete(id: String, expectedRevision: UInt64)
}

private actor AutomationScheduleServiceSpy: DuxAutomationScheduleServing {
    private var response: AutomationScheduleOverviewModel
    private var mutationResponse: AutomationScheduleOverviewModel
    private var historyResponse = AutomationScheduleHistorySuggestionFeedModel.unavailable
    private var loadCount = 0
    private var historyLoadCount = 0
    private var calls: [AutomationScheduleMutationCall] = []
    private var nextMutationFailure: AutomationScheduleServiceError?
    private var shouldSuspendNextMutation = false
    private var mutationStarted = false
    private var mutationWaiter: CheckedContinuation<Void, Never>?
    private var suspendedMutation:
        CheckedContinuation<AutomationScheduleOverviewUpdateModel, Never>?

    init(response: AutomationScheduleOverviewModel) {
        self.response = response
        mutationResponse = response
    }

    func loadAutomationScheduleOverview() async throws -> AutomationScheduleOverviewModel {
        loadCount += 1
        return response
    }

    func loadAutomationScheduleHistorySuggestions() async throws
        -> AutomationScheduleHistorySuggestionFeedModel
    {
        historyLoadCount += 1
        return historyResponse
    }

    func createAutomationSchedule(
        configuration: AutomationScheduleDraftConfigurationModel
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(.create(configuration: configuration))
    }

    func replaceAutomationSchedule(
        id: String,
        expectedRevision: UInt64,
        configuration: AutomationScheduleDraftConfigurationModel
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(
            .replace(
                id: id,
                expectedRevision: expectedRevision,
                configuration: configuration
            )
        )
    }

    func setAutomationGlobalEnabled(
        expectedRevision: UInt64,
        enabled: Bool
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(.setGlobal(expectedRevision: expectedRevision, enabled: enabled))
    }

    func resetAutomationGlobalControl(
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(.resetGlobal(expectedRevision: expectedRevision))
    }

    func enableAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(.enable(id: id, expectedRevision: expectedRevision))
    }

    func pauseAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(.pause(id: id, expectedRevision: expectedRevision))
    }

    func resumeAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(.resume(id: id, expectedRevision: expectedRevision))
    }

    func disableAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(.disable(id: id, expectedRevision: expectedRevision))
    }

    func deleteAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        try await perform(.delete(id: id, expectedRevision: expectedRevision))
    }

    func setResponse(_ value: AutomationScheduleOverviewModel) {
        response = value
        mutationResponse = value
    }

    func setMutationResponse(_ value: AutomationScheduleOverviewModel) {
        mutationResponse = value
    }

    func setHistoryResponse(_ value: AutomationScheduleHistorySuggestionFeedModel) {
        historyResponse = value
    }

    func failNextMutation(_ failure: AutomationScheduleServiceError) {
        nextMutationFailure = failure
    }

    func suspendNextMutation() {
        shouldSuspendNextMutation = true
    }

    func waitForMutationRequest() async {
        guard !mutationStarted else {
            return
        }
        await withCheckedContinuation { continuation in
            mutationWaiter = continuation
        }
    }

    func completeSuspendedMutation() {
        suspendedMutation?.resume(
            returning: AutomationScheduleOverviewUpdateModel(
                overview: mutationResponse,
                changed: true
            )
        )
        suspendedMutation = nil
    }

    func loadRequestCount() -> Int { loadCount }
    func historyLoadRequestCount() -> Int { historyLoadCount }
    func mutationCalls() -> [AutomationScheduleMutationCall] { calls }

    private func perform(
        _ call: AutomationScheduleMutationCall
    ) async throws -> AutomationScheduleOverviewUpdateModel {
        calls.append(call)
        mutationStarted = true
        mutationWaiter?.resume()
        mutationWaiter = nil
        if let nextMutationFailure {
            self.nextMutationFailure = nil
            throw nextMutationFailure
        }
        if shouldSuspendNextMutation {
            shouldSuspendNextMutation = false
            return await withCheckedContinuation { continuation in
                suspendedMutation = continuation
            }
        }
        return AutomationScheduleOverviewUpdateModel(
            overview: mutationResponse,
            changed: true
        )
    }
}

private func makeOverview(
    globalEnabled: Bool = false,
    globalRevision: UInt64? = nil,
    includeSchedule: Bool = true,
    scheduleState: DuxAutomationScheduleState = .disabled,
    scheduleRevision: UInt64 = 1,
    eligibility: DuxAutomationScheduleEligibilityStatus = .awaitingRuntimeEvidence
) throws -> AutomationScheduleOverviewModel {
    let resolvedGlobalRevision = globalRevision ?? (globalEnabled ? 1 : 0)
    let global = try AutomationGlobalControlModel(
        enabled: globalEnabled,
        source: resolvedGlobalRevision == 0 ? .default : .stored,
        revision: resolvedGlobalRevision,
        updatedAtUnixMilliseconds: resolvedGlobalRevision == 0 ? nil : 100
    )
    guard includeSchedule else {
        return try AutomationScheduleOverviewModel(
            recordVersion: 3,
            globalControl: global,
            executionAvailable: false,
            eligibleRuleCount: 1,
            schedules: [],
            scheduleEligibility: []
        )
    }
    let recurrence: AutomationScheduleRecurrenceModel? = switch scheduleState {
    case .disabled:
        nil
    case .enabled, .paused:
        try AutomationScheduleRecurrenceModel(
            recordVersion: 1,
            cursorRevision: scheduleRevision,
            recurrencePolicyRevision: 1,
            anchorAtUnixMilliseconds: 100,
            nextOccurrenceOrdinal: 1,
            nextRunAtUnixMilliseconds: 200
        )
    }
    let schedule = try AutomationScheduleModel(
        scheduleID: "automation:test",
        scope: .category(.developerArtifact),
        cadence: .monthly,
        minimumAgeSeconds: AutomationScheduleDefaults.minimumAgeSeconds,
        minimumReclaimableBytes: 0,
        maximumBytesPerRun: AutomationScheduleDefaults.maximumBytesPerRun,
        exclusions: [],
        notifyBeforeRun: true,
        notifyBeforeRunsRemaining: 3,
        confirmationMode: .requireConfirmation,
        state: scheduleState,
        recurrence: recurrence,
        revision: scheduleRevision,
        createdAtUnixMilliseconds: 100,
        updatedAtUnixMilliseconds: 100 + Int64(scheduleRevision)
    )
    let assessment = try AutomationScheduleEligibilityModel(
        recordVersion: 1,
        policyRevision: 1,
        scheduleID: schedule.scheduleID,
        scheduleRevision: schedule.revision,
        status: eligibility,
        includedStaticallyEligibleRuleCount: eligibility == .awaitingRuntimeEvidence ? 1 : 0,
        reasons: eligibility == .awaitingRuntimeEvidence
            ? [] : [.scopeRuleNotMarkedScheduleEligible]
    )
    return try AutomationScheduleOverviewModel(
        recordVersion: 3,
        globalControl: global,
        executionAvailable: false,
        eligibleRuleCount: 1,
        schedules: [schedule],
        scheduleEligibility: [assessment]
    )
}

private func makeHistorySuggestion() throws -> AutomationScheduleHistorySuggestionModel {
    try AutomationScheduleHistorySuggestionModel(
        recordVersion: 1,
        rank: 1,
        rule: DuxAutomationScheduleRuleReference(
            ruleID: "developer.cache",
            ruleRevision: 2
        ),
        successfulManualRunCount: 2,
        manualRegrowthCycleCount: 1,
        latestManualAttemptAtUnixMilliseconds: 200,
        latestRegrowthAtUnixMilliseconds: 100
    )
}

private func makeHistoryFeed(
    suggestion: AutomationScheduleHistorySuggestionModel
) throws -> AutomationScheduleHistorySuggestionFeedModel {
    try AutomationScheduleHistorySuggestionFeedModel(
        recordVersion: 1,
        derivationRevision: 1,
        sourceSessionCount: 2,
        hasOlderSourceSessions: false,
        qualifyingRuleCount: 1,
        suggestions: [suggestion]
    )
}

private func makeFullOverview() throws -> AutomationScheduleOverviewModel {
    let global = try AutomationGlobalControlModel(
        enabled: false,
        source: .default,
        revision: 0,
        updatedAtUnixMilliseconds: nil
    )
    var schedules: [AutomationScheduleModel] = []
    var assessments: [AutomationScheduleEligibilityModel] = []
    for index in 0 ..< AutomationScheduleOverviewModel.maximumScheduleCount {
        let schedule = try AutomationScheduleModel(
            scheduleID: "automation:\(index)",
            scope: .category(.developerArtifact),
            cadence: .monthly,
            minimumAgeSeconds: AutomationScheduleDefaults.minimumAgeSeconds,
            minimumReclaimableBytes: 0,
            maximumBytesPerRun: AutomationScheduleDefaults.maximumBytesPerRun,
            exclusions: [],
            notifyBeforeRun: true,
            notifyBeforeRunsRemaining: 3,
            confirmationMode: .requireConfirmation,
            state: .disabled,
            recurrence: nil,
            revision: 1,
            createdAtUnixMilliseconds: 100,
            updatedAtUnixMilliseconds: 1000 - Int64(index)
        )
        schedules.append(schedule)
        try assessments.append(
            AutomationScheduleEligibilityModel(
                recordVersion: 1,
                policyRevision: 1,
                scheduleID: schedule.scheduleID,
                scheduleRevision: schedule.revision,
                status: .blockedByStaticPolicy,
                includedStaticallyEligibleRuleCount: 0,
                reasons: [.scopeRuleNotMarkedScheduleEligible]
            )
        )
    }
    return try AutomationScheduleOverviewModel(
        recordVersion: 3,
        globalControl: global,
        executionAvailable: false,
        eligibleRuleCount: 1,
        schedules: schedules,
        scheduleEligibility: assessments
    )
}
