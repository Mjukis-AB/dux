@testable import DUX
import Foundation
import XCTest

final class MenuBarPopoverPresentationTests: XCTestCase {
    private let en = Locale(identifier: "en_US")
    private let now = Date(timeIntervalSince1970: 3600)

    func testLoadedSnapshotUsesExactConservativeHeadlineTotalPercentAndBasis() throws {
        let presentation = makePresentation(volumeState: .loaded(makeSnapshot()))

        guard case let .snapshot(snapshot, status) = presentation.volume else {
            return XCTFail("Expected a rendered capacity snapshot")
        }
        XCTAssertNil(status)
        XCTAssertEqual(snapshot.volumeName, "Macintosh HD")
        XCTAssertEqual(snapshot.pressure, .warning)
        XCTAssertEqual(snapshot.pressureTitle, "Low space")
        XCTAssertEqual(snapshot.availableHeadline, "12.7 GiB available")
        XCTAssertEqual(snapshot.totalText, "of 100 GiB total")
        XCTAssertEqual(snapshot.availablePercentText, "12.7%")
        XCTAssertEqual(snapshot.availabilityBasisText, "Available for important use")
        XCTAssertEqual(snapshot.availableFraction, 0.127, accuracy: 0.000_000_1)
        XCTAssertTrue(snapshot.freshnessText.hasPrefix("Updated "))
        XCTAssertNil(presentation.scan)
        XCTAssertTrue(presentation.actions.showScanNow)
        XCTAssertTrue(presentation.actions.scanNowEnabled)
        XCTAssertFalse(presentation.actions.showCapacityRetry)
    }

    func testNoCacheLoadingAndFailureNeverCreateCapacityValues() {
        let loading = makePresentation(volumeState: .loading)
        guard case let .loading(message) = loading.volume else {
            return XCTFail("Expected loading")
        }
        XCTAssertEqual(message, "Checking startup disk…")
        XCTAssertFalse(loading.actions.showCapacityRetry)

        let failed = makePresentation(volumeState: .failed(.unavailable))
        guard case let .failed(title, detail) = failed.volume else {
            return XCTFail("Expected failure")
        }
        XCTAssertEqual(title, "Storage capacity unavailable")
        XCTAssertFalse(detail.contains("0 GiB"))
        XCTAssertTrue(failed.actions.showCapacityRetry)
    }

    func testRefreshAndStaleRetainTheExactLastSnapshotPresentation() throws {
        let source = makeSnapshot()
        let loaded = try snapshot(from: makePresentation(volumeState: .loaded(source)))
        let refreshingPresentation = makePresentation(volumeState: .refreshing(source))
        let stalePresentation = makePresentation(
            volumeState: .stale(source, .engineUnavailable)
        )
        let refreshing = try snapshot(from: refreshingPresentation)
        let stale = try snapshot(from: stalePresentation)

        XCTAssertEqual(refreshing, loaded)
        XCTAssertEqual(stale, loaded)
        guard case let .snapshot(_, refreshingStatus) = refreshingPresentation.volume,
              case let .snapshot(_, staleStatus) = stalePresentation.volume
        else {
            return XCTFail("Expected cached snapshots")
        }
        XCTAssertEqual(refreshingStatus?.style, .refreshing)
        XCTAssertEqual(refreshingStatus?.showsProgress, true)
        XCTAssertEqual(staleStatus?.style, .stale)
        XCTAssertEqual(staleStatus?.showsProgress, false)
        XCTAssertFalse(refreshingPresentation.actions.showCapacityRetry)
        XCTAssertTrue(stalePresentation.actions.showCapacityRetry)
    }

    func testCriticalAccessibilitySummaryLeadsWithAvailableCapacity() throws {
        let snapshot = try snapshot(
            from: makePresentation(
                volumeState: .loaded(makeSnapshot(pressure: .critical))
            )
        )

        XCTAssertTrue(snapshot.isCritical)
        XCTAssertTrue(snapshot.accessibilitySummary.hasPrefix("12.7 GiB available"))
        XCTAssertTrue(snapshot.accessibilitySummary.contains("Critically low space"))
        XCTAssertTrue(snapshot.accessibilitySummary.contains("Macintosh HD"))
        XCTAssertTrue(snapshot.accessibilitySummary.contains("12.7%"))
    }

    func testMatchingOpenPressurePeriodAddsCompactTruthfulContext() throws {
        let source = makeSnapshot()
        let history = VolumePressureHistory(
            stableVolumeID: "startup",
            anchorAt: source.sampledAt,
            episodes: [
                VolumePressureEpisode(
                    level: .warning,
                    enteredAt: source.sampledAt.addingTimeInterval(-900),
                    exitedAt: nil,
                    policyRevision: 2
                ),
            ],
            hasMore: false
        )
        let matching = try snapshot(
            from: MenuBarPopoverPresentation.make(
                volumeState: .loaded(source),
                scanState: .idle,
                pressureHistory: history,
                now: now,
                locale: en
            )
        )
        XCTAssertTrue(matching.activePressurePeriodText?.contains("Warning since") == true)
        XCTAssertTrue(
            matching.activePressurePeriodText?.contains("ongoing at latest sample") == true
        )

        let mismatched = try snapshot(
            from: MenuBarPopoverPresentation.make(
                volumeState: .loaded(source),
                scanState: .idle,
                pressureHistory: VolumePressureHistory(
                    stableVolumeID: "different-volume",
                    anchorAt: source.sampledAt,
                    episodes: history.episodes,
                    hasMore: false
                ),
                now: now,
                locale: en
            )
        )
        XCTAssertNil(mismatched.activePressurePeriodText)
    }

