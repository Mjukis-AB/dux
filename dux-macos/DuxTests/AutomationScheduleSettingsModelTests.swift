@testable import DUX
import XCTest

@MainActor
final class AutomationScheduleSettingsModelTests: XCTestCase {
    func testUnavailableProductionSeamLoadsHonestDefaultOffOverview() async {
        let model = AutomationScheduleSettingsModel(
            service: UnavailableDuxAutomationScheduleService()
        )

        await model.load()
        await model.loadHistorySuggestions()

        XCTAssertEqual(model.state, .ready)
        XCTAssertEqual(model.overview, .unavailable)
        XCTAssertEqual(model.overview?.recordVersion, 2)
        XCTAssertEqual(model.overview?.globalEnabled, false)
        XCTAssertEqual(model.overview?.executionAvailable, false)
        XCTAssertEqual(model.overview?.eligibleRuleCount, 0)
        XCTAssertEqual(model.overview?.disabledDrafts, [])
        XCTAssertEqual(model.overview?.draftEligibility, [])
        XCTAssertEqual(model.historySuggestionState, .ready)
        XCTAssertEqual(model.historySuggestionFeed, .unavailable)
    }

    func testLoadCachesConfirmedOverviewAndForceReloads() async throws {
        let service = try AutomationScheduleServiceSpy(response: overview())
        let model = AutomationScheduleSettingsModel(service: service)

        await model.load()
        await model.load()
        let initialLoadCount = await service.loadCount()
        XCTAssertEqual(initialLoadCount, 1)
        XCTAssertEqual(model.state, .ready)
        XCTAssertEqual(model.overview?.disabledDrafts.count, 1)

        await model.load(force: true)
        let forcedLoadCount = await service.loadCount()
        XCTAssertEqual(forcedLoadCount, 2)
    }

    func testFailurePreservesLastConfirmedOverview() async throws {
        let confirmed = try overview()
        let service = AutomationScheduleServiceSpy(response: confirmed)
        let model = AutomationScheduleSettingsModel(service: service)
        await model.load()

        await service.failNext(.incompatibleSchema)
        await model.load(force: true)

        XCTAssertEqual(model.overview, confirmed)
        XCTAssertEqual(
            model.state,
            .failed(.service(.incompatibleSchema))
        )
    }

    func testSuggestionFailurePreservesOverviewAndLastConfirmedSuggestions() async throws {
        let confirmedOverview = try overview()
        let confirmedSuggestions = try suggestionFeed()
        let service = AutomationScheduleServiceSpy(
            response: confirmedOverview,
            historySuggestionResponse: confirmedSuggestions
        )
        let model = AutomationScheduleSettingsModel(service: service)

        await model.load()
        await model.loadHistorySuggestions()
        await service.failNextHistorySuggestion(.invalidResponse)
        await model.loadHistorySuggestions(force: true)

        XCTAssertEqual(model.overview, confirmedOverview)
        XCTAssertEqual(model.state, .ready)
        XCTAssertEqual(model.historySuggestionFeed, confirmedSuggestions)
        XCTAssertEqual(
            model.historySuggestionState,
            .failed(.service(.invalidResponse))
        )
    }

    func testOverviewFailureDoesNotHideConfirmedSuggestions() async throws {
        let confirmedSuggestions = try suggestionFeed()
        let service = try AutomationScheduleServiceSpy(
            response: overview(),
            historySuggestionResponse: confirmedSuggestions
        )
        let model = AutomationScheduleSettingsModel(service: service)

        await model.loadHistorySuggestions()
        await service.failNext(.incompatibleSchema)
        await model.load(force: true)

        XCTAssertEqual(model.historySuggestionFeed, confirmedSuggestions)
        XCTAssertEqual(model.historySuggestionState, .ready)
        XCTAssertEqual(model.state, .failed(.service(.incompatibleSchema)))
    }

    func testSuggestionLoadCachesAndForceRefreshesIndependently() async throws {
        let service = try AutomationScheduleServiceSpy(
            response: overview(),
            historySuggestionResponse: suggestionFeed()
        )
        let model = AutomationScheduleSettingsModel(service: service)

        await model.loadHistorySuggestions()
        await model.loadHistorySuggestions()
        let cachedSuggestionLoads = await service.historySuggestionLoadCount()
        let cachedOverviewLoads = await service.loadCount()
        XCTAssertEqual(cachedSuggestionLoads, 1)
        XCTAssertEqual(cachedOverviewLoads, 0)

        await model.loadHistorySuggestions(force: true)
        let refreshedSuggestionLoads = await service.historySuggestionLoadCount()
        let refreshedOverviewLoads = await service.loadCount()
        XCTAssertEqual(refreshedSuggestionLoads, 2)
        XCTAssertEqual(refreshedOverviewLoads, 0)
    }

