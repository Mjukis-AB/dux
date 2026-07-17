import Foundation

enum NotificationAuthorizationStatus: Equatable, Sendable {
    case notDetermined
    case denied
    case authorized
    case provisional
    case unknown

    var isSettled: Bool {
        switch self {
        case .denied, .authorized, .provisional: true
        case .notDetermined, .unknown: false
        }
    }
}

enum NotificationAuthorizationActivity: Equatable, Sendable {
    case loading
    case requesting
}

enum NotificationAuthorizationFailureReason: Equatable, Sendable {
    case denied
    case outcomeUnknown
    case unexpected
}

struct NotificationAuthorizationState: Equatable, Sendable {
    var status: NotificationAuthorizationStatus?
    var activity: NotificationAuthorizationActivity?
    var failure: NotificationAuthorizationFailureReason?

    static let idle = Self(status: nil, activity: nil, failure: nil)
}

enum NotificationAuthorizationAccessibility {
    static let status = "notification-permission-status"
    static let progress = "notification-permission-progress"
    static let error = "notification-permission-error"
    static let request = "notification-permission-request"
    static let openSystemSettings = "notification-permission-open-settings"
    static let refresh = "notification-permission-refresh"

    static let allIdentifiers = [
        status,
        progress,
        error,
        request,
        openSystemSettings,
        refresh,
    ]
}

struct NotificationAuthorizationPresentation: Equatable, Sendable {
    let statusTitle: String
    let statusDetail: String
    let progressLabel: String?
    let showsRequestAction: Bool
    let showsSystemSettingsAction: Bool
    let showsRefreshAction: Bool
    let errorMessage: String?
}

extension NotificationAuthorizationPresentation {
    static func make(
        state: NotificationAuthorizationState,
        locale: Locale = .current
    ) -> Self {
        let status = statusCopy(for: state.status, locale: locale)
        return Self(
            statusTitle: status.title,
            statusDetail: status.detail,
            progressLabel: progressCopy(for: state.activity, locale: locale),
            showsRequestAction: state.activity == nil
                && state.status == .notDetermined,
            showsSystemSettingsAction: state.activity == nil
                && state.status != nil
                && state.status != .notDetermined
                && state.status != .unknown,
            showsRefreshAction: state.activity == nil
                && (state.status == .denied || state.status == .unknown),
            errorMessage: failureCopy(for: state.failure, locale: locale)
        )
    }

    private static func statusCopy(
        for status: NotificationAuthorizationStatus?,
        locale: Locale
    ) -> (title: String, detail: String) {
        switch status {
        case .notDetermined:
            (
                String(localized: "Not requested", locale: locale),
                String(
                    localized: "DUX asks only when you choose Allow notifications.",
                    locale: locale
                )
            )
        case .denied:
            (
                String(localized: "Blocked by macOS", locale: locale),
                String(
                    localized:
                        "Allow DUX in System Settings › Notifications if you want low-disk alerts.",
                    locale: locale
                )
            )
        case .authorized:
            (
                String(localized: "Allowed by macOS", locale: locale),
                String(
                    localized:
                        "DUX may show low-disk alerts. No alerts are scheduled by this version of DUX.",
                    locale: locale
                )
            )
        case .provisional:
            (
                String(localized: "Delivered quietly", locale: locale),
                String(
                    localized:
                        "macOS may deliver DUX alerts quietly. No alerts are scheduled by this version of DUX.",
                    locale: locale
                )
            )
        case .unknown:
            (
                String(localized: "Notification permission unknown", locale: locale),
                String(
                    localized:
                        "This macOS version reported an unrecognized notification permission.",
                    locale: locale
                )
            )
        case nil:
            (
                String(localized: "Checking…", locale: locale),
                String(
                    localized: "Reading the current notification permission from macOS.",
                    locale: locale
                )
            )
        }
    }

    private static func progressCopy(
        for activity: NotificationAuthorizationActivity?,
        locale: Locale
    ) -> String? {
        switch activity {
        case .loading:
            String(localized: "Checking notification permission", locale: locale)
        case .requesting:
            String(localized: "Waiting for your notification choice", locale: locale)
        case nil:
            nil
        }
    }

    private static func failureCopy(
        for failure: NotificationAuthorizationFailureReason?,
        locale: Locale
    ) -> String? {
        switch failure {
        case .denied:
            String(
                localized:
                    "macOS did not allow notifications. The last confirmed permission is shown.",
                locale: locale
            )
        case .outcomeUnknown:
            String(
                localized: "macOS did not confirm the notification permission change.",
                locale: locale
            )
        case .unexpected:
            String(
                localized:
                    "Notification permission could not be changed. The last confirmed permission is shown.",
                locale: locale
            )
        case nil:
            nil
        }
    }
}
