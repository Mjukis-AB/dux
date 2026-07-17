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
                title: String(localized: "Waiting to scan Home…", locale: locale),
                facts: nil,
                locale: locale
            )
        case let .scanning(facts):
            return progress(
                title: String(localized: "Scanning Home…", locale: locale),
                facts: facts,
                locale: locale
            )
        case let .finalizing(facts):
            return progress(
                title: String(localized: "Preparing Home scan results…", locale: locale),
                facts: facts,
                locale: locale
            )
        case let .evaluating(facts):
            return progress(
                title: String(localized: "Classifying Home scan results…", locale: locale),
                facts: facts,
                locale: locale
            )
        case let .cancellationRequested(facts):
            return progress(
                title: String(localized: "Stopping Home scan…", locale: locale),
                facts: facts,
                locale: locale
            )
        case let .succeeded(summary):
            return Self(
                style: .success,
                title: String(localized: "Home scan finished", locale: locale),
                detail: [
                    completedText(at: summary.completedAt, now: now, locale: locale),
                    factsText(summary.progress, locale: locale),
                ].joined(separator: " · "),
                progressAccessibilityValue: nil,
                showsIndeterminateProgress: false
            )
        case .cancelled:
            return Self(
                style: .cancelled,
                title: String(localized: "Home scan stopped", locale: locale),
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
                title: String(localized: "Home scan couldn’t finish", locale: locale),
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
        locale: Locale
    ) -> Self {
        let detail = facts.map { factsText($0, locale: locale) }
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
        locale: Locale
    ) -> String {
        let files = facts.files.formatted(.number.locale(locale))
        let directories = facts.directories.formatted(.number.locale(locale))
        let allocation = facts.knownAllocatedBytes.map {
            String(
                localized: "\(MenuBarCapacityFormatter.gib($0, locale: locale)) allocated in Home",
                locale: locale
            )
        } ?? String(localized: "Home allocation unavailable", locale: locale)
        let issues = facts.issueCount.formatted(.number.locale(locale))
        return String(
            localized: "\(files) files · \(directories) folders · \(allocation) · \(issues) scan issues",
            locale: locale
        )
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
