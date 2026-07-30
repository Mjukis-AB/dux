import Foundation
import Observation

@MainActor
@Observable
final class SnapshotRetentionCapSettingsModel {
    private(set) var settings: SnapshotRetentionCapModel?
    private(set) var state = SnapshotRetentionCapState.idle
    private(set) var requiresAuthoritativeReload = false
    var draft = SnapshotRetentionCapDraft.defaults

    private let service: any DuxSnapshotRetentionCapServing

    @ObservationIgnored
    private var operationTask: Task<Void, Never>?
    @ObservationIgnored
    private var generation: UInt64 = 0
    @ObservationIgnored
    private var shuttingDown = false

    init(service: any DuxSnapshotRetentionCapServing) {
        self.service = service
    }

    func load(force: Bool = false) async {
        guard !shuttingDown else {
            return
        }
        if let operationTask {
            await operationTask.value
            return
        }
        guard force || settings == nil || requiresAuthoritativeReload else {
            return
        }

        generation &+= 1
        let requestGeneration = generation
        state = .loading
        let service = self.service
        let task = Task { @MainActor [weak self] in
            let result: Result<SnapshotRetentionCapModel, Error>
            do {
                result = try .success(await service.loadSnapshotRetentionCap())
            } catch {
                result = .failure(error)
            }
            guard
                let self,
                !self.shuttingDown,
                self.generation == requestGeneration
            else {
                return
            }
            self.operationTask = nil
            switch result {
            case let .success(settings):
                self.publish(settings)
                self.requiresAuthoritativeReload = false
                self.state = .ready
            case let .failure(error):
                self.state = .failed(Self.failure(for: error))
            }
        }
        operationTask = task
        await task.value
    }

    func save() async {
        guard !shuttingDown, operationTask == nil, !state.isBusy else {
            return
        }
        let capBytes: UInt64
        do {
            capBytes = try draft.capBytes()
        } catch let error as SnapshotRetentionCapDraftError {
            state = .failed(.draft(error))
            return
        } catch {
            state = .failed(.unexpected)
            return
        }
        await mutate(state: .saving) { service in
            try await service.setSnapshotRetentionCap(capBytes)
        }
    }

    func reset() async {
        guard !shuttingDown, operationTask == nil, !state.isBusy else {
            return
        }
        await mutate(state: .resetting) { service in
            try await service.resetSnapshotRetentionCap()
        }
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
        let operation = operationTask
        operation?.cancel()
        await operation?.value
        operationTask = nil
        state = settings == nil ? .idle : .ready
    }

    private func mutate(
        state: SnapshotRetentionCapState,
        _ operation: @escaping @Sendable (
            any DuxSnapshotRetentionCapServing
        ) async throws -> SnapshotRetentionCapUpdateResultModel
    ) async {
        generation &+= 1
        let requestGeneration = generation
        precondition(state == .saving || state == .resetting)
        self.state = state
        let service = self.service
        let task = Task { @MainActor [weak self] in
            let result: Result<SnapshotRetentionCapUpdateResultModel, Error>
            do {
                result = try .success(await operation(service))
            } catch {
                result = .failure(error)
            }
            guard
                let self,
                !self.shuttingDown,
                self.generation == requestGeneration
            else {
                return
            }
            self.operationTask = nil
            switch result {
            case let .success(update):
                self.publish(update.settings)
                self.requiresAuthoritativeReload = false
                self.state = .ready
            case let .failure(error):
                let failure = Self.failure(for: error)
                self.requiresAuthoritativeReload =
                    Self.requiresReload(after: failure)
                self.state = .failed(failure)
            }
        }
        operationTask = task
        await task.value
    }

    private func publish(_ settings: SnapshotRetentionCapModel) {
        self.settings = settings
        draft = SnapshotRetentionCapDraft(capBytes: settings.capBytes)
    }

    private static func failure(for error: Error) -> SnapshotRetentionCapFailure {
        if let error = error as? SnapshotRetentionCapServiceError {
            return .service(error)
        }
        if let error = error as? SnapshotRetentionCapDraftError {
            return .draft(error)
        }
        return .unexpected
    }

    private static func requiresReload(
        after failure: SnapshotRetentionCapFailure
    ) -> Bool {
        switch failure {
        case let .service(error):
            switch error {
            case .outcomeUnknown, .internalState, .invalidResponse:
                true
            case .closed, .invalidRecordVersion, .invalidClock, .incompatibleSchema,
                 .retryable, .unsafeStorage, .budgetExceeded, .corruptData, .unavailable:
                false
            }
        case .draft:
            false
        case .unexpected:
            true
        }
    }
}
