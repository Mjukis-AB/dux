import Foundation
import XCTest
@testable import DUX

@MainActor
final class MenuBarVisibilityTests: XCTestCase {
    private let en = Locale(identifier: "en_US")

    func testPreferenceDefaultsAndValidationAreBounded() throws {
        XCTAssertEqual(MenuBarVisibilityPreference.defaults.mode, .always)
        XCTAssertEqual(MenuBarVisibilityPreference.defaults.thresholdPercent, 10)
        XCTAssertNil(MenuBarVisibilityPreference(mode: .always, thresholdPercent: 0))
        XCTAssertNil(MenuBarVisibilityPreference(mode: .always, thresholdPercent: 101))
        XCTAssertEqual(
            try XCTUnwrap(
                MenuBarVisibilityPreference(mode: .belowFreePercent, thresholdPercent: 1)
            ).thresholdPercent,
            1
        )
        XCTAssertEqual(
            try XCTUnwrap(
                MenuBarVisibilityPreference(mode: .belowFreePercent, thresholdPercent: 100)
            ).thresholdPercent,
            100
        )
    }

    func testPreferenceStoreRoundTripsAndDoesNotRewriteUnknownValues() throws {
        let suiteName = "dux-menu-visibility-tests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let store = UserDefaultsMenuBarVisibilityPreferenceStore(defaults: defaults)

        XCTAssertEqual(store.load(), .defaults)
        for value in [
            "2|always|10",
            "1|future|10",
            "1|always|0",
            "1|always|101",
            "1|always",
            "broken",
        ] {
            defaults.set(value, forKey: UserDefaultsMenuBarVisibilityPreferenceStore.key)
            XCTAssertEqual(store.load(), .defaults)
            XCTAssertEqual(
                defaults.string(forKey: UserDefaultsMenuBarVisibilityPreferenceStore.key),
                value
            )
        }
        defaults.set(7, forKey: UserDefaultsMenuBarVisibilityPreferenceStore.key)
        XCTAssertEqual(store.load(), .defaults)
        XCTAssertEqual(
            defaults.object(forKey: UserDefaultsMenuBarVisibilityPreferenceStore.key) as? Int,
            7
        )

        for mode in MenuBarVisibilityMode.allCases {
            for threshold in [1, 10, 100] {
                let preference = try XCTUnwrap(
                    MenuBarVisibilityPreference(mode: mode, thresholdPercent: threshold)
                )
                store.save(preference)
                XCTAssertEqual(store.load(), preference)
                let domain = defaults.persistentDomain(forName: suiteName)
                XCTAssertEqual(domain?.count, 1)
                XCTAssertNotNil(domain?[UserDefaultsMenuBarVisibilityPreferenceStore.key])
            }
        }
    }

    func testExactBasisPointsCoverBoundariesWithoutOverflow() {
        XCTAssertEqual(
            MenuBarVisibilityEvaluator.basisPoints(availableBytes: 0, totalBytes: 10),
            0
        )
        XCTAssertEqual(
            MenuBarVisibilityEvaluator.basisPoints(availableBytes: 1, totalBytes: 10_000),
            1
        )
        XCTAssertEqual(
            MenuBarVisibilityEvaluator.basisPoints(
                availableBytes: 1_275,
                totalBytes: 10_000
            ),
            1_275
        )
        XCTAssertEqual(
            MenuBarVisibilityEvaluator.basisPoints(
                availableBytes: UInt64.max,
                totalBytes: UInt64.max
            ),
            10_000
        )
    }

    func testAlwaysVisibleAndUnknownCapacityFailOpen() throws {
        let always = try preference(.always, threshold: 10)
        let conditional = try preference(.belowFreePercent, threshold: 10)
        let states: [VolumeCapacityState] = [
            .idle,
            .loading,
            .failed(.unavailable),
        ]

        for state in states {
            XCTAssertTrue(shouldInsert(always, state: state, currentlyInserted: false))
            XCTAssertTrue(shouldInsert(conditional, state: state, currentlyInserted: false))
        }
        XCTAssertTrue(
            shouldInsert(
                always,
                state: .loaded(snapshot(available: 100, total: 100)),
                currentlyInserted: false
            )
        )
    }