    func testFilesystemFallbackAndFutureTimestampRemainHonest() throws {
        let future = makeSnapshot(
            basis: .filesystemAvailable,
            sampledAt: now.addingTimeInterval(3600)
        )
        let snapshot = try snapshot(
            from: makePresentation(volumeState: .loaded(future))
        )

        XCTAssertEqual(snapshot.availabilityBasisText, "Filesystem available")
        XCTAssertEqual(snapshot.freshnessText, "Updated just now")
        XCTAssertTrue(snapshot.accessibilitySummary.contains("Filesystem available"))
    }

    func testActiveScanPhasesUseIndeterminateProgressAndExclusiveCancel() throws {
        let facts = makeFacts()
        let phases: [AppScanPhase] = [
            .queued,
            .scanning(facts),
            .finalizing(facts),
            .evaluating(facts),
        ]

        for phase in phases {
            let presentation = makePresentation(scanState: AppScanState(
                phase: phase,
                lastSuccessful: makeSummary()
            ))
            let scan = try XCTUnwrap(presentation.scan)
            XCTAssertEqual(scan.style, .progress)
            XCTAssertTrue(scan.showsIndeterminateProgress)
            XCTAssertFalse(presentation.actions.showScanNow)
            XCTAssertTrue(presentation.actions.showScanCancel)
            XCTAssertTrue(presentation.actions.scanCancelEnabled)
        }

        let scanning = makePresentation(scanState: AppScanState(
            phase: .scanning(facts),
            lastSuccessful: makeSummary()
        ))
        let detail = try XCTUnwrap(scanning.scan?.detail)
        XCTAssertTrue(detail.contains("23,481 files"))
        XCTAssertTrue(detail.contains("812 folders"))
        XCTAssertTrue(detail.contains("18.4 GiB observed"))
        XCTAssertTrue(detail.contains("12 scan issues"))
        XCTAssertFalse(detail.contains("/Users/"))
    }

    func testCancellationRequestedDisablesRepeatedCancelUntilSettlement() throws {
        let presentation = makePresentation(scanState: AppScanState(
            phase: .cancellationRequested(makeFacts()),
            lastSuccessful: makeSummary()
        ))
        let scan = try XCTUnwrap(presentation.scan)

        XCTAssertEqual(scan.title, "Stopping scan…")
        XCTAssertTrue(scan.showsIndeterminateProgress)
        XCTAssertTrue(presentation.actions.showScanCancel)
        XCTAssertFalse(presentation.actions.scanCancelEnabled)
        XCTAssertFalse(presentation.actions.showScanNow)
    }

    func testSelectedFolderScanIsDistinctFromHomeInCompactStatus() throws {
        let active = makePresentation(scanState: AppScanState(
            phase: .scanning(makeFacts()),
            lastSuccessful: makeSummary(),
            scope: .subtree(displayName: "Caches")
        ))
        XCTAssertEqual(active.scan?.title, "Refreshing selected folder…")
        XCTAssertTrue(active.scan?.detail?.contains("18.4 GiB observed") == true)

        let finished = makePresentation(scanState: AppScanState(
            phase: .succeeded(makeSummary()),
            lastSuccessful: makeSummary(),
            scope: .subtree(displayName: "Caches")
        ))
        XCTAssertEqual(finished.scan?.title, "Folder scan finished")
    }

    func testScanPhasesWithoutAHeartbeatNeverInventZeroCounters() throws {
        for phase in [
            AppScanPhase.scanning(nil),
            .finalizing(nil),
            .evaluating(nil),
            .cancellationRequested(nil),
        ] {
            let presentation = makePresentation(scanState: AppScanState(
                phase: phase,
                lastSuccessful: makeSummary()
            ))
            let scan = try XCTUnwrap(presentation.scan)
            XCTAssertNil(scan.detail)
            XCTAssertNil(scan.progressAccessibilityValue)
            XCTAssertTrue(scan.showsIndeterminateProgress)
        }

        let unknownAllocation = makePresentation(scanState: AppScanState(
            phase: .scanning(makeFacts(knownAllocatedBytes: nil)),
            lastSuccessful: makeSummary()
        ))
        XCTAssertTrue(
            unknownAllocation.scan?.detail?.contains("Allocation unavailable") == true
        )
        XCTAssertFalse(unknownAllocation.scan?.detail?.contains("0 GiB observed") == true)
    }

