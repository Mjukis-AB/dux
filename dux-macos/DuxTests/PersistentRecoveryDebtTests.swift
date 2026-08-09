@testable import DUX
import XCTest

final class PersistentRecoveryDebtAdapterTests: XCTestCase {
    func testAdapterAcceptsExactBoundedCounts() throws {
        let mapped = try EngineService.persistentRecoveryDebt(
            RunningScanDebtCensus(
                recordVersion: 1,
                inspectedUnclaimedCount: 64,
                pristineUnclaimedCount: 60,
                unexplainedUnclaimedCount: 4,
                hasMore: true
            )
        )

        XCTAssertEqual(mapped.inspectedUnclaimedCount, 64)
        XCTAssertEqual(mapped.pristineUnclaimedCount, 60)
        XCTAssertEqual(mapped.unexplainedUnclaimedCount, 4)
        XCTAssertTrue(mapped.hasMore)
        XCTAssertEqual(mapped.displayedCount, "64+")
        XCTAssertEqual(
            mapped.accessibilityCount,
            "64 or more retained recovery records; incomplete bounded census"
        )
    }

    func testAdapterRejectsMalformedEnvelopeAndAccounting() {
        let malformed = [
            RunningScanDebtCensus(
                recordVersion: 2,
                inspectedUnclaimedCount: 0,
                pristineUnclaimedCount: 0,
                unexplainedUnclaimedCount: 0,
                hasMore: false
            ),
            RunningScanDebtCensus(
                recordVersion: 1,
                inspectedUnclaimedCount: 65,
                pristineUnclaimedCount: 65,
                unexplainedUnclaimedCount: 0,
                hasMore: true
            ),
            RunningScanDebtCensus(
                recordVersion: 1,
                inspectedUnclaimedCount: 2,
                pristineUnclaimedCount: 1,
                unexplainedUnclaimedCount: 0,
                hasMore: false
            ),
            RunningScanDebtCensus(
                recordVersion: 1,
                inspectedUnclaimedCount: 2,
                pristineUnclaimedCount: 1,
                unexplainedUnclaimedCount: 1,
                hasMore: true
            ),
        ]

        for value in malformed {
            XCTAssertThrowsError(try EngineService.persistentRecoveryDebt(value)) { error in
                XCTAssertEqual(
                    error as? PersistentRecoveryDebtServiceError,
                    .invalidResponse
                )
            }
        }
    }

    func testPresentationAndAccessibilityIdentifiersAreTruthfulAndUnique() {
        let none = PersistentRecoveryDebt(
            inspectedUnclaimedCount: 0,
            pristineUnclaimedCount: 0,
            unexplainedUnclaimedCount: 0,
            hasMore: false
        )
        let one = PersistentRecoveryDebt(
            inspectedUnclaimedCount: 1,
            pristineUnclaimedCount: 1,
            unexplainedUnclaimedCount: 0,
            hasMore: false
        )

        XCTAssertEqual(none.displayedCount, "0")
        XCTAssertEqual(none.accessibilityCount, "0 retained recovery records")
        XCTAssertEqual(one.displayedCount, "1")
        XCTAssertEqual(one.accessibilityCount, "1 retained recovery record")

        let identifiers = PersistentRecoveryDebtAccessibility.allControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
    }
}

final class LegacyRunningScanDismissalAdapterTests: XCTestCase {
    func testAdapterAcceptsExactBoundedPreview() throws {
        let observedAt = Date(timeIntervalSince1970: 1_000)
        let mapped = try EngineService.legacyRunningScanDismissalPreview(
            LegacyRunningScanDismissalPreviewInfo(
                recordVersion: 1,
                eligibleCount: 64,
                hasMore: true,
                preparedAtUnixMs: 999_999,
                expiresAtUnixMs: 1_119_999
            ),
            observedAt: observedAt
        )

        XCTAssertEqual(mapped.eligibleCount, 64)
        XCTAssertTrue(mapped.hasMore)
        XCTAssertEqual(mapped.preparedAt, Date(timeIntervalSince1970: 999.999))
        XCTAssertEqual(mapped.expiresAt, Date(timeIntervalSince1970: 1_119.999))
    }

    func testAdapterRejectsMalformedBoundsAndLifetime() {
        let observedAt = Date(timeIntervalSince1970: 1_000)
        let malformed = [
            LegacyRunningScanDismissalPreviewInfo(
                recordVersion: 2,
                eligibleCount: 1,
                hasMore: false,
                preparedAtUnixMs: 999_999,
                expiresAtUnixMs: 1_119_999
            ),
            LegacyRunningScanDismissalPreviewInfo(
                recordVersion: 1,
                eligibleCount: 0,
                hasMore: false,
                preparedAtUnixMs: 999_999,
                expiresAtUnixMs: 1_119_999
            ),
            LegacyRunningScanDismissalPreviewInfo(
                recordVersion: 1,
                eligibleCount: 65,
                hasMore: true,
                preparedAtUnixMs: 999_999,
                expiresAtUnixMs: 1_119_999
            ),
            LegacyRunningScanDismissalPreviewInfo(
                recordVersion: 1,
                eligibleCount: 1,
                hasMore: false,
                preparedAtUnixMs: 999_999,
                expiresAtUnixMs: 1_119_998
            ),
            LegacyRunningScanDismissalPreviewInfo(
                recordVersion: 1,
                eligibleCount: 1,
                hasMore: false,
                preparedAtUnixMs: 1_000_001,
                expiresAtUnixMs: 1_120_001
            ),
        ]

        for value in malformed {
            XCTAssertThrowsError(
                try EngineService.legacyRunningScanDismissalPreview(
                    value,
                    observedAt: observedAt
                )
            ) { error in
                XCTAssertEqual(
                    error as? LegacyRunningScanDismissalServiceError,
                    .invalidResponse
                )
            }
        }
    }