    func testConditionalThresholdAndOnePointRecoveryHysteresisAreExact() throws {
        let preference = try preference(.belowFreePercent, threshold: 10)

        XCTAssertTrue(
            shouldInsert(
                preference,
                state: .loaded(snapshot(available: 1_000, total: 10_000)),
                currentlyInserted: false
            )
        )
        XCTAssertFalse(
            shouldInsert(
                preference,
                state: .loaded(snapshot(available: 1_001, total: 10_000)),
                currentlyInserted: false
            )
        )
        XCTAssertTrue(
            shouldInsert(
                preference,
                state: .loaded(snapshot(available: 1_099, total: 10_000)),
                currentlyInserted: true
            )
        )
        XCTAssertFalse(
            shouldInsert(
                preference,
                state: .loaded(snapshot(available: 1_100, total: 10_000)),
                currentlyInserted: true
            )
        )
    }

    func testConditionalVisibilityUsesCachedRefreshingAndStaleSnapshots() throws {
        let preference = try preference(.belowFreePercent, threshold: 10)
        let low = snapshot(available: 9, total: 100)
        let healthy = snapshot(available: 20, total: 100)

        XCTAssertTrue(
            shouldInsert(preference, state: .refreshing(low), currentlyInserted: false)
        )
        XCTAssertFalse(
            shouldInsert(
                preference,
                state: .stale(healthy, .engineUnavailable),
                currentlyInserted: false
            )
        )
    }

    func testHundredPercentThresholdCannotHideTheOnlyEscapePath() throws {
        let preference = try preference(.belowFreePercent, threshold: 100)
        let full = VolumeCapacityState.loaded(snapshot(available: 100, total: 100))

        XCTAssertTrue(shouldInsert(preference, state: full, currentlyInserted: false))
        XCTAssertTrue(shouldInsert(preference, state: full, currentlyInserted: true))
    }

    func testAppModelLoadsPersistsAndAppliesPreferenceWithoutSampling() throws {
        let initial = try preference(.belowFreePercent, threshold: 10)
        let store = MenuBarVisibilityPreferenceStoreSpy(initial: initial)
        let monitor = MenuBarVisibilityVolumeMonitor(snapshots: [])
        let model = AppModel(
            volumeMonitor: monitor,
            menuBarVisibilityPreferenceStore: store
        )

        XCTAssertEqual(model.menuBarVisibilityPreference, initial)
        XCTAssertTrue(model.isMenuBarItemInserted)
        XCTAssertTrue(store.saved.isEmpty)

        model.setMenuBarVisibilityThresholdPercent(12)
        model.setMenuBarVisibilityThresholdPercent(12)
        model.setMenuBarVisibilityThresholdPercent(0)
        model.setMenuBarVisibilityMode(.always)

        XCTAssertEqual(store.saved.count, 2)
        XCTAssertEqual(store.saved[0].thresholdPercent, 12)
        XCTAssertEqual(store.saved[1].mode, .always)
        XCTAssertEqual(monitor.sampleCount, 0)
    }

    func testAppModelHidesFromCapacityAndReopenOverrideLastsForSession() async throws {
        let store = MenuBarVisibilityPreferenceStoreSpy(
            initial: try preference(.belowFreePercent, threshold: 10)
        )
        let monitor = MenuBarVisibilityVolumeMonitor(
            snapshots: [
                snapshot(available: 20, total: 100),
                snapshot(available: 30, total: 100),
            ]
        )
        let model = AppModel(
            engineService: MenuBarVisibilityEngineStub(),
            volumeMonitor: monitor,
            menuBarVisibilityPreferenceStore: store
        )

        await model.loadVolumeCapacity()
        XCTAssertFalse(model.isMenuBarItemInserted)

        model.revealMenuBarItemForSession()
        XCTAssertTrue(model.isMenuBarItemInserted)
        await model.refreshVolumeCapacity()

        XCTAssertTrue(model.isMenuBarItemInserted)
        XCTAssertEqual(monitor.sampleCount, 2)
        XCTAssertTrue(store.saved.isEmpty)
    }

