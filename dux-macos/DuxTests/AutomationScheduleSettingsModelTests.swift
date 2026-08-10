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
            AutomationScheduleAccessibility.historySuggestionRow(rank: 1),
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
}

private enum AutomationScheduleMutationCall: Equatable, Sendable {
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
        return .unavailable
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
