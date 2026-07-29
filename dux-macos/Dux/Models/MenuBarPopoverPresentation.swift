import Foundation

enum MenuBarPopoverAccessibility {
    static let root = "menu-popover"
    static let volumeSummary = "menu-popover-volume-summary"
    static let volumeName = "menu-popover-volume-name"
    static let pressure = "menu-popover-pressure"
    static let available = "menu-popover-available"
    static let total = "menu-popover-total"
    static let capacityBar = "menu-popover-capacity-bar"
    static let freshness = "menu-popover-freshness"
    static let capacityStatus = "menu-popover-capacity-status"
    static let capacityRetry = "menu-popover-capacity-retry"
    static let trend = "menu-popover-trend"
    static let trendChart = "menu-popover-trend-chart"
    static let activePressurePeriod = "menu-popover-active-pressure-period"
    static let scanStatus = "menu-popover-scan-status"
    static let scanProgress = "menu-popover-scan-progress"
    static let scanCancel = "menu-popover-scan-cancel"
    static let targetedReclaim = "menu-popover-targeted-reclaim"
    static let targetedReclaimProgress = "menu-popover-targeted-reclaim-progress"
    static let scanNow = "menu-popover-scan-now"
    static let openExplorer = "menu-popover-open-explorer"
    static let settings = "menu-popover-settings"
    static let quit = "menu-popover-quit"

    static let allIdentifiers = [
        root,
        volumeSummary,
        volumeName,
        pressure,
        available,
        total,
        capacityBar,
        freshness,
        capacityStatus,
        capacityRetry,
        trend,
        trendChart,
        activePressurePeriod,
        scanStatus,
        scanProgress,
        scanCancel,
        targetedReclaim,
        targetedReclaimProgress,
        scanNow,
        openExplorer,
        settings,
        quit,
    ]
}

enum MenuBarPopoverKeyboardShortcut {
    static let openExplorer: Character = "o"
    static let scanNow: Character = "r"
    static let settings: Character = ","
    static let cancelScan: Character = "."
    static let quit: Character = "q"

    static let allKeys = [openExplorer, scanNow, settings, cancelScan, quit]
}

enum MenuBarPopoverCapacityStatusStyle: Equatable, Sendable {
    case refreshing
    case stale
}

struct MenuBarPopoverCapacityStatus: Equatable, Sendable {
    let style: MenuBarPopoverCapacityStatusStyle
    let message: String
    let showsProgress: Bool
}

struct MenuBarPopoverSnapshotPresentation: Equatable, Sendable {
    let volumeName: String
    let pressure: DiskPressureLevel
    let pressureTitle: String
    let availableHeadline: String
    let totalText: String
    let availablePercentText: String
    let availabilityBasisText: String
    let availableFraction: Double
    let freshnessText: String
    let trend: MenuBarPopoverTrendPresentation?
    let activePressurePeriodText: String?
    let accessibilitySummary: String

    var isCritical: Bool { pressure == .critical }
}

struct MenuBarPopoverTrendPresentation: Equatable, Sendable {
    let change24hText: String?
    let change7dText: String?
    let chartFractions: [Double]
    let accessibilitySummary: String
}

enum MenuBarPopoverVolumePresentation: Equatable, Sendable {
    case loading(message: String)
    case snapshot(
        MenuBarPopoverSnapshotPresentation,
        status: MenuBarPopoverCapacityStatus?
    )
    case failed(title: String, detail: String)
}

enum MenuBarPopoverScanStyle: Equatable, Sendable {
    case progress
    case success
    case cancelled
    case failure
}

struct MenuBarPopoverScanPresentation: Equatable, Sendable {
    let style: MenuBarPopoverScanStyle
    let title: String
    let detail: String?
    let progressAccessibilityValue: String?
    let showsIndeterminateProgress: Bool
}

struct MenuBarPopoverActionMatrix: Equatable, Sendable {
    let showCapacityRetry: Bool
    let showScanNow: Bool
    let scanNowEnabled: Bool
    let showScanCancel: Bool
    let scanCancelEnabled: Bool

