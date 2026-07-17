import AppKit
import Foundation
import Observation
import XCTest
@testable import DUX

@MainActor
final class MenuBarLabelModeTests: XCTestCase {
    private let en = Locale(identifier: "en_US")
    private let de = Locale(identifier: "de_DE")

    func testExactConservativeGiBFormattingCoversBoundariesAndLocales() {
        let gib: UInt64 = 1 << 30
        let oneTenthGiB = (gib + 9) / 10

        XCTAssertEqual(MenuBarCapacityFormatter.gib(0, locale: en), "0 GiB")
        XCTAssertEqual(MenuBarCapacityFormatter.gib(1, locale: en), "<0.1 GiB")
        XCTAssertEqual(MenuBarCapacityFormatter.gib(oneTenthGiB - 1, locale: en), "<0.1 GiB")
        XCTAssertEqual(MenuBarCapacityFormatter.gib(oneTenthGiB, locale: en), "0.1 GiB")
        XCTAssertEqual(MenuBarCapacityFormatter.gib(gib, locale: en), "1 GiB")
        XCTAssertEqual(
            MenuBarCapacityFormatter.gib(gib + oneTenthGiB, locale: de),
            "1,1 GiB"
        )
        XCTAssertEqual(
            MenuBarCapacityFormatter.gib(UInt64.max, locale: en),
            "17179869183.9 GiB"
        )
    }

    func testExactConservativePercentFormattingDoesNotOverflowOrRoundUp() {
        XCTAssertEqual(
            MenuBarCapacityFormatter.percent(availableBytes: 0, totalBytes: 10, locale: en),
            "0%"
        )
        XCTAssertEqual(
            MenuBarCapacityFormatter.percent(availableBytes: 1, totalBytes: 10_000, locale: en),
            "<0.1%"
        )
        XCTAssertEqual(
            MenuBarCapacityFormatter.percent(availableBytes: 1_275, totalBytes: 10_000, locale: en),
            "12.7%"
        )
        XCTAssertEqual(
            MenuBarCapacityFormatter.percent(availableBytes: 1_275, totalBytes: 10_000, locale: de),
            "12,7%"
        )
        XCTAssertEqual(
            MenuBarCapacityFormatter.percent(
                availableBytes: UInt64.max,
                totalBytes: UInt64.max,
                locale: en
            ),
            "100%"
        )
    }

    func testAllModesAndVolumeStatesProduceHonestBoundedPresentation() {
        let snapshot = makeSnapshot(pressure: .warning)
        let states: [VolumeCapacityState] = [
            .idle,
            .loading,
            .failed(.unavailable),
            .loaded(snapshot),
            .refreshing(snapshot),
            .stale(snapshot, .engineUnavailable),
        ]

        for mode in MenuBarLabelMode.allCases {
            for state in states {
                let presentation = MenuBarLabelPresentation.make(
                    mode: mode,
                    volumeState: state,
                    locale: en
                )
                XCTAssertFalse(presentation.symbolName.isEmpty)
                XCTAssertTrue(presentation.accessibilityLabel.contains("DUX storage status"))
                if state.snapshot == nil || mode == .iconOnly {
                    XCTAssertNil(presentation.visibleText)
                } else {
                    XCTAssertNotNil(presentation.visibleText)
                }
            }
        }

        XCTAssertEqual(
            MenuBarLabelPresentation.make(
                mode: .freeGiB,
                volumeState: .loaded(snapshot),
                locale: en
            ).visibleText,
            "12.7 GiB"
        )
        XCTAssertEqual(
            MenuBarLabelPresentation.make(
                mode: .freePercent,
                volumeState: .loaded(snapshot),
                locale: en
            ).visibleText,
            "12.7%"
        )
        XCTAssertTrue(
            MenuBarLabelPresentation.make(
                mode: .freeGiB,
                volumeState: .refreshing(snapshot),
                locale: en
            ).accessibilityLabel.contains("last measured")
        )
        XCTAssertTrue(
            MenuBarLabelPresentation.make(
                mode: .freeGiB,
                volumeState: .stale(snapshot, .unavailable),
                locale: en
            ).accessibilityLabel.contains("latest refresh failed")
        )
    }

    func testPressureUsesShapeDistinctMonochromeSymbols() {
        let pressures: [DiskPressureLevel] = [.healthy, .warning, .critical, .unknown]
        let symbols = pressures.map { pressure in
            MenuBarLabelPresentation.make(
                mode: .iconOnly,
                volumeState: .loaded(makeSnapshot(pressure: pressure)),
                locale: en
            ).symbolName
        }

        XCTAssertEqual(Set(symbols).count, pressures.count)
        for symbol in symbols + ["externaldrive.fill", "xmark.octagon.fill"] {
            XCTAssertNotNil(NSImage(systemSymbolName: symbol, accessibilityDescription: nil))
        }
    }