    func testAccessibilityIdentifiersAreStableUniqueAndDisjoint() {
        let identifiers = LegacyRunningScanDismissalAccessibility.allControlIdentifiers
        XCTAssertEqual(identifiers.count, 9)
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })

        let neighboringIdentifiers =
            PersistentRecoveryDebtAccessibility.allControlIdentifiers
            + ClaimedRunningScanProvenanceAccessibility.allControlIdentifiers
            + CleanupHistoryClearAccessibility.allControlIdentifiers
        XCTAssertTrue(
            Set(identifiers).isDisjoint(with: Set(neighboringIdentifiers))
        )
    }
}

@MainActor
final class LegacyRunningScanDismissalSettingsModelTests: XCTestCase {
    func testCancelReleasesPreviewWithoutDismissingHistory() async {
        let service = LegacyRunningScanDismissalEngineSpy()
        let model = LegacyRunningScanDismissalSettingsModel(service: service)

        await model.prepare()
        guard let confirmation = model.confirmation else {
            XCTFail("Expected a confirmation")
            return
        }
        XCTAssertEqual(model.state, .awaitingConfirmation(confirmation))

        await model.cancel(confirmation)

        XCTAssertEqual(model.state, .idle)
        XCTAssertNil(model.confirmation)
        let counts = await service.counts()
        XCTAssertEqual(counts.prepare, 1)
        XCTAssertEqual(counts.dismiss, 0)
        XCTAssertEqual(counts.release, 1)
    }

    func testConfirmConsumesPreviewOnceAndPublishesExactResult() async {
        let service = LegacyRunningScanDismissalEngineSpy(hasMore: true)
        let model = LegacyRunningScanDismissalSettingsModel(service: service)

        await model.prepare()
        guard let confirmation = model.confirmation else {
            XCTFail("Expected a confirmation")
            return
        }
        await model.confirm(confirmation)
        await model.confirm(confirmation)

        XCTAssertEqual(
            model.state,
            .completed(
                LegacyRunningScanDismissalResultModel(
                    dismissedCount: 2,
                    hasMore: true
                )
            )
        )
        XCTAssertNil(model.confirmation)
        let counts = await service.counts()
        XCTAssertEqual(counts.prepare, 1)
        XCTAssertEqual(counts.dismiss, 1)
        XCTAssertEqual(counts.release, 1)
    }

    func testStaleConfirmationCannotTriggerDismissal() async {
        let service = LegacyRunningScanDismissalEngineSpy()
        let model = LegacyRunningScanDismissalSettingsModel(service: service)

        await model.prepare()
        guard let stale = model.confirmation else {
            XCTFail("Expected a confirmation")
            return
        }
        await model.cancel(stale)
        await model.confirm(stale)

        XCTAssertEqual(model.state, .idle)
        let counts = await service.counts()
        XCTAssertEqual(counts.dismiss, 0)
        XCTAssertEqual(counts.release, 1)
    }

    func testOutcomeUnknownIsNotPresentedAsOrdinaryFailure() async {
        let service = LegacyRunningScanDismissalEngineSpy(
            dismissalFailure: .outcomeUnknown
        )
        let model = LegacyRunningScanDismissalSettingsModel(service: service)

        await model.prepare()
        guard let confirmation = model.confirmation else {
            XCTFail("Expected a confirmation")
            return
        }
        await model.confirm(confirmation)

        XCTAssertEqual(model.state, .outcomeUnknown)
        let counts = await service.counts()
        XCTAssertEqual(counts.dismiss, 1)
        XCTAssertEqual(counts.release, 1)
    }

    func testShutdownReleasesPendingPreviewAndFencesFutureRequests() async {
        let service = LegacyRunningScanDismissalEngineSpy()
        let model = LegacyRunningScanDismissalSettingsModel(service: service)

        await model.prepare()
        await model.shutdown()
        await model.prepare()

        XCTAssertEqual(model.state, .idle)
        XCTAssertNil(model.confirmation)
        let counts = await service.counts()
        XCTAssertEqual(counts.prepare, 1)
        XCTAssertEqual(counts.dismiss, 0)
        XCTAssertEqual(counts.release, 1)
    }
}

final class ClaimedRunningScanProvenanceAdapterTests: XCTestCase {
    func testAdapterAcceptsFiveWayBoundedAccounting() throws {
        let mapped = try EngineService.claimedRunningScanProvenance(
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 64,
                sameHostCurrentBootCount: 40,
                sameHostPriorBootCount: 10,
                foreignHostCount: 5,
                storedUnprovenCount: 9,
                currentContextUnavailableCount: 0,
                hasMore: true
            )
        )

