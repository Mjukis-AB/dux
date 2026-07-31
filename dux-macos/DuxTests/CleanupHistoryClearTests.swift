import Foundation
import XCTest
@testable import DUX

final class CleanupHistoryClearServiceTests: XCTestCase {
    func testValidPreviewMapsExactlyAndExplicitReleaseReleasesRawLease() async throws {
        let info = cleanupHistoryClearPreviewInfo(sessionCount: 3)
        let rawPreview = FakeCleanupHistoryClearPreviewSession(info: info)
        let engine = FakeCleanupHistoryClearEngine(
            preview: rawPreview,
            clearResult: CleanupHistoryClearResult(
                recordVersion: 1,
                clearedSessionCount: 3
            )
        )
        let service = EngineService(engine: engine)

        let lease = try await service.prepareCleanupHistoryClear()

        XCTAssertEqual(lease.preview.sessionCount, 3)
        XCTAssertEqual(
            lease.preview.oldestStartedAt,
            Date(timeIntervalSince1970: Double(info.oldestStartedAtUnixMs) / 1_000)
        )
        XCTAssertEqual(
            lease.preview.newestStartedAt,
            Date(timeIntervalSince1970: Double(info.newestStartedAtUnixMs) / 1_000)
        )
        XCTAssertEqual(
            lease.preview.preparedAt,
            Date(timeIntervalSince1970: Double(info.preparedAtUnixMs) / 1_000)
        )
        XCTAssertEqual(
            lease.preview.expiresAt,
            Date(timeIntervalSince1970: Double(info.expiresAtUnixMs) / 1_000)
        )

        await lease.release()
        XCTAssertEqual(rawPreview.releaseCount, 1)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testFutureSkewedNewestTimestampIsAcceptedAndMappedExactly() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let info = CleanupHistoryClearPreviewInfo(
            recordVersion: 1,
            sessionCount: 2,
            oldestStartedAtUnixMs: now - 10_000,
            newestStartedAtUnixMs: now + 30_000,
            preparedAtUnixMs: now - 1_000,
            expiresAtUnixMs: now + 60_000
        )
        XCTAssertGreaterThan(
            info.newestStartedAtUnixMs,
            info.preparedAtUnixMs
        )
        let rawPreview = FakeCleanupHistoryClearPreviewSession(info: info)
        let service = EngineService(
            engine: FakeCleanupHistoryClearEngine(
                preview: rawPreview,
                clearResult: CleanupHistoryClearResult(
                    recordVersion: 1,
                    clearedSessionCount: 2
                )
            )
        )

        let lease = try await service.prepareCleanupHistoryClear()

        XCTAssertEqual(
            lease.preview.newestStartedAt,
            Date(timeIntervalSince1970: Double(info.newestStartedAtUnixMs) / 1_000)
        )
        XCTAssertGreaterThan(
            lease.preview.newestStartedAt,
            lease.preview.preparedAt
        )
        await lease.release()
        XCTAssertEqual(rawPreview.releaseCount, 1)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testWrongRecordVersionPreviewIsRejectedAndReleased() async {
        let malformed = cleanupHistoryClearPreviewInfo(
            recordVersion: 2,
            sessionCount: 3
        )
        let rawPreview = FakeCleanupHistoryClearPreviewSession(info: malformed)
        let service = EngineService(
            engine: FakeCleanupHistoryClearEngine(
                preview: rawPreview,
                clearResult: CleanupHistoryClearResult(
                    recordVersion: 1,
                    clearedSessionCount: 3
                )
            )
        )

        do {
            _ = try await service.prepareCleanupHistoryClear()
            XCTFail("Expected malformed preview rejection")
        } catch {
            XCTAssertEqual(
                error as? CleanupHistoryClearServiceError,
                .invalidResponse
            )
        }

        XCTAssertEqual(rawPreview.releaseCount, 1)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }

    func testForeignServiceRejectsWithoutConsumptionThenOwnerCanClear() async throws {
        let ownerPreview = FakeCleanupHistoryClearPreviewSession(
            info: cleanupHistoryClearPreviewInfo(sessionCount: 2)
        )
        let ownerEngine = FakeCleanupHistoryClearEngine(
            preview: ownerPreview,
            clearResult: CleanupHistoryClearResult(
                recordVersion: 1,
                clearedSessionCount: 2
            )
        )
        let foreignEngine = FakeCleanupHistoryClearEngine(
            preview: FakeCleanupHistoryClearPreviewSession(
                info: cleanupHistoryClearPreviewInfo(sessionCount: 1)
            ),
            clearResult: CleanupHistoryClearResult(
                recordVersion: 1,
                clearedSessionCount: 1
            )
        )
        let owner = EngineService(engine: ownerEngine)
        let foreign = EngineService(engine: foreignEngine)
        let lease = try await owner.prepareCleanupHistoryClear()

        do {
            _ = try await foreign.clearCleanupHistory(lease)
            XCTFail("Expected foreign-engine rejection")
        } catch {
            XCTAssertEqual(
                error as? CleanupHistoryClearServiceError,
                .wrongEngine
            )
        }

        XCTAssertEqual(foreignEngine.clearCount, 0)
        let result = try await owner.clearCleanupHistory(lease)
        XCTAssertEqual(result, CleanupHistoryClearResultModel(clearedSessionCount: 2))
        XCTAssertEqual(ownerEngine.clearCount, 1)
        let ownerClosed = await owner.close()
        let foreignClosed = await foreign.close()
        XCTAssertTrue(ownerClosed)
        XCTAssertTrue(foreignClosed)
    }

    func testMalformedPostMutationResultIsOutcomeUnknownAndLeaseStaysConsumed() async throws {
        let rawPreview = FakeCleanupHistoryClearPreviewSession(
            info: cleanupHistoryClearPreviewInfo(sessionCount: 4)
        )
        let engine = FakeCleanupHistoryClearEngine(
            preview: rawPreview,
            clearResult: CleanupHistoryClearResult(
                recordVersion: 2,
                clearedSessionCount: 4
            )
        )
        let service = EngineService(engine: engine)
        let lease = try await service.prepareCleanupHistoryClear()

        do {
            _ = try await service.clearCleanupHistory(lease)
            XCTFail("Expected uncertain post-mutation response")
        } catch {
            XCTAssertEqual(
                error as? CleanupHistoryClearServiceError,
                .outcomeUnknown
            )
        }
        do {
            _ = try await service.clearCleanupHistory(lease)
            XCTFail("Expected consume-once preview rejection")
        } catch {
            XCTAssertEqual(
                error as? CleanupHistoryClearServiceError,
                .previewUnavailable
            )
        }

        XCTAssertEqual(engine.clearCount, 1)
        XCTAssertEqual(rawPreview.releaseCount, 0)
        let closed = await service.close()
        XCTAssertTrue(closed)
    }
}

@MainActor
final class CleanupHistoryClearAppModelTests: XCTestCase {
    func testStaleConfirmationCannotClearOrCancelCurrentPreview() async {
        let service = CleanupHistoryClearEngineSpy()
        let model = AppModel(engineService: service)
        await model.prepareCleanupHistoryClear()
        guard let confirmation = model.cleanupHistoryClearConfirmation else {
            return XCTFail("Expected exact cleanup-history confirmation")
        }
        let stale = CleanupHistoryClearConfirmation(
            generation: confirmation.generation &+ 1,
            preview: confirmation.preview
        )

        await model.confirmCleanupHistoryClear(stale)
        await model.cancelCleanupHistoryClear(stale)

        XCTAssertEqual(
            model.cleanupHistoryClearState,
            .awaitingConfirmation(confirmation)
        )
        var clearCount = await service.clearRequestCount()
        var releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(clearCount, 0)
        XCTAssertEqual(releaseCount, 0)

        await model.cancelCleanupHistoryClear(confirmation)

        XCTAssertEqual(model.cleanupHistoryClearState, .idle)
        XCTAssertNil(model.cleanupHistoryClearConfirmation)
        clearCount = await service.clearRequestCount()
        releaseCount = await service.previewReleaseCount()
        XCTAssertEqual(clearCount, 0)
        XCTAssertEqual(releaseCount, 1)
    }

