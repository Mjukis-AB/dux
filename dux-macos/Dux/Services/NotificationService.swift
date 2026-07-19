import UserNotifications

enum NotificationServiceError: Error, Equatable, Sendable {
    case denied
    case unexpected
}

protocol NotificationServing: Sendable {
    func authorizationStatus() async -> NotificationAuthorizationStatus
    func requestAuthorization() async throws
    func deliverDiskPressure(
        _ delivery: DiskPressureNotificationDelivery
    ) async throws
}

extension NotificationServing {
    func deliverDiskPressure(
        _: DiskPressureNotificationDelivery
    ) async throws {
        throw NotificationServiceError.unexpected
    }
}

protocol UserNotificationCenterClient: Sendable {
    func authorizationStatus() async -> NotificationAuthorizationStatus
    func requestAuthorization(options: UNAuthorizationOptions) async throws
    func deliver(_ delivery: DiskPressureNotificationDelivery) async throws
}

extension UserNotificationCenterClient {
    func deliver(_: DiskPressureNotificationDelivery) async throws {}
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

    func deliverDiskPressure(
        _ delivery: DiskPressureNotificationDelivery
    ) async throws {
        do {
            try await center.deliver(delivery)
        } catch {
            throw Self.map(error)
        }
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

    func deliver(_ delivery: DiskPressureNotificationDelivery) async throws {
        let content = UNMutableNotificationContent()
        content.title = delivery.title
        content.body = delivery.body
        content.userInfo = delivery.userInfo
        let request = UNNotificationRequest(
            identifier: delivery.identifier,
            content: content,
            trigger: nil
        )
        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Void, Error>) in
            UNUserNotificationCenter.current().add(request) { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
        }
    }
}
