@testable import DUX
import Foundation
import XCTest

@MainActor
final class HomeScanAppModelTests: XCTestCase {
    func testInitialLoadNeverStartsDiscoveryScan() async {
        let service = HomeScanServiceSpy(responses: [])
        let engine = HomeScanEngineStub()
        let model = AppModel(
            engineService: engine,
            volumeMonitor: HomeScanVolumeMonitorStub(),
            homeScanService: service
        )

        await model.loadInitialState()

        let startCount = await service.startCount()
        let storageThiefRequestCount =
            await engine.recurringStorageThiefRequestCount()
        XCTAssertEqual(startCount, 0)
        XCTAssertEqual(storageThiefRequestCount, 0)
        XCTAssertEqual(model.scanState, .idle)
        XCTAssertEqual(model.cleanupHistoryState, .loaded)
        XCTAssertTrue(model.cleanupHistoryRecords.isEmpty)
        XCTAssertNil(model.cleanupHistoryStorageThiefRanking)
        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .idle)
    }

    func testRecurringStorageThiefLoadsCoalesceAndFailedRefreshKeepsCache() async {
        let ranking = recurringStorageThiefRanking(bytesRegrownPerDay: 4_096)
        let service = HomeScanEngineStub(
            controlsRecurringStorageThiefReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )

        let first = Task { @MainActor in
            await model.loadRecurringStorageThieves()
        }
        await service.waitForRecurringStorageThiefRequestCount(1)
        let second = Task { @MainActor in
            await model.loadRecurringStorageThieves()
        }
        await Task.yield()

        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .loading)
        var requestCount = await service.recurringStorageThiefRequestCount()
        XCTAssertEqual(requestCount, 1)

        await service.resolveRecurringStorageThief(
            at: 0,
            result: .success(ranking)
        )
        await first.value
        await second.value

        XCTAssertEqual(model.cleanupHistoryStorageThiefRanking, ranking)
        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .loaded)
        let firstReadAt = model.cleanupHistoryStorageThiefReadAt
        XCTAssertNotNil(firstReadAt)

        let refresh = Task { @MainActor in
            await model.refreshRecurringStorageThieves()
        }
        await service.waitForRecurringStorageThiefRequestCount(2)

        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .loading)
        XCTAssertEqual(model.cleanupHistoryStorageThiefRanking, ranking)
        XCTAssertEqual(model.cleanupHistoryStorageThiefReadAt, firstReadAt)

        await service.resolveRecurringStorageThief(
            at: 1,
            result: .failure(.retryable)
        )
        await refresh.value

        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .failed(.retryable))
        XCTAssertEqual(model.cleanupHistoryStorageThiefRanking, ranking)
        XCTAssertEqual(model.cleanupHistoryStorageThiefReadAt, firstReadAt)
        requestCount = await service.recurringStorageThiefRequestCount()
        XCTAssertEqual(requestCount, 2)
    }

    func testDurableScanRefreshesRecurringStorageThievesOnlyAfterRequested() async {
        let initial = recurringStorageThiefRanking(bytesRegrownPerDay: 1_024)
        let refreshed = recurringStorageThiefRanking(bytesRegrownPerDay: 8_192)
        let firstScan = HomeScanTaskSpy(polls: [
            .success(
                successPoll(
                    revision: 1,
                    result: successfulResult(scanID: "scan:before-ranking")
                )
            ),
        ])
        let secondScan = HomeScanTaskSpy(polls: [
            .success(
                successPoll(
                    revision: 1,
                    result: successfulResult(scanID: "scan:after-ranking")
                )
            ),
        ])
        let engine = HomeScanEngineStub(
            controlsRecurringStorageThiefReplies: true
        )
        let model = AppModel(
            engineService: engine,
            volumeMonitor: HomeScanVolumeMonitorStub(),
            homeScanService: HomeScanServiceSpy(
                responses: [
                    .success(.started(firstScan)),
                    .success(.started(secondScan)),
                ]
            ),
            homeScanClock: ManualHomeScanClock()
        )

        await model.startHomeScan()

        var requestCount = await engine.recurringStorageThiefRequestCount()
        XCTAssertEqual(requestCount, 0)
        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .idle)

        let initialLoad = Task { @MainActor in
            await model.loadRecurringStorageThieves()
        }
        await engine.waitForRecurringStorageThiefRequestCount(1)
        await engine.resolveRecurringStorageThief(
            at: 0,
            result: .success(initial)
        )
        await initialLoad.value
        XCTAssertEqual(model.cleanupHistoryStorageThiefRanking, initial)

        await model.startHomeScan()
        await engine.waitForRecurringStorageThiefRequestCount(2)

        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .loading)
        XCTAssertEqual(model.cleanupHistoryStorageThiefRanking, initial)

        await engine.resolveRecurringStorageThief(
            at: 1,
            result: .success(refreshed)
        )
        for _ in 0 ..< 100
            where model.cleanupHistoryStorageThiefRanking != refreshed
        {
            await Task.yield()
        }

        XCTAssertEqual(model.cleanupHistoryStorageThiefRanking, refreshed)
        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .loaded)
        requestCount = await engine.recurringStorageThiefRequestCount()
        XCTAssertEqual(requestCount, 2)
    }

    func testCleanupHistoryInvalidationFencesLateRecurringStorageThiefReply() async {
        let service = HomeScanEngineStub(
            controlsRecurringStorageThiefReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )

        let load = Task { @MainActor in
            await model.loadRecurringStorageThieves()
        }
        await service.waitForRecurringStorageThiefRequestCount(1)

        model.invalidateCleanupHistoryOperations()
        await service.resolveRecurringStorageThief(
            at: 0,
            result: .success(
                recurringStorageThiefRanking(bytesRegrownPerDay: 16_384)
            )
        )
        await load.value

        XCTAssertNil(model.cleanupHistoryStorageThiefRanking)
        XCTAssertNil(model.cleanupHistoryStorageThiefReadAt)
        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .idle)
        let requestCount = await service.recurringStorageThiefRequestCount()
        XCTAssertEqual(requestCount, 1)
    }

    func testCleanupHistoryClearFencesPriorRankingAndPublishesOneReconciliation()
        async
    {
        let initial = recurringStorageThiefRanking(bytesRegrownPerDay: 1_024)
        let stale = recurringStorageThiefRanking(bytesRegrownPerDay: 32_768)
        let reconciled = emptyRecurringStorageThiefRanking()
        let service = HomeScanEngineStub(
            controlsRecurringStorageThiefReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )

        let initialLoad = Task { @MainActor in
            await model.loadRecurringStorageThieves()
        }
        await service.waitForRecurringStorageThiefRequestCount(1)
        await service.resolveRecurringStorageThief(
            at: 0,
            result: .success(initial)
        )
        await initialLoad.value

        let staleRefresh = Task { @MainActor in
            await model.refreshRecurringStorageThieves()
        }
        await service.waitForRecurringStorageThiefRequestCount(2)
        await model.prepareCleanupHistoryClear()
        let confirmation = try? XCTUnwrap(model.cleanupHistoryClearConfirmation)
        XCTAssertNotNil(confirmation)

        let clear = Task { @MainActor in
            guard let confirmation else {
                return
            }
            await model.confirmCleanupHistoryClear(confirmation)
        }
        await service.waitForRecurringStorageThiefRequestCount(3)

        XCTAssertNil(model.cleanupHistoryStorageThiefRanking)
        XCTAssertNil(model.cleanupHistoryStorageThiefReadAt)
        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .loading)

        await service.resolveRecurringStorageThief(
            at: 1,
            result: .success(stale)
        )
        await staleRefresh.value

        XCTAssertNil(model.cleanupHistoryStorageThiefRanking)
        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .loading)

        await service.resolveRecurringStorageThief(
            at: 2,
            result: .success(reconciled)
        )
        await clear.value

        XCTAssertEqual(model.cleanupHistoryStorageThiefRanking, reconciled)
        XCTAssertNotNil(model.cleanupHistoryStorageThiefReadAt)
        XCTAssertEqual(model.cleanupHistoryStorageThiefState, .loaded)
        XCTAssertEqual(
            model.cleanupHistoryClearState,
            .completed(
                CleanupHistoryClearResultModel(clearedSessionCount: 1)
            )
        )
        let requestCount = await service.recurringStorageThiefRequestCount()
        XCTAssertEqual(requestCount, 3)
    }

    func testInitialLoadPublishesPathFreeCleanupHistory() async {
        let counts = CleanupHistoryStatusCounts(
            planned: 0,
            validating: 0,
            dryRun: 0,
            effectStarted: 0,
            trashed: 0,
            removed: 1,
            evicted: 0,
            skipped: 0,
            rejected: 0,
            failed: 0,
            changedSincePlan: 0,
            interrupted: 0,
            unavailable: 0,
            outcomeUnknown: 0,
            total: 1
        )
        let record = CleanupHistorySessionSummaryModel(
            sessionID: "session:test",
            planID: "plan:test",
            format: .complete,
            sourceScanID: "scan:test",
            startedAt: Date(timeIntervalSince1970: 1),
            completedAt: Date(timeIntervalSince1970: 2),
            planCreatedAt: Date(timeIntervalSince1970: 1),
            planExpiresAt: Date(timeIntervalSince1970: 3),
            mode: .permanentSafe,
            trigger: .manual,
            status: .completed,
            estimatedBytes: 512,
            verifiedCapacityDeltaBytes: 512,
            cancellationRequested: false,
            itemTotal: 1,
            pathTotal: 1,
            evidenceTotal: 1,
            itemStatusCounts: counts,
            pathStatusCounts: counts
        )
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [record], nextCursor: nil)
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )

        await model.loadInitialState()

        XCTAssertEqual(model.cleanupHistoryState, .loaded)
        XCTAssertEqual(model.cleanupHistoryRecords, [record])
    }

    func testCleanupHistorySelectionPublishesExactPathFreeDetail() async {
        let summary = cleanupHistorySummary(sessionID: "session:one")
        let detail = cleanupHistoryDetail(summary: summary)
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            cleanupHistoryDetailResult: .success(detail)
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        await model.selectCleanupHistorySession(summary.sessionID)

        XCTAssertEqual(model.selectedCleanupHistorySessionID, summary.sessionID)
        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
    }

    func testCleanupHistoryDetailRemainsVisibleWhileRuleOutcomesLoadAndFail() async {
        let summary = cleanupHistorySummary(sessionID: "session:outcome-failure")
        let detail = cleanupHistoryDetail(summary: summary)
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            cleanupHistoryDetailResult: .success(detail),
            controlsCleanupHistoryRuleOutcomeReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let selection = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await service.waitForCleanupHistoryRuleOutcomeRequestCount(1)

        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loading)

        await service.resolveCleanupHistoryRuleOutcome(
            at: 0,
            result: .failure(.retryable)
        )
        await selection.value

        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .failed(.retryable))
    }

    func testCleanupHistorySelectionPublishesExactRuleOutcomeBatch() async {
        let summary = cleanupHistorySummary(sessionID: "session:outcome-success")
        let detail = cleanupHistoryDetail(summary: summary)
        let batch = cleanupHistoryRuleOutcomeBatch(
            detail: detail,
            state: .regrown(
                cleanedAt: Date(timeIntervalSince1970: 2),
                zeroObservedAt: Date(timeIntervalSince1970: 3),
                observedAt: Date(timeIntervalSince1970: 8),
                observedBytes: 4096
            )
        )
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            cleanupHistoryDetailResult: .success(detail),
            cleanupHistoryRuleOutcomeResult: .success(batch)
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        await model.selectCleanupHistorySession(summary.sessionID)

        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loaded(batch))
        let requestCount = await service.cleanupHistoryRuleOutcomeRequestCount()
        XCTAssertEqual(requestCount, 1)
    }

    func testCleanupHistoryRepeatedSelectionRequeriesRuleOutcomes() async {
        let summary = cleanupHistorySummary(sessionID: "session:outcome-repeat")
        let detail = cleanupHistoryDetail(summary: summary)
        let firstBatch = cleanupHistoryRuleOutcomeBatch(
            detail: detail,
            state: .awaitingComparableScan(
                cleanedAt: Date(timeIntervalSince1970: 2)
            )
        )
        let secondBatch = cleanupHistoryRuleOutcomeBatch(
            detail: detail,
            state: .zeroBaselineObserved(
                cleanedAt: Date(timeIntervalSince1970: 2),
                observedAt: Date(timeIntervalSince1970: 5)
            )
        )
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            cleanupHistoryDetailResult: .success(detail),
            controlsCleanupHistoryRuleOutcomeReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let firstSelection = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await service.waitForCleanupHistoryRuleOutcomeRequestCount(1)
        await service.resolveCleanupHistoryRuleOutcome(at: 0, result: .success(firstBatch))
        await firstSelection.value
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loaded(firstBatch))

        let repeatedSelection = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await service.waitForCleanupHistoryRuleOutcomeRequestCount(2)
        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loading)
        await service.resolveCleanupHistoryRuleOutcome(at: 1, result: .success(secondBatch))
        await repeatedSelection.value

        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loaded(secondBatch))
        let requestCount = await service.cleanupHistoryRuleOutcomeRequestCount()
        XCTAssertEqual(requestCount, 2)
    }

    func testCleanupHistoryRapidSelectionAndCloseFenceLateRuleOutcomes() async {
        let firstSummary = cleanupHistorySummary(sessionID: "session:outcome-first")
        let secondSummary = cleanupHistorySummary(sessionID: "session:outcome-second")
        let firstDetail = cleanupHistoryDetail(summary: firstSummary)
        let secondDetail = cleanupHistoryDetail(summary: secondSummary)
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(
                records: [firstSummary, secondSummary],
                nextCursor: nil
            ),
            controlsCleanupHistoryDetailReplies: true,
            controlsCleanupHistoryRuleOutcomeReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let firstSelection = Task { @MainActor in
            await model.selectCleanupHistorySession(firstSummary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(1)
        await service.resolveCleanupHistoryDetail(at: 0, result: .success(firstDetail))
        await service.waitForCleanupHistoryRuleOutcomeRequestCount(1)

        let secondSelection = Task { @MainActor in
            await model.selectCleanupHistorySession(secondSummary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(2)
        await service.resolveCleanupHistoryDetail(at: 1, result: .success(secondDetail))
        await service.waitForCleanupHistoryRuleOutcomeRequestCount(2)

        await service.resolveCleanupHistoryRuleOutcome(
            at: 0,
            result: .success(cleanupHistoryRuleOutcomeBatch(detail: firstDetail))
        )
        await firstSelection.value
        XCTAssertEqual(model.selectedCleanupHistorySessionID, secondSummary.sessionID)
        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(secondDetail))
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loading)

        model.closeCleanupHistorySession()
        await service.resolveCleanupHistoryRuleOutcome(
            at: 1,
            result: .success(cleanupHistoryRuleOutcomeBatch(detail: secondDetail))
        )
        await secondSelection.value

        XCTAssertNil(model.selectedCleanupHistorySessionID)
        XCTAssertEqual(model.cleanupHistoryDetailState, .idle)
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .idle)
    }

    func testCleanupHistoryRefreshReloadsRuleOutcomes() async {
        let summary = cleanupHistorySummary(sessionID: "session:outcome-refresh")
        let detail = cleanupHistoryDetail(summary: summary)
        let firstBatch = cleanupHistoryRuleOutcomeBatch(detail: detail)
        let refreshedBatch = cleanupHistoryRuleOutcomeBatch(
            detail: detail,
            state: .laterSizeObserved(
                cleanedAt: Date(timeIntervalSince1970: 2),
                observedAt: Date(timeIntervalSince1970: 7),
                observedBytes: 2048
            )
        )
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            cleanupHistoryDetailResult: .success(detail),
            controlsCleanupHistoryRuleOutcomeReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let selection = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await service.waitForCleanupHistoryRuleOutcomeRequestCount(1)
        await service.resolveCleanupHistoryRuleOutcome(at: 0, result: .success(firstBatch))
        await selection.value
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loaded(firstBatch))

        let refresh = Task { @MainActor in
            await model.refreshCleanupHistory()
        }
        await service.waitForCleanupHistoryRuleOutcomeRequestCount(2)
        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loading)
        await service.resolveCleanupHistoryRuleOutcome(at: 1, result: .success(refreshedBatch))
        await refresh.value

        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loaded(refreshedBatch))
        let requestCount = await service.cleanupHistoryRuleOutcomeRequestCount()
        XCTAssertEqual(requestCount, 2)
    }

    func testCleanupHistoryLegacyDetailSkipsRuleOutcomeQuery() async {
        let summary = cleanupHistorySummary(
            sessionID: "session:outcome-legacy",
            format: .legacyIncomplete
        )
        let detail = cleanupHistoryDetail(summary: summary)
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            cleanupHistoryDetailResult: .success(detail),
            controlsCleanupHistoryRuleOutcomeReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        await model.selectCleanupHistorySession(summary.sessionID)

        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
        XCTAssertEqual(
            model.cleanupHistoryRuleOutcomeState,
            .unavailableForLegacyRecord
        )
        let requestCount = await service.cleanupHistoryRuleOutcomeRequestCount()
        XCTAssertEqual(requestCount, 0)
    }

    func testSuccessfulScanFencesAndReloadsSelectedRuleOutcomes() async {
        let summary = cleanupHistorySummary(sessionID: "session:outcome-after-scan")
        let detail = cleanupHistoryDetail(summary: summary)
        let beforeScan = cleanupHistoryRuleOutcomeBatch(
            detail: detail,
            state: .awaitingComparableScan(
                cleanedAt: Date(timeIntervalSince1970: 2)
            )
        )
        let afterScan = cleanupHistoryRuleOutcomeBatch(
            detail: detail,
            state: .zeroBaselineObserved(
                cleanedAt: Date(timeIntervalSince1970: 2),
                observedAt: Date(timeIntervalSince1970: 9)
            )
        )
        let engine = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(
                records: [summary],
                nextCursor: nil
            ),
            cleanupHistoryDetailResult: .success(detail),
            controlsCleanupHistoryRuleOutcomeReplies: true
        )
        let scanTask = HomeScanTaskSpy(polls: [
            .success(
                successPoll(
                    revision: 1,
                    result: successfulResult(scanID: "scan:outcome-refresh")
                )
            ),
        ])
        let model = AppModel(
            engineService: engine,
            volumeMonitor: HomeScanVolumeMonitorStub(),
            homeScanService: HomeScanServiceSpy(
                responses: [.success(.started(scanTask))]
            ),
            homeScanClock: ManualHomeScanClock()
        )
        await model.loadCleanupHistory()

        let selection = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await engine.waitForCleanupHistoryRuleOutcomeRequestCount(1)
        await engine.resolveCleanupHistoryRuleOutcome(at: 0, result: .success(beforeScan))
        await selection.value
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loaded(beforeScan))
        XCTAssertNotNil(model.cleanupHistoryRuleOutcomesReadAt)

        await model.startHomeScan()
        await engine.waitForCleanupHistoryRuleOutcomeRequestCount(2)
        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loading)
        XCTAssertNil(model.cleanupHistoryRuleOutcomesReadAt)

        await engine.resolveCleanupHistoryRuleOutcome(at: 1, result: .success(afterScan))
        for _ in 0 ..< 100 where model.cleanupHistoryRuleOutcomeState != .loaded(afterScan) {
            await Task.yield()
        }
        XCTAssertEqual(model.cleanupHistoryRuleOutcomeState, .loaded(afterScan))
        XCTAssertNotNil(model.cleanupHistoryRuleOutcomesReadAt)
    }

    func testCleanupHistoryDetailFailureCanRetry() async {
        let summary = cleanupHistorySummary(sessionID: "session:retry")
        let detail = cleanupHistoryDetail(summary: summary)
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            controlsCleanupHistoryDetailReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let first = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(1)
        await service.resolveCleanupHistoryDetail(
            at: 0,
            result: .failure(.retryable)
        )
        await first.value
        XCTAssertEqual(model.cleanupHistoryDetailState, .failed(.retryable))

        let retry = Task { @MainActor in
            await model.retryCleanupHistorySession()
        }
        await service.waitForCleanupHistoryDetailRequestCount(2)
        await service.resolveCleanupHistoryDetail(at: 1, result: .success(detail))
        await retry.value

        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(detail))
    }

    func testCleanupHistoryRapidSelectionIgnoresLatePreviousReply() async {
        let firstSummary = cleanupHistorySummary(sessionID: "session:first")
        let secondSummary = cleanupHistorySummary(sessionID: "session:second")
        let firstDetail = cleanupHistoryDetail(summary: firstSummary)
        let secondDetail = cleanupHistoryDetail(summary: secondSummary)
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(
                records: [firstSummary, secondSummary],
                nextCursor: nil
            ),
            controlsCleanupHistoryDetailReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let first = Task { @MainActor in
            await model.selectCleanupHistorySession(firstSummary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(1)
        let second = Task { @MainActor in
            await model.selectCleanupHistorySession(secondSummary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(2)

        await service.resolveCleanupHistoryDetail(at: 0, result: .success(firstDetail))
        await first.value
        XCTAssertEqual(model.selectedCleanupHistorySessionID, secondSummary.sessionID)
        XCTAssertEqual(model.cleanupHistoryDetailState, .loading)

        await service.resolveCleanupHistoryDetail(at: 1, result: .success(secondDetail))
        await second.value
        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(secondDetail))
    }

    func testCleanupHistoryCloseIgnoresLateReply() async {
        let summary = cleanupHistorySummary(sessionID: "session:close")
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            controlsCleanupHistoryDetailReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let selection = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(1)
        model.closeCleanupHistorySession()
        await service.resolveCleanupHistoryDetail(
            at: 0,
            result: .success(cleanupHistoryDetail(summary: summary))
        )
        await selection.value

        XCTAssertNil(model.selectedCleanupHistorySessionID)
        XCTAssertEqual(model.cleanupHistoryDetailState, .idle)
    }

    func testCleanupHistoryRefreshClearsMissingSelectionAndFencesLateReply() async {
        let summary = cleanupHistorySummary(sessionID: "session:removed")
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            controlsCleanupHistoryDetailReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let selection = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(1)
        await service.setCleanupHistoryPage(
            CleanupHistoryPageModel(records: [], nextCursor: nil)
        )
        await model.refreshCleanupHistory()
        await service.resolveCleanupHistoryDetail(
            at: 0,
            result: .success(cleanupHistoryDetail(summary: summary))
        )
        await selection.value

        XCTAssertTrue(model.cleanupHistoryRecords.isEmpty)
        XCTAssertNil(model.selectedCleanupHistorySessionID)
        XCTAssertEqual(model.cleanupHistoryDetailState, .idle)
    }

    func testCleanupHistoryRefreshReloadsSelectedDetail() async {
        let initialSummary = cleanupHistorySummary(
            sessionID: "session:refresh",
            estimatedBytes: 512
        )
        let refreshedSummary = cleanupHistorySummary(
            sessionID: initialSummary.sessionID,
            estimatedBytes: 1024
        )
        let initialDetail = cleanupHistoryDetail(summary: initialSummary)
        let refreshedDetail = cleanupHistoryDetail(summary: refreshedSummary)
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(
                records: [initialSummary],
                nextCursor: nil
            ),
            controlsCleanupHistoryDetailReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let selection = Task { @MainActor in
            await model.selectCleanupHistorySession(initialSummary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(1)
        await service.resolveCleanupHistoryDetail(at: 0, result: .success(initialDetail))
        await selection.value
        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(initialDetail))

        await service.setCleanupHistoryPage(
            CleanupHistoryPageModel(records: [refreshedSummary], nextCursor: nil)
        )
        let refresh = Task { @MainActor in
            await model.refreshCleanupHistory()
        }
        await service.waitForCleanupHistoryDetailRequestCount(2)
        XCTAssertEqual(model.cleanupHistoryDetailState, .loading)
        await service.resolveCleanupHistoryDetail(at: 1, result: .success(refreshedDetail))
        await refresh.value

        XCTAssertEqual(model.cleanupHistoryRecords, [refreshedSummary])
        XCTAssertEqual(model.cleanupHistoryDetailState, .loaded(refreshedDetail))
    }

    func testCleanupHistoryShutdownInvalidationIgnoresLateReply() async {
        let summary = cleanupHistorySummary(sessionID: "session:shutdown")
        let service = HomeScanEngineStub(
            cleanupHistoryPage: CleanupHistoryPageModel(records: [summary], nextCursor: nil),
            controlsCleanupHistoryDetailReplies: true
        )
        let model = AppModel(
            engineService: service,
            volumeMonitor: HomeScanVolumeMonitorStub()
        )
        await model.loadCleanupHistory()

        let selection = Task { @MainActor in
            await model.selectCleanupHistorySession(summary.sessionID)
        }
        await service.waitForCleanupHistoryDetailRequestCount(1)
        model.invalidateCleanupHistoryOperations()
        await service.resolveCleanupHistoryDetail(
            at: 0,
            result: .success(cleanupHistoryDetail(summary: summary))
        )
        await selection.value

        XCTAssertNil(model.selectedCleanupHistorySessionID)
        XCTAssertEqual(model.cleanupHistoryDetailState, .idle)
    }

    func testConcurrentStartsCoalesceAndPublishOnlyMeasuredProgressThenSuccess() async {
        let facts = ScanProgressFacts(
            files: 4,
            directories: 2,
            knownAllocatedBytes: 1024,
            issueCount: 1
        )
        let summary = successfulResult(progress: facts)
        let task = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: nil)),
                .success(activePoll(revision: 3, progress: facts)),
                .success(successPoll(revision: 6, result: summary)),
            ]
        )
        let service = HomeScanServiceSpy(responses: [.success(.started(task))])
        let clock = ManualHomeScanClock()
        let model = model(service: service, clock: clock)

        let first = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)
        XCTAssertEqual(model.scanState.phase, .scanning(nil))

        let second = Task { @MainActor in await model.startHomeScan() }
        await Task.yield()
        let startCount = await service.startCount()
        XCTAssertEqual(startCount, 1)

        await clock.advance()
        await clock.waitForSleepCount(2)
        XCTAssertEqual(model.scanState.phase, .scanning(facts))
        await clock.advance()
        await first.value
        await second.value

        let expected = try! XCTUnwrap(summary.successfulSummary)
        XCTAssertEqual(model.scanState.phase, .succeeded(expected))
        XCTAssertEqual(model.scanState.lastSuccessful, expected)
        let pollCount = await task.pollCount()
        XCTAssertEqual(pollCount, 3)
    }

    func testProgressiveEventsAreRetainedAndResetForEachGeneration() async {
        let event = HomeScanEvent(sequence: 1, kind: .started)
        let task = HomeScanTaskSpy(polls: [
            .success(successPoll(revision: 2, result: successfulResult(), events: [event], eventCursor: 1, oldestEventSequence: 1)),
        ])
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: ManualHomeScanClock()
        )

        await model.startHomeScan()
        XCTAssertEqual(model.latestHomeScanEvents, [event])
    }

    func testCancellationRemainsRequestedUntilRustReportsTerminal() async {
        let facts = ScanProgressFacts(
            files: 8,
            directories: 3,
            knownAllocatedBytes: 2048,
            issueCount: 0
        )
        let task = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: facts)),
                .success(
                    activePoll(
                        revision: 3,
                        progress: facts,
                        cancellationRequested: true
                    )
                ),
                .success(cancelledPoll(revision: 5)),
            ]
        )
        let clock = ManualHomeScanClock()
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: clock
        )
        let scan = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)

        await model.cancelHomeScan()
        XCTAssertEqual(model.scanState.phase, .cancellationRequested(facts))
        let cancelCount = await task.cancelCount()
        XCTAssertEqual(cancelCount, 1)

        await clock.advance()
        await clock.waitForSleepCount(2)
        XCTAssertEqual(model.scanState.phase, .cancellationRequested(facts))
        await clock.advance()
        await scan.value
        XCTAssertEqual(model.scanState.phase, .cancelled)
    }

    func testLateSuccessAfterCancellationIsTruthfulAndRetainsPreviousSuccessOnRetryFailure() async {
        let firstFacts = ScanProgressFacts(
            files: 1,
            directories: 1,
            knownAllocatedBytes: 64,
            issueCount: 0
        )
        let firstResult = successfulResult(scanID: "scan:first", progress: firstFacts)
        let firstTask = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: firstFacts)),
                .success(successPoll(revision: 4, result: firstResult, cancelled: true)),
            ]
        )
        let service = HomeScanServiceSpy(
            responses: [
                .success(.started(firstTask)),
                .failure(.queueFull),
            ]
        )
        let clock = ManualHomeScanClock()
        let model = model(service: service, clock: clock)
        let scan = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)
        await model.cancelHomeScan()
        await clock.advance()
        await scan.value

        let expected = try! XCTUnwrap(firstResult.successfulSummary)
        XCTAssertEqual(model.scanState.phase, .succeeded(expected))
        XCTAssertEqual(model.scanState.lastSuccessful, expected)

        await model.startHomeScan()
        XCTAssertEqual(model.scanState.phase, .failed(.busy))
        XCTAssertEqual(model.scanState.lastSuccessful, expected)
        let startCount = await service.startCount()
        XCTAssertEqual(startCount, 2)
    }

    func testShutdownCancelsTaskDrainsDriverAndRejectsLatePublication() async {
        let facts = ScanProgressFacts(
            files: 10,
            directories: 2,
            knownAllocatedBytes: 512,
            issueCount: 0
        )
        let task = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: facts)),
                .success(successPoll(revision: 4, result: successfulResult(progress: facts))),
            ]
        )
        let clock = ManualHomeScanClock()
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: clock
        )
        let scan = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)
        XCTAssertEqual(model.scanState.phase, .scanning(facts))

        await model.shutdownHomeScan()
        await scan.value

        let cancelCount = await task.cancelCount()
        let pollCount = await task.pollCount()
        XCTAssertEqual(cancelCount, 1)
        XCTAssertEqual(pollCount, 1)
        XCTAssertEqual(model.scanState.phase, .scanning(facts))
        await model.startHomeScan()
        let finalPollCount = await task.pollCount()
        XCTAssertEqual(finalPollCount, 1)
    }

    func testTerminalRuntimeQuiescenceCancelsScanBeforeJoiningRetainedDriver() async {
        let facts = ScanProgressFacts(
            files: 10,
            directories: 2,
            knownAllocatedBytes: 512,
            issueCount: 0
        )
        let task = HomeScanTaskSpy(
            polls: [
                .success(activePoll(revision: 2, progress: facts)),
                .success(cancelledPoll(revision: 3)),
            ]
        )
        let clock = ManualHomeScanClock()
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: clock
        )
        let scan = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)

        let drain = model.beginTerminalRuntimeQuiescence()
        for _ in 0 ..< 1_000 where await task.cancelCount() == 0 {
            await Task.yield()
        }
        let cancelCount = await task.cancelCount()
        XCTAssertEqual(cancelCount, 1)

        // Lets a broken implementation escape its retained-driver wait so the
        // regression fails instead of hanging the test process.
        await clock.advance()
        await drain.value
        await scan.value
        let pollCount = await task.pollCount()
        XCTAssertEqual(pollCount, 1)
    }

    func testPollFailureRequestsCancellationBeforeReleasingActiveTask() async {
        let task = HomeScanTaskSpy(polls: [.failure(.invalidResponse)])
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: ManualHomeScanClock()
        )

        await model.startHomeScan()

        XCTAssertEqual(model.scanState.phase, .failed(.invalidResponse))
        let cancelCount = await task.cancelCount()
        XCTAssertEqual(cancelCount, 1)
    }

    func testSubtreeSuccessPreservesLastSuccessfulHomeBaseline() async {
        let homeResult = successfulResult(scanID: "scan:home")
        let subtreeResult = successfulResult(scanID: "scan:folder")
        let homeService = HomeScanServiceSpy(
            responses: [.success(.started(HomeScanTaskSpy(polls: [
                .success(successPoll(revision: 1, result: homeResult)),
            ])))]
        )
        let model = model(service: homeService, clock: ManualHomeScanClock())
        await model.startHomeScan()
        let homeSummary = try! XCTUnwrap(homeResult.successfulSummary)

        let subtreeService = SubtreeScanServiceSpy(
            response: .success(.started(HomeScanTaskSpy(polls: [
                .success(successPoll(revision: 1, result: subtreeResult)),
            ])))
        )
        let outcome = await model.startSubtreeScan(
            sourceScanID: "scan:home",
            nodeID: 42,
            displayName: "Caches",
            using: subtreeService
        )

        let subtreeSummary = try! XCTUnwrap(subtreeResult.successfulSummary)
        XCTAssertEqual(outcome, .succeeded(subtreeSummary))
        XCTAssertEqual(model.scanState.phase, .succeeded(subtreeSummary))
        XCTAssertEqual(model.scanState.scope, .subtree(displayName: "Caches"))
        XCTAssertEqual(model.scanState.lastSuccessful, homeSummary)
        let request = await subtreeService.lastRequest()
        XCTAssertEqual(request?.scanID, "scan:home")
        XCTAssertEqual(request?.nodeID, 42)
    }

    func testDifferentScanRequestIsBusyWithoutReplacingActiveScope() async {
        let task = HomeScanTaskSpy(polls: [
            .success(activePoll(revision: 1, progress: nil)),
            .success(successPoll(revision: 2, result: successfulResult())),
        ])
        let clock = ManualHomeScanClock()
        let model = model(
            service: HomeScanServiceSpy(responses: [.success(.started(task))]),
            clock: clock
        )
        let home = Task { @MainActor in await model.startHomeScan() }
        await clock.waitForSleepCount(1)

        let outcome = await model.startSubtreeScan(
            sourceScanID: "scan:home",
            nodeID: 7,
            displayName: "Build",
            using: SubtreeScanServiceSpy(response: .failure(.busy))
        )

        XCTAssertEqual(outcome, .failed(.busy))
        XCTAssertEqual(model.scanState.scope, .home)
        XCTAssertEqual(model.scanState.phase, .scanning(nil))
        await clock.advance()
        await home.value
    }

    func testExactSubtreeStartsCoalesceOntoOneTask() async {
        let task = HomeScanTaskSpy(polls: [
            .success(activePoll(revision: 1, progress: nil)),
            .success(successPoll(revision: 2, result: successfulResult(scanID: "scan:new"))),
        ])
        let service = SubtreeScanServiceSpy(response: .success(.started(task)))
        let clock = ManualHomeScanClock()
        let model = model(service: HomeScanServiceSpy(responses: []), clock: clock)

        let first = Task { @MainActor in
            await model.startSubtreeScan(
                sourceScanID: "scan:old",
                nodeID: 9,
                displayName: "Caches",
                using: service
            )
        }
        await clock.waitForSleepCount(1)
        let second = Task { @MainActor in
            await model.startSubtreeScan(
                sourceScanID: "scan:old",
                nodeID: 9,
                displayName: "Caches",
                using: service
            )
        }
        await Task.yield()
        let startsBeforeCompletion = await service.startCount()
        XCTAssertEqual(startsBeforeCompletion, 1)
        await clock.advance()
        let firstOutcome = await first.value
        let secondOutcome = await second.value
        let finalStartCount = await service.startCount()
        XCTAssertEqual(firstOutcome, secondOutcome)
        XCTAssertEqual(finalStartCount, 1)
    }

    private func model(
        service: HomeScanServiceSpy,
        clock: ManualHomeScanClock
    ) -> AppModel {
        AppModel(
            engineService: HomeScanEngineStub(),
            homeScanService: service,
            homeScanClock: clock
        )
    }
}