    func testShutdownFencesFutureLoads() async throws {
        let service = try AutomationScheduleServiceSpy(response: overview())
        let model = AutomationScheduleSettingsModel(service: service)

        await model.shutdown()
        await model.load()
        await model.loadHistorySuggestions()

        let loadCount = await service.loadCount()
        let historySuggestionLoadCount = await service.historySuggestionLoadCount()
        XCTAssertEqual(loadCount, 0)
        XCTAssertEqual(historySuggestionLoadCount, 0)
        XCTAssertNil(model.overview)
        XCTAssertNil(model.historySuggestionFeed)
        XCTAssertEqual(model.state, .idle)
        XCTAssertEqual(model.historySuggestionState, .idle)
    }

    func testViewAccessibilityIdentifiersAreStableUniqueAndDisjoint() {
        let identifiers = AutomationScheduleAccessibility.allStaticIdentifiers
        XCTAssertEqual(identifiers.count, 18)
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
        XCTAssertEqual(
            Set([AutomationScheduleAccessibility.draftRow(0),
                 AutomationScheduleAccessibility.draftRow(1)]).count,
            2
        )
        XCTAssertNotEqual(
            AutomationScheduleAccessibility.draftRow(0),
            AutomationScheduleAccessibility.draftEligibility(0)
        )
        XCTAssertNotEqual(
            AutomationScheduleAccessibility.historySuggestionRow(rank: 1),
            AutomationScheduleAccessibility.historySuggestionRow(rank: 2)
        )
        XCTAssertFalse(
            identifiers.contains(
                AutomationScheduleAccessibility.historySuggestionRow(rank: 1)
            )
        )

        let existing = Set(
            SnapshotRetentionCapAccessibility.allControlIdentifiers
                + CleanupHistoryClearAccessibility.allControlIdentifiers
                + DiskPressurePolicyAccessibility.allControlIdentifiers
                + PermanentCleanupPolicyAccessibility.allControlIdentifiers
        )
        XCTAssertTrue(Set(identifiers).isDisjoint(with: existing))
        XCTAssertFalse(existing.contains(AutomationScheduleAccessibility.draftRow(0)))
        XCTAssertFalse(
            existing.contains(AutomationScheduleAccessibility.draftEligibility(0))
        )
        XCTAssertFalse(
            existing.contains(
                AutomationScheduleAccessibility.historySuggestionRow(rank: 1)
            )
        )
    }

    func testViewCopyDistinguishesFailuresAndFormatsLimits() {
        XCTAssertNotEqual(
            AutomationScheduleSettingsView.message(
                for: .service(.incompatibleSchema)
            ),
            AutomationScheduleSettingsView.message(
                for: .service(.invalidResponse)
            )
        )
        XCTAssertTrue(
            AutomationScheduleSettingsView.message(for: .unexpected)
                .contains("No cleanup was scheduled")
        )
        XCTAssertEqual(AutomationScheduleSettingsView.duration(86400), "1 day")
        XCTAssertEqual(AutomationScheduleSettingsView.duration(2_592_000), "30 days")
        XCTAssertEqual(AutomationScheduleSettingsView.duration(3600), "1 hour")
        for failure in [
            AutomationScheduleSettingsFailure.service(.unavailable),
            .service(.incompatibleSchema),
            .service(.invalidResponse),
            .model(.invalidHistorySuggestionFeed),
            .unexpected,
        ] {
            let message = AutomationScheduleSettingsView.historySuggestionMessage(
                for: failure
            )
            XCTAssertTrue(message.localizedCaseInsensitiveContains("no scan"))
            XCTAssertTrue(message.localizedCaseInsensitiveContains("schedule"))
            XCTAssertTrue(message.localizedCaseInsensitiveContains("cleanup"))
        }
    }

    func testSuggestionCoverageDisclosesBoundedOlderAndOmittedHistory() throws {
        let suggestions = try (1 ... 12).map { rank in
            try historySuggestion(
                rank: UInt16(rank),
                ruleID: "developer.rule\(rank)",
                latestRegrowthAt: Int64(10000 - rank)
            )
        }
        let feed = try AutomationScheduleHistorySuggestionFeedModel(
            recordVersion: 1,
            derivationRevision: 1,
            sourceSessionCount: 32,
            hasOlderSourceSessions: true,
            qualifyingRuleCount: 14,
            suggestions: suggestions
        )

        let message = AutomationScheduleSettingsView.historySuggestionCoverage(feed)
        XCTAssertTrue(message.contains("32 recent stored manual sessions"))
        XCTAssertTrue(message.contains("Older stored manual sessions"))
        XCTAssertTrue(message.contains("2 additional history patterns"))
        XCTAssertTrue(message.contains("not shown"))
    }

