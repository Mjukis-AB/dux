import Foundation
import Observation

protocol DuxAutomationScheduleServing: Sendable {
    func loadAutomationScheduleOverview() async throws
        -> AutomationScheduleOverviewModel
    func loadAutomationScheduleHistorySuggestions() async throws
        -> AutomationScheduleHistorySuggestionFeedModel
    func loadAutomationScheduleAuthoringCatalog() async throws
        -> AutomationScheduleAuthoringCatalogModel
    func createAutomationSchedule(
        configuration: AutomationScheduleDraftConfigurationModel
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func createAutomationCategorySchedule(
        configuration: AutomationScheduleCategoryDraftConfigurationModel
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func rebindAutomationCategorySchedule(
        id: String,
        expectedRevision: UInt64,
        configuration: AutomationScheduleCategoryDraftConfigurationModel
    ) async throws -> AutomationScheduleOverviewUpdateModel
    func replaceAutomationSchedule(
        id: String,
        expectedRevision: UInt64,
        configuration: AutomationScheduleDraftConfigurationModel
    ) async throws -> AutomationScheduleOverviewUpdateModel
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

    func loadAutomationScheduleAuthoringCatalog() async throws
        -> AutomationScheduleAuthoringCatalogModel
    {
        .unavailable
    }
}

enum AutomationScheduleServiceError: Error, Equatable, Sendable {
    case unavailable
    case incompatibleSchema
    case retryable
    case invalidRequest
    case authoringCatalogStale
    case invalidAuthoringSelection
    case draftLimitExceeded
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
    case editorReviewRequired
    case draft(AutomationScheduleEditorDraftError)
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
    case creatingSchedule
    case editingSchedule(String)
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
             .resettingGlobalControl, .creatingSchedule, .editingSchedule,
             .enablingSchedule, .pausingSchedule,
             .resumingSchedule, .disablingSchedule, .deletingSchedule:
            true
        case .idle, .ready, .failed:
            false
        }
    }

