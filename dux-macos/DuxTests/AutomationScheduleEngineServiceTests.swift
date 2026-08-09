@testable import DUX
import Foundation
import XCTest

@MainActor
final class AutomationScheduleEngineServiceTests: XCTestCase {
    func testLoadsEmptyOverviewOffMainThread() async throws {
        let engine = AutomationScheduleOverviewEngine(
            overview: generatedAutomationScheduleOverview()
        )
        let service = EngineService(engine: engine)

        let overview = try await service.loadAutomationScheduleOverview()

        XCTAssertEqual(engine.executedOnMainThread, false)
        XCTAssertEqual(engine.loadCount, 1)
        XCTAssertEqual(overview.disabledDrafts, [])
        XCTAssertFalse(overview.globalEnabled)
        XCTAssertFalse(overview.executionAvailable)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testMapsEveryPathFreeDraftFieldWithoutGrantingAuthority() throws {
        let exclusionA = AutomationScheduleRuleReference(
            ruleId: "developer.a",
            ruleRevision: 1
        )
        let exclusionZ = AutomationScheduleRuleReference(
            ruleId: "developer.z",
            ruleRevision: 2
        )
        let raw = generatedAutomationScheduleOverview(
            eligibleRuleCount: 14,
            drafts: [
                generatedAutomationScheduleDraft(
                    scheduleID: "schedule:newest",
                    cadence: .lowDiskOnly,
                    minimumAgeSeconds: 604_800,
                    minimumReclaimableBytes: 1_073_741_824,
                    maximumBytesPerRun: 5_368_709_120,
                    excludedRules: [exclusionA, exclusionZ],
                    confirmationMode: .fullyAutomatic,
                    revision: 8,
                    createdAtUnixMS: 100,
                    updatedAtUnixMS: 300,
                    notificationsRemaining: 2
                ),
                generatedAutomationScheduleDraft(
                    scheduleID: "schedule:older",
                    scope: .rule(
                        ruleId: "developer.rust.target",
                        ruleRevision: 3
                    ),
                    cadence: .weekly,
                    notifyBeforeRun: false,
                    revision: 2,
                    createdAtUnixMS: 50,
                    updatedAtUnixMS: 200,
                    notificationsRemaining: 0
                ),
            ]
        )

        let overview = try EngineService.automationScheduleOverview(raw)

        XCTAssertEqual(overview.eligibleRuleCount, 14)
        XCTAssertEqual(overview.disabledDrafts.map(\.id), ["schedule:newest", "schedule:older"])
        let newest = try XCTUnwrap(overview.disabledDrafts.first)
        XCTAssertEqual(newest.scope, .category(.developerArtifact))
        XCTAssertEqual(newest.cadence, .lowDiskOnly)
        XCTAssertEqual(newest.minimumAgeSeconds, 604_800)
        XCTAssertEqual(newest.minimumReclaimableBytes, 1_073_741_824)
        XCTAssertEqual(newest.maximumBytesPerRun, 5_368_709_120)
        XCTAssertEqual(newest.exclusions.map(\.ruleID), ["developer.a", "developer.z"])
        XCTAssertEqual(newest.confirmationMode, .fullyAutomatic)
        XCTAssertEqual(newest.notifyBeforeRunsRemaining, 2)
        XCTAssertFalse(newest.enabled)
        guard case let .rule(rule) = overview.disabledDrafts[1].scope else {
            return XCTFail("Expected an exact-rule scope")
        }
        XCTAssertEqual(rule.ruleID, "developer.rust.target")
        XCTAssertEqual(rule.ruleRevision, 3)
    }

    func testRejectsOpenedGatesAndExcessEligibleRuleCount() {
        let malformed = [
            generatedAutomationScheduleOverview(globalEnabled: true),
            generatedAutomationScheduleOverview(executionAvailable: true),
            generatedAutomationScheduleOverview(eligibleRuleCount: 257),
        ]

        for overview in malformed {
            assertInvalidResponse(overview)
        }
    }

    func testRejectsNonCanonicalOrderAndMalformedGrammar() {
        let newest = generatedAutomationScheduleDraft(
            scheduleID: "schedule:newest",
            updatedAtUnixMS: 300
        )
        let older = generatedAutomationScheduleDraft(
            scheduleID: "schedule:older",
            updatedAtUnixMS: 200
        )
        let malformed = [
            generatedAutomationScheduleOverview(recordVersion: 2),
            generatedAutomationScheduleOverview(
                drafts: [generatedAutomationScheduleDraft(recordVersion: 2)]
            ),
            generatedAutomationScheduleOverview(drafts: [older, newest]),
            generatedAutomationScheduleOverview(
                drafts: [generatedAutomationScheduleDraft(scheduleID: "bad/schedule")]
            ),
            generatedAutomationScheduleOverview(
                drafts: [
                    generatedAutomationScheduleDraft(
                        scope: .rule(ruleId: "Developer.bad", ruleRevision: 1)
                    ),
                ]
            ),
            generatedAutomationScheduleOverview(
                drafts: [
                    generatedAutomationScheduleDraft(
                        excludedRules: [
                            AutomationScheduleRuleReference(
                                ruleId: "developer.bad_",
                                ruleRevision: 1
                            ),
                        ]
                    ),
                ]
            ),
        ]

        for overview in malformed {
            assertInvalidResponse(overview)
        }
    }

    func testRejectsMoreThanSixtyFourDrafts() {
        let drafts = (0 ... AutomationScheduleOverviewModel.maximumDraftCount).map {
            index in
            generatedAutomationScheduleDraft(
                scheduleID: "schedule:\(index)",
                updatedAtUnixMS: Int64(1000 - index)
            )
        }

        assertInvalidResponse(
            generatedAutomationScheduleOverview(drafts: drafts)
        )
    }

    func testMapsGeneratedFailuresConservatively() {
        XCTAssertEqual(EngineService.automationScheduleError(.Closed), .unavailable)
        XCTAssertEqual(EngineService.automationScheduleError(.Busy), .unavailable)
        XCTAssertEqual(EngineService.automationScheduleError(.Unavailable), .unavailable)
        XCTAssertEqual(
            EngineService.automationScheduleError(.InvalidRecordVersion),
            .incompatibleSchema
        )
        XCTAssertEqual(
            EngineService.automationScheduleError(.IncompatibleSchema),
            .incompatibleSchema
        )
        XCTAssertEqual(
            EngineService.automationScheduleError(.CorruptData),
            .invalidResponse
        )
        XCTAssertEqual(
            EngineService.automationScheduleError(.InternalState),
            .invalidResponse
        )
    }

    func testCloseFenceRejectsSubsequentLoadWithTypedUnavailableError() async {
        let service = EngineService(
            engine: AutomationScheduleOverviewEngine(
                overview: generatedAutomationScheduleOverview()
            )
        )
        let closed = await service.close()
        XCTAssertTrue(closed)

        do {
            _ = try await service.loadAutomationScheduleOverview()
            XCTFail("Expected the closed service to reject the load")
        } catch {
            XCTAssertEqual(error as? AutomationScheduleServiceError, .unavailable)
        }
    }

    func testGeneratedFailureUsesTypedServiceError() async {
        let service = EngineService(
            engine: AutomationScheduleOverviewEngine(error: .IncompatibleSchema)
        )

        do {
            _ = try await service.loadAutomationScheduleOverview()
            XCTFail("Expected the generated failure to be mapped")
        } catch {
            XCTAssertEqual(
                error as? AutomationScheduleServiceError,
                .incompatibleSchema
            )
        }
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testAppModelUsesItsEngineServiceByDefaultAndPreservesOverrideInjection() async throws {
        let engine = AutomationScheduleOverviewEngine(
            overview: generatedAutomationScheduleOverview(eligibleRuleCount: 7)
        )
        let engineService = EngineService(engine: engine)
        let defaultModel = AppModel(engineService: engineService)

        await defaultModel.automationScheduleSettings.load()

        XCTAssertEqual(defaultModel.automationScheduleSettings.overview?.eligibleRuleCount, 7)
        XCTAssertEqual(engine.loadCount, 1)
        await defaultModel.automationScheduleSettings.shutdown()

        let override = try FixedAutomationScheduleService(
            overview: AutomationScheduleOverviewModel(
                recordVersion: 1,
                globalEnabled: false,
                executionAvailable: false,
                eligibleRuleCount: 11,
                disabledDrafts: []
            )
        )
        let injectedModel = AppModel(
            engineService: engineService,
            automationScheduleService: override
        )

        await injectedModel.automationScheduleSettings.load()

        XCTAssertEqual(injectedModel.automationScheduleSettings.overview?.eligibleRuleCount, 11)
        XCTAssertEqual(engine.loadCount, 1)
        await injectedModel.automationScheduleSettings.shutdown()
        let closed = await engineService.close()
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

private struct FixedAutomationScheduleService: DuxAutomationScheduleServing {
    let overview: AutomationScheduleOverviewModel

    func loadAutomationScheduleOverview() async throws
        -> AutomationScheduleOverviewModel
    {
        overview
    }
}

private final class AutomationScheduleOverviewEngine: DuxEngine, @unchecked Sendable {
    private let overview: AutomationScheduleOverview?
    private let error: AutomationScheduleDraftError?
    private(set) var executedOnMainThread: Bool?
    private(set) var loadCount = 0

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("AutomationScheduleOverviewEngine cannot be lifted: \(handle)")
    }

    init(overview: AutomationScheduleOverview) {
        self.overview = overview
        error = nil
        super.init(noHandle: NoHandle())
    }

    init(error: AutomationScheduleDraftError) {
        overview = nil
        self.error = error
        super.init(noHandle: NoHandle())
    }

    override func getAutomationScheduleOverview() throws
        -> AutomationScheduleOverview
    {
        executedOnMainThread = Thread.isMainThread
        loadCount += 1
        if let error {
            throw error
        }
        return overview!
    }

    override func close() -> Bool {
        true
    }
}

private func generatedAutomationScheduleOverview(
    recordVersion: UInt32 = 1,
    globalEnabled: Bool = false,
    executionAvailable: Bool = false,
    eligibleRuleCount: UInt16 = 0,
    drafts: [AutomationScheduleDraft] = []
) -> AutomationScheduleOverview {
    AutomationScheduleOverview(
        recordVersion: recordVersion,
        globalEnabled: globalEnabled,
        executionAvailable: executionAvailable,
        eligibleRuleCount: eligibleRuleCount,
        disabledDrafts: drafts
    )
}

private func generatedAutomationScheduleDraft(
    recordVersion: UInt32 = 1,
    scheduleID: String = "schedule:test",
    scope: AutomationScheduleScope = .category(category: .developerArtifact),
    cadence: AutomationScheduleCadence = .monthly,
    minimumAgeSeconds: UInt64 = 2_592_000,
    minimumReclaimableBytes: UInt64 = 0,
    maximumBytesPerRun: UInt64 = 26_843_545_600,
    excludedRules: [AutomationScheduleRuleReference] = [],
    notifyBeforeRun: Bool = true,
    confirmationMode: AutomationScheduleConfirmationMode = .requireConfirmation,
    revision: UInt64 = 1,
    createdAtUnixMS: Int64 = 100,
    updatedAtUnixMS: Int64 = 200,
    notificationsRemaining: UInt8 = 3
) -> AutomationScheduleDraft {
    AutomationScheduleDraft(
        recordVersion: recordVersion,
        scheduleId: scheduleID,
        scope: scope,
        cadence: cadence,
        minimumAgeSeconds: minimumAgeSeconds,
        minimumReclaimableBytes: minimumReclaimableBytes,
        maximumBytesPerRun: maximumBytesPerRun,
        excludedRules: excludedRules,
        notifyBeforeRun: notifyBeforeRun,
        confirmationMode: confirmationMode,
        enabled: false,
        revision: revision,
        createdAtUnixMs: createdAtUnixMS,
        updatedAtUnixMs: updatedAtUnixMS,
        preRunNotificationsRemaining: notificationsRemaining
    )
}