    static func make(
        volumeState: VolumeCapacityState,
        scanState: AppScanState
    ) -> Self {
        let showCapacityRetry = switch volumeState {
        case .stale, .failed: true
        case .idle, .loading, .refreshing, .loaded: false
        }
        return switch scanState.phase {
        case .queued, .scanning, .finalizing, .evaluating:
            Self(
                showCapacityRetry: showCapacityRetry,
                showScanNow: false,
                scanNowEnabled: false,
                showScanCancel: true,
                scanCancelEnabled: true
            )
        case .cancellationRequested:
            Self(
                showCapacityRetry: showCapacityRetry,
                showScanNow: false,
                scanNowEnabled: false,
                showScanCancel: true,
                scanCancelEnabled: false
            )
        case .idle, .succeeded, .cancelled, .failed:
            Self(
                showCapacityRetry: showCapacityRetry,
                showScanNow: true,
                scanNowEnabled: true,
                showScanCancel: false,
                scanCancelEnabled: false
            )
        }
    }
}

struct MenuBarPopoverPresentation: Equatable, Sendable {
    let volume: MenuBarPopoverVolumePresentation
    let scan: MenuBarPopoverScanPresentation?
    let actions: MenuBarPopoverActionMatrix
}

extension MenuBarPopoverPresentation {
    static func make(
        volumeState: VolumeCapacityState,
        scanState: AppScanState,
        trend: VolumeCapacityTrend? = nil,
        pressureHistory: VolumePressureHistory? = nil,
        now: Date = .now,
        locale: Locale = .current
    ) -> Self {
        Self(
            volume: volumePresentation(
                for: volumeState,
                trend: trend,
                pressureHistory: pressureHistory,
                now: now,
                locale: locale
            ),
            scan: scanPresentation(
                for: scanState,
                now: now,
                locale: locale
            ),
            actions: .make(volumeState: volumeState, scanState: scanState)
        )
    }

    private static func volumePresentation(
        for state: VolumeCapacityState,
        trend: VolumeCapacityTrend?,
        pressureHistory: VolumePressureHistory?,
        now: Date,
        locale: Locale
    ) -> MenuBarPopoverVolumePresentation {
        switch state {
        case .idle, .loading:
            .loading(
                message: String(localized: "Checking startup disk…", locale: locale)
            )
        case let .loaded(snapshot):
            .snapshot(
                snapshotPresentation(
                    snapshot,
                    trend: trend,
                    pressureHistory: pressureHistory,
                    now: now,
                    locale: locale
                ),
                status: nil
            )
        case let .refreshing(snapshot):
            .snapshot(
                snapshotPresentation(
                    snapshot,
                    trend: trend,
                    pressureHistory: pressureHistory,
                    now: now,
                    locale: locale
                ),
                status: MenuBarPopoverCapacityStatus(
                    style: .refreshing,
                    message: String(localized: "Updating capacity…", locale: locale),
                    showsProgress: true
                )
            )
        case let .stale(snapshot, failure):
            .snapshot(
                snapshotPresentation(
                    snapshot,
                    trend: trend,
                    pressureHistory: pressureHistory,
                    now: now,
                    locale: locale
                ),
                status: MenuBarPopoverCapacityStatus(
                    style: .stale,
                    message: staleCapacityMessage(for: failure, locale: locale),
                    showsProgress: false
                )
            )
        case let .failed(failure):
            .failed(
                title: String(localized: "Storage capacity unavailable", locale: locale),
                detail: capacityFailureDetail(for: failure, locale: locale)
            )
        }
    }