        XCTAssertEqual(mapped.inspectedClaimedCount, 64)
        XCTAssertEqual(mapped.sameHostCurrentBootCount, 40)
        XCTAssertEqual(mapped.sameHostPriorBootCount, 10)
        XCTAssertEqual(mapped.foreignHostCount, 5)
        XCTAssertEqual(mapped.storedUnprovenCount, 9)
        XCTAssertEqual(mapped.currentContextUnavailableCount, 0)
        XCTAssertTrue(mapped.hasMore)
        XCTAssertEqual(mapped.displayedCount, "64+")
        XCTAssertEqual(
            mapped.accessibilityCount,
            "64 or more claimed running scan records; incomplete bounded census"
        )
        XCTAssertEqual(
            mapped.accessibilityDistribution,
            "Current startup session 40; earlier startup session on this Mac 10; "
                + "different host 5; identity not stored 9; current identity unavailable 0"
        )

        let unavailable = try EngineService.claimedRunningScanProvenance(
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 5,
                sameHostCurrentBootCount: 0,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 2,
                currentContextUnavailableCount: 3,
                hasMore: false
            )
        )
        XCTAssertEqual(unavailable.storedUnprovenCount, 2)
        XCTAssertEqual(unavailable.currentContextUnavailableCount, 3)
    }

    func testAdapterRejectsMalformedEnvelopeAccountingAndTruncation() {
        let malformed = [
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 2,
                inspectedClaimedCount: 0,
                sameHostCurrentBootCount: 0,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: false
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 65,
                sameHostCurrentBootCount: 65,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: true
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 2,
                sameHostCurrentBootCount: 1,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: false
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 2,
                sameHostCurrentBootCount: 1,
                sameHostPriorBootCount: 1,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: true
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 2,
                sameHostCurrentBootCount: 1,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 1,
                hasMore: false
            ),
            ClaimedRunningScanProvenanceCensus(
                recordVersion: 1,
                inspectedClaimedCount: 64,
                sameHostCurrentBootCount: .max,
                sameHostPriorBootCount: .max,
                foreignHostCount: .max,
                storedUnprovenCount: .max,
                currentContextUnavailableCount: .max,
                hasMore: true
            ),
        ]

        for value in malformed {
            XCTAssertThrowsError(
                try EngineService.claimedRunningScanProvenance(value)
            ) { error in
                XCTAssertEqual(
                    error as? ClaimedRunningScanProvenanceServiceError,
                    .invalidResponse
                )
            }
        }
    }

    func testPresentationAndAccessibilityIdentifiersAreTruthfulAndUnique() {
        let none = ClaimedRunningScanProvenance(
            inspectedClaimedCount: 0,
            sameHostCurrentBootCount: 0,
            sameHostPriorBootCount: 0,
            foreignHostCount: 0,
            storedUnprovenCount: 0,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        let one = ClaimedRunningScanProvenance(
            inspectedClaimedCount: 1,
            sameHostCurrentBootCount: 1,
            sameHostPriorBootCount: 0,
            foreignHostCount: 0,
            storedUnprovenCount: 0,
            currentContextUnavailableCount: 0,
            hasMore: false
        )

        XCTAssertEqual(none.displayedCount, "0")
        XCTAssertEqual(none.accessibilityCount, "0 claimed running scan records")
        XCTAssertEqual(one.displayedCount, "1")
        XCTAssertEqual(one.accessibilityCount, "1 claimed running scan record")

        let identifiers =
            ClaimedRunningScanProvenanceAccessibility.allControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
        XCTAssertTrue(
            Set(identifiers).isDisjoint(
                with: Set(PersistentRecoveryDebtAccessibility.allControlIdentifiers)
            )
        )
    }
}

final class CleanupRecoveryDiagnosticsAdapterTests: XCTestCase {
    func testAdapterAcceptsIndependentPhaseAndProvenanceAccounting() throws {
        let mapped = try EngineService.cleanupRecoveryDiagnostics(
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 1,
                inspectedActiveCount: 64,
                runningCount: 50,
                recoveringCount: 14,
                sameHostCurrentBootCount: 40,
                sameHostPriorBootCount: 10,
                foreignHostCount: 5,
                storedUnprovenCount: 9,
                currentContextUnavailableCount: 0,
                hasMore: true
            )
        )

        XCTAssertEqual(mapped.inspectedActiveCount, 64)
        XCTAssertEqual(mapped.runningCount, 50)
        XCTAssertEqual(mapped.recoveringCount, 14)
        XCTAssertEqual(mapped.sameHostCurrentBootCount, 40)
        XCTAssertEqual(mapped.sameHostPriorBootCount, 10)
        XCTAssertEqual(mapped.foreignHostCount, 5)
        XCTAssertEqual(mapped.storedUnprovenCount, 9)
        XCTAssertEqual(mapped.currentContextUnavailableCount, 0)
        XCTAssertTrue(mapped.hasMore)
        XCTAssertEqual(mapped.displayedCount, "64+")
        XCTAssertEqual(
            mapped.accessibilityCount,
            "64 or more active cleanup records; incomplete bounded census"
        )

