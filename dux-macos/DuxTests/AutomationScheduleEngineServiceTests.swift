@testable import DUX
import Foundation
import XCTest

final class AutomationScheduleEngineServiceTests: XCTestCase {
    func testOverviewLoadsOffMainThreadAndProjectsClosedDefault() async throws {
        let engine = AutomationScheduleEngine(overview: generatedOverview())
        let service = EngineService(engine: engine)

        let overview = try await service.loadAutomationScheduleOverview()

        XCTAssertEqual(engine.executedOnMainThread, false)
        XCTAssertEqual(overview.recordVersion, 3)
        XCTAssertEqual(overview.globalControl.source, .default)
        XCTAssertEqual(overview.globalControl.revision, 0)
        XCTAssertFalse(overview.executionAvailable)
        XCTAssertTrue(overview.schedules.isEmpty)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testOverviewProjectsGlobalActivationScheduleStateAndUTCCursorExactly() throws {
        let recurrence = generatedRecurrence(
            cursorRevision: 4,
            anchorAtUnixMS: 1_700_000_000_000,
            nextOccurrenceOrdinal: 5,
            nextRunAtUnixMS: 1_702_592_000_000
        )
        let schedule = generatedSchedule(
            state: .paused,
            pauseReason: .user,
            recurrence: recurrence,
            revision: 7
        )
        let generated = generatedOverview(
            globalControl: generatedGlobalControl(
                enabled: true,
                source: .stored,
                revision: 3,
                updatedAtUnixMS: 300
            ),
            eligibleRuleCount: 1,
            schedules: [schedule],
            assessments: [
                generatedEligibility(
                    scheduleRevision: 7,
                    status: .awaitingRuntimeEvidence,
                    includedRuleCount: 1,
                    reasons: []
                ),
            ]
        )

        let overview = try EngineService.automationScheduleOverview(generated)

        XCTAssertTrue(overview.globalControl.enabled)
        XCTAssertEqual(overview.globalControl.revision, 3)
        XCTAssertEqual(overview.schedules[0].state, .paused(.user))
        XCTAssertEqual(overview.schedules[0].recurrence?.cursorRevision, 4)
        XCTAssertEqual(
            overview.schedules[0].recurrence?.anchorAtUnixMilliseconds,
            1_700_000_000_000
        )
        XCTAssertEqual(
            overview.schedules[0].recurrence?.nextRunAtUnixMilliseconds,
            1_702_592_000_000
        )
    }

    func testOverviewRejectsEveryImpossibleGlobalControlShape() {
        for control in [
            generatedGlobalControl(enabled: true, source: .default),
            generatedGlobalControl(source: .stored, revision: 0, updatedAtUnixMS: nil),
            generatedGlobalControl(source: .default, revision: 1, updatedAtUnixMS: nil),
            generatedGlobalControl(source: .stored, revision: 1, updatedAtUnixMS: -1),
        ] {
            assertInvalidResponse(generatedOverview(globalControl: control))
        }
    }

    func testOverviewRejectsStatePauseAndRecurrenceMismatches() {
        let recurrence = generatedRecurrence()
        for schedule in [
            generatedSchedule(state: .disabled, pauseReason: .user),
            generatedSchedule(state: .disabled, recurrence: recurrence),
            generatedSchedule(state: .enabled),
            generatedSchedule(state: .enabled, pauseReason: .failure, recurrence: recurrence),
            generatedSchedule(state: .paused, recurrence: recurrence),
            generatedSchedule(
                cadence: .lowDiskOnly,
                state: .enabled,
                recurrence: recurrence
            ),
        ] {
            assertInvalidResponse(
                generatedOverview(
                    eligibleRuleCount: 1,
                    schedules: [schedule],
                    assessments: [generatedEligibility()]
                )
            )
        }
    }

    func testOverviewRejectsMalformedRecurrenceAndOpenExecutionGate() {
        let malformed = generatedSchedule(
            state: .enabled,
            recurrence: generatedRecurrence(nextRunAtUnixMS: 100)
        )
        assertInvalidResponse(
            generatedOverview(
                eligibleRuleCount: 1,
                schedules: [malformed],
                assessments: [generatedEligibility()]
            )
        )
        assertInvalidResponse(generatedOverview(executionAvailable: true))
    }

    func testOverviewRejectsDuplicateUnorderedOrMismatchedSchedules() {
        let newer = generatedSchedule(
            scheduleID: "automation:newer",
            revision: 2,
            updatedAtUnixMS: 300
        )
        let older = generatedSchedule(
            scheduleID: "automation:older",
            updatedAtUnixMS: 200
        )
        assertInvalidResponse(
            generatedOverview(
                eligibleRuleCount: 1,
                schedules: [newer, newer],
                assessments: [
                    generatedEligibility(scheduleID: newer.scheduleId, scheduleRevision: 2),
                    generatedEligibility(scheduleID: newer.scheduleId, scheduleRevision: 2),
                ]
            )
        )
        assertInvalidResponse(
            generatedOverview(
                eligibleRuleCount: 1,
                schedules: [older, newer],
                assessments: [
                    generatedEligibility(scheduleID: older.scheduleId),
                    generatedEligibility(scheduleID: newer.scheduleId, scheduleRevision: 2),
                ]
            )
        )
        assertInvalidResponse(
            generatedOverview(
                eligibleRuleCount: 1,
                schedules: [newer, older],
                assessments: [
                    generatedEligibility(scheduleID: older.scheduleId),
                    generatedEligibility(scheduleID: newer.scheduleId, scheduleRevision: 2),
                ]
            )
        )
    }

    func testCreateAndReplaceMapCompleteInputsOffMainAndReturnCompleteOverview() async throws {
        let update = generatedOverviewUpdate(overview: generatedOverview())
        let engine = AutomationScheduleEngine(overview: generatedOverview(), update: update)
        let service = EngineService(engine: engine)
        let exclusion = try DuxAutomationScheduleRuleReference(
            ruleID: "developer.excluded",
            ruleRevision: 3
        )
        let category = try AutomationScheduleDraftConfigurationModel(
            scope: .category(.browserCache),
            cadence: .weekly,
            minimumAgeSeconds: 3601,
            minimumReclaimableBytes: 53_687_091_200,
            maximumBytesPerRun: 26_843_545_600,
            exclusions: [exclusion],
            notifyBeforeRun: false,
            confirmationMode: .fullyAutomatic
        )
        let exactRule = try DuxAutomationScheduleRuleReference(
            ruleID: "developer.cache",
            ruleRevision: 4
        )
        let rule = try AutomationScheduleDraftConfigurationModel(
            scope: .rule(exactRule),
            cadence: .lowDiskOnly,
            minimumAgeSeconds: 0,
            minimumReclaimableBytes: 0,
            maximumBytesPerRun: 1,
            exclusions: [],
            notifyBeforeRun: true,
            confirmationMode: .requireConfirmation
        )

        let created = try await service.createAutomationSchedule(configuration: category)
        let replaced = try await service.replaceAutomationSchedule(
            id: "automation:test",
            expectedRevision: 9,
            configuration: rule
        )

        XCTAssertTrue(created.changed)
        XCTAssertTrue(replaced.changed)
        XCTAssertEqual(engine.executedMutationOnMainThreads, [false, false])
        guard case let .create(input) = engine.calls[0] else {
            return XCTFail("Expected create call")
        }
        XCTAssertEqual(input.recordVersion, 1)
        XCTAssertEqual(input.scope, .category(category: .browserCache))
        XCTAssertEqual(input.cadence, .weekly)
        XCTAssertEqual(input.minimumAgeSeconds, 3601)
        XCTAssertEqual(input.minimumReclaimableBytes, 53_687_091_200)
        XCTAssertEqual(input.maximumBytesPerRun, 26_843_545_600)
        XCTAssertEqual(
            input.excludedRules,
            [
                AutomationScheduleRuleReference(
                    ruleId: "developer.excluded",
                    ruleRevision: 3
                ),
            ]
        )
        XCTAssertFalse(input.notifyBeforeRun)
        XCTAssertEqual(input.confirmationMode, .fullyAutomatic)

        guard case let .replace(id, revision, replacement) = engine.calls[1] else {
            return XCTFail("Expected replace call")
        }
        XCTAssertEqual(id, "automation:test")
        XCTAssertEqual(revision, 9)
        XCTAssertEqual(
            replacement.scope,
            .rule(ruleId: "developer.cache", ruleRevision: 4)
        )
        XCTAssertEqual(replacement.cadence, .lowDiskOnly)
        XCTAssertEqual(replacement.minimumAgeSeconds, 0)
        XCTAssertEqual(replacement.maximumBytesPerRun, 1)
        XCTAssertTrue(replacement.notifyBeforeRun)
        XCTAssertEqual(replacement.confirmationMode, .requireConfirmation)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testActivationManagementMethodsRunOffMainAndReturnCompleteOverview() async throws {
        let update = generatedOverviewUpdate(
            overview: generatedOverview(
                globalControl: generatedGlobalControl(
                    enabled: true,
                    source: .stored,
                    revision: 2,
                    updatedAtUnixMS: 200
                )
            )
        )
        let engine = AutomationScheduleEngine(overview: generatedOverview(), update: update)
        let service = EngineService(engine: engine)

        let set = try await service.setAutomationGlobalEnabled(
            expectedRevision: 1,
            enabled: true
        )
        let reset = try await service.resetAutomationGlobalControl(expectedRevision: 2)
        let enabled = try await service.enableAutomationSchedule(
            id: "automation:test",
            expectedRevision: 3
        )
        let paused = try await service.pauseAutomationSchedule(
            id: "automation:test",
            expectedRevision: 4
        )
        let resumed = try await service.resumeAutomationSchedule(
            id: "automation:test",
            expectedRevision: 5
        )
        let disabled = try await service.disableAutomationSchedule(
            id: "automation:test",
            expectedRevision: 6
        )
        let deleted = try await service.deleteAutomationSchedule(
            id: "automation:test",
            expectedRevision: 7
        )
        let results = [set, reset, enabled, paused, resumed, disabled, deleted]

        XCTAssertTrue(results.allSatisfy(\.changed))
        XCTAssertTrue(results.allSatisfy { $0.overview.globalControl.enabled })
        XCTAssertTrue(engine.executedMutationOnMainThreads.allSatisfy { !$0 })
        XCTAssertEqual(
            engine.calls,
            [
                .setGlobal(expectedRevision: 1, enabled: true),
                .resetGlobal(expectedRevision: 2),
                .enable(id: "automation:test", expectedRevision: 3),
                .pause(id: "automation:test", expectedRevision: 4),
                .resume(id: "automation:test", expectedRevision: 5),
                .disable(id: "automation:test", expectedRevision: 6),
                .delete(id: "automation:test", expectedRevision: 7),
            ]
        )
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testMalformedPostCreateUpdateBecomesOutcomeUnknown() async throws {
        let update = generatedOverviewUpdate(
            recordVersion: 2,
            overview: generatedOverview()
        )
        let service = EngineService(
            engine: AutomationScheduleEngine(overview: generatedOverview(), update: update)
        )
        let configuration = try AutomationScheduleDraftConfigurationModel(
            scope: .category(.developerArtifact),
            cadence: .monthly,
            minimumAgeSeconds: 0,
            minimumReclaimableBytes: 0,
            maximumBytesPerRun: 1,
            exclusions: [],
            notifyBeforeRun: true,
            confirmationMode: .requireConfirmation
        )
        do {
            _ = try await service.createAutomationSchedule(
                configuration: configuration
            )
            XCTFail("Expected uncertain post-create outcome")
        } catch {
            XCTAssertEqual(error as? AutomationScheduleServiceError, .outcomeUnknown)
        }
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testGeneratedErrorsMapToActionableNativeFailures() {
        for (generated, expected) in [
            (AutomationScheduleDraftError.Closed, .unavailable),
            (.Unavailable, .unavailable),
            (.Busy, .retryable),
            (.BudgetExceeded, .retryable),
            (.IncompatibleSchema, .incompatibleSchema),
            (.InvalidMinimumAge, .invalidRequest),
            (.InvalidMinimumReclaimableBytes, .invalidRequest),
            (.InvalidMaximumBytesPerRun, .invalidRequest),
            (.DraftLimitExceeded, .draftLimitExceeded),
            (.InvalidRevision, .invalidRequest),
            (.NotFound, .notFound),
            (.RevisionConflict, .revisionConflict),
            (.InvalidStateTransition, .invalidStateTransition),
            (.StaticPolicyBlocked, .staticPolicyBlocked),
            (.ActivationUnavailable, .activationUnavailable),
            (.UnsafeStorage, .unsafeStorage),
            (.CorruptData, .corruptData),
            (.OutcomeUnknown, .outcomeUnknown),
            (.InternalState, .invalidResponse),
        ] as [(AutomationScheduleDraftError, AutomationScheduleServiceError)] {
            XCTAssertEqual(EngineService.automationScheduleError(generated), expected)
        }
    }

    func testClosedServiceFencesLoadsAndMutations() async throws {
        let service = EngineService(
            engine: AutomationScheduleEngine(overview: generatedOverview())
        )
        let closed = await service.close()
        XCTAssertTrue(closed)

        do {
            _ = try await service.loadAutomationScheduleOverview()
            XCTFail("Expected closed load")
        } catch {
            XCTAssertEqual(error as? AutomationScheduleServiceError, .unavailable)
        }
        do {
            _ = try await service.setAutomationGlobalEnabled(
                expectedRevision: 0,
                enabled: true
            )
            XCTFail("Expected closed mutation")
        } catch {
            XCTAssertEqual(error as? AutomationScheduleServiceError, .unavailable)
        }
        let configuration = try AutomationScheduleDraftConfigurationModel(
            scope: .category(.developerArtifact),
            cadence: .monthly,
            minimumAgeSeconds: 0,
            minimumReclaimableBytes: 0,
            maximumBytesPerRun: 1,
            exclusions: [],
            notifyBeforeRun: true,
            confirmationMode: .requireConfirmation
        )
        do {
            _ = try await service.createAutomationSchedule(configuration: configuration)
            XCTFail("Expected closed create")
        } catch {
            XCTAssertEqual(error as? AutomationScheduleServiceError, .unavailable)
        }
    }

    func testSuggestionFeedStillProjectsOffMainAndRemainsReadOnly() async throws {
        let feed = AutomationScheduleSuggestionFeed(
            recordVersion: 1,
            derivationRevision: 1,
            sourceSessionCount: 2,
            hasOlderSourceSessions: false,
            qualifyingRuleCount: 1,
            suggestions: [
                AutomationScheduleSuggestion(
                    recordVersion: 1,
                    rank: 1,
                    ruleId: "developer.cache",
                    ruleRevision: 1,
                    successfulManualRunCount: 2,
                    manualRegrowthCycleCount: 1,
                    latestManualAttemptAtUnixMs: 200,
                    latestRegrowthAtUnixMs: 100
                ),
            ]
        )
        let engine = AutomationScheduleSuggestionEngine(feed: feed)
        let service = EngineService(engine: engine)

        let projected = try await service.loadAutomationScheduleHistorySuggestions()

        XCTAssertEqual(engine.executedOnMainThread, false)
        XCTAssertEqual(projected.suggestions.first?.rule.ruleID, "developer.cache")
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    private func assertInvalidResponse(
        _ overview: AutomationScheduleOverview,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertThrowsError(
            try EngineService.automationScheduleOverview(overview),
            file: file,
            line: line
        ) { error in
            XCTAssertEqual(
                error as? AutomationScheduleServiceError,
                .invalidResponse,
                file: file,
                line: line
            )
        }
    }
}

private enum AutomationEngineCall: Equatable {
    case create(input: AutomationScheduleDraftInput)
    case replace(id: String, expectedRevision: UInt64, input: AutomationScheduleDraftInput)
    case setGlobal(expectedRevision: UInt64, enabled: Bool)
    case resetGlobal(expectedRevision: UInt64)
    case enable(id: String, expectedRevision: UInt64)
    case pause(id: String, expectedRevision: UInt64)
    case resume(id: String, expectedRevision: UInt64)
    case disable(id: String, expectedRevision: UInt64)
    case delete(id: String, expectedRevision: UInt64)
}

private final class AutomationScheduleEngine: DuxEngine, @unchecked Sendable {
    private let overview: AutomationScheduleOverview
    private let update: AutomationScheduleOverviewUpdate
    private let error: AutomationScheduleDraftError?
    private(set) var executedOnMainThread: Bool?
    private(set) var executedMutationOnMainThreads: [Bool] = []
    private(set) var calls: [AutomationEngineCall] = []

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("AutomationScheduleEngine cannot be lifted: \(handle)")
    }

    init(
        overview: AutomationScheduleOverview,
        update: AutomationScheduleOverviewUpdate? = nil,
        error: AutomationScheduleDraftError? = nil
    ) {
        self.overview = overview
        self.update = update ?? generatedOverviewUpdate(overview: overview)
        self.error = error
        super.init(noHandle: NoHandle())
    }

    override func getAutomationScheduleOverview() throws -> AutomationScheduleOverview {
        executedOnMainThread = Thread.isMainThread
        if let error {
            throw error
        }
        return overview
    }

    override func setAutomationGlobalEnabled(
        expectedRevision: UInt64,
        enabled: Bool
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(.setGlobal(expectedRevision: expectedRevision, enabled: enabled))
    }

    override func createAutomationScheduleDraft(
        input: AutomationScheduleDraftInput
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(.create(input: input))
    }

    override func replaceAutomationScheduleDraft(
        scheduleId: String,
        expectedRevision: UInt64,
        input: AutomationScheduleDraftInput
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(
            .replace(
                id: scheduleId,
                expectedRevision: expectedRevision,
                input: input
            )
        )
    }

    override func resetAutomationGlobalControl(
        expectedRevision: UInt64
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(.resetGlobal(expectedRevision: expectedRevision))
    }

    override func enableAutomationSchedule(
        scheduleId: String,
        expectedRevision: UInt64
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(.enable(id: scheduleId, expectedRevision: expectedRevision))
    }

    override func pauseAutomationSchedule(
        scheduleId: String,
        expectedRevision: UInt64
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(.pause(id: scheduleId, expectedRevision: expectedRevision))
    }

    override func resumeAutomationSchedule(
        scheduleId: String,
        expectedRevision: UInt64
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(.resume(id: scheduleId, expectedRevision: expectedRevision))
    }

    override func disableAutomationSchedule(
        scheduleId: String,
        expectedRevision: UInt64
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(.disable(id: scheduleId, expectedRevision: expectedRevision))
    }

    override func deleteAutomationScheduleDraft(
        scheduleId: String,
        expectedRevision: UInt64
    ) throws -> AutomationScheduleOverviewUpdate {
        try mutation(.delete(id: scheduleId, expectedRevision: expectedRevision))
    }

    override func close() -> Bool { true }

    private func mutation(_ call: AutomationEngineCall) throws
        -> AutomationScheduleOverviewUpdate
    {
        executedMutationOnMainThreads.append(Thread.isMainThread)
        calls.append(call)
        if let error {
            throw error
        }
        return update
    }
}

private final class AutomationScheduleSuggestionEngine: DuxEngine, @unchecked Sendable {
    private let feed: AutomationScheduleSuggestionFeed
    private(set) var executedOnMainThread: Bool?

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("AutomationScheduleSuggestionEngine cannot be lifted: \(handle)")
    }

    init(feed: AutomationScheduleSuggestionFeed) {
        self.feed = feed
        super.init(noHandle: NoHandle())
    }

    override func getAutomationScheduleSuggestions() throws
        -> AutomationScheduleSuggestionFeed
    {
        executedOnMainThread = Thread.isMainThread
        return feed
    }

    override func close() -> Bool { true }
}

private func generatedGlobalControl(
    recordVersion: UInt32 = 1,
    enabled: Bool = false,
    source: AutomationGlobalControlSource = .default,
    revision: UInt64 = 0,
    updatedAtUnixMS: Int64? = nil
) -> AutomationGlobalControlStatus {
    AutomationGlobalControlStatus(
        recordVersion: recordVersion,
        enabled: enabled,
        source: source,
        revision: revision,
        updatedAtUnixMs: updatedAtUnixMS
    )
}

private func generatedRecurrence(
    recordVersion: UInt32 = 1,
    cursorRevision: UInt64 = 1,
    recurrencePolicyRevision: UInt32 = 1,
    anchorAtUnixMS: Int64 = 100,
    nextOccurrenceOrdinal: UInt64 = 1,
    nextRunAtUnixMS: Int64 = 200
) -> AutomationSchedulePeriodicRecurrence {
    AutomationSchedulePeriodicRecurrence(
        recordVersion: recordVersion,
        cursorRevision: cursorRevision,
        recurrencePolicyRevision: recurrencePolicyRevision,
        anchorAtUnixMs: anchorAtUnixMS,
        nextOccurrenceOrdinal: nextOccurrenceOrdinal,
        nextRunAtUnixMs: nextRunAtUnixMS
    )
}

private func generatedSchedule(
    recordVersion: UInt32 = 1,
    scheduleID: String = "automation:test",
    cadence: AutomationScheduleCadence = .monthly,
    state: AutomationScheduleState = .disabled,
    pauseReason: AutomationSchedulePauseReason? = nil,
    recurrence: AutomationSchedulePeriodicRecurrence? = nil,
    revision: UInt64 = 1,
    updatedAtUnixMS: Int64 = 200
) -> AutomationScheduleStatus {
    AutomationScheduleStatus(
        recordVersion: recordVersion,
        scheduleId: scheduleID,
        scope: .category(category: .developerArtifact),
        cadence: cadence,
        minimumAgeSeconds: 2_592_000,
        minimumReclaimableBytes: 0,
        maximumBytesPerRun: 26_843_545_600,
        excludedRules: [],
        notifyBeforeRun: true,
        confirmationMode: .requireConfirmation,
        state: state,
        pauseReason: pauseReason,
        recurrence: recurrence,
        revision: revision,
        createdAtUnixMs: 100,
        updatedAtUnixMs: updatedAtUnixMS,
        preRunNotificationsRemaining: 3
    )
}

private func generatedEligibility(
    recordVersion: UInt32 = 1,
    policyRevision: UInt32 = 1,
    scheduleID: String = "automation:test",
    scheduleRevision: UInt64 = 1,
    status: AutomationScheduleEligibilityStatus = .blockedByStaticPolicy,
    includedRuleCount: UInt16 = 0,
    reasons: [AutomationScheduleEligibilityReason] = [.scopeRuleNotMarkedScheduleEligible]
) -> AutomationScheduleEligibilityAssessment {
    AutomationScheduleEligibilityAssessment(
        recordVersion: recordVersion,
        policyRevision: policyRevision,
        scheduleId: scheduleID,
        scheduleRevision: scheduleRevision,
        status: status,
        includedStaticallyEligibleRuleCount: includedRuleCount,
        reasons: reasons
    )
}

private func generatedOverview(
    recordVersion: UInt32 = 3,
    globalControl: AutomationGlobalControlStatus = generatedGlobalControl(),
    executionAvailable: Bool = false,
    eligibleRuleCount: UInt16 = 0,
    schedules: [AutomationScheduleStatus] = [],
    assessments: [AutomationScheduleEligibilityAssessment]? = nil
) -> AutomationScheduleOverview {
    AutomationScheduleOverview(
        recordVersion: recordVersion,
        globalControl: globalControl,
        executionAvailable: executionAvailable,
        eligibleRuleCount: eligibleRuleCount,
        schedules: schedules,
        scheduleEligibility: assessments ?? schedules.map {
            generatedEligibility(
                scheduleID: $0.scheduleId,
                scheduleRevision: $0.revision
            )
        }
    )
}

private func generatedOverviewUpdate(
    recordVersion: UInt32 = 1,
    overview: AutomationScheduleOverview,
    changed: Bool = true
) -> AutomationScheduleOverviewUpdate {
    AutomationScheduleOverviewUpdate(
        recordVersion: recordVersion,
        overview: overview,
        changed: changed
    )
}
