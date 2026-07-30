import Foundation
import Observation

protocol CLIInstallerServing: Sendable {
    func loadStatus() async throws -> CLIInstallationStatus
    func prepare(_ action: CLIInstallationAction) async throws
        -> CLIInstallationConfirmation
    func perform(_ confirmation: CLIInstallationConfirmation) async throws
        -> CLIInstallationUpdate
    func discard(_ confirmation: CLIInstallationConfirmation) async
    func close() async
}

@MainActor
@Observable
final class CLIInstallationModel {
    private(set) var state = CLIInstallationViewState.idle
    private(set) var confirmation: CLIInstallationConfirmation?

    private let service: any CLIInstallerServing

    @ObservationIgnored
    private var operationTask: Task<Void, Never>?
    @ObservationIgnored
    private var generation: UInt64 = 0
    @ObservationIgnored
    private var operationIsConfirmedMutation = false
    @ObservationIgnored
    private var shuttingDown = false

    init(service: any CLIInstallerServing = CLIInstallerService()) {
        self.service = service
    }

    func loadStatus(force: Bool = false) async {
        guard !shuttingDown else {
            return
        }
        if let operationTask {
            await operationTask.value
            return
        }
        guard force || state.status == nil || state.requiresAuthoritativeReload else {
            return
        }

        generation &+= 1
        let requestGeneration = generation
        let priorStatus = state.status
        state = CLIInstallationViewState(
            status: priorStatus,
            activity: .loading,
            failure: nil,
            requiresAuthoritativeReload: state.requiresAuthoritativeReload
        )
        let service = self.service
        let task = Task { @MainActor [weak self] in
            let result: Result<CLIInstallationStatus, CLIInstallationFailure>
            do {
                result = .success(try await service.loadStatus())
            } catch let error as CLIInstallerServiceError {
                result = .failure(.service(error))
            } catch {
                result = .failure(.unexpected)
            }
            guard
                let self,
                !self.shuttingDown,
                self.generation == requestGeneration
            else {
                return
            }
            switch result {
            case let .success(status):
                self.state = CLIInstallationViewState(
                    status: status,
                    activity: nil,
                    failure: nil,
                    requiresAuthoritativeReload: false
                )
            case let .failure(failure):
                self.state = CLIInstallationViewState(
                    status: priorStatus,
                    activity: nil,
                    failure: failure,
                    requiresAuthoritativeReload:
                        self.state.requiresAuthoritativeReload
                )
            }
            self.operationTask = nil
        }
        operationTask = task
        await task.value
    }

    func prepare(_ action: CLIInstallationAction) async {
        guard
            !shuttingDown,
            operationTask == nil,
            confirmation == nil,
            actionIsAvailable(action)
        else {
            return
        }

        generation &+= 1
        let requestGeneration = generation
        let priorStatus = state.status
        state = CLIInstallationViewState(
            status: priorStatus,
            activity: action == .uninstall ? .preparingUninstall : .preparingInstall,
            failure: nil,
            requiresAuthoritativeReload: false
        )
        let service = self.service
        let task = Task { @MainActor [weak self] in
            let result: Result<CLIInstallationConfirmation, CLIInstallationFailure>
            do {
                result = .success(try await service.prepare(action))
            } catch let error as CLIInstallerServiceError {
                result = .failure(.service(error))
            } catch {
                result = .failure(.unexpected)
            }
            guard
                let self,
                !self.shuttingDown,
                self.generation == requestGeneration
            else {
                if case let .success(confirmation) = result {
                    await service.discard(confirmation)
                }
                return
            }
            switch result {
            case let .success(confirmation):
                self.confirmation = confirmation
                self.state = CLIInstallationViewState(
                    status: priorStatus,
                    activity: nil,
                    failure: nil,
                    requiresAuthoritativeReload: false
                )
            case let .failure(failure):
                self.state = CLIInstallationViewState(
                    status: priorStatus,
                    activity: nil,
                    failure: failure,
                    requiresAuthoritativeReload: Self.requiresReload(failure)
                )
            }
            self.operationTask = nil
        }
        operationTask = task
        await task.value
    }

    func confirmPreparedAction() async {
        guard
            !shuttingDown,
            operationTask == nil,
            let confirmation
        else {
            return
        }
        self.confirmation = nil
        generation &+= 1
        let requestGeneration = generation
        let priorStatus = state.status
        state = CLIInstallationViewState(
            status: priorStatus,
            activity: confirmation.action == .uninstall ? .uninstalling : .installing,
            failure: nil,
            requiresAuthoritativeReload: false
        )
        operationIsConfirmedMutation = true
        let service = self.service
        let task = Task { @MainActor [weak self] in
            let result: Result<CLIInstallationUpdate, CLIInstallationFailure>
            do {
                result = .success(try await service.perform(confirmation))
            } catch let error as CLIInstallerServiceError {
                result = .failure(.service(error))
            } catch {
                result = .failure(.unexpected)
            }
            guard
                let self,
                self.generation == requestGeneration
            else {
                return
            }
            self.operationIsConfirmedMutation = false
            if !self.shuttingDown {
                switch result {
                case let .success(update):
                    self.state = CLIInstallationViewState(
                        status: update.status,
                        activity: nil,
                        failure: nil,
                        requiresAuthoritativeReload: false
                    )
                case let .failure(failure):
                    let requiresReload = Self.requiresReload(
                        failure,
                        afterMutationAttempt: true
                    )
                    self.state = CLIInstallationViewState(
                        status: requiresReload ? nil : priorStatus,
                        activity: nil,
                        failure: failure,
                        requiresAuthoritativeReload: requiresReload
                    )
                }
            }
            self.operationTask = nil
        }
        operationTask = task
        await task.value
    }

    func cancelPreparedAction() async {
        guard !operationIsConfirmedMutation else {
            return
        }
        generation &+= 1
        operationTask?.cancel()
        if let operationTask {
            await operationTask.value
        }
        self.operationTask = nil
        if let confirmation {
            self.confirmation = nil
            await service.discard(confirmation)
        }
        state.activity = nil
    }

    func dismissFailure() {
        state.failure = nil
    }

    func shutdown() async {
        guard !shuttingDown else {
            if let operationTask {
                await operationTask.value
            }
            return
        }
        shuttingDown = true
        generation &+= 1
        if !operationIsConfirmedMutation {
            operationTask?.cancel()
        }
        if let operationTask {
            await operationTask.value
        }
        self.operationTask = nil
        if let confirmation {
            self.confirmation = nil
            await service.discard(confirmation)
        }
        await service.close()
    }

    private func actionIsAvailable(_ action: CLIInstallationAction) -> Bool {
        guard
            !state.isBusy,
            !state.requiresAuthoritativeReload,
            let status = state.status
        else {
            return false
        }
        let presentation = CLIInstallationPresentation.make(status: status)
        if action == .uninstall {
            return presentation.offersUninstall
        }
        return presentation.primaryAction == action
    }

    private static func requiresReload(
        _ failure: CLIInstallationFailure,
        afterMutationAttempt: Bool = false
    ) -> Bool {
        switch failure {
        case .service(.outcomeUnknown), .service(.confirmationChanged):
            true
        case .unexpected:
            afterMutationAttempt
        case .service:
            false
        }
    }
}