        let unavailable = try EngineService.cleanupRecoveryDiagnostics(
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 1,
                inspectedActiveCount: 5,
                runningCount: 2,
                recoveringCount: 3,
                sameHostCurrentBootCount: 0,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 2,
                currentContextUnavailableCount: 3,
                hasMore: false
            )
        )
        XCTAssertEqual(unavailable.storedUnprovenCount, 2)
        XCTAssertEqual(unavailable.currentContextUnavailableCount, 3)
    }

    func testAdapterRejectsMalformedPhaseProvenanceAndTruncation() {
        let malformed = [
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 2,
                inspectedActiveCount: 0,
                runningCount: 0,
                recoveringCount: 0,
                sameHostCurrentBootCount: 0,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: false
            ),
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 1,
                inspectedActiveCount: 65,
                runningCount: 65,
                recoveringCount: 0,
                sameHostCurrentBootCount: 65,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: true
            ),
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 1,
                inspectedActiveCount: 2,
                runningCount: 1,
                recoveringCount: 0,
                sameHostCurrentBootCount: 2,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: false
            ),
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 1,
                inspectedActiveCount: 2,
                runningCount: 2,
                recoveringCount: 0,
                sameHostCurrentBootCount: 1,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: false
            ),
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 1,
                inspectedActiveCount: 2,
                runningCount: 2,
                recoveringCount: 0,
                sameHostCurrentBootCount: 1,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 1,
                hasMore: false
            ),
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 1,
                inspectedActiveCount: 2,
                runningCount: 2,
                recoveringCount: 0,
                sameHostCurrentBootCount: 2,
                sameHostPriorBootCount: 0,
                foreignHostCount: 0,
                storedUnprovenCount: 0,
                currentContextUnavailableCount: 0,
                hasMore: true
            ),
            CleanupRecoveryDiagnosticCensus(
                recordVersion: 1,
                inspectedActiveCount: 64,
                runningCount: .max,
                recoveringCount: .max,
                sameHostCurrentBootCount: .max,
                sameHostPriorBootCount: .max,
                foreignHostCount: .max,
                storedUnprovenCount: .max,
                currentContextUnavailableCount: .max,
                hasMore: true
            ),
        ]

        for value in malformed {
            XCTAssertThrowsError(
                try EngineService.cleanupRecoveryDiagnostics(value)
            ) { error in
                XCTAssertEqual(
                    error as? CleanupRecoveryDiagnosticsServiceError,
                    .invalidResponse
                )
            }
        }
    }

    func testPresentationAndAccessibilityIdentifiersAreTruthfulAndDisjoint() {
        let none = CleanupRecoveryDiagnostics(
            inspectedActiveCount: 0,
            runningCount: 0,
            recoveringCount: 0,
            sameHostCurrentBootCount: 0,
            sameHostPriorBootCount: 0,
            foreignHostCount: 0,
            storedUnprovenCount: 0,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        let one = CleanupRecoveryDiagnostics(
            inspectedActiveCount: 1,
            runningCount: 1,
            recoveringCount: 0,
            sameHostCurrentBootCount: 1,
            sameHostPriorBootCount: 0,
            foreignHostCount: 0,
            storedUnprovenCount: 0,
            currentContextUnavailableCount: 0,
            hasMore: false
        )

        XCTAssertEqual(none.displayedCount, "0")
        XCTAssertEqual(none.accessibilityCount, "0 active cleanup records")
        XCTAssertEqual(one.displayedCount, "1")
        XCTAssertEqual(one.accessibilityCount, "1 active cleanup record")

        let identifiers = CleanupRecoveryDiagnosticsAccessibility.allControlIdentifiers
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
        let existingIdentifiers =
            [
                MenuBarLabelAccessibility.picker,
                ApplicationUpdateAccessibility.check,
                ApplicationUpdateAccessibility.status,
            ]
                + DiskPressurePolicyAccessibility.allControlIdentifiers
                + PermanentCleanupPolicyAccessibility.allControlIdentifiers
                + CleanupExclusionsAccessibility.allStaticControlIdentifiers
                + ProjectDiscoveryRootsAccessibility.allStaticControlIdentifiers
                + DirectCargoEnrollmentAccessibility.allControlIdentifiers
                + CleanupHistoryClearAccessibility.allControlIdentifiers
                + PersistentRecoveryDebtAccessibility.allControlIdentifiers
                + LegacyRunningScanDismissalAccessibility.allControlIdentifiers
                + ClaimedRunningScanProvenanceAccessibility.allControlIdentifiers
        XCTAssertTrue(
            Set(identifiers).isDisjoint(
                with: Set(existingIdentifiers)
            )
        )
    }
}

@MainActor
final class PersistentRecoveryDebtAppModelTests: XCTestCase {
    func testLoadIsLazyCoalescedAndFailedRefreshKeepsEarlierResult() async {
        let service = PersistentRecoveryDebtEngineSpy()
        let model = AppModel(engineService: service)

        var requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 0)
        XCTAssertNil(model.persistentRecoveryDebt)
        XCTAssertEqual(model.persistentRecoveryDebtState, .idle)

        let first = Task { @MainActor in
            await model.loadPersistentRecoveryDebt()
        }
        await service.waitForRequestCount(1)
        let second = Task { @MainActor in
            await model.loadPersistentRecoveryDebt()
        }
        await Task.yield()
        requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
        XCTAssertEqual(model.persistentRecoveryDebtState, .loading)

