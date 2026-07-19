@testable import DUX
import Foundation
import XCTest

final class ExplorerPresentationTests: XCTestCase {
    private let en = Locale(identifier: "en_US")
    private let now = Date(timeIntervalSince1970: 3_600)

    func testEveryUncachedVolumeStateAvoidsInventedCapacity() {
        for state in [VolumeCapacityState.idle, .loading] {
            guard case let .loading(message) = make(volumeState: state).capacity else {
                return XCTFail("Expected loading presentation")
            }
            XCTAssertEqual(message, "Checking startup disk…")
        }

        for failure in [
            VolumeCapacityFailure.unavailable,
            .invalidObservation,
            .engineUnavailable,
        ] {
            let presentation = make(volumeState: .failed(failure))
            guard case let .failed(title, detail) = presentation.capacity else {
                return XCTFail("Expected failed presentation")
            }
            XCTAssertEqual(title, "Storage capacity unavailable")
            XCTAssertFalse(detail.contains("0 GiB"))
            XCTAssertTrue(presentation.actions.refreshCapacityEnabled)
        }
    }

    func testLoadedFilesystemCapacityShowsExactUsedAvailableAndTotalBreakdown() throws {
        let snapshot = makeSnapshot(
            basis: .filesystemAvailable,
            filesystemAvailableBytes: 25 * gib,
            importantAvailableBytes: nil
        )
        let capacity = try capacitySnapshot(from: make(volumeState: .loaded(snapshot)))

        XCTAssertEqual(capacity.volumeName, "Macintosh HD")
        XCTAssertEqual(capacity.availableValue, "25 GiB")
        XCTAssertEqual(capacity.usedValue, "75 GiB")
        XCTAssertEqual(capacity.totalValue, "100 GiB")
        XCTAssertEqual(capacity.availabilityBasis, "Filesystem available")
        guard case let .known(usedFraction) = capacity.breakdown else {
            return XCTFail("Expected a known filesystem breakdown")
        }
        XCTAssertEqual(usedFraction, 0.75, accuracy: 0.000_001)
        XCTAssertTrue(capacity.accessibilitySummary.contains("75 GiB used"))
    }

    func testImportantOnlyCapacityKeepsUsedBreakdownExplicitlyUnavailable() throws {
        let snapshot = makeSnapshot(
            basis: .importantUsage,
            filesystemAvailableBytes: nil,
            importantAvailableBytes: 20 * gib
        )
        let capacity = try capacitySnapshot(from: make(volumeState: .loaded(snapshot)))

        XCTAssertEqual(capacity.availableValue, "20 GiB")
        XCTAssertEqual(capacity.usedValue, "Unavailable")
        XCTAssertEqual(capacity.availabilityBasis, "Available for important use")
        guard case let .unavailable(message) = capacity.breakdown else {
            return XCTFail("Expected unavailable used-space breakdown")
        }
        XCTAssertTrue(message.contains("unavailable"))
        XCTAssertFalse(capacity.accessibilitySummary.contains("0 GiB used"))
    }

    func testRefreshingAndEveryStaleReasonRetainLastConfirmedCapacity() throws {
        let snapshot = makeSnapshot()
        let loaded = try capacitySnapshot(from: make(volumeState: .loaded(snapshot)))

        let refreshingPresentation = make(volumeState: .refreshing(snapshot))
        let refreshing = try capacitySnapshot(from: refreshingPresentation)
        XCTAssertEqual(refreshing, loaded)
        guard case let .snapshot(_, status) = refreshingPresentation.capacity,
              case let .refreshing(message) = status
        else {
            return XCTFail("Expected refreshing status")
        }
        XCTAssertEqual(message, "Updating capacity…")
        XCTAssertFalse(refreshingPresentation.actions.refreshCapacityEnabled)

        for failure in [
            VolumeCapacityFailure.unavailable,
            .invalidObservation,
            .engineUnavailable,
        ] {
            let presentation = make(volumeState: .stale(snapshot, failure))
            XCTAssertEqual(try capacitySnapshot(from: presentation), loaded)
            guard case let .snapshot(_, status) = presentation.capacity,
                  case let .stale(message) = status
            else {
                return XCTFail("Expected stale status")
            }
            XCTAssertTrue(message.contains("last confirmed sample"))
            XCTAssertTrue(presentation.actions.refreshCapacityEnabled)
        }
    }

