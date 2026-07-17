import Foundation

enum ExplorerAccessibility {
    static let root = "explorer"
    static let sidebar = "explorer-sidebar"
    static let overviewDestination = "explorer-destination-overview"
    static let settingsShortcut = "explorer-settings-shortcut"
    static let capacityCard = "explorer-capacity-card"
    static let capacityBar = "explorer-capacity-bar"
    static let available = "explorer-capacity-available"
    static let used = "explorer-capacity-used"
    static let total = "explorer-capacity-total"
    static let pressure = "explorer-pressure"
    static let freshness = "explorer-capacity-freshness"
    static let scanCard = "explorer-scan-card"
    static let scanStatus = "explorer-scan-status"
    static let coverage = "explorer-coverage"
    static let refreshCapacity = "explorer-refresh-capacity"
    static let scanNow = "explorer-scan-now"
    static let cancelScan = "explorer-cancel-scan"

    static let allIdentifiers = [
        root,
        sidebar,
        overviewDestination,
        settingsShortcut,
        capacityCard,
        capacityBar,
        available,
        used,
        total,
        pressure,
        freshness,
        scanCard,
        scanStatus,
        coverage,
        refreshCapacity,
        scanNow,
        cancelScan,
    ]
}

enum ExplorerDestination: String, CaseIterable, Identifiable, Sendable {
    case overview

    var id: Self { self }
}

enum ExplorerKeyboardShortcut {
    static let scanNow: Character = "r"
    static let cancelScan: Character = "."
    static let settings: Character = ","

    static let allKeys = [scanNow, cancelScan, settings]
}

enum ExplorerCapacityStatus: Equatable, Sendable {
    case refreshing(message: String)
    case stale(message: String)
}

enum ExplorerCapacityBreakdown: Equatable, Sendable {
    case known(usedFraction: Double)
    case unavailable(message: String)
}

struct ExplorerCapacitySnapshotPresentation: Equatable, Sendable {
    let volumeName: String
    let pressure: DiskPressureLevel
    let pressureTitle: String
    let availableValue: String
    let usedValue: String
    let totalValue: String
    let availabilityBasis: String
    let freshness: String
    let breakdown: ExplorerCapacityBreakdown
    let accessibilitySummary: String
}

enum ExplorerCapacityPresentation: Equatable, Sendable {
    case loading(message: String)
    case snapshot(
        ExplorerCapacitySnapshotPresentation,
        status: ExplorerCapacityStatus?
    )
    case failed(title: String, detail: String)
}

struct ExplorerCoveragePresentation: Equatable, Sendable {
    let coverage: AppScanCoverage
    let title: String
    let detail: String
}

struct ExplorerActionPresentation: Equatable, Sendable {
    let refreshCapacityEnabled: Bool
    let showScanNow: Bool
    let scanNowEnabled: Bool
    let showCancelScan: Bool
    let cancelScanEnabled: Bool
}

struct ExplorerPresentation: Equatable, Sendable {
    let capacity: ExplorerCapacityPresentation
    let coverage: ExplorerCoveragePresentation
    let scan: ExplorerScanPresentation?
    let actions: ExplorerActionPresentation
}

extension ExplorerPresentation {
    static func make(
        volumeState: VolumeCapacityState,
        scanState: AppScanState,
        now: Date = .now,
        locale: Locale = .current
    ) -> Self {
        return Self(
            capacity: capacityPresentation(
                for: volumeState,
                now: now,
                locale: locale
            ),
            coverage: coveragePresentation(for: scanState, locale: locale),
            scan: ExplorerScanPresentation.make(
                scanState: scanState,
                now: now,
                locale: locale
            ),
            actions: actionPresentation(
                volumeState: volumeState,
                scanState: scanState
            )
        )
    }

