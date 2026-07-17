import ServiceManagement

enum LoginItemServiceError: Error, Equatable, Sendable {
    case invalidSignature
    case denied
    case serviceUnavailable
    case unexpected
}

protocol LoginItemServing: Sendable {
    func status() async -> LoginItemSystemStatus
    func register() async throws
    func unregister() async throws
}

actor LoginItemService: LoginItemServing {
    func status() -> LoginItemSystemStatus {
        Self.map(SMAppService.mainApp.status)
    }

    func register() throws {
        do {
            try SMAppService.mainApp.register()
        } catch {
            throw Self.map(error)
        }
    }

    func unregister() async throws {
        do {
            try await SMAppService.mainApp.unregister()
        } catch {
            throw Self.map(error)
        }
    }

    nonisolated static func map(
        _ status: SMAppService.Status
    ) -> LoginItemSystemStatus {
        switch status {
        case .notRegistered: .notRegistered
        case .enabled: .enabled
        case .requiresApproval: .requiresApproval
        case .notFound: .notFound
        @unknown default: .unknown
        }
    }

    nonisolated static func map(_ error: Error) -> LoginItemServiceError {
        let error = error as NSError
        // The public constant is macOS 15+, while SMAppService itself is
        // macOS 13+. Keep the stable domain spelling for the deployment floor.
        guard error.domain == "SMAppServiceErrorDomain" else {
            return .unexpected
        }

        return switch error.code {
        case kSMErrorInvalidSignature:
            .invalidSignature
        case kSMErrorLaunchDeniedByUser, kSMErrorAuthorizationFailure:
            .denied
        case kSMErrorServiceUnavailable:
            .serviceUnavailable
        default:
            .unexpected
        }
    }
}