    func testNoSessionSummaryDoesNotDenyDurableHistory() {
        let presentation = make(scanState: .idle)

        XCTAssertEqual(presentation.coverage.coverage, .unknown)
        XCTAssertEqual(presentation.coverage.title, "No Home scan loaded")
        XCTAssertTrue(presentation.coverage.detail.contains("in this window"))
        XCTAssertFalse(presentation.coverage.detail.contains("never"))
        XCTAssertNil(presentation.scan)
    }

    func testEveryCoverageClassificationIsScopedToHome() {
        let cases: [(AppScanCoverage, String)] = [
            (.complete, "Complete Home coverage"),
            (.limitedAccess, "Limited Home access"),
            (.partial, "Partial Home coverage"),
            (.unknown, "Home coverage unavailable"),
        ]

        for (coverage, expectedTitle) in cases {
            let summary = makeSummary(coverage: coverage, coveragePermille: nil)
            let presentation = make(scanState: AppScanState(
                phase: .succeeded(summary),
                lastSuccessful: summary
            ))
            XCTAssertEqual(presentation.coverage.coverage, coverage)
            XCTAssertEqual(presentation.coverage.title, expectedTitle)
            XCTAssertTrue(presentation.coverage.detail.contains("Home"))
            XCTAssertTrue(
                presentation.coverage.detail.contains("12 access issues"),
                presentation.coverage.detail
            )
            XCTAssertFalse(presentation.coverage.detail.contains("reported as"))
        }
    }

    func testCoveragePermilleAppearsOnlyWhenReported() {
        let reported = makeSummary(coverage: .partial, coveragePermille: 975)
        let withCoverage = make(scanState: AppScanState(
            phase: .succeeded(reported),
            lastSuccessful: reported
        ))
        XCTAssertTrue(withCoverage.coverage.detail.contains("97.5%"))

        let omitted = makeSummary(coverage: .partial, coveragePermille: nil)
        let withoutCoverage = make(scanState: AppScanState(
            phase: .succeeded(omitted),
            lastSuccessful: omitted
        ))
        XCTAssertFalse(withoutCoverage.coverage.detail.contains("%"))
    }

    func testLastConfirmedCoverageStaysVisibleAcrossActiveCancelledAndFailedPhases() {
        let previous = makeSummary(coverage: .limitedAccess, coveragePermille: 800)
        let phases: [AppScanPhase] = [
            .queued,
            .scanning(makeFacts()),
            .finalizing(makeFacts()),
            .evaluating(makeFacts()),
            .cancellationRequested(makeFacts()),
            .cancelled,
            .failed(.storageUnavailable),
        ]

        for phase in phases {
            let presentation = make(scanState: AppScanState(
                phase: phase,
                lastSuccessful: previous
            ))
            XCTAssertEqual(presentation.coverage.coverage, .limitedAccess)
            XCTAssertEqual(presentation.coverage.title, "Limited Home access")
            XCTAssertTrue(presentation.coverage.detail.contains("80%"))
        }
    }

    func testSubtreeScanUsesFolderCopyWithoutReplacingHomeCoverage() throws {
        let home = makeSummary(coverage: .limitedAccess, coveragePermille: 800)
        let subtree = makeSummary(coverage: .complete, coveragePermille: 1_000)
        let state = AppScanState(
            phase: .succeeded(subtree),
            lastSuccessful: home,
            scope: .subtree(displayName: "Caches")
        )

        let presentation = make(scanState: state)

        XCTAssertEqual(presentation.coverage.coverage, .limitedAccess)
        XCTAssertEqual(presentation.coverage.title, "Limited Home access")
        let scan = try XCTUnwrap(presentation.scan)
        XCTAssertEqual(scan.title, "Refreshed snapshot ready for “Caches”")
        XCTAssertTrue(scan.detail?.contains("selected folder") == true)
        XCTAssertFalse(scan.detail?.contains("allocated in Home") == true)
    }

