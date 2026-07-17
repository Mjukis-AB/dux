import Foundation

enum MenuBarVisibilityMode: String, CaseIterable, Identifiable, Sendable {
    case always = "always"
    case belowFreePercent = "below_free_percent"

    var id: String { rawValue }

    func localizedTitle(locale: Locale = .current) -> String {
        switch self {
        case .always:
            String(localized: "Always visible", locale: locale)
        case .belowFreePercent:
            String(localized: "Only when free space is low", locale: locale)
        }
    }
}

struct MenuBarVisibilityPreference: Equatable, Sendable {
    static let defaultThresholdPercent = 10
    static let defaults = Self(
        uncheckedMode: .always,
        thresholdPercent: defaultThresholdPercent
    )

    let mode: MenuBarVisibilityMode
    let thresholdPercent: Int

    init?(mode: MenuBarVisibilityMode, thresholdPercent: Int) {
        guard (1 ... 100).contains(thresholdPercent) else {
            return nil
        }
        self.mode = mode
        self.thresholdPercent = thresholdPercent
    }

    private init(uncheckedMode: MenuBarVisibilityMode, thresholdPercent: Int) {
        mode = uncheckedMode
        self.thresholdPercent = thresholdPercent
    }

    func changing(mode: MenuBarVisibilityMode) -> Self {
        Self(uncheckedMode: mode, thresholdPercent: thresholdPercent)
    }

    func changing(thresholdPercent: Int) -> Self? {
        Self(mode: mode, thresholdPercent: thresholdPercent)
    }
}

@MainActor
protocol MenuBarVisibilityPreferenceStoring {
    func load() -> MenuBarVisibilityPreference
    func save(_ preference: MenuBarVisibilityPreference)
}

@MainActor
struct UserDefaultsMenuBarVisibilityPreferenceStore: MenuBarVisibilityPreferenceStoring {
    static let key = "menuBar.visibility.v1"

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    func load() -> MenuBarVisibilityPreference {
        guard let encoded = defaults.string(forKey: Self.key) else {
            return .defaults
        }
        let fields = encoded.split(separator: "|", omittingEmptySubsequences: false)
        guard fields.count == 3,
              fields[0] == "1",
              let mode = MenuBarVisibilityMode(rawValue: String(fields[1])),
              let threshold = Int(fields[2]),
              let preference = MenuBarVisibilityPreference(
                mode: mode,
                thresholdPercent: threshold
              ) else {
            return .defaults
        }
        return preference
    }

    func save(_ preference: MenuBarVisibilityPreference) {
        defaults.set(
            "1|\(preference.mode.rawValue)|\(preference.thresholdPercent)",
            forKey: Self.key
        )
    }
}

enum MenuBarVisibilityEvaluator {
    static let recoveryMarginBasisPoints: UInt64 = 100

    static func shouldInsert(
        preference: MenuBarVisibilityPreference,
        volumeState: VolumeCapacityState,
        currentlyInserted: Bool
    ) -> Bool {
        guard preference.mode == .belowFreePercent else {
            return true
        }
        guard let snapshot = volumeState.snapshot else {
            return true
        }

        let availableBasisPoints = basisPoints(
            availableBytes: snapshot.effectiveAvailableBytes,
            totalBytes: snapshot.totalBytes
        )
        let thresholdBasisPoints = UInt64(preference.thresholdPercent) * 100
        if currentlyInserted {
            guard thresholdBasisPoints < 10_000 else {
                return true
            }
            let recoveryBoundary = min(
                thresholdBasisPoints + recoveryMarginBasisPoints,
                10_000
            )
            return availableBasisPoints < recoveryBoundary
        }
        return availableBasisPoints <= thresholdBasisPoints
    }

    static func basisPoints(
        availableBytes: UInt64,
        totalBytes: UInt64
    ) -> UInt64 {
        precondition(totalBytes > 0)
        precondition(availableBytes <= totalBytes)
        let product = availableBytes.multipliedFullWidth(by: 10_000)
        return totalBytes.dividingFullWidth(product).quotient
    }
}

enum MenuBarVisibilityAccessibility {
    static let mode = "menu-bar-visibility-mode"
    static let threshold = "menu-bar-visibility-threshold"

    static let allIdentifiers = [mode, threshold]
}