    func isMutating(scheduleID: String) -> Bool {
        switch self {
        case let .editingSchedule(id), let .enablingSchedule(id), let .pausingSchedule(id),
             let .resumingSchedule(id), let .disablingSchedule(id),
             let .deletingSchedule(id):
            id == scheduleID
        case .idle, .loading, .ready, .settingGlobalEnabled,
             .settingGlobalDisabled, .resettingGlobalControl, .creatingSchedule,
             .failed:
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
    private(set) var authoringCatalog: AutomationScheduleAuthoringCatalogModel?
    private(set) var authoringCatalogState = AutomationScheduleSettingsState.idle
    private(set) var authoringCatalogIsStale = false
    private(set) var requiresRefresh = false
    private(set) var editor: AutomationScheduleEditorSession?

    var authoringCatalogIsFresh: Bool {
        authoringCatalog != nil && authoringCatalogState == .ready
            && !authoringCatalogIsStale
    }

    var categoryEditorHasIncludedRules: Bool {
        guard
            let editor,
            let selection = editor.categorySelection
        else {
            return true
        }
        return selection.accepts(exclusions: editor.draft.exclusions)
    }

    var canReviewCategorySelectionAgain: Bool {
        guard
            !requiresRefresh,
            !state.isBusy,
            authoringCatalogIsFresh,
            let catalog = authoringCatalog,
            let editor,
            editor.requiresReReview,
            let selection = editor.categorySelection,
            let current = catalog.categories.first(where: {
                $0.category == selection.category.category
            })
        else {
            return false
        }
        if case let .rebindCategory(scheduleID, _, _) = editor.mode {
            guard
                let schedule = schedule(id: scheduleID),
                schedule.state == .disabled,
                schedule.scope == .category(current.category)
            else {
                return false
            }
            return true
        }
        let currentRules = Set(current.rules.map(\.rule))
        return editor.draft.exclusions.allSatisfy(currentRules.contains)
            && editor.draft.exclusions.count < current.rules.count
    }

    var canReviewEditedScheduleAgain: Bool {
        guard
            !requiresRefresh,
            let editor,
            editor.requiresReReview,
            case let .edit(scheduleID, _) = editor.mode,
            let schedule = schedule(id: scheduleID)
        else {
            return false
        }
        return schedule.state == .disabled
    }

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
    private var authoringCatalogOperationTask: Task<Void, Never>?
    @ObservationIgnored
    private var authoringCatalogGeneration: UInt64 = 0
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

    func loadAuthoringCatalog(force: Bool = false) async {
        guard !shuttingDown else {
            return
        }
        if let authoringCatalogOperationTask {
            await authoringCatalogOperationTask.value
            return
        }
        guard force || authoringCatalog == nil else {
            return
        }

        authoringCatalogGeneration &+= 1
        let requestGeneration = authoringCatalogGeneration
        if authoringCatalog != nil {
            authoringCatalogIsStale = true
        }
        authoringCatalogState = .loading
        let service = service
        let task = Task { @MainActor [weak self] in
            let result: Result<AutomationScheduleAuthoringCatalogModel, Error>
            do {
                result = try await .success(
                    service.loadAutomationScheduleAuthoringCatalog()
                )
            } catch {
                result = .failure(error)
            }
            guard
                let self,
                !self.shuttingDown,
                authoringCatalogGeneration == requestGeneration
            else {
                return
            }
            authoringCatalogOperationTask = nil
            switch result {
            case let .success(catalog):
                authoringCatalog = catalog
                authoringCatalogIsStale = false
                authoringCatalogState = .ready
                fenceChangedCategoryEditor(using: catalog)
            case let .failure(error):
                authoringCatalogIsStale = authoringCatalog != nil
                if var editor, editor.categorySelection != nil {
                    editor.requiresReReview = true
                    self.editor = editor
                }
                authoringCatalogState = .failed(Self.failure(for: error))
            }
        }
        authoringCatalogOperationTask = task
        await task.value
    }

    func beginCreatingSchedule(
        from suggestion: AutomationScheduleHistorySuggestionModel
    ) {
        guard beginEditorIsAllowed(), overview != nil else {
            return
        }
        guard
            historySuggestionFeed?.suggestions.contains(suggestion) == true
        else {
            state = .failed(.service(.invalidRequest))
            return
        }
        guard let overview, overview.schedules.count < AutomationScheduleOverviewModel.maximumScheduleCount else {
            state = .failed(.service(.draftLimitExceeded))
            return
        }
        editor = AutomationScheduleEditorSession(
            mode: .create(suggestion: suggestion.rule),
            draft: AutomationScheduleEditorDraft(scope: .rule(suggestion.rule)),
            requiresReReview: false
        )
        state = .ready
    }

    func beginCreatingCategorySchedule(
        from category: AutomationScheduleAuthoringCategoryModel
    ) {
        guard
            beginEditorIsAllowed(),
            overview != nil,
            authoringCatalogIsFresh,
            let catalog = authoringCatalog,
            catalog.categories.contains(category)
        else {
            return
        }
        guard let overview,
              overview.schedules.count < AutomationScheduleOverviewModel.maximumScheduleCount
        else {
            state = .failed(.service(.draftLimitExceeded))
            return
        }
        do {
            let selection = try AutomationScheduleCategoryAuthoringSelection(
                catalog: catalog,
                category: category
            )
            editor = AutomationScheduleEditorSession(
                mode: .createCategory(selection: selection),
                draft: AutomationScheduleEditorDraft(scope: .category(category.category)),
                requiresReReview: false
            )
            state = .ready
        } catch let error as AutomationScheduleModelError {
            state = .failed(.model(error))
        } catch {
            state = .failed(.unexpected)
        }
    }

    func canReviewCategorySchedule(id: String) -> Bool {
        guard
            beginEditorIsAllowed(),
            authoringCatalogIsFresh,
            let catalog = authoringCatalog,
            let (schedule, assessment) = scheduleAndAssessment(id: id),
            schedule.state == .disabled,
            assessment.status == .blockedByStaticPolicy,
            case let .category(category) = schedule.scope
        else {
            return false
        }
        return catalog.categories.contains { $0.category == category }
    }

    func beginReviewingCategorySchedule(id: String) {
        guard
            canReviewCategorySchedule(id: id),
            authoringCatalogIsFresh,
            let catalog = authoringCatalog,
            let schedule = schedule(id: id),
            schedule.state == .disabled,
            case let .category(category) = schedule.scope,
            let current = catalog.categories.first(where: { $0.category == category })
        else {
            return
        }
        do {
            let selection = try AutomationScheduleCategoryAuthoringSelection(
                catalog: catalog,
                category: current
            )
            editor = AutomationScheduleEditorSession(
                mode: .rebindCategory(
                    scheduleID: schedule.scheduleID,
                    expectedRevision: schedule.revision,
                    selection: selection
                ),
                draft: Self.categoryReviewDraft(schedule: schedule, selection: selection),
                requiresReReview: false
            )
            state = .ready
        } catch let error as AutomationScheduleModelError {
            state = .failed(.model(error))
        } catch {
            state = .failed(.unexpected)
        }
    }

    func beginEditingSchedule(id: String) {
        guard beginEditorIsAllowed() else {
            return
        }
        guard let schedule = schedule(id: id) else {
            state = .failed(.service(.notFound))
            return
        }
        guard schedule.state == .disabled else {
            state = .failed(.service(.invalidStateTransition))
            return
        }
        editor = AutomationScheduleEditorSession(
            mode: .edit(
                scheduleID: schedule.scheduleID,
                expectedRevision: schedule.revision
            ),
            draft: AutomationScheduleEditorDraft(schedule: schedule),
            requiresReReview: false
        )
        state = .ready
    }

    func setEditorDraft(_ draft: AutomationScheduleEditorDraft) {
        guard var editor, !editor.requiresReReview, !state.isBusy else {
            return
        }
        guard editor.preservesReviewedImmutableFields(in: draft) else {
            state = .failed(.draft(.immutableFieldsChanged))
            return
        }
        editor.draft = draft
        self.editor = editor
        if case .failed(.draft) = state {
            state = .ready
        }
    }

    func setCategoryRule(
        _ rule: DuxAutomationScheduleRuleReference,
        included: Bool
    ) {
        guard
            var editor,
            !editor.requiresReReview,
            !state.isBusy,
            let selection = editor.categorySelection,
            selection.category.rules.contains(where: { $0.rule == rule })
        else {
            return
        }
        var exclusions = Set(editor.draft.exclusions)
        if included {
            exclusions.remove(rule)
        } else {
            exclusions.insert(rule)
        }
        editor.draft.exclusions = exclusions.sorted()
        guard editor.preservesReviewedImmutableFields(in: editor.draft) else {
            state = .failed(.draft(.invalidCatalogSelection))
            return
        }
        self.editor = editor
        if case .failed(.draft) = state {
            state = .ready
        }
    }

    func cancelEditor() {
        guard !state.isBusy else {
            return
        }
        editor = nil
        if case .failed(.draft) = state {
            state = .ready
        } else if state == .failed(.editorReviewRequired) {
            state = .ready
        }
    }

    func reviewEditedScheduleAgain() {
        guard
            !requiresRefresh,
            !state.isBusy,
            let editor,
            editor.requiresReReview,
            case let .edit(scheduleID, _) = editor.mode,
            let schedule = schedule(id: scheduleID),
            schedule.state == .disabled
        else {
            return
        }
        self.editor = AutomationScheduleEditorSession(
            mode: .edit(
                scheduleID: schedule.scheduleID,
                expectedRevision: schedule.revision
            ),
            draft: AutomationScheduleEditorDraft(schedule: schedule),
            requiresReReview: false
        )
        state = .ready
    }

    func reviewCategorySelectionAgain() {
        guard
            canReviewCategorySelectionAgain,
            let catalog = authoringCatalog,
            let editor,
            let previous = editor.categorySelection,
            let current = catalog.categories.first(where: {
                $0.category == previous.category.category
            }),
            let selection = try? AutomationScheduleCategoryAuthoringSelection(
                catalog: catalog,
                category: current
            )
        else {
            return
        }
        switch editor.mode {
        case .createCategory:
            self.editor = AutomationScheduleEditorSession(
                mode: .createCategory(selection: selection),
                draft: editor.draft,
                requiresReReview: false
            )
        case let .rebindCategory(scheduleID, _, _):
            guard
                let schedule = schedule(id: scheduleID),
                schedule.state == .disabled,
                schedule.scope == .category(current.category)
            else {
                return
            }
            self.editor = AutomationScheduleEditorSession(
                mode: .rebindCategory(
                    scheduleID: schedule.scheduleID,
                    expectedRevision: schedule.revision,
                    selection: selection
                ),
                draft: Self.categoryReviewDraft(schedule: schedule, selection: selection),
                requiresReReview: false
            )
        case .create, .edit:
            return
        }
        state = .ready
    }

    func saveEditor(
        decimalSeparator: String? = Locale.current.decimalSeparator
    ) async {
        guard var editor else {
            return
        }
        guard !editor.requiresReReview else {
            state = .failed(.editorReviewRequired)
            return
        }
        guard editor.preservesReviewedImmutableFields(in: editor.draft) else {
            state = .failed(.draft(.immutableFieldsChanged))
            return
        }
        guard beginMutationIsAllowed() else {
            return
        }

        let configuration: AutomationScheduleDraftConfigurationModel
        do {
            configuration = try editor.draft.configuration(
                decimalSeparator: decimalSeparator
            )
        } catch let error as AutomationScheduleEditorDraftError {
            state = .failed(.draft(error))
            return
        } catch {
            state = .failed(.unexpected)
            return
        }

        switch editor.mode {
        case .create:
            guard
                let overview,
                overview.schedules.count < AutomationScheduleOverviewModel.maximumScheduleCount
            else {
                state = .failed(.service(.draftLimitExceeded))
                return
            }
            await mutate(state: .creatingSchedule, closeEditorOnSuccess: true) { service in
                try await service.createAutomationSchedule(
                    configuration: configuration
                )
            }
        case let .createCategory(selection):
            guard
                authoringCatalogIsFresh,
                let catalog = authoringCatalog,
                catalog.contains(selection)
            else {
                editor.requiresReReview = true
                self.editor = editor
                state = .failed(.editorReviewRequired)
                return
            }
            let categoryConfiguration: AutomationScheduleCategoryDraftConfigurationModel
            do {
                categoryConfiguration = try editor.categoryConfiguration(
                    decimalSeparator: decimalSeparator
                )
            } catch let error as AutomationScheduleEditorDraftError {
                state = .failed(.draft(error))
                return
            } catch {
                state = .failed(.unexpected)
                return
            }
            guard
                let overview,
                overview.schedules.count < AutomationScheduleOverviewModel.maximumScheduleCount
            else {
                state = .failed(.service(.draftLimitExceeded))
                return
            }
            await mutate(state: .creatingSchedule, closeEditorOnSuccess: true) { service in
                try await service.createAutomationCategorySchedule(
                    configuration: categoryConfiguration
                )
            }
        case let .rebindCategory(scheduleID, expectedRevision, selection):
            guard
                authoringCatalogIsFresh,
                let catalog = authoringCatalog,
                catalog.contains(selection)
            else {
                editor.requiresReReview = true
                self.editor = editor
                state = .failed(.editorReviewRequired)
                return
            }
            let categoryConfiguration: AutomationScheduleCategoryDraftConfigurationModel
            do {
                categoryConfiguration = try editor.categoryConfiguration(
                    decimalSeparator: decimalSeparator
                )
            } catch let error as AutomationScheduleEditorDraftError {
                state = .failed(.draft(error))
                return
            } catch {
                state = .failed(.unexpected)
                return
            }
            guard let schedule = schedule(id: scheduleID) else {
                editor.requiresReReview = true
                self.editor = editor
                state = .failed(.service(.notFound))
                return
            }
            guard
                schedule.state == .disabled,
                schedule.scope == .category(selection.category.category)
            else {
                editor.requiresReReview = true
                self.editor = editor
                state = .failed(.service(.invalidStateTransition))
                return
            }
            guard schedule.revision == expectedRevision else {
                editor.requiresReReview = true
                self.editor = editor
                state = .failed(.editorReviewRequired)
                return
            }
            await mutate(
                state: .editingSchedule(scheduleID),
                closeEditorOnSuccess: true,
                fenceStaleEditorFailure: true
            ) { service in
                try await service.rebindAutomationCategorySchedule(
                    id: scheduleID,
                    expectedRevision: expectedRevision,
                    configuration: categoryConfiguration
                )
            }
        case let .edit(scheduleID, expectedRevision):
            guard let schedule = schedule(id: scheduleID) else {
                editor.requiresReReview = true
                self.editor = editor
                state = .failed(.service(.notFound))
                return
            }
            guard schedule.state == .disabled else {
                editor.requiresReReview = true
                self.editor = editor
                state = .failed(.service(.invalidStateTransition))
                return
            }
            guard schedule.revision == expectedRevision else {
                editor.requiresReReview = true
                self.editor = editor
                state = .failed(.editorReviewRequired)
                return
            }
            await mutate(
                state: .editingSchedule(scheduleID),
                closeEditorOnSuccess: true,
                fenceStaleEditorFailure: true
            ) { service in
                try await service.replaceAutomationSchedule(
                    id: scheduleID,
                    expectedRevision: expectedRevision,
                    configuration: configuration
                )
            }
        }
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
            await authoringCatalogOperationTask?.value
            return
        }
        shuttingDown = true
        generation &+= 1
        historySuggestionGeneration &+= 1
        authoringCatalogGeneration &+= 1
        let operation = operationTask
        let historySuggestionOperation = historySuggestionOperationTask
        let authoringCatalogOperation = authoringCatalogOperationTask
        operation?.cancel()
        historySuggestionOperation?.cancel()
        authoringCatalogOperation?.cancel()
        await operation?.value
        await historySuggestionOperation?.value
        await authoringCatalogOperation?.value
        operationTask = nil
        historySuggestionOperationTask = nil
        authoringCatalogOperationTask = nil
        editor = nil
        state = overview == nil ? .idle : .ready
        historySuggestionState = historySuggestionFeed == nil ? .idle : .ready
        authoringCatalogState = authoringCatalog == nil ? .idle : .ready
    }

    private func mutate(
        state mutationState: AutomationScheduleSettingsState,
        closeEditorOnSuccess: Bool = false,
        fenceStaleEditorFailure: Bool = false,
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
                if closeEditorOnSuccess {
                    editor = nil
                }
                state = .ready
            case let .failure(error):
                let failure = Self.failure(for: error)
                if case let .service(serviceError) = failure,
                   serviceError == .outcomeUnknown || serviceError == .revisionConflict
                   || fenceStaleEditorFailure
                   && (serviceError == .notFound || serviceError == .invalidStateTransition)
                {
                    requiresRefresh = true
                    if var editor {
                        editor.requiresReReview = true
                        self.editor = editor
                    }
                } else if case let .service(serviceError) = failure,
                          serviceError == .authoringCatalogStale
                          || serviceError == .invalidAuthoringSelection
                {
                    authoringCatalogIsStale = true
                    if var editor {
                        editor.requiresReReview = true
                        self.editor = editor
                    }
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

    private func beginEditorIsAllowed() -> Bool {
        !shuttingDown && editor == nil && operationTask == nil && !state.isBusy
            && !requiresRefresh
    }

    private func fenceChangedCategoryEditor(
        using catalog: AutomationScheduleAuthoringCatalogModel
    ) {
        guard
            var editor,
            let selection = editor.categorySelection,
            !catalog.contains(selection)
        else {
            return
        }
        editor.requiresReReview = true
        self.editor = editor
        state = .failed(.editorReviewRequired)
    }

    private func schedule(id: String) -> AutomationScheduleModel? {
        overview?.schedules.first { $0.scheduleID == id }
    }

    private static func categoryReviewDraft(
        schedule: AutomationScheduleModel,
        selection: AutomationScheduleCategoryAuthoringSelection
    ) -> AutomationScheduleEditorDraft {
        var draft = AutomationScheduleEditorDraft(schedule: schedule)
        let currentRules = Set(selection.category.rules.map(\.rule))
        draft.exclusions = schedule.exclusions.filter(currentRules.contains)
        return draft
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