    private func overview() throws -> AutomationScheduleOverviewModel {
        let draft = try AutomationScheduleDraftModel(
            scheduleID: "schedule:test",
            scope: .category(.developerArtifact),
            cadence: .monthly,
            minimumAgeSeconds: AutomationScheduleDefaults.minimumAgeSeconds,
            minimumReclaimableBytes: 0,
            maximumBytesPerRun: AutomationScheduleDefaults.maximumBytesPerRun,
            exclusions: [],
            notifyBeforeRun: true,
            notifyBeforeRunsRemaining: 3,
            confirmationMode: .requireConfirmation,
            enabled: false,
            revision: 1,
            createdAtUnixMilliseconds: 1,
            updatedAtUnixMilliseconds: 1
        )
        return try AutomationScheduleOverviewModel(
            recordVersion: 2,
            globalEnabled: false,
            executionAvailable: false,
            eligibleRuleCount: 0,
            disabledDrafts: [draft],
            draftEligibility: [
                AutomationScheduleDraftEligibilityModel(
                    recordVersion: 1,
                    policyRevision: 1,
                    scheduleID: draft.scheduleID,
                    draftRevision: draft.revision,
                    status: .blockedByStaticPolicy,
                    includedStaticallyEligibleRuleCount: 0,
                    reasons: [.categoryHasNoScheduleEligibleRules]
                ),
            ]
        )
    }

    private func suggestionFeed() throws
        -> AutomationScheduleHistorySuggestionFeedModel
    {
        try AutomationScheduleHistorySuggestionFeedModel(
            recordVersion: 1,
            derivationRevision: 1,
            sourceSessionCount: 3,
            hasOlderSourceSessions: false,
            qualifyingRuleCount: 1,
            suggestions: [historySuggestion()]
        )
    }

    private func historySuggestion(
        rank: UInt16 = 1,
        ruleID: String = "developer.rust.target",
        latestRegrowthAt: Int64 = 900
    ) throws -> AutomationScheduleHistorySuggestionModel {
        try AutomationScheduleHistorySuggestionModel(
            recordVersion: 1,
            rank: rank,
            rule: DuxAutomationScheduleRuleReference(
                ruleID: ruleID,
                ruleRevision: 1
            ),
            successfulManualRunCount: 2,
            manualRegrowthCycleCount: 1,
            latestManualAttemptAtUnixMilliseconds: 1000,
            latestRegrowthAtUnixMilliseconds: latestRegrowthAt
        )
    }
}

private actor AutomationScheduleServiceSpy: DuxAutomationScheduleServing {
    private let response: AutomationScheduleOverviewModel
    private let historySuggestionResponse: AutomationScheduleHistorySuggestionFeedModel
    private var loads = 0
    private var historySuggestionLoads = 0
    private var nextFailure: AutomationScheduleServiceError?
    private var nextHistorySuggestionFailure: AutomationScheduleServiceError?

    init(
        response: AutomationScheduleOverviewModel,
        historySuggestionResponse: AutomationScheduleHistorySuggestionFeedModel = .unavailable
    ) {
        self.response = response
        self.historySuggestionResponse = historySuggestionResponse
    }

    func loadAutomationScheduleOverview() async throws
        -> AutomationScheduleOverviewModel
    {
        loads += 1
        if let nextFailure {
            self.nextFailure = nil
            throw nextFailure
        }
        return response
    }

    func loadAutomationScheduleHistorySuggestions() async throws
        -> AutomationScheduleHistorySuggestionFeedModel
    {
        historySuggestionLoads += 1
        if let nextHistorySuggestionFailure {
            self.nextHistorySuggestionFailure = nil
            throw nextHistorySuggestionFailure
        }
        return historySuggestionResponse
    }

    func failNext(_ failure: AutomationScheduleServiceError) {
        nextFailure = failure
    }

    func failNextHistorySuggestion(_ failure: AutomationScheduleServiceError) {
        nextHistorySuggestionFailure = failure
    }

    func loadCount() -> Int {
        loads
    }

    func historySuggestionLoadCount() -> Int {
        historySuggestionLoads
    }
}