    func testModeTitlesAndAccessibilityIdentifiersAreCompleteAndStable() {
        XCTAssertEqual(MenuBarVisibilityMode.allCases.count, 2)
        XCTAssertEqual(
            Set(MenuBarVisibilityMode.allCases.map { $0.localizedTitle(locale: en) }).count,
            2
        )
        XCTAssertEqual(
            MenuBarVisibilityAccessibility.allIdentifiers,
            ["menu-bar-visibility-mode", "menu-bar-visibility-threshold"]
        )
        XCTAssertEqual(Set(MenuBarVisibilityAccessibility.allIdentifiers).count, 2)
    }

    private func preference(
        _ mode: MenuBarVisibilityMode,
        threshold: Int
    ) throws -> MenuBarVisibilityPreference {
        try XCTUnwrap(
            MenuBarVisibilityPreference(mode: mode, thresholdPercent: threshold)
        )
    }

    private func shouldInsert(
        _ preference: MenuBarVisibilityPreference,
        state: VolumeCapacityState,
        currentlyInserted: Bool
    ) -> Bool {
        MenuBarVisibilityEvaluator.shouldInsert(
            preference: preference,
            volumeState: state,
            currentlyInserted: currentlyInserted
        )
    }

    private func snapshot(available: UInt64, total: UInt64) -> VolumeCapacitySnapshot {
        VolumeCapacitySnapshot(
            stableVolumeID: "test-volume",
            displayName: "Startup",
            filesystem: "APFS",
            isInternal: true,
            isRemovable: false,
            totalBytes: total,
            filesystemAvailableBytes: available,
            importantAvailableBytes: available,
            effectiveAvailableBytes: available,
            availabilityBasis: .importantUsage,
            pressure: .unknown,
            criticalBoundaryBytes: nil,
            warningBoundaryBytes: nil,
            historyDisposition: nil,
            sampledAt: Date(timeIntervalSince1970: 1)
        )
    }
}

@MainActor
private final class MenuBarVisibilityPreferenceStoreSpy: MenuBarVisibilityPreferenceStoring {
    private let initial: MenuBarVisibilityPreference
    private(set) var saved: [MenuBarVisibilityPreference] = []

    init(initial: MenuBarVisibilityPreference) {
        self.initial = initial
    }

    func load() -> MenuBarVisibilityPreference { initial }

    func save(_ preference: MenuBarVisibilityPreference) {
        saved.append(preference)
    }
}

private final class MenuBarVisibilityVolumeMonitor: VolumeMonitoring, @unchecked Sendable {
    private let lock = NSLock()
    private var snapshots: [VolumeCapacitySnapshot]
    private(set) var sampleCount = 0

    init(snapshots: [VolumeCapacitySnapshot]) {
        self.snapshots = snapshots
    }

    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot {
        lock.withLock {
            sampleCount += 1
            return snapshots.removeFirst()
        }
    }
}

private actor MenuBarVisibilityEngineStub: EngineServing {
    func loadStatus() async throws -> EngineStatus {
        EngineStatus(libraryVersion: "test", ffiContractVersion: 8, executedOffMainThread: true)
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
        _ = configuration
        throw EngineServiceError.unexpected("not used")
    }

    func resetDiskPressurePolicy() async throws -> DiskPressurePolicyUpdateResult {
        throw EngineServiceError.unexpected("not used")
    }
}
