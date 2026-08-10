import Foundation
import Observation

protocol DuxAutomationScheduleServing: Sendable {
    func loadAutomationScheduleOverview() async throws
        -> AutomationScheduleOverviewModel
    func loadAutomationScheduleHistorySuggestions() async throws
        -> AutomationScheduleHistorySuggestionFeedModel
    func setAutomationGlobalEnabled(
        expectedRevision: UInt64,
        enabled: Bool
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func resetAutomationGlobalControl(
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func enableAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func pauseAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func resumeAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func disableAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func deleteAutomationSchedule(
        id: String,
        expectedRevision: UInt64
    ) async throws -> AutomationScheduleOverviewUpdateModel
}

struct UnavailableDuxAutomationScheduleService: DuxAutomationScheduleServing {
    func loadAutomationScheduleOverview() async throws
        -> AutomationScheduleOverviewModel
    {
        .unavailable
    }

    func loadAutomationScheduleHistorySuggestions() async throws
        -> AutomationScheduleHistorySuggestionFeedModel
    {
        .unavailable
    }
}

enum AutomationScheduleServiceError: Error, Equatable, Sendable {
    case unavailable
    case incompatibleSchema
    case retryable
    case invalidRequest
    case notFound
    case revisionConflict
    case invalidStateTransition
    case staticPolicyBlocked
    case activationUnavailable
    case unsafeStorage
    case corruptData
    case outcomeUnknown
    case invalidResponse
}

enum AutomationScheduleSettingsFailure: Equatable, Sendable {
    case confirmationRequired
    case deletionConfirmationRequired
    case service(AutomationScheduleServiceError)
    case model(AutomationScheduleModelError)
    case unexpected
}

enum AutomationScheduleSettingsState: Equatable, Sendable {
    case idle
    case loading
    case ready
    case settingGlobalEnabled
    case settingGlobalDisabled
    case resettingGlobalControl
    case enablingSchedule(String)
    case pausingSchedule(String)
    case resumingSchedule(String)
    case disablingSchedule(String)
    case deletingSchedule(String)
    case failed(AutomationScheduleSettingsFailure)

    var isLoading: Bool { self == .loading }

    var isBusy: Bool {
        switch self {
        case .loading, .settingGlobalEnabled, .settingGlobalDisabled,
             .resettingGlobalControl, .enablingSchedule, .pausingSchedule,
             .resumingSchedule, .disablingSchedule, .deletingSchedule:
            true
        case .idle, .ready, .failed:
            false
        }
    }

    func isMutating(scheduleID: String) -> Bool {
        switch self {
        case let .enablingSchedule(id), let .pausingSchedule(id),
             let .resumingSchedule(id), let .disablingSchedule(id),
             let .deletingSchedule(id):
            id == scheduleID
        case .idle, .loading, .ready, .settingGlobalEnabled,
             .settingGlobalDisabled, .resettingGlobalControl, .failed:
            false
        }
    }
}

@MainActor
@Observable
final class AutomationScheduleSettingsModel {
    static let globalEnableConfirmation = "ENABLE AUTOMATIONS"

    private(set) var overview: AutomationScheduleOverviewModel?
    private(set) var state = AutomationScheduleSettingsState.idle
    private(set) var historySuggestionFeed: AutomationScheduleHistorySuggestionFeedModel?
    private(set) var historySuggestionState = AutomationScheduleSettingsState.idle
    private(set) var requiresRefresh = false

    private let service: any DuxAutomationScheduleServing

    @ObservationIgnored
    private var operationTask: Task<Void, Never>?
    @ObservationIgnored
    private var generation: UInt64 = 0
    @ObservationIgnored
    private var historySuggestionOperationTask: Task<Void, Never>?
    @ObservationIgnored
    private var historySuggestionGeneration: UInt64 = 0
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
                requiresRefresh = false
                state = .ready
            case let .failure(error):
                state = .failed(Self.failure(for: error))
            }
        }
        operationTask = task
        await task.value
    }

    func loadHistorySuggestions(force: Bool = false) async {
        guard !shuttingDown else {
            return
        }
        if let historySuggestionOperationTask {
            await historySuggestionOperationTask.value
            return
        }
        guard force || historySuggestionFeed == nil else {
            return
        }

        historySuggestionGeneration &+= 1
        let requestGeneration = historySuggestionGeneration
        historySuggestionState = .loading
        let service = service
        let task = Task { @MainActor [weak self] in
            let result: Result<AutomationScheduleHistorySuggestionFeedModel, Error>
            do {
                result = try await .success(
                    service.loadAutomationScheduleHistorySuggestions()
                )
            } catch {
                result = .failure(error)
            }
            guard
                let self,
                !self.shuttingDown,
                historySuggestionGeneration == requestGeneration
            else {
                return
            }
            historySuggestionOperationTask = nil
            switch result {
            case let .success(feed):
                historySuggestionFeed = feed
                historySuggestionState = .ready
            case let .failure(error):
                historySuggestionState = .failed(Self.failure(for: error))
            }
        }
        historySuggestionOperationTask = task
        await task.value
    }

    func setGlobalEnabled(
        _ enabled: Bool,
        confirmation: String? = nil
    ) async {
        guard let overview, beginMutationIsAllowed() else {
            return
        }
        guard overview.globalControl.enabled != enabled else {
            state = .ready
            return
        }
        if enabled, confirmation != Self.globalEnableConfirmation {
            state = .failed(.confirmationRequired)
            return
        }
        let expectedRevision = overview.globalControl.revision
        await mutate(
            state: enabled ? .settingGlobalEnabled : .settingGlobalDisabled
        ) { service in
            try await service.setAutomationGlobalEnabled(
                expectedRevision: expectedRevision,
                enabled: enabled
            )
        }
    }

    func resetGlobalControl() async {
        guard let overview, beginMutationIsAllowed() else {
            return
        }
        let expectedRevision = overview.globalControl.revision
        await mutate(state: .resettingGlobalControl) { service in
            try await service.resetAutomationGlobalControl(
                expectedRevision: expectedRevision
            )
        }
    }

    func enableSchedule(id: String) async {
        guard
            let (schedule, assessment) = scheduleAndAssessment(id: id),
            beginMutationIsAllowed()
        else {
            rejectMissingScheduleIfNeeded(id: id)
            return
        }
        guard schedule.state == .disabled else {
            state = .failed(.service(.invalidStateTransition))
            return
        }
        guard assessment.status == .awaitingRuntimeEvidence else {
            state = .failed(.service(.staticPolicyBlocked))
            return
        }
        await mutate(state: .enablingSchedule(id)) { service in
            try await service.enableAutomationSchedule(
                id: id,
                expectedRevision: schedule.revision
            )
        }
    }

    func pauseSchedule(id: String) async {
        guard let schedule = schedule(id: id), beginMutationIsAllowed() else {
            rejectMissingScheduleIfNeeded(id: id)
            return
        }
        guard schedule.state == .enabled else {
            state = .failed(.service(.invalidStateTransition))
            return
        }
        await mutate(state: .pausingSchedule(id)) { service in
            try await service.pauseAutomationSchedule(
                id: id,
                expectedRevision: schedule.revision
            )
        }
    }

    func resumeSchedule(id: String) async {
        guard let schedule = schedule(id: id), beginMutationIsAllowed() else {
            rejectMissingScheduleIfNeeded(id: id)
            return
        }
        guard schedule.state == .paused(.user) else {
            state = .failed(.service(.invalidStateTransition))
            return
        }
        await mutate(state: .resumingSchedule(id)) { service in
            try await service.resumeAutomationSchedule(
                id: id,
                expectedRevision: schedule.revision
            )
        }
    }

    func disableSchedule(id: String) async {
        guard let schedule = schedule(id: id), beginMutationIsAllowed() else {
            rejectMissingScheduleIfNeeded(id: id)
            return
        }
        guard schedule.state != .disabled else {
            state = .ready
            return
        }
        await mutate(state: .disablingSchedule(id)) { service in
            try await service.disableAutomationSchedule(
                id: id,
                expectedRevision: schedule.revision
            )
        }
    }

    func deleteSchedule(id: String, confirmed: Bool) async {
        guard confirmed else {
            state = .failed(.deletionConfirmationRequired)
            return
        }
        guard let schedule = schedule(id: id), beginMutationIsAllowed() else {
            rejectMissingScheduleIfNeeded(id: id)
            return
        }
        await mutate(state: .deletingSchedule(id)) { service in
            try await service.deleteAutomationSchedule(
                id: id,
                expectedRevision: schedule.revision
            )
        }
    }

    func shutdown() async {
        guard !shuttingDown else {
            await operationTask?.value
            await historySuggestionOperationTask?.value
            return
        }
        shuttingDown = true
        generation &+= 1
        historySuggestionGeneration &+= 1
        let operation = operationTask
        let historySuggestionOperation = historySuggestionOperationTask
        operation?.cancel()
        historySuggestionOperation?.cancel()
        await operation?.value
        await historySuggestionOperation?.value
        operationTask = nil
        historySuggestionOperationTask = nil
        state = overview == nil ? .idle : .ready
        historySuggestionState = historySuggestionFeed == nil ? .idle : .ready
    }

    private func mutate(
        state mutationState: AutomationScheduleSettingsState,
        operation: @escaping @Sendable (
            any DuxAutomationScheduleServing
        ) async throws -> AutomationScheduleOverviewUpdateModel
    ) async {
        generation &+= 1
        let requestGeneration = generation
        state = mutationState
        let service = service
        let task = Task { @MainActor [weak self] in
            let result: Result<AutomationScheduleOverviewUpdateModel, Error>
            do {
                result = try .success(await operation(service))
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
            case let .success(update):
                overview = update.overview
                requiresRefresh = false
                state = .ready
            case let .failure(error):
                let failure = Self.failure(for: error)
                if case let .service(serviceError) = failure,
                   serviceError == .outcomeUnknown || serviceError == .revisionConflict
                {
                    requiresRefresh = true
                }
                state = .failed(failure)
            }
        }
        operationTask = task
        await task.value
    }

    private func beginMutationIsAllowed() -> Bool {
        !shuttingDown && operationTask == nil && !state.isBusy && !requiresRefresh
    }

    private func schedule(id: String) -> AutomationScheduleModel? {
        overview?.schedules.first { $0.scheduleID == id }
    }

    private func scheduleAndAssessment(
        id: String
    ) -> (AutomationScheduleModel, AutomationScheduleEligibilityModel)? {
        guard
            let overview,
            let index = overview.schedules.firstIndex(where: { $0.scheduleID == id })
        else {
            return nil
        }
        return (overview.schedules[index], overview.scheduleEligibility[index])
    }

    private func rejectMissingScheduleIfNeeded(id: String) {
        guard !shuttingDown, operationTask == nil, !requiresRefresh,
              schedule(id: id) == nil
        else {
            return
        }
        state = .failed(.service(.notFound))
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