    private static func capacityPresentation(
        for state: VolumeCapacityState,
        now: Date,
        locale: Locale
    ) -> ExplorerCapacityPresentation {
        switch state {
        case .idle, .loading:
            return .loading(
                message: String(localized: "Checking startup disk…", locale: locale)
            )
        case let .loaded(snapshot):
            return .snapshot(
                snapshotPresentation(snapshot, now: now, locale: locale),
                status: nil
            )
        case let .refreshing(snapshot):
            return .snapshot(
                snapshotPresentation(snapshot, now: now, locale: locale),
                status: .refreshing(
                    message: String(localized: "Updating capacity…", locale: locale)
                )
            )
        case let .stale(snapshot, failure):
            return .snapshot(
                snapshotPresentation(snapshot, now: now, locale: locale),
                status: .stale(message: staleMessage(for: failure, locale: locale))
            )
        case let .failed(failure):
            return .failed(
                title: String(localized: "Storage capacity unavailable", locale: locale),
                detail: capacityFailureDetail(for: failure, locale: locale)
            )
        }
    }

    private static func snapshotPresentation(
        _ snapshot: VolumeCapacitySnapshot,
        now: Date,
        locale: Locale
    ) -> ExplorerCapacitySnapshotPresentation {
        let volumeName = snapshot.displayName ?? String(
            localized: "Startup Disk",
            locale: locale
        )
        let availableValue = MenuBarCapacityFormatter.gib(
            snapshot.effectiveAvailableBytes,
            locale: locale
        )
        let totalValue = MenuBarCapacityFormatter.gib(snapshot.totalBytes, locale: locale)
        let usedValue = snapshot.usedBytes.map {
            MenuBarCapacityFormatter.gib($0, locale: locale)
        } ?? String(localized: "Unavailable", locale: locale)
        let availabilityBasis = switch snapshot.availabilityBasis {
        case .importantUsage:
            String(localized: "Available for important use", locale: locale)
        case .filesystemAvailable:
            String(localized: "Filesystem available", locale: locale)
        }
        let pressureTitle = pressureTitle(for: snapshot.pressure, locale: locale)
        let freshness = String(
            localized: "Updated \(relativeText(for: snapshot.sampledAt, now: now, locale: locale))",
            locale: locale
        )
        let breakdown: ExplorerCapacityBreakdown = if let usedFraction = snapshot.usedFraction {
            .known(usedFraction: min(max(usedFraction, 0), 1))
        } else {
            .unavailable(
                message: String(
                    localized: "Used-space breakdown is unavailable from this capacity sample.",
                    locale: locale
                )
            )
        }
        let availableSummary = String(
            localized: "\(availableValue) available",
            locale: locale
        )
        let summaryParts = [
            volumeName,
            pressureTitle,
            availableSummary,
            String(localized: "\(usedValue) used", locale: locale),
            String(localized: "\(totalValue) total", locale: locale),
            availabilityBasis,
            freshness,
        ]
        let summary = if snapshot.pressure == .critical {
            ([availableSummary] + summaryParts.filter { $0 != availableSummary })
                .joined(separator: ". ")
        } else {
            summaryParts.joined(separator: ". ")
        }
        return ExplorerCapacitySnapshotPresentation(
            volumeName: volumeName,
            pressure: snapshot.pressure,
            pressureTitle: pressureTitle,
            availableValue: availableValue,
            usedValue: usedValue,
            totalValue: totalValue,
            availabilityBasis: availabilityBasis,
            freshness: freshness,
            breakdown: breakdown,
            accessibilitySummary: summary
        )
    }

