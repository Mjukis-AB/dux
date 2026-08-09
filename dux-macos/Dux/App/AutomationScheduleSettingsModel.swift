import Foundation
import Observation

protocol DuxAutomationScheduleServing: Sendable {
    func loadAutomationScheduleOverview() async throws
        -> AutomationScheduleOverviewModel
}

struct UnavailableDuxAutomationScheduleService: DuxAutomationScheduleServing {
    func loadAutomationScheduleOverview() async throws
        -> AutomationScheduleOverviewModel
    {
        .unavailable
    }
}

enum AutomationScheduleServiceError: Error, Equatable, Sendable {
    case unavailable
    case incompatibleSchema
    case invalidResponse
}

enum AutomationScheduleSettingsFailure: Equatable, Sendable {
    case service(AutomationScheduleServiceError)
    case model(AutomationScheduleModelError)
    case unexpected
}

enum AutomationScheduleSettingsState: Equatable, Sendable {
    case idle
    case loading
    case ready
    case failed(AutomationScheduleSettingsFailure)

    var isLoading: Bool { self == .loading }
}

@MainActor
@Observable
final class AutomationScheduleSettingsModel {
    private(set) var overview: AutomationScheduleOverviewModel?
    private(set) var state = AutomationScheduleSettingsState.idle

    private let service: any DuxAutomationScheduleServing

    @ObservationIgnored
    private var operationTask: Task<Void, Never>?
    @ObservationIgnored
    private var generation: UInt64 = 0
    @ObservationIgnored
    private var shuttingDown = false

    init(service: any DuxAutomationScheduleServing) {
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
        guard force || overview == nil else {
            return
        }

        generation &+= 1
        let requestGeneration = generation
        state = .loading
        let service = service
        let task = Task { @MainActor [weak self] in
            let result: Result<AutomationScheduleOverviewModel, Error>
            do {
                result = try await .success(
                    service.loadAutomationScheduleOverview()
                )
            } catch {
                result = .failure(error)
            }
            guard
                let self,
                !self.shuttingDown,
                generation == requestGeneration
            else {
                return
            }
            operationTask = nil
            switch result {
            case let .success(overview):
                self.overview = overview
                state = .ready
            case let .failure(error):
                state = .failed(Self.failure(for: error))
            }
        }
        operationTask = task
        await task.value
    }

    func shutdown() async {
        guard !shuttingDown else {
            await operationTask?.value
            return
        }
        shuttingDown = true
        generation &+= 1
        let operation = operationTask
        operation?.cancel()
        await operation?.value
        operationTask = nil
        state = overview == nil ? .idle : .ready
    }

    private static func failure(for error: Error) -> AutomationScheduleSettingsFailure {
        if let error = error as? AutomationScheduleServiceError {
            return .service(error)
        }
        if let error = error as? AutomationScheduleModelError {
            return .model(error)
        }
        return .unexpected
    }
}