    private static func snapshotPresentation(
        _ snapshot: VolumeCapacitySnapshot,
        trend: VolumeCapacityTrend?,
        pressureHistory: VolumePressureHistory?,
        now: Date,
        locale: Locale
    ) -> MenuBarPopoverSnapshotPresentation {
        let volumeName = snapshot.displayName ?? String(
            localized: "Startup Disk",
            locale: locale
        )
        let available = MenuBarCapacityFormatter.gib(
            snapshot.effectiveAvailableBytes,
            locale: locale
        )
        let total = MenuBarCapacityFormatter.gib(snapshot.totalBytes, locale: locale)
        let percent = MenuBarCapacityFormatter.percent(
            availableBytes: snapshot.effectiveAvailableBytes,
            totalBytes: snapshot.totalBytes,
            locale: locale
        )
        let pressureTitle = pressureTitle(for: snapshot.pressure, locale: locale)
        let basis = switch snapshot.availabilityBasis {
        case .importantUsage:
            String(localized: "Available for important use", locale: locale)
        case .filesystemAvailable:
            String(localized: "Filesystem available", locale: locale)
        }
        let headline = String(localized: "\(available) available", locale: locale)
        let totalText = String(localized: "of \(total) total", locale: locale)
        let freshness = freshnessText(
            sampledAt: snapshot.sampledAt,
            now: now,
            locale: locale
        )
        let summaryParts: [String] = if snapshot.pressure == .critical {
            [
                headline,
                pressureTitle,
                volumeName,
                totalText,
                percent,
                basis,
                freshness,
            ]
        } else {
            [
                volumeName,
                pressureTitle,
                headline,
                totalText,
                percent,
                basis,
                freshness,
            ]
        }
        let mappedTrend = trend.map { Self.trendPresentation($0, locale: locale) }
        let activePressurePeriod = activePressurePeriodText(
            snapshot: snapshot,
            history: pressureHistory,
            locale: locale
        )
        return MenuBarPopoverSnapshotPresentation(
            volumeName: volumeName,
            pressure: snapshot.pressure,
            pressureTitle: pressureTitle,
            availableHeadline: headline,
            totalText: totalText,
            availablePercentText: percent,
            availabilityBasisText: basis,
            availableFraction: Double(snapshot.effectiveAvailableBytes)
                / Double(snapshot.totalBytes),
            freshnessText: freshness,
            trend: mappedTrend,
            activePressurePeriodText: activePressurePeriod,
            accessibilitySummary: summaryParts.joined(separator: ". ")
        )
    }

    private static func activePressurePeriodText(
        snapshot: VolumeCapacitySnapshot,
        history: VolumePressureHistory?,
        locale: Locale
    ) -> String? {
        guard
            let history,
            history.stableVolumeID == snapshot.stableVolumeID,
            history.anchorAt == snapshot.sampledAt,
            let episode = history.episodes.first,
            episode.exitedAt == nil,
            (episode.level == .warning && snapshot.pressure == .warning)
                || (episode.level == .critical && snapshot.pressure == .critical)
        else {
            return nil
        }
        let level = episode.level == .critical
            ? String(localized: "Critical", locale: locale)
            : String(localized: "Warning", locale: locale)
        let entered = episode.enteredAt.formatted(
            .dateTime.locale(locale).hour().minute()
        )
        return String(
            localized: "\(level) since \(entered) · ongoing at latest sample",
            locale: locale
        )
    }

    private static func trendPresentation(
        _ trend: VolumeCapacityTrend,
        locale: Locale
    ) -> MenuBarPopoverTrendPresentation {
        let change24hText = trend.change24h.map {
            signedCapacityText($0.availableBytes, locale: locale)
        }
        let change7dText = trend.change7d.map {
            signedCapacityText($0.availableBytes, locale: locale)
        }
        let fractions = trend.points.map { point in
            Double(point.availableBytes) / Double(point.totalBytes)
        }
        let summary = [
            change24hText.map { String(localized: "24 hours \($0)", locale: locale) },
            change7dText.map { String(localized: "7 days \($0)", locale: locale) },
        ]
        .compactMap { $0 }
        .joined(separator: ", ")
        return MenuBarPopoverTrendPresentation(
            change24hText: change24hText,
            change7dText: change7dText,
            chartFractions: fractions,
            accessibilitySummary: summary.isEmpty
                ? String(localized: "Trend history is warming up", locale: locale)
                : summary
        )
    }

    private static func signedCapacityText(_ value: Int64, locale: Locale) -> String {
        let magnitude = MenuBarCapacityFormatter.gib(value.magnitude, locale: locale)
        if value > 0 { return "+\(magnitude)" }
        if value < 0 { return "-\(magnitude)" }
        return magnitude
    }