    func testTerminalQuiescenceJoinsCancelAfterPreviewSlotIsCleared() async {
        let service = CleanupHistoryClearEngineSpy()
        let model = AppModel(engineService: service)
        await model.prepareCleanupHistoryClear()
        guard let confirmation = model.cleanupHistoryClearConfirmation else {
            return XCTFail("Expected exact cleanup-history confirmation")
        }
        await service.suspendNextPreviewRelease()

        let cancellation = Task { @MainActor in
            await model.cancelCleanupHistoryClear(confirmation)
        }
        await service.waitForPreviewRelease()
        XCTAssertNil(model.cleanupHistoryClearConfirmation)

        let completion = CleanupHistoryTerminalCompletionProbe()
        let terminal = Task { @MainActor in
            await model.quiesceForTerminalRuntime()
            await completion.finish()
        }
        await Task.yield()
        let completedBeforeRelease = await completion.count()
        XCTAssertEqual(completedBeforeRelease, 0)

        await service.completePreviewRelease()
        await cancellation.value
        await terminal.value
        let completedAfterRelease = await completion.count()
        XCTAssertEqual(completedAfterRelease, 1)
    }

    func testConfirmedClearBlocksConcurrentHistoryReadsAndRefreshesExactlyOnce() async {
        let terminalRecord = cleanupHistoryClearSummary(sessionID: "session:terminal")
        let service = CleanupHistoryClearEngineSpy(
            historyPage: CleanupHistoryPageModel(
                records: [terminalRecord],
                nextCursor: nil
            ),
            suspendClear: true
        )
        let model = AppModel(engineService: service)
        await model.prepareCleanupHistoryClear()
        guard let confirmation = model.cleanupHistoryClearConfirmation else {
            return XCTFail("Expected exact cleanup-history confirmation")
        }

        let clearing = Task { @MainActor in
            await model.confirmCleanupHistoryClear(confirmation)
        }
        await service.waitForClearRequest()
        XCTAssertTrue(model.cleanupHistoryClearState.isClearing)

        await model.loadCleanupHistory()
        await model.refreshCleanupHistory()
        await model.loadMoreCleanupHistory()

        var historyCount = await service.historyRequestCount()
        XCTAssertEqual(historyCount, 0)
        await service.completeSuspendedClear()
        await clearing.value

        let prepareCount = await service.prepareRequestCount()
        let clearCount = await service.clearRequestCount()
        historyCount = await service.historyRequestCount()
        XCTAssertEqual(prepareCount, 1)
        XCTAssertEqual(clearCount, 1)
        XCTAssertEqual(historyCount, 1)
        XCTAssertEqual(model.cleanupHistoryRecords, [terminalRecord])
        XCTAssertEqual(model.cleanupHistoryState, .loaded)
        XCTAssertEqual(
            model.cleanupHistoryClearState,
            .completed(CleanupHistoryClearResultModel(clearedSessionCount: 3))
        )
    }