private actor SubtreeScanServiceSpy: DuxSnapshotSubtreeScanServing {
    struct Request: Equatable, Sendable {
        let scanID: String
        let nodeID: UInt64
    }

    private let response: Result<HomeScanStartDisposition, HomeScanServiceError>
    private var requests: [Request] = []

    init(response: Result<HomeScanStartDisposition, HomeScanServiceError>) {
        self.response = response
    }

    func startSubtreeScan(
        sourceScanID: String,
        nodeID: UInt64
    ) async throws -> HomeScanStartDisposition {
        requests.append(Request(scanID: sourceScanID, nodeID: nodeID))
        return try response.get()
    }

    func lastRequest() -> Request? {
        requests.last
    }

    func startCount() -> Int {
        requests.count
    }
}

private actor HomeScanServiceSpy: HomeScanServing {
    private var responses: [Result<HomeScanStartDisposition, HomeScanServiceError>]
    private var starts = 0

    init(responses: [Result<HomeScanStartDisposition, HomeScanServiceError>]) {
        self.responses = responses
    }

    func startHomeScan() async throws -> HomeScanStartDisposition {
        starts += 1
        guard !responses.isEmpty else {
            throw HomeScanServiceError.invalidResponse
        }
        return try responses.removeFirst().get()
    }

    func startCount() -> Int { starts }
}