    private static func scanPresentation(
        for state: AppScanState,
        now: Date,
        locale: Locale
    ) -> MenuBarPopoverScanPresentation? {
        switch state.phase {
        case .idle:
            return nil
        case .queued:
            return progressPresentation(
                title: state.scope?.isHome == false
                    ? String(localized: "Waiting to refresh selected folder…", locale: locale)
                    : String(localized: "Waiting to scan Home…", locale: locale),
                facts: nil,
                locale: locale
            )
        case let .scanning(facts):
            return progressPresentation(
                title: state.scope?.isHome == false
                    ? String(localized: "Refreshing selected folder…", locale: locale)
                    : String(localized: "Scanning Home…", locale: locale),
                facts: facts,
                locale: locale
            )
        case let .finalizing(facts):
            return progressPresentation(
                title: state.scope?.isHome == false
                    ? String(localized: "Preparing folder scan results…", locale: locale)
                    : String(localized: "Preparing scan results…", locale: locale),
                facts: facts,
                locale: locale
            )
        case let .evaluating(facts):
            return progressPresentation(
                title: state.scope?.isHome == false
                    ? String(localized: "Classifying folder scan results…", locale: locale)
                    : String(localized: "Classifying scan results…", locale: locale),
                facts: facts,
                locale: locale
            )
        case let .cancellationRequested(facts):
            return progressPresentation(
                title: state.scope?.isHome == false
                    ? String(localized: "Stopping folder scan…", locale: locale)
                    : String(localized: "Stopping scan…", locale: locale),
                facts: facts,
                locale: locale
            )
        case let .succeeded(summary):
            let detail = [
                completedText(at: summary.completedAt, now: now, locale: locale),
                factsText(summary.progress, locale: locale),
                coverageTitle(summary.coverage, locale: locale),
            ].joined(separator: " · ")
            return MenuBarPopoverScanPresentation(
                style: .success,
                title: state.scope?.isHome == false
                    ? String(localized: "Folder scan finished", locale: locale)
                    : String(localized: "Scan finished", locale: locale),
                detail: detail,
                progressAccessibilityValue: nil,
                showsIndeterminateProgress: false
            )
        case .cancelled:
            return MenuBarPopoverScanPresentation(
                style: .cancelled,
                title: state.scope?.isHome == false
                    ? String(localized: "Folder scan stopped", locale: locale)
                    : String(localized: "Scan stopped", locale: locale),
                detail: retainedResultsText(
                    hasPrevious: state.lastSuccessful != nil,
                    locale: locale
                ),
                progressAccessibilityValue: nil,
                showsIndeterminateProgress: false
            )
        case let .failed(failure):
            return MenuBarPopoverScanPresentation(
                style: .failure,
                title: state.scope?.isHome == false
                    ? String(localized: "Folder scan couldn’t finish", locale: locale)
                    : String(localized: "Scan couldn’t finish", locale: locale),
                detail: [
                    scanFailureDetail(failure, locale: locale),
                    retainedResultsText(
                        hasPrevious: state.lastSuccessful != nil,
                        locale: locale
                    ),
                ].joined(separator: " "),
                progressAccessibilityValue: nil,
                showsIndeterminateProgress: false
            )
        }
    }

    private static func progressPresentation(
        title: String,
        facts: ScanProgressFacts?,
        locale: Locale
    ) -> MenuBarPopoverScanPresentation {
        let detail = facts.map { factsText($0, locale: locale) }
        return MenuBarPopoverScanPresentation(
            style: .progress,
            title: title,
            detail: detail,
            progressAccessibilityValue: detail,
            showsIndeterminateProgress: true
        )
    }

    private static func factsText(_ facts: ScanProgressFacts, locale: Locale) -> String {
        let files = facts.files.formatted(.number.locale(locale))
        let directories = facts.directories.formatted(.number.locale(locale))
        let allocation = facts.knownAllocatedBytes.map {
            String(
                localized: "\(MenuBarCapacityFormatter.gib($0, locale: locale)) observed",
                locale: locale
            )
        } ?? String(localized: "Allocation unavailable", locale: locale)
        let issues = facts.issueCount.formatted(.number.locale(locale))
        return String(
            localized: "\(files) files · \(directories) folders · \(allocation) · \(issues) scan issues",
            locale: locale
        )
    }

