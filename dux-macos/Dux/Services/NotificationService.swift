import UserNotifications

enum NotificationServiceError: Error, Equatable, Sendable {
    case denied
    case unexpected
}

protocol NotificationServing: Sendable {
    func authorizationStatus() async -> NotificationAuthorizationStatus
    func requestAuthorization() async throws
}

protocol UserNotificationCenterClient: Sendable {
    func authorizationStatus() async -> NotificationAuthorizationStatus
    func requestAuthorization(options: UNAuthorizationOptions) async throws
}

actor NotificationService: NotificationServing {
    nonisolated static let authorizationOptions: UNAuthorizationOptions = [
        .alert,
        .sound,
    ]

    private let center: any UserNotificationCenterClient

    init(center: (any UserNotificationCenterClient)? = nil) {
        self.center = center ?? SystemUserNotificationCenterClient()
    }

    func authorizationStatus() async -> NotificationAuthorizationStatus {
        await center.authorizationStatus()
    }

    func requestAuthorization() async throws {
        guard await center.authorizationStatus() == .notDetermined else {
            return
        }
        try await center.requestAuthorization(options: Self.authorizationOptions)
    }

    nonisolated static func map(
        _ status: UNAuthorizationStatus
    ) -> NotificationAuthorizationStatus {
        switch status {
        case .notDetermined: .notDetermined
        case .denied: .denied
        case .authorized: .authorized
        case .provisional: .provisional
        @unknown default: .unknown
        }
    }

    nonisolated static func map(_ error: Error) -> NotificationServiceError {
        let error = error as NSError
        guard error.domain == UNErrorDomain else {
            return .unexpected
        }
        return error.code == UNError.notificationsNotAllowed.rawValue
            ? .denied
            : .unexpected
    }
}

private actor SystemUserNotificationCenterClient: UserNotificationCenterClient {
    func authorizationStatus() async -> NotificationAuthorizationStatus {
        await withCheckedContinuation { continuation in
            UNUserNotificationCenter.current().getNotificationSettings { settings in
                continuation.resume(
                    returning: NotificationService.map(settings.authorizationStatus)
                )
            }
        }
    }

    func requestAuthorization(options: UNAuthorizationOptions) async throws {
        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Void, Error>) in
            UNUserNotificationCenter.current().requestAuthorization(
                options: options
            ) { _, error in
                if let error {
                    continuation.resume(throwing: NotificationService.map(error))
                } else {
                    continuation.resume(returning: ())
                }
            }
        }
    }
}