    func testEveryActiveScanPhaseShowsOnlyCancelAction() throws {
        let phases: [AppScanPhase] = [
            .queued,
            .scanning(nil),
            .finalizing(makeFacts()),
            .evaluating(makeFacts()),
        ]

        for phase in phases {
            let presentation = make(scanState: AppScanState(
                phase: phase,
                lastSuccessful: makeSummary()
            ))
            XCTAssertFalse(presentation.actions.showScanNow)
            XCTAssertFalse(presentation.actions.scanNowEnabled)
            XCTAssertTrue(presentation.actions.showCancelScan)
            XCTAssertTrue(presentation.actions.cancelScanEnabled)
            XCTAssertEqual(try XCTUnwrap(presentation.scan).style, .progress)
        }
    }

    func testCancellationRequestedKeepsActionsExclusiveAndDisablesRepeat() throws {
        let presentation = make(scanState: AppScanState(
            phase: .cancellationRequested(makeFacts()),
            lastSuccessful: makeSummary()
        ))

        XCTAssertFalse(presentation.actions.showScanNow)
        XCTAssertTrue(presentation.actions.showCancelScan)
        XCTAssertFalse(presentation.actions.cancelScanEnabled)
        XCTAssertEqual(try XCTUnwrap(presentation.scan).title, "Stopping Home scan…")
    }

    func testEveryInactiveScanPhaseShowsOnlyScanAction() {
        let summary = makeSummary()
        let phases: [AppScanPhase] = [
            .idle,
            .succeeded(summary),
            .cancelled,
            .failed(.scanFailed),
        ]

        for phase in phases {
            let presentation = make(scanState: AppScanState(
                phase: phase,
                lastSuccessful: summary
            ))
            XCTAssertTrue(presentation.actions.showScanNow)
            XCTAssertTrue(presentation.actions.scanNowEnabled)
            XCTAssertFalse(presentation.actions.showCancelScan)
            XCTAssertFalse(presentation.actions.cancelScanEnabled)
        }
    }

    func testHomeAllocationCopyCannotBeMistakenForStartupVolumeCapacity() throws {
        let facts = makeFacts()
        let presentation = make(scanState: AppScanState(
            phase: .scanning(facts),
            lastSuccessful: nil
        ))
        let detail = try XCTUnwrap(presentation.scan?.detail)

        XCTAssertTrue(detail.contains("18.4 GiB allocated in Home"), detail)
        XCTAssertFalse(detail.contains("available"))

        let unavailable = make(scanState: AppScanState(
            phase: .scanning(makeFacts(knownAllocatedBytes: nil)),
            lastSuccessful: nil
        ))
        XCTAssertTrue(unavailable.scan?.detail?.contains("Home allocation unavailable") == true)
        XCTAssertFalse(unavailable.scan?.detail?.contains("0 GiB") == true)
    }

    func testTerminalScanCopyRetainsOnlyConfirmedAggregate() throws {
        let previous = makeSummary()
        for phase in [
            AppScanPhase.cancelled,
            .failed(.taskExpired),
            .failed(.outcomeUnknown),
        ] {
            let presentation = make(scanState: AppScanState(
                phase: phase,
                lastSuccessful: previous
            ))
            let detail = try XCTUnwrap(presentation.scan?.detail)
            XCTAssertTrue(detail.contains("Last confirmed Home scan aggregate"))
            XCTAssertFalse(detail.contains("unchanged"))
        }
    }