    private static func coveragePresentation(
        for state: AppScanState,
        locale: Locale
    ) -> ExplorerCoveragePresentation {
        let summary: AppScanSummary? = switch state.phase {
        case let .succeeded(summary): summary
        case .idle, .queued, .scanning, .finalizing, .evaluating,
             .cancellationRequested, .cancelled, .failed:
            state.lastSuccessful
        }
        guard let summary else {
            return ExplorerCoveragePresentation(
                coverage: .unknown,
                title: String(localized: "No Home scan loaded", locale: locale),
                detail: String(
                    localized: "Run a Home scan to load current aggregate coverage in this window without changing files.",
                    locale: locale
                )
            )
        }

        let issueCount = summary.progress.issueCount.formatted(.number.locale(locale))
        let reportedCoverage = summary.coveragePermille.map {
            (Double($0) / 1_000).formatted(
                .percent.locale(locale).precision(.fractionLength(0 ... 1))
            )
        }
        func detail(_ base: String) -> String {
            guard let reportedCoverage else {
                return base
            }
            return [
                base,
                String(
                    localized: "Home coverage reported as \(reportedCoverage).",
                    locale: locale
                ),
            ].joined(separator: " ")
        }
        switch summary.coverage {
        case .complete:
            return ExplorerCoveragePresentation(
                coverage: .complete,
                title: String(localized: "Complete Home coverage", locale: locale),
                detail: detail(
                    String(
                        localized: "The latest confirmed Home scan completed with \(issueCount) access issues.",
                        locale: locale
                    )
                )
            )
        case .limitedAccess:
            return ExplorerCoveragePresentation(
                coverage: .limitedAccess,
                title: String(localized: "Limited Home access", locale: locale),
                detail: detail(
                    String(
                        localized: "The latest confirmed Home scan observed \(issueCount) access issues.",
                        locale: locale
                    )
                )
            )
        case .partial:
            return ExplorerCoveragePresentation(
                coverage: .partial,
                title: String(localized: "Partial Home coverage", locale: locale),
                detail: detail(
                    String(
                        localized: "The latest confirmed Home scan observed \(issueCount) access issues and did not cover every readable item.",
                        locale: locale
                    )
                )
            )
        case .unknown:
            return ExplorerCoveragePresentation(
                coverage: .unknown,
                title: String(localized: "Home coverage unavailable", locale: locale),
                detail: detail(
                    String(
                        localized: "The latest confirmed Home scan did not report a reliable coverage classification and observed \(issueCount) access issues.",
                        locale: locale
                    )
                )
            )
        }
    }

    private static func actionPresentation(
        volumeState: VolumeCapacityState,
        scanState: AppScanState
    ) -> ExplorerActionPresentation {
        let refreshEnabled = switch volumeState {
        case .idle, .loading, .refreshing: false
        case .loaded, .stale, .failed: true
        }
        switch scanState.phase {
        case .queued, .scanning, .finalizing, .evaluating:
            return ExplorerActionPresentation(
                refreshCapacityEnabled: refreshEnabled,
                showScanNow: false,
                scanNowEnabled: false,
                showCancelScan: true,
                cancelScanEnabled: true
            )
        case .cancellationRequested:
            return ExplorerActionPresentation(
                refreshCapacityEnabled: refreshEnabled,
                showScanNow: false,
                scanNowEnabled: false,
                showCancelScan: true,
                cancelScanEnabled: false
            )
        case .idle, .succeeded, .cancelled, .failed:
            return ExplorerActionPresentation(
                refreshCapacityEnabled: refreshEnabled,
                showScanNow: true,
                scanNowEnabled: true,
                showCancelScan: false,
                cancelScanEnabled: false
            )
        }
    }

    private static func relativeText(
        for date: Date,
        now: Date,
        locale: Locale
    ) -> String {
        guard now.timeIntervalSince(date) >= 60 else {
            return String(localized: "just now", locale: locale)
        }
        let formatter = RelativeDateTimeFormatter()
        formatter.locale = locale
        formatter.unitsStyle = .full
        return formatter.localizedString(for: date, relativeTo: now)
    }

    private static func staleMessage(
        for failure: VolumeCapacityFailure,
        locale: Locale
    ) -> String {
        switch failure {
        case .unavailable:
            String(
                localized: "The latest capacity check was unavailable. Showing the last confirmed sample.",
                locale: locale
            )
        case .invalidObservation:
            String(
                localized: "The latest capacity check was invalid. Showing the last confirmed sample.",
                locale: locale
            )
        case .engineUnavailable:
            String(
                localized: "The storage engine could not classify the latest capacity. Showing the last confirmed sample.",
                locale: locale
            )
        }
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
                localized: "The storage engine is unavailable.",
                locale: locale
            )
        }
    }
}