        let initial = PersistentRecoveryDebt(
            inspectedUnclaimedCount: 2,
            pristineUnclaimedCount: 1,
            unexplainedUnclaimedCount: 1,
            hasMore: false
        )
        await service.resolve(at: 0, with: .success(initial))
        await first.value
        await second.value

        XCTAssertEqual(model.persistentRecoveryDebt, initial)
        XCTAssertEqual(model.persistentRecoveryDebtState, .loaded)
        let readAt = model.persistentRecoveryDebtReadAt
        XCTAssertNotNil(readAt)

        let refresh = Task { @MainActor in
            await model.refreshPersistentRecoveryDebt()
        }
        await service.waitForRequestCount(2)
        await service.resolve(at: 1, with: .failure(.retryable))
        await refresh.value

        XCTAssertEqual(model.persistentRecoveryDebt, initial)
        XCTAssertEqual(model.persistentRecoveryDebtReadAt, readAt)
        XCTAssertEqual(model.persistentRecoveryDebtState, .failed(.retryable))
    }

    func testInvalidationFencesLateReplyAndFutureLoads() async {
        let service = PersistentRecoveryDebtEngineSpy()
        let model = AppModel(engineService: service)

        let load = Task { @MainActor in
            await model.loadPersistentRecoveryDebt()
        }
        await service.waitForRequestCount(1)
        model.invalidatePersistentRecoveryDebtOperations()
        await service.resolve(
            at: 0,
            with: .success(
                PersistentRecoveryDebt(
                    inspectedUnclaimedCount: 1,
                    pristineUnclaimedCount: 1,
                    unexplainedUnclaimedCount: 0,
                    hasMore: false
                )
            )
        )
        await load.value

        XCTAssertNil(model.persistentRecoveryDebt)
        XCTAssertNil(model.persistentRecoveryDebtReadAt)
        XCTAssertEqual(model.persistentRecoveryDebtState, .idle)

        await model.loadPersistentRecoveryDebt()
        let requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
    }
}

@MainActor
final class ClaimedRunningScanProvenanceAppModelTests: XCTestCase {
    func testLoadIsLazyCoalescedAndFailedRefreshKeepsEarlierResult() async {
        let service = ClaimedRunningScanProvenanceEngineSpy()
        let model = AppModel(engineService: service)

        var requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 0)
        XCTAssertNil(model.claimedRunningScanProvenance)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .idle)

        let first = Task { @MainActor in
            await model.loadClaimedRunningScanProvenance()
        }
        await service.waitForRequestCount(1)
        let second = Task { @MainActor in
            await model.loadClaimedRunningScanProvenance()
        }
        await Task.yield()
        requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .loading)

        let initial = ClaimedRunningScanProvenance(
            inspectedClaimedCount: 3,
            sameHostCurrentBootCount: 1,
            sameHostPriorBootCount: 1,
            foreignHostCount: 0,
            storedUnprovenCount: 1,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        await service.resolve(at: 0, with: .success(initial))
        await first.value
        await second.value

        XCTAssertEqual(model.claimedRunningScanProvenance, initial)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .loaded)
        let readAt = model.claimedRunningScanProvenanceReadAt
        XCTAssertNotNil(readAt)

        let refresh = Task { @MainActor in
            await model.refreshClaimedRunningScanProvenance()
        }
        await service.waitForRequestCount(2)
        await service.resolve(at: 1, with: .failure(.retryable))
        await refresh.value

        XCTAssertEqual(model.claimedRunningScanProvenance, initial)
        XCTAssertEqual(model.claimedRunningScanProvenanceReadAt, readAt)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .failed(.retryable))
    }

    func testInvalidationFencesLateReplyAndFutureLoads() async {
        let service = ClaimedRunningScanProvenanceEngineSpy()
        let model = AppModel(engineService: service)

        let load = Task { @MainActor in
            await model.loadClaimedRunningScanProvenance()
        }
        await service.waitForRequestCount(1)
        model.invalidateClaimedRunningScanProvenanceOperations()
        await service.resolve(
            at: 0,
            with: .success(
                ClaimedRunningScanProvenance(
                    inspectedClaimedCount: 1,
                    sameHostCurrentBootCount: 1,
                    sameHostPriorBootCount: 0,
                    foreignHostCount: 0,
                    storedUnprovenCount: 0,
                    currentContextUnavailableCount: 0,
                    hasMore: false
                )
            )
        )
        await load.value

        XCTAssertNil(model.claimedRunningScanProvenance)
        XCTAssertNil(model.claimedRunningScanProvenanceReadAt)
        XCTAssertEqual(model.claimedRunningScanProvenanceState, .idle)

        await model.loadClaimedRunningScanProvenance()
        let requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
    }
}