private actor HomeScanTaskSpy: HomeScanTask {
    private var polls: [Result<HomeScanTaskPoll, HomeScanServiceError>]
    private var pollsObserved = 0
    private var cancellations = 0

    init(polls: [Result<HomeScanTaskPoll, HomeScanServiceError>]) {
        self.polls = polls
    }

    func poll() async throws -> HomeScanTaskPoll {
        pollsObserved += 1
        guard !polls.isEmpty else {
            throw HomeScanServiceError.invalidResponse
        }
        return try polls.removeFirst().get()
    }

    func requestCancellation() async throws -> HomeScanCancelOutcome {
        cancellations += 1
        return cancellations == 1 ? .requested : .alreadyRequested
    }

    func pollCount() -> Int { pollsObserved }
    func cancelCount() -> Int { cancellations }
}

private actor ManualHomeScanClock: HomeScanPollingClock {
    private struct Sleeper {
        let id: UUID
        let continuation: CheckedContinuation<Void, any Error>
    }

    private var sleepers: [Sleeper] = []
    private var cancelled: Set<UUID> = []
    private var sleepCount = 0

    func sleepUntilNextPoll() async throws {
        try Task.checkCancellation()
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation {
                (continuation: CheckedContinuation<Void, any Error>) in
                sleepCount += 1
                if cancelled.remove(id) != nil || Task.isCancelled {
                    continuation.resume(throwing: CancellationError())
                } else {
                    sleepers.append(Sleeper(id: id, continuation: continuation))
                }
            }
        } onCancel: {
            Task { await self.cancel(id) }
        }
    }

    func waitForSleepCount(_ expected: Int) async {
        while sleepCount < expected {
            await Task.yield()
        }
    }

    func advance() {
        guard !sleepers.isEmpty else {
            return
        }
        sleepers.removeFirst().continuation.resume()
    }

    private func cancel(_ id: UUID) {
        if let index = sleepers.firstIndex(where: { $0.id == id }) {
            sleepers.remove(at: index).continuation.resume(throwing: CancellationError())
        } else {
            cancelled.insert(id)
        }
    }
}