    func testAccessibilityAlwaysDisclosesPressureCapacityPercentAndBasis() {
        let important = MenuBarLabelPresentation.make(
            mode: .iconOnly,
            volumeState: .loaded(makeSnapshot(pressure: .critical)),
            locale: en
        )
        XCTAssertTrue(important.accessibilityLabel.contains("Critically low space"))
        XCTAssertTrue(important.accessibilityLabel.contains("12.7 GiB available"))
        XCTAssertTrue(important.accessibilityLabel.contains("12.7% available"))
        XCTAssertTrue(important.accessibilityLabel.contains("Available for important use"))
        XCTAssertTrue(important.accessibilityLabel.contains("Measured"))

        let fallback = MenuBarLabelPresentation.make(
            mode: .freePercent,
            volumeState: .loaded(
                makeSnapshot(pressure: .healthy, basis: .filesystemAvailable)
            ),
            locale: en
        )
        XCTAssertTrue(fallback.accessibilityLabel.contains("Filesystem available"))
    }

    func testPreferenceStoreDefaultsRoundTripsAndDoesNotRewriteUnknownValues() throws {
        let suiteName = "dux-menu-label-tests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removeObject(forKey: UserDefaultsMenuBarLabelPreferenceStore.key) }
        let store = UserDefaultsMenuBarLabelPreferenceStore(defaults: defaults)

        XCTAssertEqual(store.load(), .freeGiB)
        defaults.set("future_mode", forKey: UserDefaultsMenuBarLabelPreferenceStore.key)
        XCTAssertEqual(store.load(), .freeGiB)
        XCTAssertEqual(
            defaults.string(forKey: UserDefaultsMenuBarLabelPreferenceStore.key),
            "future_mode"
        )
        defaults.set(7, forKey: UserDefaultsMenuBarLabelPreferenceStore.key)
        XCTAssertEqual(store.load(), .freeGiB)
        XCTAssertEqual(
            defaults.object(forKey: UserDefaultsMenuBarLabelPreferenceStore.key) as? Int,
            7
        )

        for mode in MenuBarLabelMode.allCases {
            store.save(mode)
            XCTAssertEqual(store.load(), mode)
        }
    }

    func testAppModelLoadsObservesAndPersistsAChangedModeExactlyOnce() async {
        let store = MenuBarLabelPreferenceStoreSpy(initial: .iconOnly)
        let model = AppModel(menuBarLabelPreferenceStore: store)
        let observed = expectation(description: "Menu label observation invalidated")

        XCTAssertEqual(model.menuBarLabelMode, .iconOnly)
        XCTAssertTrue(store.saved.isEmpty)
        withObservationTracking {
            _ = model.menuBarLabelMode
        } onChange: {
            observed.fulfill()
        }
        model.menuBarLabelMode = .freePercent
        await fulfillment(of: [observed], timeout: 1)
        model.menuBarLabelMode = .freePercent
        XCTAssertEqual(store.saved, [.freePercent])
    }

    func testModeTitlesAndAccessibilityIdentifiersAreCompleteAndStable() {
        XCTAssertEqual(MenuBarLabelMode.allCases.count, 3)
        XCTAssertEqual(
            Set(MenuBarLabelMode.allCases.map { $0.localizedTitle(locale: en) }).count,
            3
        )
        XCTAssertEqual(
            MenuBarLabelPresentation.accessibilityIdentifier,
            "menu-bar-storage-status"
        )
        XCTAssertEqual(MenuBarLabelAccessibility.picker, "menu-bar-label-mode")
    }

    private func makeSnapshot(
        pressure: DiskPressureLevel,
        basis: VolumeCapacityBasis = .importantUsage
    ) -> VolumeCapacitySnapshot {
        let gib: UInt64 = 1 << 30
        let available = 12 * gib + (7 * gib + 9) / 10
        return VolumeCapacitySnapshot(
            stableVolumeID: "test-volume",
            displayName: "Startup",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: 100 * gib,
            filesystemAvailableBytes: basis == .filesystemAvailable ? available : nil,
            importantAvailableBytes: basis == .importantUsage ? available : nil,
            effectiveAvailableBytes: available,
            availabilityBasis: basis,
            pressure: pressure,
            criticalBoundaryBytes: 5 * gib,
            warningBoundaryBytes: 10 * gib,
            historyDisposition: .stored,
            sampledAt: Date(timeIntervalSince1970: 1)
        )
    }
}

@MainActor
private final class MenuBarLabelPreferenceStoreSpy: MenuBarLabelPreferenceStoring {
    private let initial: MenuBarLabelMode
    private(set) var saved: [MenuBarLabelMode] = []

    init(initial: MenuBarLabelMode) {
        self.initial = initial
    }

    func load() -> MenuBarLabelMode { initial }

    func save(_ mode: MenuBarLabelMode) {
        saved.append(mode)
    }
}