@MainActor
final class CleanupRecoveryDiagnosticsAppModelTests: XCTestCase {
    func testLoadIsLazyCoalescedAndFailedRefreshKeepsEarlierResult() async {
        let service = CleanupRecoveryDiagnosticsEngineSpy()
        let model = AppModel(engineService: service)

        var requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 0)
        XCTAssertNil(model.cleanupRecoveryDiagnostics)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsState, .idle)

        let first = Task { @MainActor in
            await model.loadCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(1)
        let second = Task { @MainActor in
            await model.loadCleanupRecoveryDiagnostics()
        }
        await Task.yield()
        requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsState, .loading)

        let initial = CleanupRecoveryDiagnostics(
            inspectedActiveCount: 3,
            runningCount: 2,
            recoveringCount: 1,
            sameHostCurrentBootCount: 1,
            sameHostPriorBootCount: 1,
            foreignHostCount: 0,
            storedUnprovenCount: 1,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        await service.resolve(at: 0, with: .success(initial))
        await first.value
        await second.value

        XCTAssertEqual(model.cleanupRecoveryDiagnostics, initial)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsState, .loaded)
        let readAt = model.cleanupRecoveryDiagnosticsReadAt
        XCTAssertNotNil(readAt)

        let refresh = Task { @MainActor in
            await model.refreshCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(2)
        await service.resolve(at: 1, with: .failure(.retryable))
        await refresh.value

        XCTAssertEqual(model.cleanupRecoveryDiagnostics, initial)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsReadAt, readAt)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsState, .failed(.retryable))
    }

    func testInvalidationFencesLateReplyAndFutureLoads() async {
        let service = CleanupRecoveryDiagnosticsEngineSpy()
        let model = AppModel(engineService: service)

        let load = Task { @MainActor in
            await model.loadCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(1)
        model.invalidateCleanupRecoveryDiagnosticsOperations()
        await service.resolve(
            at: 0,
            with: .success(
                CleanupRecoveryDiagnostics(
                    inspectedActiveCount: 1,
                    runningCount: 1,
                    recoveringCount: 0,
                    sameHostCurrentBootCount: 1,
                    sameHostPriorBootCount: 0,
                    foreignHostCount: 0,
                    storedUnprovenCount: 0,
                    currentContextUnavailableCount: 0,
                    hasMore: false
                )
            )
        )
        await load.value

        XCTAssertNil(model.cleanupRecoveryDiagnostics)
        XCTAssertNil(model.cleanupRecoveryDiagnosticsReadAt)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsState, .idle)

        await model.loadCleanupRecoveryDiagnostics()
        let requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
    }

    func testSettingsDismissalFencesLateReplyButAllowsFutureLoad() async {
        let service = CleanupRecoveryDiagnosticsEngineSpy()
        let model = AppModel(engineService: service)

        let dismissedLoad = Task { @MainActor in
            await model.loadCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(1)
        model.dismissCleanupRecoveryDiagnosticsPresentation()
        await service.resolve(
            at: 0,
            with: .success(
                CleanupRecoveryDiagnostics(
                    inspectedActiveCount: 1,
                    runningCount: 1,
                    recoveringCount: 0,
                    sameHostCurrentBootCount: 1,
                    sameHostPriorBootCount: 0,
                    foreignHostCount: 0,
                    storedUnprovenCount: 0,
                    currentContextUnavailableCount: 0,
                    hasMore: false
                )
            )
        )
        await dismissedLoad.value

        XCTAssertNil(model.cleanupRecoveryDiagnostics)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsState, .idle)

        let reopenedLoad = Task { @MainActor in
            await model.loadCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(2)
        let reopened = CleanupRecoveryDiagnostics(
            inspectedActiveCount: 0,
            runningCount: 0,
            recoveringCount: 0,
            sameHostCurrentBootCount: 0,
            sameHostPriorBootCount: 0,
            foreignHostCount: 0,
            storedUnprovenCount: 0,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        await service.resolve(at: 1, with: .success(reopened))
        await reopenedLoad.value

        XCTAssertEqual(model.cleanupRecoveryDiagnostics, reopened)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsState, .loaded)
    }

    func testTerminalRuntimeFencesLateReplyAndFutureLoads() async {
        let service = CleanupRecoveryDiagnosticsEngineSpy()
        let model = AppModel(engineService: service)

        let load = Task { @MainActor in
            await model.loadCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(1)
        let quiescence = model.beginTerminalRuntimeQuiescence()
        await service.resolve(
            at: 0,
            with: .success(
                CleanupRecoveryDiagnostics(
                    inspectedActiveCount: 1,
                    runningCount: 1,
                    recoveringCount: 0,
                    sameHostCurrentBootCount: 1,
                    sameHostPriorBootCount: 0,
                    foreignHostCount: 0,
                    storedUnprovenCount: 0,
                    currentContextUnavailableCount: 0,
                    hasMore: false
                )
            )
        )
        await load.value
        await quiescence.value

        XCTAssertNil(model.cleanupRecoveryDiagnostics)
        XCTAssertNil(model.cleanupRecoveryDiagnosticsReadAt)
        await model.loadCleanupRecoveryDiagnostics()
        let requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 1)
    }

    func testSupersededRefreshCannotOverwriteNewerObservation() async {
        let service = CleanupRecoveryDiagnosticsEngineSpy()
        let model = AppModel(engineService: service)

        let initialLoad = Task { @MainActor in
            await model.loadCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(1)
        let empty = CleanupRecoveryDiagnostics(
            inspectedActiveCount: 0,
            runningCount: 0,
            recoveringCount: 0,
            sameHostCurrentBootCount: 0,
            sameHostPriorBootCount: 0,
            foreignHostCount: 0,
            storedUnprovenCount: 0,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        await service.resolve(at: 0, with: .success(empty))
        await initialLoad.value

        let olderRefresh = Task { @MainActor in
            await model.refreshCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(2)
        let newerRefresh = Task { @MainActor in
            await model.refreshCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(3)
        let newest = CleanupRecoveryDiagnostics(
            inspectedActiveCount: 2,
            runningCount: 1,
            recoveringCount: 1,
            sameHostCurrentBootCount: 0,
            sameHostPriorBootCount: 1,
            foreignHostCount: 0,
            storedUnprovenCount: 1,
            currentContextUnavailableCount: 0,
            hasMore: false
        )
        await service.resolve(at: 2, with: .success(newest))
        await newerRefresh.value
        await service.resolve(at: 1, with: .success(empty))
        await olderRefresh.value

        XCTAssertEqual(model.cleanupRecoveryDiagnostics, newest)
        XCTAssertEqual(model.cleanupRecoveryDiagnosticsState, .loaded)
    }

    func testRuntimeRefreshRemainsLazyUntilObservationWasRequested() async {
        let service = CleanupRecoveryDiagnosticsEngineSpy()
        let model = AppModel(engineService: service)

        await model.refreshCleanupRecoveryDiagnosticsIfLoaded()
        var requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 0)

        let load = Task { @MainActor in
            await model.loadCleanupRecoveryDiagnostics()
        }
        await service.waitForRequestCount(1)
        await service.resolve(
            at: 0,
            with: .success(
                CleanupRecoveryDiagnostics(
                    inspectedActiveCount: 0,
                    runningCount: 0,
                    recoveringCount: 0,
                    sameHostCurrentBootCount: 0,
                    sameHostPriorBootCount: 0,
                    foreignHostCount: 0,
                    storedUnprovenCount: 0,
                    currentContextUnavailableCount: 0,
                    hasMore: false
                )
            )
        )
        await load.value

        let refresh = Task { @MainActor in
            await model.refreshCleanupRecoveryDiagnosticsIfLoaded()
        }
        await service.waitForRequestCount(2)
        await service.resolve(
            at: 1,
            with: .success(
                CleanupRecoveryDiagnostics(
                    inspectedActiveCount: 0,
                    runningCount: 0,
                    recoveringCount: 0,
                    sameHostCurrentBootCount: 0,
                    sameHostPriorBootCount: 0,
                    foreignHostCount: 0,
                    storedUnprovenCount: 0,
                    currentContextUnavailableCount: 0,
                    hasMore: false
                )
            )
        )
        await refresh.value
        requestCount = await service.requestCount()
        XCTAssertEqual(requestCount, 2)
    }
}

private actor PersistentRecoveryDebtEngineSpy: EngineServing {
    private struct PendingReply {
        var continuation:
            CheckedContinuation<PersistentRecoveryDebt, any Error>?
    }

    private var requests = 0
    private var replies: [PendingReply] = []

    func loadStatus() async throws -> EngineStatus {
        throw EngineServiceError.unavailable
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        DiskPressurePolicy(
            source: .default,
            revision: 0,
            configuration: .defaults,
            updatedAtUnixMilliseconds: nil
        )
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .stored,
                revision: 1,
                configuration: configuration,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: try await loadDiskPressurePolicy(),
            changed: true
        )
    }

    func loadPersistentRecoveryDebt() async throws -> PersistentRecoveryDebt {
        requests += 1
        return try await withCheckedThrowingContinuation { continuation in
            replies.append(PendingReply(continuation: continuation))
        }
    }

    func requestCount() -> Int {
        requests
    }

    func waitForRequestCount(_ expected: Int) async {
        while requests < expected {
            await Task.yield()
        }
    }

    func resolve(
        at index: Int,
        with result:
        Result<PersistentRecoveryDebt, PersistentRecoveryDebtServiceError>
    ) {
        guard replies.indices.contains(index),
              let continuation = replies[index].continuation
        else {
            XCTFail("Missing persistent recovery debt reply \(index)")
            return
        }
        replies[index].continuation = nil
        switch result {
        case let .success(observation):
            continuation.resume(returning: observation)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }
}

private actor ClaimedRunningScanProvenanceEngineSpy: EngineServing {
    private struct PendingReply {
        var continuation:
            CheckedContinuation<ClaimedRunningScanProvenance, any Error>?
    }

    private var requests = 0
    private var replies: [PendingReply] = []

    func loadStatus() async throws -> EngineStatus {
        throw EngineServiceError.unavailable
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        DiskPressurePolicy(
            source: .default,
            revision: 0,
            configuration: .defaults,
            updatedAtUnixMilliseconds: nil
        )
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .stored,
                revision: 1,
                configuration: configuration,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: try await loadDiskPressurePolicy(),
            changed: true
        )
    }

    func loadClaimedRunningScanProvenance() async throws
        -> ClaimedRunningScanProvenance
    {
        requests += 1
        return try await withCheckedThrowingContinuation { continuation in
            replies.append(PendingReply(continuation: continuation))
        }
    }

    func requestCount() -> Int {
        requests
    }

    func waitForRequestCount(_ expected: Int) async {
        while requests < expected {
            await Task.yield()
        }
    }

    func resolve(
        at index: Int,
        with result:
        Result<
            ClaimedRunningScanProvenance,
            ClaimedRunningScanProvenanceServiceError
        >
    ) {
        guard replies.indices.contains(index),
              let continuation = replies[index].continuation
        else {
            XCTFail("Missing claimed running scan provenance reply \(index)")
            return
        }
        replies[index].continuation = nil
        switch result {
        case let .success(observation):
            continuation.resume(returning: observation)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }
}

private actor CleanupRecoveryDiagnosticsEngineSpy: EngineServing {
    private struct PendingReply {
        var continuation:
            CheckedContinuation<CleanupRecoveryDiagnostics, any Error>?
    }

    private var requests = 0
    private var replies: [PendingReply] = []

    func loadStatus() async throws -> EngineStatus {
        throw EngineServiceError.unavailable
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        DiskPressurePolicy(
            source: .default,
            revision: 0,
            configuration: .defaults,
            updatedAtUnixMilliseconds: nil
        )
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .stored,
                revision: 1,
                configuration: configuration,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: try await loadDiskPressurePolicy(),
            changed: true
        )
    }

    func loadCleanupRecoveryDiagnostics() async throws
        -> CleanupRecoveryDiagnostics
    {
        requests += 1
        return try await withCheckedThrowingContinuation { continuation in
            replies.append(PendingReply(continuation: continuation))
        }
    }

    func requestCount() -> Int {
        requests
    }

    func waitForRequestCount(_ expected: Int) async {
        while requests < expected {
            await Task.yield()
        }
    }

    func resolve(
        at index: Int,
        with result: Result<
            CleanupRecoveryDiagnostics,
            CleanupRecoveryDiagnosticsServiceError
        >
    ) {
        guard replies.indices.contains(index),
              let continuation = replies[index].continuation
        else {
            XCTFail("Missing cleanup recovery diagnostics reply \(index)")
            return
        }
        replies[index].continuation = nil
        switch result {
        case let .success(observation):
            continuation.resume(returning: observation)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }
}

private actor LegacyRunningScanDismissalEngineSpy: EngineServing {
    private let hasMore: Bool
    private let dismissalFailure: LegacyRunningScanDismissalServiceError?
    private var prepareRequests = 0
    private var dismissRequests = 0
    private var releaseRequests = 0

    init(
        hasMore: Bool = false,
        dismissalFailure: LegacyRunningScanDismissalServiceError? = nil
    ) {
        self.hasMore = hasMore
        self.dismissalFailure = dismissalFailure
    }

    func loadStatus() async throws -> EngineStatus {
        throw EngineServiceError.unavailable
    }

    func observeVolumeCapacity(
        _ snapshot: VolumeCapacitySnapshot
    ) async throws -> VolumeCapacitySnapshot {
        snapshot
    }

    func loadDiskPressurePolicy() async throws -> DiskPressurePolicy {
        DiskPressurePolicy(
            source: .default,
            revision: 0,
            configuration: .defaults,
            updatedAtUnixMilliseconds: nil
        )
    }

    func setDiskPressurePolicy(
        _ configuration: DiskPressurePolicyConfiguration
    ) async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: DiskPressurePolicy(
                source: .stored,
                revision: 1,
                configuration: configuration,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        DiskPressurePolicyUpdateResult(
            policy: try await loadDiskPressurePolicy(),
            changed: true
        )
    }

    func prepareLegacyRunningScanDismissal() async throws
        -> any DuxLegacyRunningScanDismissalPreviewLease
    {
        prepareRequests += 1
        let preview = LegacyRunningScanDismissalPreviewModel(
            eligibleCount: 2,
            hasMore: hasMore,
            preparedAt: Date(),
            expiresAt: Date().addingTimeInterval(120)
        )
        return LegacyRunningScanDismissalPreviewLeaseSpy(
            preview: preview,
            releaseHandler: { [weak self] in
                await self?.recordRelease()
            }
        )
    }

    func dismissLegacyRunningScans(
        _ preview: any DuxLegacyRunningScanDismissalPreviewLease
    ) async throws -> LegacyRunningScanDismissalResultModel {
        dismissRequests += 1
        if let dismissalFailure {
            throw dismissalFailure
        }
        return LegacyRunningScanDismissalResultModel(
            dismissedCount: preview.preview.eligibleCount,
            hasMore: preview.preview.hasMore
        )
    }

    func counts() -> (prepare: Int, dismiss: Int, release: Int) {
        (prepareRequests, dismissRequests, releaseRequests)
    }

    private func recordRelease() {
        releaseRequests += 1
    }
}

private final class LegacyRunningScanDismissalPreviewLeaseSpy:
    DuxLegacyRunningScanDismissalPreviewLease, @unchecked Sendable
{
    let preview: LegacyRunningScanDismissalPreviewModel

    private let releaseHandler: @Sendable () async -> Void
    private let lock = NSLock()
    private var available = true

    init(
        preview: LegacyRunningScanDismissalPreviewModel,
        releaseHandler: @escaping @Sendable () async -> Void
    ) {
        self.preview = preview
        self.releaseHandler = releaseHandler
    }

    func release() async {
        let shouldRelease = lock.withLock {
            guard available else {
                return false
            }
            available = false
            return true
        }
        if shouldRelease {
            await releaseHandler()
        }
    }
}