    private static func completedText(at date: Date, now: Date, locale: Locale) -> String {
        String(
            localized: "Completed \(relativeText(for: date, now: now, locale: locale))",
            locale: locale
        )
    }

    private static func freshnessText(
        sampledAt: Date,
        now: Date,
        locale: Locale
    ) -> String {
        String(
            localized: "Updated \(relativeText(for: sampledAt, now: now, locale: locale))",
            locale: locale
        )
    }

    private static func relativeText(for date: Date, now: Date, locale: Locale) -> String {
        guard now.timeIntervalSince(date) >= 60 else {
            return String(localized: "just now", locale: locale)
        }
        let formatter = RelativeDateTimeFormatter()
        formatter.locale = locale
        formatter.unitsStyle = .full
        return formatter.localizedString(for: date, relativeTo: now)
    }

    private static func pressureTitle(
        for pressure: DiskPressureLevel,
        locale: Locale
    ) -> String {
        switch pressure {
        case .healthy: String(localized: "Healthy", locale: locale)
        case .warning: String(localized: "Low space", locale: locale)
        case .critical: String(localized: "Critically low space", locale: locale)
        case .unknown: String(localized: "Pressure unknown", locale: locale)
        }
    }

    private static func coverageTitle(
        _ coverage: AppScanCoverage,
        locale: Locale
    ) -> String {
        switch coverage {
        case .complete: String(localized: "Complete coverage", locale: locale)
        case .limitedAccess: String(localized: "Limited access", locale: locale)
        case .partial: String(localized: "Partial coverage", locale: locale)
        case .unknown: String(localized: "Coverage unavailable", locale: locale)
        }
    }

    private static func capacityFailureDetail(
        for failure: VolumeCapacityFailure,
        locale: Locale
    ) -> String {
        switch failure {
        case .unavailable:
            String(
                localized: "macOS did not report startup-disk capacity.",
                locale: locale
            )
        case .invalidObservation:
            String(
                localized: "The startup-disk capacity response was invalid.",
                locale: locale
            )
        case .engineUnavailable:
            String(
                localized: "The storage engine could not classify the latest capacity.",
                locale: locale
            )
        }
    }

    private static func staleCapacityMessage(
        for failure: VolumeCapacityFailure,
        locale: Locale
    ) -> String {
        switch failure {
        case .unavailable:
            String(localized: "The latest capacity check was unavailable.", locale: locale)
        case .invalidObservation:
            String(localized: "The latest capacity check was invalid.", locale: locale)
        case .engineUnavailable:
            String(localized: "The latest capacity check could not be classified.", locale: locale)
        }
    }

    private static func scanFailureDetail(
        _ failure: AppScanFailure,
        locale: Locale
    ) -> String {
        switch failure {
        case .busy:
            String(localized: "Another scan is already using this location.", locale: locale)
        case .rootUnavailable:
            String(localized: "The Home folder could not be scanned safely.", locale: locale)
        case .incompatibleStorage:
            String(localized: "This scan requires a newer DUX storage format.", locale: locale)
        case .closed, .storageUnavailable:
            String(localized: "The storage engine is unavailable.", locale: locale)
        case .taskExpired:
            String(localized: "The scan status expired before it could be confirmed.", locale: locale)
        case .scanFailed:
            String(localized: "The filesystem scan failed.", locale: locale)
        case .snapshotRejected:
            String(localized: "The scan finished, but its snapshot was rejected.", locale: locale)
        case .outcomeUnknown:
            String(localized: "The scan outcome could not be confirmed.", locale: locale)
        case .invalidResponse:
            String(localized: "The storage engine returned an invalid scan response.", locale: locale)
        case .unexpected:
            String(localized: "An unexpected scan error occurred.", locale: locale)
        }
    }

    private static func retainedResultsText(
        hasPrevious: Bool,
        locale: Locale
    ) -> String {
        if hasPrevious {
            return String(
                localized: "Last confirmed scan results remain available.",
                locale: locale
            )
        }
        return String(localized: "No confirmed scan results are available.", locale: locale)
    }
}