    func testChangedAndOutcomeUnknownNeverRetryMutation() async {
        let cases: [
            (CleanupHistoryClearServiceError, CleanupHistoryClearState)
        ] = [
            (.changedSincePreview, .failed(.changedSincePreview)),
            (.outcomeUnknown, .outcomeUnknown),
        ]

        for (failure, expectedState) in cases {
            let service = CleanupHistoryClearEngineSpy(clearFailure: failure)
            let model = AppModel(engineService: service)
            await model.prepareCleanupHistoryClear()
            guard let confirmation = model.cleanupHistoryClearConfirmation else {
                XCTFail("Expected exact cleanup-history confirmation")
                continue
            }

            await model.confirmCleanupHistoryClear(confirmation)
            await model.confirmCleanupHistoryClear(confirmation)

            XCTAssertEqual(model.cleanupHistoryClearState, expectedState)
            let prepareCount = await service.prepareRequestCount()
            let clearCount = await service.clearRequestCount()
            let historyCount = await service.historyRequestCount()
            let releaseCount = await service.previewReleaseCount()
            XCTAssertEqual(prepareCount, 1)
            XCTAssertEqual(clearCount, 1)
            XCTAssertEqual(historyCount, 1)
            XCTAssertEqual(releaseCount, 1)
        }
    }

