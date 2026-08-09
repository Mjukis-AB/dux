@testable import DUX
import XCTest

@MainActor
final class AutomationScheduleSettingsModelTests: XCTestCase {
    func testUnavailableProductionSeamLoadsHonestDefaultOffOverview() async {
        let model = AutomationScheduleSettingsModel(
            service: UnavailableDuxAutomationScheduleService()
        )

        await model.load()

        XCTAssertEqual(model.state, .ready)
        XCTAssertEqual(model.overview, .unavailable)
        XCTAssertEqual(model.overview?.recordVersion, 1)
        XCTAssertEqual(model.overview?.globalEnabled, false)
        XCTAssertEqual(model.overview?.executionAvailable, false)
        XCTAssertEqual(model.overview?.eligibleRuleCount, 0)
        XCTAssertEqual(model.overview?.disabledDrafts, [])
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

    func testShutdownFencesFutureLoads() async throws {
        let service = try AutomationScheduleServiceSpy(response: overview())
        let model = AutomationScheduleSettingsModel(service: service)

        await model.shutdown()
        await model.load()

        let loadCount = await service.loadCount()
        XCTAssertEqual(loadCount, 0)
        XCTAssertNil(model.overview)
        XCTAssertEqual(model.state, .idle)
    }

    func testViewAccessibilityIdentifiersAreStableUniqueAndDisjoint() {
        let identifiers = AutomationScheduleAccessibility.allStaticIdentifiers
        XCTAssertEqual(identifiers.count, 11)
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
        XCTAssertEqual(
            Set([AutomationScheduleAccessibility.draftRow(0),
                 AutomationScheduleAccessibility.draftRow(1)]).count,
            2
        )

        let existing = Set(
            SnapshotRetentionCapAccessibility.allControlIdentifiers
                + CleanupHistoryClearAccessibility.allControlIdentifiers
                + DiskPressurePolicyAccessibility.allControlIdentifiers
                + PermanentCleanupPolicyAccessibility.allControlIdentifiers
        )
        XCTAssertTrue(Set(identifiers).isDisjoint(with: existing))
        XCTAssertFalse(existing.contains(AutomationScheduleAccessibility.draftRow(0)))
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
            recordVersion: 1,
            globalEnabled: false,
            executionAvailable: false,
            eligibleRuleCount: 0,
            disabledDrafts: [draft]
        )
    }
}

private actor AutomationScheduleServiceSpy: DuxAutomationScheduleServing {
    private let response: AutomationScheduleOverviewModel
    private var loads = 0
    private var nextFailure: AutomationScheduleServiceError?

    init(response: AutomationScheduleOverviewModel) {
        self.response = response
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

    func failNext(_ failure: AutomationScheduleServiceError) {
        nextFailure = failure
    }

    func loadCount() -> Int {
        loads
    }
}
