import Foundation

enum ExplorerScanStyle: Equatable, Sendable {
    case progress
    case success
    case cancelled
    case failure
}

struct ExplorerScanPresentation: Equatable, Sendable {
    let style: ExplorerScanStyle
    let title: String
    let detail: String?
    let progressAccessibilityValue: String?
    let showsIndeterminateProgress: Bool
}

extension ExplorerScanPresentation {
    static func make(
        scanState: AppScanState,
        now: Date = .now,
        locale: Locale = .current
    ) -> Self? {
        switch scanState.phase {
        case .idle:
            return nil
        case .queued:
            return progress(
                title: scopeTitle(
                    state: scanState,
                    home: "Waiting to scan Home…",
                    subtree: "Waiting to refresh",
                    locale: locale
                ),
                facts: nil,
                scope: scanState.scope,
                locale: locale
            )
        case let .scanning(facts):
            return progress(
                title: scopeTitle(
                    state: scanState,
                    home: "Scanning Home…",
                    subtree: "Refreshing",
                    locale: locale
                ),
                facts: facts,
                scope: scanState.scope,
                locale: locale
            )
        case let .finalizing(facts):
            return progress(
                title: scopeTitle(
                    state: scanState,
                    home: "Preparing Home scan results…",
                    subtree: "Preparing refreshed snapshot for",
                    locale: locale
                ),
                facts: facts,
                scope: scanState.scope,
                locale: locale
            )
        case let .evaluating(facts):
            return progress(
                title: scopeTitle(
                    state: scanState,
                    home: "Classifying Home scan results…",
                    subtree: "Classifying refreshed snapshot for",
                    locale: locale
                ),
                facts: facts,
                scope: scanState.scope,
                locale: locale
            )
        case let .cancellationRequested(facts):
            return progress(
                title: scopeTitle(
                    state: scanState,
                    home: "Stopping Home scan…",
                    subtree: "Stopping refresh of",
                    locale: locale
                ),
                facts: facts,
                scope: scanState.scope,
                locale: locale
            )
        case let .succeeded(summary):
            return Self(
                style: .success,
                title: scopeTitle(
                    state: scanState,
                    home: "Home scan finished",
                    subtree: "Refreshed snapshot ready for",
                    locale: locale
                ),
                detail: [
                    completedText(at: summary.completedAt, now: now, locale: locale),
                    factsText(summary.progress, scope: scanState.scope, locale: locale),
                ].joined(separator: " · "),
                progressAccessibilityValue: nil,
                showsIndeterminateProgress: false
            )
        case .cancelled:
            return Self(
                style: .cancelled,
                title: scopeTitle(
                    state: scanState,
                    home: "Home scan stopped",
                    subtree: "Folder refresh stopped for",
                    locale: locale
                ),
                detail: retainedResultText(
                    hasPrevious: scanState.lastSuccessful != nil,
                    locale: locale
                ),
                progressAccessibilityValue: nil,
                showsIndeterminateProgress: false
            )
        case let .failed(failure):
            return Self(
                style: .failure,
                title: scopeTitle(
                    state: scanState,
                    home: "Home scan couldn’t finish",
                    subtree: "Folder refresh couldn’t finish for",
                    locale: locale
                ),
                detail: [
                    failureDetail(failure, locale: locale),
                    retainedResultText(
                        hasPrevious: scanState.lastSuccessful != nil,
                        locale: locale
                    ),
                ].joined(separator: " "),
                progressAccessibilityValue: nil,
                showsIndeterminateProgress: false
            )
        }
    }

    private static func progress(
        title: String,
        facts: ScanProgressFacts?,
        scope: AppScanScope?,
        locale: Locale
    ) -> Self {
        let detail = facts.map { factsText($0, scope: scope, locale: locale) }
        return Self(
            style: .progress,
            title: title,
            detail: detail,
            progressAccessibilityValue: detail,
            showsIndeterminateProgress: true
        )
    }