    func testRuntimeShutdownWaitsThroughTerminalRefreshBeforeClosingEngine() async {
        let service = CleanupHistoryClearEngineSpy(
            suspendClear: true,
            suspendHistory: true
        )
        let model = AppModel(engineService: service)
        let maintenance = CleanupHistoryRuntimeMaintenanceSpy()
        let capacity = CleanupHistoryRuntimeCapacitySpy()
        let reviews = CleanupHistoryRuntimeReviewSpy()
        let scans = CleanupHistoryRuntimeScanSpy()
        let runtime = AppRuntime(
            model: model,
            engineService: service,
            scheduler: maintenance,
            capacityScheduler: capacity,
            reviews: reviews,
            scans: scans
        )
        await model.prepareCleanupHistoryClear()
        guard let confirmation = model.cleanupHistoryClearConfirmation else {
            return XCTFail("Expected exact cleanup-history confirmation")
        }
        let clearing = Task { @MainActor in
            await model.confirmCleanupHistoryClear(confirmation)
        }
        await service.waitForClearRequest()

        let firstShutdown = Task { @MainActor in
            await runtime.shutdown()
        }
        await service.completeSuspendedClear()
        await service.waitForHistoryRequest()

        var closeCount = await service.engineCloseCount()
        var clearCount = await service.clearRequestCount()
        var historyCount = await service.historyRequestCount()
        XCTAssertEqual(closeCount, 0)
        XCTAssertEqual(clearCount, 1)
        XCTAssertEqual(historyCount, 1)

        let secondShutdown = Task { @MainActor in
            await runtime.shutdown()
        }
        await service.completeSuspendedHistory()
        await clearing.value
        await firstShutdown.value
        await secondShutdown.value

        clearCount = await service.clearRequestCount()
        historyCount = await service.historyRequestCount()
        closeCount = await service.engineCloseCount()
        let maintenanceStops = await maintenance.stopCount()
        let capacityStops = await capacity.stopCount()
        let reviewShutdowns = await reviews.shutdownCount()
        XCTAssertEqual(clearCount, 1)
        XCTAssertEqual(historyCount, 1)
        XCTAssertEqual(closeCount, 1)
        XCTAssertEqual(maintenanceStops, 1)
        XCTAssertEqual(capacityStops, 1)
        XCTAssertEqual(reviewShutdowns, 1)
        XCTAssertEqual(scans.shutdownCount, 1)
        XCTAssertEqual(model.cleanupHistoryClearState, .idle)
    }
}

@MainActor
final class CleanupHistoryClearSettingsTests: XCTestCase {
    func testAccessibilityIdentifiersAreNonemptyAndUnique() {
        let identifiers = CleanupHistoryClearAccessibility.allControlIdentifiers

        XCTAssertFalse(identifiers.contains(where: \.isEmpty))
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
    }

    func testEveryFailureHasAUserFacingMessageAndUncertainCopyIsHonest() {
        let failures: [CleanupHistoryClearServiceError] = [
            .closed,
            .nothingToClear,
            .activeCleanup,
            .changedSincePreview,
            .previewExpired,
            .wrongEngine,
            .previewUnavailable,
            .incompatibleSchema,
            .retryable,
            .unsafeStorage,
            .budgetExceeded,
            .corruptData,
            .outcomeUnknown,
            .unavailable,
            .internalState,
            .invalidResponse,
        ]

        for failure in failures {
            XCTAssertFalse(
                DuxSettingsView.message(for: failure)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                    .isEmpty
            )
        }

        let uncertain = DuxSettingsView.cleanupHistoryClearOutcomeUnknownMessage
            .lowercased()
        XCTAssertTrue(uncertain.contains("attempted one read-only history refresh"))
        XCTAssertTrue(uncertain.contains("retried nothing"))
        XCTAssertTrue(uncertain.contains("no file cleanup"))
    }
}

private final class FakeCleanupHistoryClearPreviewSession:
    CleanupHistoryClearPreviewSession, @unchecked Sendable
{
    private let returnedInfo: CleanupHistoryClearPreviewInfo
    private let lock = NSLock()
    private var releases = 0

    var releaseCount: Int {
        lock.withLock { releases }
    }

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("Unsupported test initializer: \(handle)")
    }

    init(info: CleanupHistoryClearPreviewInfo) {
        returnedInfo = info
        super.init(noHandle: NoHandle())
    }

    override func info() throws -> CleanupHistoryClearPreviewInfo {
        returnedInfo
    }

    override func release() throws -> CleanupHistoryClearPreviewReleaseOutcome {
        lock.withLock {
            releases += 1
            return releases == 1 ? .released : .alreadyUnavailable
        }
    }
}

private final class FakeCleanupHistoryClearEngine: DuxEngine, @unchecked Sendable {
    private let returnedPreview: CleanupHistoryClearPreviewSession
    private let returnedClearResult: CleanupHistoryClearResult
    private let lock = NSLock()
    private var clears = 0

    var clearCount: Int {
        lock.withLock { clears }
    }

    required init(unsafeFromHandle handle: UInt64) {
        fatalError("Unsupported test initializer: \(handle)")
    }