    func testTerminalScanPresentationsRetainPreviousSuccessAndAllowRetry() throws {
        let previous = makeSummary()
        for phase in [
            AppScanPhase.cancelled,
            .failed(.storageUnavailable),
            .failed(.taskExpired),
            .failed(.outcomeUnknown),
        ] {
            let presentation = makePresentation(scanState: AppScanState(
                phase: phase,
                lastSuccessful: previous
            ))
            let scan = try XCTUnwrap(presentation.scan)
            XCTAssertTrue(
                scan.detail?.contains("Last confirmed scan results remain available.") == true
            )
            XCTAssertFalse(scan.detail?.contains("unchanged") == true)
            XCTAssertTrue(presentation.actions.showScanNow)
            XCTAssertTrue(presentation.actions.scanNowEnabled)
            XCTAssertFalse(presentation.actions.showScanCancel)
        }

        let succeeded = makePresentation(scanState: AppScanState(
            phase: .succeeded(previous),
            lastSuccessful: previous
        ))
        XCTAssertEqual(succeeded.scan?.style, .success)
        XCTAssertTrue(succeeded.scan?.detail?.contains("Complete coverage") == true)
        XCTAssertTrue(succeeded.scan?.detail?.contains("23,481 files") == true)
    }

    func testAccessibilityIdentifiersAndKeyboardShortcutsAreStableAndUnique() {
        XCTAssertEqual(MenuBarPopoverAccessibility.root, "menu-popover")
        XCTAssertEqual(
            MenuBarPopoverAccessibility.openExplorer,
            "menu-popover-open-explorer"
        )
        XCTAssertEqual(
            Set(MenuBarPopoverAccessibility.allIdentifiers).count,
            MenuBarPopoverAccessibility.allIdentifiers.count
        )
        XCTAssertEqual(
            Set(MenuBarPopoverKeyboardShortcut.allKeys).count,
            MenuBarPopoverKeyboardShortcut.allKeys.count
        )
        XCTAssertEqual(MenuBarPopoverKeyboardShortcut.openExplorer, "o")
        XCTAssertEqual(MenuBarPopoverKeyboardShortcut.scanNow, "r")
        XCTAssertEqual(MenuBarPopoverKeyboardShortcut.settings, ",")
        XCTAssertEqual(MenuBarPopoverKeyboardShortcut.cancelScan, ".")
        XCTAssertEqual(MenuBarPopoverKeyboardShortcut.quit, "q")
    }

    private func makePresentation(
        volumeState: VolumeCapacityState? = nil,
        scanState: AppScanState = .idle
    ) -> MenuBarPopoverPresentation {
        MenuBarPopoverPresentation.make(
            volumeState: volumeState ?? .loaded(makeSnapshot()),
            scanState: scanState,
            now: now,
            locale: en
        )
    }

    private func snapshot(
        from presentation: MenuBarPopoverPresentation
    ) throws -> MenuBarPopoverSnapshotPresentation {
        guard case let .snapshot(snapshot, _) = presentation.volume else {
            throw PresentationTestError.notSnapshot
        }
        return snapshot
    }

    private func makeSnapshot(
        pressure: DiskPressureLevel = .warning,
        basis: VolumeCapacityBasis = .importantUsage,
        sampledAt: Date? = nil
    ) -> VolumeCapacitySnapshot {
        let gib: UInt64 = 1 << 30
        let available = 12 * gib + (7 * gib + 9) / 10
        return VolumeCapacitySnapshot(
            stableVolumeID: "startup",
            displayName: "Macintosh HD",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 100 * gib,
            filesystemAvailableBytes: basis == .filesystemAvailable ? available : 10 * gib,
            importantAvailableBytes: basis == .importantUsage ? available : nil,
            effectiveAvailableBytes: available,
            availabilityBasis: basis,
            pressure: pressure,
            criticalBoundaryBytes: 5 * gib,
            warningBoundaryBytes: 10 * gib,
            historyDisposition: .stored,
            sampledAt: sampledAt ?? now.addingTimeInterval(-120)
        )
    }

    private func makeFacts() -> ScanProgressFacts {
        let gib: UInt64 = 1 << 30
        return makeFacts(
            knownAllocatedBytes: 18 * gib + (4 * gib + 9) / 10
        )
    }

    private func makeFacts(knownAllocatedBytes: UInt64?) -> ScanProgressFacts {
        ScanProgressFacts(
            files: 23481,
            directories: 812,
            knownAllocatedBytes: knownAllocatedBytes,
            issueCount: 12
        )
    }

    private func makeSummary() -> AppScanSummary {
        AppScanSummary(
            scanID: "scan:test",
            startedAt: now.addingTimeInterval(-180),
            completedAt: now.addingTimeInterval(-60),
            progress: makeFacts(),
            logicalBytes: 20 * (1 << 30),
            coverage: .complete,
            coveragePermille: 1000,
            snapshotAvailable: true
        )
    }
}

private enum PresentationTestError: Error {
    case notSnapshot
}