private actor HomeScanEngineStub: EngineServing {
    private struct PendingCleanupHistoryDetailReply {
        let sessionID: String
        var continuation:
            CheckedContinuation<CleanupHistorySessionDetailModel, any Error>?
    }

    private struct PendingCleanupHistoryRuleOutcomeReply {
        let sessionID: String
        let detail: CleanupHistorySessionDetailModel
        var continuation:
            CheckedContinuation<CleanupHistoryRuleOutcomeBatchModel, any Error>?
    }

    private struct PendingRecurringStorageThiefReply {
        var continuation:
            CheckedContinuation<CleanupHistoryStorageThiefRankingModel, any Error>?
    }

    private var cleanupHistoryPage: CleanupHistoryPageModel
    private let cleanupHistoryDetailResult:
        Result<CleanupHistorySessionDetailModel, CleanupHistoryServiceError>?
    private let controlsCleanupHistoryDetailReplies: Bool
    private var pendingCleanupHistoryDetailReplies:
        [PendingCleanupHistoryDetailReply] = []
    private let cleanupHistoryRuleOutcomeResult:
        Result<CleanupHistoryRuleOutcomeBatchModel, CleanupHistoryServiceError>?
    private let controlsCleanupHistoryRuleOutcomeReplies: Bool
    private var cleanupHistoryRuleOutcomeRequests:
        [(sessionID: String, detail: CleanupHistorySessionDetailModel)] = []
    private var pendingCleanupHistoryRuleOutcomeReplies:
        [PendingCleanupHistoryRuleOutcomeReply] = []
    private let recurringStorageThiefResult:
        Result<CleanupHistoryStorageThiefRankingModel, CleanupHistoryServiceError>?
    private let controlsRecurringStorageThiefReplies: Bool
    private var recurringStorageThiefRequests = 0
    private var pendingRecurringStorageThiefReplies:
        [PendingRecurringStorageThiefReply] = []

    init(
        cleanupHistoryPage: CleanupHistoryPageModel = CleanupHistoryPageModel(
            records: [],
            nextCursor: nil
        ),
        cleanupHistoryDetailResult:
        Result<CleanupHistorySessionDetailModel, CleanupHistoryServiceError>? = nil,
        controlsCleanupHistoryDetailReplies: Bool = false,
        cleanupHistoryRuleOutcomeResult:
        Result<CleanupHistoryRuleOutcomeBatchModel, CleanupHistoryServiceError>? = nil,
        controlsCleanupHistoryRuleOutcomeReplies: Bool = false,
        recurringStorageThiefResult:
        Result<CleanupHistoryStorageThiefRankingModel, CleanupHistoryServiceError>? = nil,
        controlsRecurringStorageThiefReplies: Bool = false
    ) {
        self.cleanupHistoryPage = cleanupHistoryPage
        self.cleanupHistoryDetailResult = cleanupHistoryDetailResult
        self.controlsCleanupHistoryDetailReplies =
            controlsCleanupHistoryDetailReplies
        self.cleanupHistoryRuleOutcomeResult = cleanupHistoryRuleOutcomeResult
        self.controlsCleanupHistoryRuleOutcomeReplies =
            controlsCleanupHistoryRuleOutcomeReplies
        self.recurringStorageThiefResult = recurringStorageThiefResult
        self.controlsRecurringStorageThiefReplies =
            controlsRecurringStorageThiefReplies
    }

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 12, executedOffMainThread: true)
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
            policy: DiskPressurePolicy(
                source: .default,
                revision: 1,
                configuration: .defaults,
                updatedAtUnixMilliseconds: 1
            ),
            changed: true
        )
    }

    func loadRecentCleanupHistory(
        cursor _: CleanupHistoryCursorModel?,
        limit _: UInt16
    ) async throws -> CleanupHistoryPageModel {
        cleanupHistoryPage
    }

    func loadCleanupHistorySession(
        sessionID: String
    ) async throws -> CleanupHistorySessionDetailModel {
        if let cleanupHistoryDetailResult {
            return try cleanupHistoryDetailResult.get()
        }
        guard controlsCleanupHistoryDetailReplies else {
            throw CleanupHistoryServiceError.unavailable
        }
        return try await withCheckedThrowingContinuation { continuation in
            pendingCleanupHistoryDetailReplies.append(
                PendingCleanupHistoryDetailReply(
                    sessionID: sessionID,
                    continuation: continuation
                )
            )
        }
    }

    func loadCleanupHistoryRuleOutcomes(
        sessionID: String,
        detail: CleanupHistorySessionDetailModel
    ) async throws -> CleanupHistoryRuleOutcomeBatchModel {
        cleanupHistoryRuleOutcomeRequests.append((sessionID, detail))
        if let cleanupHistoryRuleOutcomeResult {
            return try cleanupHistoryRuleOutcomeResult.get()
        }
        guard controlsCleanupHistoryRuleOutcomeReplies else {
            throw CleanupHistoryServiceError.unavailable
        }
        return try await withCheckedThrowingContinuation { continuation in
            pendingCleanupHistoryRuleOutcomeReplies.append(
                PendingCleanupHistoryRuleOutcomeReply(
                    sessionID: sessionID,
                    detail: detail,
                    continuation: continuation
                )
            )
        }
    }

    func loadRecurringStorageThieves() async throws
        -> CleanupHistoryStorageThiefRankingModel
    {
        recurringStorageThiefRequests += 1
        if let recurringStorageThiefResult {
            return try recurringStorageThiefResult.get()
        }
        guard controlsRecurringStorageThiefReplies else {
            throw CleanupHistoryServiceError.unavailable
        }
        return try await withCheckedThrowingContinuation { continuation in
            pendingRecurringStorageThiefReplies.append(
                PendingRecurringStorageThiefReply(
                    continuation: continuation
                )
            )
        }
    }

    func prepareCleanupHistoryClear() async throws
        -> any DuxCleanupHistoryClearPreviewLease
    {
        HomeScanCleanupHistoryClearPreviewLease()
    }

    func clearCleanupHistory(
        _: any DuxCleanupHistoryClearPreviewLease
    ) async throws -> CleanupHistoryClearResultModel {
        CleanupHistoryClearResultModel(clearedSessionCount: 1)
    }

    func setCleanupHistoryPage(_ page: CleanupHistoryPageModel) {
        cleanupHistoryPage = page
    }

    func waitForCleanupHistoryDetailRequestCount(_ expected: Int) async {
        while pendingCleanupHistoryDetailReplies.count < expected {
            await Task.yield()
        }
    }

    func cleanupHistoryRuleOutcomeRequestCount() -> Int {
        cleanupHistoryRuleOutcomeRequests.count
    }

    func recurringStorageThiefRequestCount() -> Int {
        recurringStorageThiefRequests
    }

    func waitForRecurringStorageThiefRequestCount(_ expected: Int) async {
        while recurringStorageThiefRequests < expected {
            await Task.yield()
        }
    }

    func waitForCleanupHistoryRuleOutcomeRequestCount(_ expected: Int) async {
        while pendingCleanupHistoryRuleOutcomeReplies.count < expected {
            await Task.yield()
        }
    }

    func resolveCleanupHistoryRuleOutcome(
        at index: Int,
        result: Result<CleanupHistoryRuleOutcomeBatchModel, CleanupHistoryServiceError>
    ) {
        guard pendingCleanupHistoryRuleOutcomeReplies.indices.contains(index),
              let continuation =
              pendingCleanupHistoryRuleOutcomeReplies[index].continuation
        else {
            XCTFail("No pending cleanup-history rule-outcome request at index \(index)")
            return
        }
        pendingCleanupHistoryRuleOutcomeReplies[index].continuation = nil
        switch result {
        case let .success(batch):
            continuation.resume(returning: batch)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }

    func resolveRecurringStorageThief(
        at index: Int,
        result:
            Result<CleanupHistoryStorageThiefRankingModel, CleanupHistoryServiceError>
    ) {
        guard pendingRecurringStorageThiefReplies.indices.contains(index),
              let continuation =
              pendingRecurringStorageThiefReplies[index].continuation
        else {
            XCTFail("No pending recurring-storage-thief request at index \(index)")
            return
        }
        pendingRecurringStorageThiefReplies[index].continuation = nil
        switch result {
        case let .success(ranking):
            continuation.resume(returning: ranking)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }

    func resolveCleanupHistoryDetail(
        at index: Int,
        result: Result<CleanupHistorySessionDetailModel, CleanupHistoryServiceError>
    ) {
        guard pendingCleanupHistoryDetailReplies.indices.contains(index),
              let continuation =
              pendingCleanupHistoryDetailReplies[index].continuation
        else {
            XCTFail("No pending cleanup-history detail request at index \(index)")
            return
        }
        pendingCleanupHistoryDetailReplies[index].continuation = nil
        switch result {
        case let .success(detail):
            continuation.resume(returning: detail)
        case let .failure(error):
            continuation.resume(throwing: error)
        }
    }
}

private func cleanupHistorySummary(
    sessionID: String,
    estimatedBytes: UInt64 = 512,
    format: CleanupHistoryRecordFormat = .complete
) -> CleanupHistorySessionSummaryModel {
    let counts = CleanupHistoryStatusCounts(
        planned: 0,
        validating: 0,
        dryRun: 0,
        effectStarted: 0,
        trashed: 0,
        removed: 1,
        evicted: 0,
        skipped: 0,
        rejected: 0,
        failed: 0,
        changedSincePlan: 0,
        interrupted: 0,
        unavailable: 0,
        outcomeUnknown: 0,
        total: 1
    )
    return CleanupHistorySessionSummaryModel(
        sessionID: sessionID,
        planID: "plan:\(sessionID)",
        format: format,
        sourceScanID: format == .complete ? "scan:\(sessionID)" : nil,
        startedAt: Date(timeIntervalSince1970: 1),
        completedAt: Date(timeIntervalSince1970: 2),
        planCreatedAt: format == .complete ? Date(timeIntervalSince1970: 1) : nil,
        planExpiresAt: format == .complete ? Date(timeIntervalSince1970: 3) : nil,
        mode: .permanentSafe,
        trigger: .manual,
        status: .completed,
        estimatedBytes: estimatedBytes,
        verifiedCapacityDeltaBytes: Int64(estimatedBytes),
        cancellationRequested: format == .complete ? false : nil,
        itemTotal: 1,
        pathTotal: 1,
        evidenceTotal: format == .complete ? 1 : 0,
        itemStatusCounts: counts,
        pathStatusCounts: counts
    )
}

private func cleanupHistoryDetail(
    summary: CleanupHistorySessionSummaryModel
) -> CleanupHistorySessionDetailModel {
    CleanupHistorySessionDetailModel(
        summary: summary,
        items: [
            CleanupHistoryItemModel(
                ordinal: 0,
                ruleID: "developer.rust-target",
                ruleRevision: 1,
                category: summary.format == .complete ? .developerArtifact : nil,
                safety: summary.format == .complete ? .safeRegenerable : nil,
                action: summary.format == .complete
                    ? .removeKnownRegenerableContents
                    : nil,
                ruleScheduleEligible: summary.format == .complete ? false : nil,
                newestModificationAt: summary.format == .complete
                    ? Date(timeIntervalSince1970: 1)
                    : nil,
                estimatedBytes: summary.estimatedBytes,
                status: .removed,
                errorRecorded: false,
                errorCategory: nil,
                pathCount: 1,
                evidenceCount: summary.format == .complete ? 1 : 0
            ),
        ],
        warnings: summary.format == .complete ? [.permanentRemovalCannotBeUndone] : []
    )
}

private func cleanupHistoryRuleOutcomeBatch(
    detail: CleanupHistorySessionDetailModel,
    state: CleanupHistoryRuleOutcomeState = .awaitingComparableScan(
        cleanedAt: Date(timeIntervalSince1970: 2)
    )
) -> CleanupHistoryRuleOutcomeBatchModel {
    CleanupHistoryRuleOutcomeBatchModel(
        sessionID: detail.summary.sessionID,
        outcomes: detail.items.map { item in
            CleanupHistoryRuleOutcomeModel(
                itemOrdinal: item.ordinal,
                ruleID: item.ruleID,
                ruleRevision: item.ruleRevision,
                state: state
            )
        }
    )
}

private func recurringStorageThiefRanking(
    bytesRegrownPerDay: UInt64
) -> CleanupHistoryStorageThiefRankingModel {
    CleanupHistoryStorageThiefRankingModel(
        permanentSafeSessionCount: 3,
        manualCleanupSessionCount: 2,
        rankedRuleCount: 1,
        hasOlderPermanentSafeSessions: false,
        groups: [
            CleanupHistoryStorageThiefGroupModel(
                rank: 1,
                ruleID: "developer.rust.target",
                latestRuleRevision: 3,
                observedRevisionCount: 1,
                successfulCleanupCount: 2,
                successfulManualCleanupCount: 2,
                observedRegrowthCycleCount: 1,
                manualRegrowthCycleCount: 1,
                totalObservedRegrownBytes: bytesRegrownPerDay,
                totalRegrowthDurationSeconds: 86_400,
                totalRegrowthDurationNanoseconds: 0,
                bytesRegrownPerDay: bytesRegrownPerDay,
                rateCapped: false,
                latestCleanupAt: Date(timeIntervalSince1970: 10),
                latestRegrowthAt: Date(timeIntervalSince1970: 20),
                automationHistoryThresholdMet: true
            ),
        ]
    )
}

private func emptyRecurringStorageThiefRanking()
    -> CleanupHistoryStorageThiefRankingModel
{
    CleanupHistoryStorageThiefRankingModel(
        permanentSafeSessionCount: 0,
        manualCleanupSessionCount: 0,
        rankedRuleCount: 0,
        hasOlderPermanentSafeSessions: false,
        groups: []
    )
}

private final class HomeScanCleanupHistoryClearPreviewLease:
    DuxCleanupHistoryClearPreviewLease,
    @unchecked Sendable
{
    let preview = CleanupHistoryClearPreviewModel(
        sessionCount: 1,
        oldestStartedAt: Date(timeIntervalSince1970: 1),
        newestStartedAt: Date(timeIntervalSince1970: 2),
        preparedAt: Date(timeIntervalSince1970: 3),
        expiresAt: Date(timeIntervalSince1970: 60)
    )

    func release() async {}
}

private struct HomeScanVolumeMonitorStub: VolumeMonitoring {
    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot {
        VolumeCapacitySnapshot(
            stableVolumeID: nil,
            displayName: "Test",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 100,
            filesystemAvailableBytes: 50,
            importantAvailableBytes: 60,
            effectiveAvailableBytes: 60,
            availabilityBasis: .importantUsage,
            pressure: .healthy,
            criticalBoundaryBytes: 5,
            warningBoundaryBytes: 10,
            historyDisposition: .notStoredMissingStableIdentity,
            sampledAt: Date(timeIntervalSince1970: 1)
        )
    }
}

private func activePoll(
    revision: UInt64,
    progress: ScanProgressFacts?,
    cancellationRequested: Bool = false,
    events: [HomeScanEvent] = [],
    eventCursor: UInt64 = 0,
    oldestEventSequence: UInt64 = 0
) -> HomeScanTaskPoll {
    HomeScanTaskPoll(
        phase: .running,
        stage: .scanning,
        cancellationRequested: cancellationRequested,
        revision: revision,
        progress: progress,
        eventsTruncated: false,
        failure: nil,
        result: nil,
        events: events,
        eventCursor: eventCursor,
        oldestAvailableEventSequence: oldestEventSequence
    )
}

private func successPoll(
    revision: UInt64,
    result: HomeScanTaskResult,
    cancelled: Bool = false,
    events: [HomeScanEvent] = [],
    eventCursor: UInt64 = 0,
    oldestEventSequence: UInt64 = 0
) -> HomeScanTaskPoll {
    HomeScanTaskPoll(
        phase: .succeeded,
        stage: .terminal,
        cancellationRequested: cancelled,
        revision: revision,
        progress: nil,
        eventsTruncated: false,
        failure: nil,
        result: result,
        events: events,
        eventCursor: eventCursor,
        oldestAvailableEventSequence: oldestEventSequence
    )
}

private func cancelledPoll(revision: UInt64) -> HomeScanTaskPoll {
    HomeScanTaskPoll(
        phase: .cancelled,
        stage: .terminal,
        cancellationRequested: true,
        revision: revision,
        progress: nil,
        eventsTruncated: false,
        failure: nil,
        result: nil
    )
}

private func successfulResult(
    scanID: String = "scan:test",
    progress: ScanProgressFacts = ScanProgressFacts(
        files: 1,
        directories: 1,
        knownAllocatedBytes: 1,
        issueCount: 0
    )
) -> HomeScanTaskResult {
    HomeScanTaskResult(
        scanID: scanID,
        startedAt: Date(timeIntervalSince1970: 1),
        completedAt: Date(timeIntervalSince1970: 2),
        succeeded: true,
        directoryCount: progress.directories,
        fileCount: progress.files,
        logicalBytes: 2048,
        allocatedBytes: progress.knownAllocatedBytes,
        coverage: progress.issueCount == 0 ? .complete : .partial,
        coveragePermille: progress.issueCount == 0 ? 1000 : nil,
        issueCount: progress.issueCount,
        snapshotAvailable: true,
        candidateEvaluation: .succeeded(candidateCount: 0)
    )
}