    private static func factsText(
        _ facts: ScanProgressFacts,
        scope: AppScanScope? = .home,
        locale: Locale
    ) -> String {
        let files = facts.files.formatted(.number.locale(locale))
        let directories = facts.directories.formatted(.number.locale(locale))
        let allocation = facts.knownAllocatedBytes.map { bytes in
            let value = MenuBarCapacityFormatter.gib(bytes, locale: locale)
            return scope?.isHome == false
                ? String(localized: "\(value) allocated in selected folder", locale: locale)
                : String(localized: "\(value) allocated in Home", locale: locale)
        } ?? (scope?.isHome == false
            ? String(localized: "Selected-folder allocation unavailable", locale: locale)
            : String(localized: "Home allocation unavailable", locale: locale))
        let issues = facts.issueCount.formatted(.number.locale(locale))
        return String(
            localized: "\(files) files · \(directories) folders · \(allocation) · \(issues) scan issues",
            locale: locale
        )
    }

    private static func scopeTitle(
        state: AppScanState,
        home: String.LocalizationValue,
        subtree: String.LocalizationValue,
        locale: Locale
    ) -> String {
        switch state.scope {
        case let .subtree(displayName):
            return "\(String(localized: subtree, locale: locale)) “\(displayName)”"
        case let .startupVolume(displayName):
            let phaseTitle: String.LocalizationValue = switch state.phase {
            case .queued: "Waiting to scan disk"
            case .scanning: "Scanning disk"
            case .finalizing: "Preparing disk scan results"
            case .evaluating: "Classifying disk scan results"
            case .cancellationRequested: "Stopping disk scan"
            case .succeeded: "Disk scan finished"
            case .cancelled: "Disk scan stopped"
            case .failed: "Disk scan couldn’t finish"
            case .idle: home
            }
            return "\(String(localized: phaseTitle, locale: locale)) · \(displayName)"
        case .home, nil:
            return String(localized: home, locale: locale)
        }
    }

    private static func completedText(
        at date: Date,
        now: Date,
        locale: Locale
    ) -> String {
        let relative: String
        if now.timeIntervalSince(date) < 60 {
            relative = String(localized: "just now", locale: locale)
        } else {
            let formatter = RelativeDateTimeFormatter()
            formatter.locale = locale
            formatter.unitsStyle = .full
            relative = formatter.localizedString(for: date, relativeTo: now)
        }
        return String(localized: "Completed \(relative)", locale: locale)
    }

    private static func retainedResultText(
        hasPrevious: Bool,
        locale: Locale
    ) -> String {
        if hasPrevious {
            return String(
                localized: "Last confirmed Home scan aggregate remains available.",
                locale: locale
            )
        }
        return String(
            localized: "No confirmed Home scan aggregate is loaded.",
            locale: locale
        )
    }

    private static func failureDetail(
        _ failure: AppScanFailure,
        locale: Locale
    ) -> String {
        switch failure {
        case .closed:
            String(localized: "The storage engine is unavailable.", locale: locale)
        case .busy:
            String(localized: "Another scan is already using this location.", locale: locale)
        case .rootUnavailable:
            String(localized: "The Home folder could not be scanned safely.", locale: locale)
        case .storageUnavailable:
            String(localized: "The latest Home scan could not be stored.", locale: locale)
        case .incompatibleStorage:
            String(
                localized: "This scan requires a newer DUX storage format.",
                locale: locale
            )
        case .taskExpired:
            String(
                localized: "The scan status expired before it could be confirmed.",
                locale: locale
            )
        case .scanFailed:
            String(localized: "The filesystem scan failed.", locale: locale)
        case .snapshotRejected:
            String(
                localized: "The scan finished, but its snapshot was rejected.",
                locale: locale
            )
        case .outcomeUnknown:
            String(localized: "The scan outcome could not be confirmed.", locale: locale)
        case .invalidResponse:
            String(
                localized: "The storage engine returned an invalid scan response.",
                locale: locale
            )
        case .unexpected:
            String(localized: "An unexpected scan error occurred.", locale: locale)
        }
    }
}