    func testAccessibilityAndShortcutContractsAreStableAndUnique() {
        XCTAssertEqual(
            ExplorerDestination.allCases,
            [.overview, .snapshot, .recommendations, .cleanupHistory]
        )
        XCTAssertEqual(ExplorerAccessibility.root, "explorer")
        XCTAssertEqual(
            ExplorerAccessibility.snapshotSubtreeRescan,
            "explorer-snapshot-subtree-rescan"
        )
        XCTAssertEqual(
            ExplorerAccessibility.snapshotSubtreeScanStatus,
            "explorer-snapshot-subtree-scan-status"
        )
        XCTAssertEqual(
            Set(ExplorerAccessibility.allIdentifiers).count,
            ExplorerAccessibility.allIdentifiers.count
        )
        XCTAssertEqual(ExplorerKeyboardShortcut.scanNow, "r")
        XCTAssertEqual(ExplorerKeyboardShortcut.cancelScan, ".")
        XCTAssertEqual(ExplorerKeyboardShortcut.settings, ",")
        XCTAssertEqual(
            Set(ExplorerKeyboardShortcut.allKeys).count,
            ExplorerKeyboardShortcut.allKeys.count
        )
    }

    func testCriticalAccessibilitySummaryLeadsWithAvailableCapacity() throws {
        let snapshot = VolumeCapacitySnapshot(
            stableVolumeID: "startup",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 100 * gib,
            filesystemAvailableBytes: 4 * gib,
            importantAvailableBytes: 3 * gib,
            effectiveAvailableBytes: 3 * gib,
            availabilityBasis: .importantUsage,
            pressure: .critical,
            criticalBoundaryBytes: 5 * gib,
            warningBoundaryBytes: 10 * gib,
            historyDisposition: .stored,
            sampledAt: now
        )
        let capacity = try capacitySnapshot(from: make(volumeState: .loaded(snapshot)))

        XCTAssertTrue(capacity.accessibilitySummary.hasPrefix("3 GiB available"))
    }

    private func make(
        volumeState: VolumeCapacityState? = nil,
        scanState: AppScanState = .idle
    ) -> ExplorerPresentation {
        ExplorerPresentation.make(
            volumeState: volumeState ?? .loaded(makeSnapshot()),
            scanState: scanState,
            now: now,
            locale: en
        )
    }

    private func capacitySnapshot(
        from presentation: ExplorerPresentation
    ) throws -> ExplorerCapacitySnapshotPresentation {
        guard case let .snapshot(snapshot, _) = presentation.capacity else {
            throw ExplorerPresentationTestError.notSnapshot
        }
        return snapshot
    }

    private var gib: UInt64 { 1 << 30 }

    private func makeSnapshot(
        basis: VolumeCapacityBasis = .importantUsage,
        filesystemAvailableBytes: UInt64? = 25 * (1 << 30),
        importantAvailableBytes: UInt64? = 20 * (1 << 30)
    ) -> VolumeCapacitySnapshot {
        let effective = basis == .importantUsage
            ? try! XCTUnwrap(importantAvailableBytes)
            : try! XCTUnwrap(filesystemAvailableBytes)
        return VolumeCapacitySnapshot(
            stableVolumeID: "startup",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 100 * gib,
            filesystemAvailableBytes: filesystemAvailableBytes,
            importantAvailableBytes: importantAvailableBytes,
            effectiveAvailableBytes: effective,
            availabilityBasis: basis,
            pressure: .warning,
            criticalBoundaryBytes: 5 * gib,
            warningBoundaryBytes: 10 * gib,
            historyDisposition: .stored,
            sampledAt: now
        )
    }

    private func makeFacts(
        knownAllocatedBytes: UInt64? = 18 * (1 << 30) + (4 * (1 << 30) + 9) / 10
    ) -> ScanProgressFacts {
        ScanProgressFacts(
            files: 23_481,
            directories: 812,
            knownAllocatedBytes: knownAllocatedBytes,
            issueCount: 12
        )
    }

    private func makeSummary(
        coverage: AppScanCoverage = .complete,
        coveragePermille: UInt16? = 1_000
    ) -> AppScanSummary {
        AppScanSummary(
            scanID: "scan-overview",
            startedAt: now.addingTimeInterval(-120),
            completedAt: now,
            progress: makeFacts(),
            logicalBytes: 19 * gib,
            coverage: coverage,
            coveragePermille: coveragePermille,
            snapshotAvailable: true
        )
    }
}

private enum ExplorerPresentationTestError: Error {
    case notSnapshot
}
