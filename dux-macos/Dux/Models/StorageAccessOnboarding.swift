import Foundation

@MainActor
protocol StorageAccessIntroductionPreferenceStoring {
    func loadAcknowledged() -> Bool
    func saveAcknowledged()
}

@MainActor
struct UserDefaultsStorageAccessIntroductionPreferenceStore:
    StorageAccessIntroductionPreferenceStoring
{
    static let key = "storageAccess.introduction.v1"
    private static let acknowledgedValue = "1|acknowledged"

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    func loadAcknowledged() -> Bool {
        defaults.string(forKey: Self.key) == Self.acknowledgedValue
    }

    func saveAcknowledged() {
        defaults.set(Self.acknowledgedValue, forKey: Self.key)
    }
}

struct StorageAccessEvidence: Equatable, Sendable {
    static let sampledLocationCount: UInt8 = 3

    let readableLocationCount: UInt8
    let unreadableLocationCount: UInt8
    let unobservedLocationCount: UInt8
    let observedAt: Date

    init?(
        readableLocationCount: UInt8,
        unreadableLocationCount: UInt8,
        unobservedLocationCount: UInt8,
        observedAt: Date
    ) {
        let total = UInt16(readableLocationCount)
            + UInt16(unreadableLocationCount)
            + UInt16(unobservedLocationCount)
        guard total == UInt16(Self.sampledLocationCount),
              observedAt.timeIntervalSince1970.isFinite,
              observedAt.timeIntervalSince1970 >= 0 else {
            return nil
        }
        self.readableLocationCount = readableLocationCount
        self.unreadableLocationCount = unreadableLocationCount
        self.unobservedLocationCount = unobservedLocationCount
        self.observedAt = observedAt
    }
}

enum StorageAccessProbeState: Equatable, Sendable {
    case idle
    case checking(previous: StorageAccessEvidence?)
    case observed(StorageAccessEvidence)
    case failed(previous: StorageAccessEvidence?)

    var evidence: StorageAccessEvidence? {
        switch self {
        case let .checking(previous), let .failed(previous): previous
        case let .observed(evidence): evidence
        case .idle: nil
        }
    }
}

enum StorageAccessAccessibility {
    static let introduction = "storage-access-introduction"
    static let introductionContinue = "storage-access-introduction-continue"
    static let broaderAnalysis = "storage-access-broader-analysis"
    static let guidance = "storage-access-guidance"
    static let probeStatus = "storage-access-probe-status"
    static let openSystemSettings = "storage-access-open-settings"
    static let refresh = "storage-access-refresh"

    static let allIdentifiers = [
        introduction,
        introductionContinue,
        broaderAnalysis,
        guidance,
        probeStatus,
        openSystemSettings,
        refresh,
    ]
}

struct StorageAccessOnboardingPresentation: Equatable, Sendable {
    let showsBroaderAnalysisAction: Bool
    let showsGuidance: Bool
    let statusTitle: String?
    let statusDetail: String?
    let showsProgress: Bool
    let showsSystemSettingsAction: Bool
    let showsRefreshAction: Bool
}

extension StorageAccessOnboardingPresentation {
    static func make(
        scanState: AppScanState,
        broaderAnalysisRequested: Bool,
        probeState: StorageAccessProbeState,
        locale: Locale = .current
    ) -> Self {
        guard broaderAnalysisRequested else {
            return Self(
                showsBroaderAnalysisAction: latestSummary(in: scanState).map {
                    $0.coverage != .complete
                } ?? false,
                showsGuidance: false,
                statusTitle: nil,
                statusDetail: nil,
                showsProgress: false,
                showsSystemSettingsAction: false,
                showsRefreshAction: false
            )
        }

        let status = statusCopy(for: probeState, locale: locale)
        return Self(
            showsBroaderAnalysisAction: false,
            showsGuidance: true,
            statusTitle: status.title,
            statusDetail: status.detail,
            showsProgress: status.showsProgress,
            showsSystemSettingsAction: true,
            showsRefreshAction: !status.showsProgress
        )
    }

    private static func latestSummary(in state: AppScanState) -> AppScanSummary? {
        if case let .succeeded(summary) = state.phase {
            return summary
        }
        return state.lastSuccessful
    }

    private static func statusCopy(
        for state: StorageAccessProbeState,
        locale: Locale
    ) -> (title: String, detail: String, showsProgress: Bool) {
        switch state {
        case .idle:
            return (
                String(localized: "Access not checked", locale: locale),
                String(
                    localized:
                        "DUX has not sampled broader Library access in this session.",
                    locale: locale
                ),
                false
            )
        case let .checking(previous):
            return (
                String(localized: "Checking observed access…", locale: locale),
                previous.map { evidenceDetail($0, locale: locale) }
                    ?? String(
                        localized:
                            "Sampling three fixed Library locations without reading file contents.",
                        locale: locale
                    ),
                true
            )
        case let .observed(evidence):
            return (
                evidenceTitle(evidence, locale: locale),
                evidenceDetail(evidence, locale: locale),
                false
            )
        case let .failed(previous):
            return (
                String(localized: "Access check couldn’t finish", locale: locale),
                previous.map {
                    String(
                        localized:
                            "The latest check failed. Last observation: \(evidenceDetail($0, locale: locale))",
                        locale: locale
                    )
                } ?? String(
                    localized:
                        "DUX could not sample broader Library access. No permission status was inferred.",
                    locale: locale
                ),
                false
            )
        }
    }

    private static func evidenceTitle(
        _ evidence: StorageAccessEvidence,
        locale: Locale
    ) -> String {
        if evidence.unreadableLocationCount > 0 {
            return String(
                localized: "Some broader locations were unreadable",
                locale: locale
            )
        }
        if evidence.unobservedLocationCount > 0 {
            return String(localized: "Broader access remains uncertain", locale: locale)
        }
        return String(localized: "Sampled locations were readable", locale: locale)
    }

    private static func evidenceDetail(
        _ evidence: StorageAccessEvidence,
        locale: Locale
    ) -> String {
        let readable = evidence.readableLocationCount.formatted(.number.locale(locale))
        let unreadable = evidence.unreadableLocationCount.formatted(.number.locale(locale))
        let unobserved = evidence.unobservedLocationCount.formatted(.number.locale(locale))
        return String(
            localized:
                "Observed \(readable) readable, \(unreadable) unreadable, and \(unobserved) unobserved sampled Library locations. macOS does not expose an authoritative Full Disk Access status.",
            locale: locale
        )
    }
}
