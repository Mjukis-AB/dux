import Foundation

enum MenuBarLabelMode: String, CaseIterable, Identifiable, Sendable {
    case iconOnly = "icon_only"
    case freeGiB = "free_gib"
    case freePercent = "free_percent"

    static let defaultMode = Self.freeGiB

    var id: String { rawValue }

    func localizedTitle(locale: Locale = .current) -> String {
        switch self {
        case .iconOnly:
            String(localized: "Icon only", locale: locale)
        case .freeGiB:
            String(localized: "Icon and free space (GiB)", locale: locale)
        case .freePercent:
            String(localized: "Icon and free space (%)", locale: locale)
        }
    }
}

@MainActor
protocol MenuBarLabelPreferenceStoring {
    func load() -> MenuBarLabelMode
    func save(_ mode: MenuBarLabelMode)
}

@MainActor
struct UserDefaultsMenuBarLabelPreferenceStore: MenuBarLabelPreferenceStoring {
    static let key = "menuBar.labelMode.v1"

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    func load() -> MenuBarLabelMode {
        guard
            let rawValue = defaults.string(forKey: Self.key),
            let mode = MenuBarLabelMode(rawValue: rawValue)
        else {
            return .defaultMode
        }
        return mode
    }

    func save(_ mode: MenuBarLabelMode) {
        defaults.set(mode.rawValue, forKey: Self.key)
    }
}

enum MenuBarCapacityFormatter {
    private static let bytesPerGiB: UInt64 = 1 << 30

    static func gib(_ bytes: UInt64, locale: Locale = .current) -> String {
        let whole = bytes / bytesPerGiB
        let remainder = bytes % bytesPerGiB
        let tenths = remainder * 10 / bytesPerGiB
        if bytes > 0, whole == 0, tenths == 0 {
            return "<0\(decimalSeparator(for: locale))1 GiB"
        }
        guard tenths > 0 else {
            return "\(whole) GiB"
        }
        return "\(whole)\(decimalSeparator(for: locale))\(tenths) GiB"
    }

    static func percent(
        availableBytes: UInt64,
        totalBytes: UInt64,
        locale: Locale = .current
    ) -> String {
        precondition(totalBytes > 0)
        precondition(availableBytes <= totalBytes)
        let product = availableBytes.multipliedFullWidth(by: 1_000)
        let tenths = totalBytes.dividingFullWidth(product).quotient
        if availableBytes > 0, tenths == 0 {
            return "<0\(decimalSeparator(for: locale))1%"
        }
        let whole = tenths / 10
        let fraction = tenths % 10
        guard fraction > 0 else {
            return "\(whole)%"
        }
        return "\(whole)\(decimalSeparator(for: locale))\(fraction)%"
    }

    private static func decimalSeparator(for locale: Locale) -> String {
        locale.decimalSeparator ?? "."
    }
}

@MainActor
struct MenuBarLabelPresentation: Equatable {
    static let accessibilityIdentifier = "menu-bar-storage-status"

    let symbolName: String
    let visibleText: String?
    let accessibilityLabel: String

    static func make(
        mode: MenuBarLabelMode,
        volumeState: VolumeCapacityState,
        locale: Locale = .current
    ) -> Self {
        switch volumeState {
        case .idle, .loading:
            return unavailable(
                symbolName: "externaldrive.fill",
                detail: String(localized: "Checking startup disk…", locale: locale),
                locale: locale
            )
        case .failed:
            return unavailable(
                symbolName: "xmark.octagon.fill",
                detail: String(localized: "Storage capacity unavailable", locale: locale),
                locale: locale
            )
        case let .loaded(snapshot):
            return snapshotPresentation(
                mode: mode,
                snapshot: snapshot,
                freshness: nil,
                locale: locale
            )
        case let .refreshing(snapshot):
            return snapshotPresentation(
                mode: mode,
                snapshot: snapshot,
                freshness: String(
                    localized: "Refreshing capacity; showing the last measured value",
                    locale: locale
                ),
                locale: locale
            )
        case let .stale(snapshot, _):
            return snapshotPresentation(
                mode: mode,
                snapshot: snapshot,
                freshness: String(
                    localized: "Last known capacity; the latest refresh failed",
                    locale: locale
                ),
                locale: locale
            )
        }
    }

    private static func unavailable(
        symbolName: String,
        detail: String,
        locale: Locale
    ) -> Self {
        Self(
            symbolName: symbolName,
            visibleText: nil,
            accessibilityLabel: [
                String(localized: "DUX storage status", locale: locale),
                detail,
            ].joined(separator: ", ")
        )
    }

    private static func snapshotPresentation(
        mode: MenuBarLabelMode,
        snapshot: VolumeCapacitySnapshot,
        freshness: String?,
        locale: Locale
    ) -> Self {
        let gib = MenuBarCapacityFormatter.gib(
            snapshot.effectiveAvailableBytes,
            locale: locale
        )
        let percent = MenuBarCapacityFormatter.percent(
            availableBytes: snapshot.effectiveAvailableBytes,
            totalBytes: snapshot.totalBytes,
            locale: locale
        )
        let visibleText: String? = switch mode {
        case .iconOnly: nil
        case .freeGiB: gib
        case .freePercent: percent
        }
        let basis = switch snapshot.availabilityBasis {
        case .importantUsage:
            String(localized: "Available for important use", locale: locale)
        case .filesystemAvailable:
            String(localized: "Filesystem available", locale: locale)
        }
        var accessibilityParts = [
            String(localized: "DUX storage status", locale: locale),
            DiskPressureBadge.localizedTitle(for: snapshot.pressure, locale: locale),
            String(localized: "\(gib) available", locale: locale),
            String(localized: "\(percent) available", locale: locale),
            basis,
            String(
                localized: "Measured \(formatted(snapshot.sampledAt, locale: locale))",
                locale: locale
            ),
        ]
        if let freshness {
            accessibilityParts.append(freshness)
        }
        return Self(
            symbolName: symbol(for: snapshot.pressure),
            visibleText: visibleText,
            accessibilityLabel: accessibilityParts.joined(separator: ", ")
        )
    }

    private static func symbol(for pressure: DiskPressureLevel) -> String {
        switch pressure {
        case .healthy: "checkmark.circle.fill"
        case .warning: "exclamationmark.triangle.fill"
        case .critical: "exclamationmark.octagon.fill"
        case .unknown: "questionmark.circle"
        }
    }

    private static func formatted(_ date: Date, locale: Locale) -> String {
        let formatter = DateFormatter()
        formatter.locale = locale
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }
}
