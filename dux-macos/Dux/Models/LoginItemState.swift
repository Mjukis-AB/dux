import Foundation

enum LoginItemSystemStatus: Equatable, Sendable {
    case notRegistered
    case enabled
    case requiresApproval
    case notFound
    case unknown

    var registrationRequested: Bool {
        switch self {
        case .enabled, .requiresApproval: true
        case .notRegistered, .notFound, .unknown: false
        }
    }
}

enum LoginItemActivity: Equatable, Sendable {
    case loading
    case registering
    case unregistering
}

enum LoginItemFailureReason: Equatable, Sendable {
    case invalidSignature
    case denied
    case serviceUnavailable
    case outcomeUnknown
    case unexpected
}

enum LoginItemFailure: Equatable, Sendable {
    case registration(LoginItemFailureReason)
    case unregistration(LoginItemFailureReason)
}

struct LoginItemState: Equatable, Sendable {
    var status: LoginItemSystemStatus?
    var activity: LoginItemActivity?
    var failure: LoginItemFailure?

    static let idle = Self(status: nil, activity: nil, failure: nil)

    var isBusy: Bool { activity != nil }
}

enum LoginItemAccessibility {
    static let toggle = "launch-at-login-toggle"
    static let status = "launch-at-login-status"
    static let progress = "launch-at-login-progress"
    static let error = "launch-at-login-error"
    static let openSystemSettings = "launch-at-login-open-settings"
    static let refresh = "launch-at-login-refresh"

    static let allIdentifiers = [
        toggle,
        status,
        progress,
        error,
        openSystemSettings,
        refresh,
    ]
}

struct LoginItemPresentation: Equatable, Sendable {
    let toggleOn: Bool
    let toggleEnabled: Bool
    let statusTitle: String
    let statusDetail: String
    let progressLabel: String?
    let showsApprovalAction: Bool
    let showsRefreshAction: Bool
    let errorMessage: String?
}

extension LoginItemPresentation {
    static func make(
        state: LoginItemState,
        locale: Locale = .current
    ) -> Self {
        let status = statusCopy(for: state.status, locale: locale)
        return Self(
            toggleOn: state.status?.registrationRequested ?? false,
            toggleEnabled: state.activity == nil
                && state.status != nil
                && state.status != .notFound
                && state.status != .unknown,
            statusTitle: status.title,
            statusDetail: status.detail,
            progressLabel: progressCopy(for: state.activity, locale: locale),
            showsApprovalAction: state.activity == nil
                && state.status == .requiresApproval,
            showsRefreshAction: state.activity == nil
                && (state.status == .notFound || state.status == .unknown),
            errorMessage: failureCopy(for: state.failure, locale: locale)
        )
    }

    private static func statusCopy(
        for status: LoginItemSystemStatus?,
        locale: Locale
    ) -> (title: String, detail: String) {
        switch status {
        case .notRegistered:
            (
                String(localized: "Off", locale: locale),
                String(localized: "DUX starts only when you open it.", locale: locale)
            )
        case .enabled:
            (
                String(localized: "Enabled by macOS", locale: locale),
                String(
                    localized: "DUX will open as a menu bar app after you sign in.",
                    locale: locale
                )
            )
        case .requiresApproval:
            (
                String(localized: "Approval required", locale: locale),
                String(
                    localized: "DUX is registered, but macOS will not launch it until you allow it in Login Items.",
                    locale: locale
                )
            )
        case .notFound:
            (
                String(localized: "Login item unavailable", locale: locale),
                String(
                    localized: "macOS could not access Launch at Login for this copy of DUX.",
                    locale: locale
                )
            )
        case .unknown:
            (
                String(localized: "Login item status unknown", locale: locale),
                String(
                    localized: "This macOS version reported an unrecognized Login Items status.",
                    locale: locale
                )
            )
        case nil:
            (
                String(localized: "Checking…", locale: locale),
                String(
                    localized: "Reading the current Login Items setting from macOS.",
                    locale: locale
                )
            )
        }
    }

    private static func progressCopy(
        for activity: LoginItemActivity?,
        locale: Locale
    ) -> String? {
        switch activity {
        case .loading:
            String(localized: "Checking Launch at Login", locale: locale)
        case .registering:
            String(localized: "Enabling Launch at Login", locale: locale)
        case .unregistering:
            String(localized: "Disabling Launch at Login", locale: locale)
        case nil:
            nil
        }
    }

    private static func failureCopy(
        for failure: LoginItemFailure?,
        locale: Locale
    ) -> String? {
        guard let failure else {
            return nil
        }
        let reason = switch failure {
        case let .registration(reason), let .unregistration(reason): reason
        }
        switch reason {
        case .invalidSignature:
            return String(
                localized: "Launch at Login requires a signed DUX app in a stable location.",
                locale: locale
            )
        case .denied:
            return String(
                localized: "macOS denied the Login Items change. The last confirmed setting is shown.",
                locale: locale
            )
        case .serviceUnavailable:
            return String(
                localized: "The macOS Login Items service is unavailable. The last confirmed setting is shown.",
                locale: locale
            )
        case .outcomeUnknown:
            return String(
                localized: "macOS did not confirm the requested Login Items change.",
                locale: locale
            )
        case .unexpected:
            return String(
                localized: "Launch at Login could not be changed. The last confirmed setting is shown.",
                locale: locale
            )
        }
    }
}