    init(
        preview: CleanupHistoryClearPreviewSession,
        clearResult: CleanupHistoryClearResult
    ) {
        returnedPreview = preview
        returnedClearResult = clearResult
        super.init(noHandle: NoHandle())
    }

    override func prepareCleanupHistoryClear() throws
        -> CleanupHistoryClearPreviewSession
    {
        returnedPreview
    }

    override func clearCleanupHistory(
        preview _: CleanupHistoryClearPreviewSession
    ) throws -> CleanupHistoryClearResult {
        lock.withLock {
            clears += 1
        }
        return returnedClearResult
    }

    override func close() -> Bool {
        true
    }
}

private actor CleanupHistoryClearEngineSpy: EngineServing, DuxEngineClosing {
    private let tracker = CleanupHistoryClearReleaseTracker()
    private let preview = cleanupHistoryClearPreviewModel(sessionCount: 3)
    private let historyPage: CleanupHistoryPageModel
    private let clearFailure: CleanupHistoryClearServiceError?
    private var shouldSuspendClear: Bool
    private var shouldSuspendHistory: Bool
    private var prepareCount = 0
    private var clearCount = 0
    private var historyCount = 0
    private var closeCount = 0
    private var clearWaiters: [CheckedContinuation<Void, Never>] = []
    private var historyWaiters: [CheckedContinuation<Void, Never>] = []
    private var suspendedClear: CheckedContinuation<Void, Never>?
    private var suspendedHistory: CheckedContinuation<Void, Never>?

    init(
        historyPage: CleanupHistoryPageModel = CleanupHistoryPageModel(
            records: [],
            nextCursor: nil
        ),
        clearFailure: CleanupHistoryClearServiceError? = nil,
        suspendClear: Bool = false,
        suspendHistory: Bool = false
    ) {
        self.historyPage = historyPage
        self.clearFailure = clearFailure
        shouldSuspendClear = suspendClear
        shouldSuspendHistory = suspendHistory
    }

    func loadStatus() async throws -> EngineStatus {
        EngineStatus(
            libraryVersion: "test",
            ffiContractVersion: 32,
            executedOffMainThread: true
        )
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

    func prepareCleanupHistoryClear() async throws
        -> any DuxCleanupHistoryClearPreviewLease
    {
        prepareCount += 1
        return CleanupHistoryClearTestLease(preview: preview, tracker: tracker)
    }

    func clearCleanupHistory(
        _: any DuxCleanupHistoryClearPreviewLease
    ) async throws -> CleanupHistoryClearResultModel {
        clearCount += 1
        clearWaiters.forEach { $0.resume() }
        clearWaiters.removeAll()
        if shouldSuspendClear {
            shouldSuspendClear = false
            await withCheckedContinuation { continuation in
                suspendedClear = continuation
            }
        }
        if let clearFailure {
            throw clearFailure
        }
        return CleanupHistoryClearResultModel(
            clearedSessionCount: preview.sessionCount
        )
    }

    func loadRecentCleanupHistory(
        cursor _: CleanupHistoryCursorModel?,
        limit _: UInt16
    ) async throws -> CleanupHistoryPageModel {
        historyCount += 1
        historyWaiters.forEach { $0.resume() }
        historyWaiters.removeAll()
        if shouldSuspendHistory {
            shouldSuspendHistory = false
            await withCheckedContinuation { continuation in
                suspendedHistory = continuation
            }
        }
        return historyPage
    }

    func close() async -> Bool {
        closeCount += 1
        return true
    }

    func prepareRequestCount() -> Int { prepareCount }
    func clearRequestCount() -> Int { clearCount }
    func historyRequestCount() -> Int { historyCount }
    func engineCloseCount() -> Int { closeCount }
    func previewReleaseCount() async -> Int { await tracker.count() }

    func suspendNextPreviewRelease() async {
        await tracker.suspendNextRelease()
    }

    func waitForPreviewRelease() async {
        await tracker.waitForRelease()
    }

    func completePreviewRelease() async {
        await tracker.completeRelease()
    }

    func waitForClearRequest() async {
        guard clearCount == 0 else {
            return
        }
        await withCheckedContinuation { continuation in
            clearWaiters.append(continuation)
        }
    }

    func waitForHistoryRequest() async {
        guard historyCount == 0 else {
            return
        }
        await withCheckedContinuation { continuation in
            historyWaiters.append(continuation)
        }
    }

    func completeSuspendedClear() {
        suspendedClear?.resume()
        suspendedClear = nil
    }

    func completeSuspendedHistory() {
        suspendedHistory?.resume()
        suspendedHistory = nil
    }
}

private final class CleanupHistoryClearTestLease:
    DuxCleanupHistoryClearPreviewLease, @unchecked Sendable
{
    let preview: CleanupHistoryClearPreviewModel
    private let tracker: CleanupHistoryClearReleaseTracker

    init(
        preview: CleanupHistoryClearPreviewModel,
        tracker: CleanupHistoryClearReleaseTracker
    ) {
        self.preview = preview
        self.tracker = tracker
    }

    func release() async {
        await tracker.recordRelease()
    }
}

private actor CleanupHistoryClearReleaseTracker {
    private var releases = 0
    private var shouldSuspendRelease = false
    private var releaseContinuation: CheckedContinuation<Void, Never>?
    private var releaseWaiters: [CheckedContinuation<Void, Never>] = []

    func recordRelease() async {
        releases += 1
        let waiters = releaseWaiters
        releaseWaiters.removeAll()
        for waiter in waiters {
            waiter.resume()
        }
        guard shouldSuspendRelease else {
            return
        }
        shouldSuspendRelease = false
        await withCheckedContinuation { continuation in
            releaseContinuation = continuation
        }
    }

    func count() -> Int {
        releases
    }

    func suspendNextRelease() {
        shouldSuspendRelease = true
    }

    func waitForRelease() async {
        guard releases == 0 else {
            return
        }
        await withCheckedContinuation { continuation in
            releaseWaiters.append(continuation)
        }
    }

    func completeRelease() {
        releaseContinuation?.resume()
        releaseContinuation = nil
    }
}

private actor CleanupHistoryTerminalCompletionProbe {
    private var completions = 0

    func finish() {
        completions += 1
    }

    func count() -> Int {
        completions
    }
}

private actor CleanupHistoryRuntimeMaintenanceSpy: DuxMaintenanceScheduling {
    private var stops = 0

    func start() async {}
    func signal(_: DuxMaintenanceTrigger) async {}
    func stop() async { stops += 1 }
    func quiesceForTerminalRuntime() async { await stop() }
    func stopCount() -> Int { stops }
}

private actor CleanupHistoryRuntimeCapacitySpy: DuxCapacityScheduling {
    private var stops = 0

    func start() async {}
    func signal(_: DuxCapacitySamplingTrigger) async {}
    func stop() async { stops += 1 }
    func quiesceForTerminalRuntime() async { await stop() }
    func stopCount() -> Int { stops }
}

private actor CleanupHistoryRuntimeReviewSpy: DuxReviewManaging {
    private var shutdowns = 0

    func renewNow() async {}
    func shutdown() async { shutdowns += 1 }
    func shutdownCount() -> Int { shutdowns }
}

@MainActor
private final class CleanupHistoryRuntimeScanSpy: DuxScanManaging {
    private(set) var shutdownCount = 0

    func shutdownTargetedReclaimScan() async {}

    func shutdownHomeScan() async {
        shutdownCount += 1
    }
}

private func cleanupHistoryClearPreviewInfo(
    recordVersion: UInt32 = 1,
    sessionCount: UInt64
) -> CleanupHistoryClearPreviewInfo {
    let now = Int64(Date().timeIntervalSince1970 * 1_000)
    return CleanupHistoryClearPreviewInfo(
        recordVersion: recordVersion,
        sessionCount: sessionCount,
        oldestStartedAtUnixMs: now - 10_000,
        newestStartedAtUnixMs: now - 5_000,
        preparedAtUnixMs: now - 1_000,
        expiresAtUnixMs: now + 60_000
    )
}

private func cleanupHistoryClearPreviewModel(
    sessionCount: UInt64
) -> CleanupHistoryClearPreviewModel {
    CleanupHistoryClearPreviewModel(
        sessionCount: sessionCount,
        oldestStartedAt: Date(timeIntervalSince1970: 1),
        newestStartedAt: Date(timeIntervalSince1970: 2),
        preparedAt: Date(timeIntervalSince1970: 3),
        expiresAt: Date(timeIntervalSince1970: 63)
    )
}

private func cleanupHistoryClearSummary(
    sessionID: String
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
        format: .complete,
        sourceScanID: "scan:\(sessionID)",
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
}
