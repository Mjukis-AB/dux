import Foundation
import SwiftUI

enum AutomationScheduleAccessibility {
    static let section = "automation-schedules-section"
    static let globalStatus = "automation-schedules-global-status"
    static let globalToggle = "automation-schedules-global-toggle"
    static let globalEnableConfirmation = "automation-schedules-global-enable-confirmation"
    static let globalEnableConfirm = "automation-schedules-global-enable-confirm"
    static let globalEnableCancel = "automation-schedules-global-enable-cancel"
    static let globalReset = "automation-schedules-global-reset"
    static let executionStatus = "automation-schedules-execution-status"
    static let eligibleRuleCount = "automation-schedules-eligible-rule-count"
    static let scheduleCount = "automation-schedules-schedule-count"
    static let defaults = "automation-schedules-defaults"
    static let safetyDisclosure = "automation-schedules-safety-disclosure"
    static let utcDisclosure = "automation-schedules-utc-disclosure"
    static let scheduleList = "automation-schedules-list"
    static let progress = "automation-schedules-progress"
    static let error = "automation-schedules-error"
    static let reload = "automation-schedules-reload"
    static let scheduleRowPrefix = "automation-schedules-row-"
    static let scheduleEligibilityPrefix = "automation-schedules-eligibility-"
    static let scheduleStatePrefix = "automation-schedules-state-"
    static let scheduleRecurrencePrefix = "automation-schedules-recurrence-"
    static let scheduleEnablePrefix = "automation-schedules-enable-"
    static let schedulePausePrefix = "automation-schedules-pause-"
    static let scheduleResumePrefix = "automation-schedules-resume-"
    static let scheduleDisablePrefix = "automation-schedules-disable-"
    static let scheduleDeletePrefix = "automation-schedules-delete-"
    static let scheduleEditPrefix = "automation-schedules-edit-"
    static let scheduleCategoryReviewPrefix = "automation-schedules-category-review-"
    static let scheduleCategoryReviewDisclosurePrefix =
        "automation-schedules-category-review-disclosure-"
    static let historySuggestionSection =
        "automation-schedules-history-suggestions-section"
    static let historySuggestionCoverage =
        "automation-schedules-history-suggestions-coverage"
    static let historySuggestionList =
        "automation-schedules-history-suggestions-list"
    static let historySuggestionEmpty =
        "automation-schedules-history-suggestions-empty"
    static let historySuggestionProgress =
        "automation-schedules-history-suggestions-progress"
    static let historySuggestionError =
        "automation-schedules-history-suggestions-error"
    static let historySuggestionRefresh =
        "automation-schedules-history-suggestions-refresh"
    static let historySuggestionRowPrefix =
        "automation-schedules-history-suggestion-rank-"
    static let historySuggestionCreatePrefix =
        "automation-schedules-history-suggestion-create-rank-"
    static let authoringCatalogSection = "automation-schedules-authoring-catalog-section"
    static let authoringCatalogStatus = "automation-schedules-authoring-catalog-status"
    static let authoringCatalogRefresh = "automation-schedules-authoring-catalog-refresh"
    static let authoringCatalogLoading = "automation-schedules-authoring-catalog-loading"
    static let authoringCatalogError = "automation-schedules-authoring-catalog-error"
    static let authoringCatalogEmpty = "automation-schedules-authoring-catalog-empty"
    static let authoringCatalogCreate = "automation-schedules-authoring-catalog-create"
    static let authoringCatalogCategoryPrefix =
        "automation-schedules-authoring-catalog-category-"
    static let editor = "automation-schedules-editor"
    static let editorScope = "automation-schedules-editor-scope"
    static let editorCadence = "automation-schedules-editor-cadence"
    static let editorMinimumAgeValue = "automation-schedules-editor-minimum-age-value"
    static let editorMinimumAgeUnit = "automation-schedules-editor-minimum-age-unit"
    static let editorMinimumSize = "automation-schedules-editor-minimum-size"
    static let editorMaximumPerRun = "automation-schedules-editor-maximum-per-run"
    static let editorNotify = "automation-schedules-editor-notify"
    static let editorConfirmation = "automation-schedules-editor-confirmation"
    static let editorExclusions = "automation-schedules-editor-exclusions"
    static let editorIncludedRules = "automation-schedules-editor-included-rules"
    static let editorIncludedRuleSummary = "automation-schedules-editor-included-rule-summary"
    static let editorIncludedRulePrefix = "automation-schedules-editor-included-rule-"
    static let editorDisclosure = "automation-schedules-editor-disclosure"
    static let editorCategoryRebindDisclosure =
        "automation-schedules-editor-category-rebind-disclosure"
    static let editorSave = "automation-schedules-editor-save"
    static let editorCancel = "automation-schedules-editor-cancel"
    static let editorReview = "automation-schedules-editor-review"
    static let editorProgress = "automation-schedules-editor-progress"
    static let editorError = "automation-schedules-editor-error"

    static let allStaticIdentifiers = [
        section,
        globalStatus,
        globalToggle,
        globalEnableConfirmation,
        globalEnableConfirm,
        globalEnableCancel,
        globalReset,
        executionStatus,
        eligibleRuleCount,
        scheduleCount,
        defaults,
        safetyDisclosure,
        utcDisclosure,
        scheduleList,
        progress,
        error,
        reload,
        historySuggestionSection,
        historySuggestionCoverage,
        historySuggestionList,
        historySuggestionEmpty,
        historySuggestionProgress,
        historySuggestionError,
        historySuggestionRefresh,
        authoringCatalogSection,
        authoringCatalogStatus,
        authoringCatalogRefresh,
        authoringCatalogLoading,
        authoringCatalogError,
        authoringCatalogEmpty,
        authoringCatalogCreate,
        editor,
        editorScope,
        editorCadence,
        editorMinimumAgeValue,
        editorMinimumAgeUnit,
        editorMinimumSize,
        editorMaximumPerRun,
        editorNotify,
        editorConfirmation,
        editorExclusions,
        editorIncludedRules,
        editorIncludedRuleSummary,
        editorDisclosure,
        editorCategoryRebindDisclosure,
        editorSave,
        editorCancel,
        editorReview,
        editorProgress,
        editorError,
    ]

    static func scheduleRow(_ index: Int) -> String {
        scheduleRowPrefix + String(index)
    }

    static func scheduleEligibility(_ index: Int) -> String {
        scheduleEligibilityPrefix + String(index)
    }

    static func scheduleState(_ index: Int) -> String {
        scheduleStatePrefix + String(index)
    }

    static func scheduleRecurrence(_ index: Int) -> String {
        scheduleRecurrencePrefix + String(index)
    }

    static func scheduleEnable(_ index: Int) -> String {
        scheduleEnablePrefix + String(index)
    }

    static func schedulePause(_ index: Int) -> String {
        schedulePausePrefix + String(index)
    }

    static func scheduleResume(_ index: Int) -> String {
        scheduleResumePrefix + String(index)
    }

    static func scheduleDisable(_ index: Int) -> String {
        scheduleDisablePrefix + String(index)
    }

    static func scheduleDelete(_ index: Int) -> String {
        scheduleDeletePrefix + String(index)
    }

    static func scheduleEdit(_ index: Int) -> String {
        scheduleEditPrefix + String(index)
    }

    static func scheduleCategoryReview(_ index: Int) -> String {
        scheduleCategoryReviewPrefix + String(index)
    }

    static func scheduleCategoryReviewDisclosure(_ index: Int) -> String {
        scheduleCategoryReviewDisclosurePrefix + String(index)
    }

    static func historySuggestionRow(rank: UInt16) -> String {
        historySuggestionRowPrefix + String(rank)
    }

    static func historySuggestionCreate(rank: UInt16) -> String {
        historySuggestionCreatePrefix + String(rank)
    }

    static func authoringCatalogCategory(_ index: Int) -> String {
        authoringCatalogCategoryPrefix + String(index)
    }

    static func editorIncludedRule(_ index: Int) -> String {
        editorIncludedRulePrefix + String(index)
    }
}

struct AutomationScheduleSettingsView: View {
    @Bindable var settings: AutomationScheduleSettingsModel

    @State private var showingGlobalEnableConfirmation = false
    @State private var globalEnableConfirmation = ""
    @State private var pendingDeletionScheduleID: String?

    var body: some View {
        Section("Automations") {
            VStack(alignment: .leading, spacing: 12) {
                Label(
                    settings.overview?.globalControl.enabled == true
                        ? "Automation state is enabled" : "Automation state is disabled",
                    systemImage: "lock.shield.fill"
                )
                .font(.headline)
                .foregroundStyle(.secondary)

                Text(
                    "These controls save activation state only. This build has no production "
                        + "schedule source, cannot select cleanup work, and cannot execute "
                        + "cleanup from an automation schedule."
                )
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.safetyDisclosure
                )

                if let overview = settings.overview {
                    globalControls(overview)
                    statusGrid(overview)

                    Text(
                        "Weekly and monthly anchors and next-run instants are computed and "
                            + "stored by the Rust core in UTC. Time-zone changes affect "
                            + "presentation only, so the displayed local hour can change."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.utcDisclosure
                    )

                    Text(
                        "Saved state never replaces shipped static policy, current runtime "
                            + "evidence, fresh planning, or pre-effect revalidation. Execution "
                            + "remains independently unavailable."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)

                    defaultsCard
                }

                authoringCatalogSection
                historySuggestionSection

                if let overview = settings.overview {
                    scheduleSection(overview)
                }

                if settings.state.isBusy {
                    ProgressView(progressLabel)
                        .controlSize(.small)
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.progress
                        )
                }

                if case let .failed(failure) = settings.state {
                    Label(Self.message(for: failure), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.error
                        )

                    Button(settings.requiresRefresh ? "Refresh automation state" : "Try again") {
                        Task { await settings.load(force: true) }
                    }
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.reload
                    )
                    .accessibilityHint(
                        "Reloads saved state only and never starts cleanup"
                    )
                }

                Text(
                    "AI, history suggestions, and these settings cannot approve or run "
                        + "cleanup. Every future run still requires a fresh bounded plan and "
                        + "all independent safety gates."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(AutomationScheduleAccessibility.section)
            .confirmationDialog(
                "Delete this automation schedule?",
                isPresented: deletionConfirmationIsPresented
            ) {
                Button("Delete schedule", role: .destructive) {
                    guard let id = pendingDeletionScheduleID else {
                        return
                    }
                    pendingDeletionScheduleID = nil
                    Task { await settings.deleteSchedule(id: id, confirmed: true) }
                }
                Button("Cancel", role: .cancel) {
                    pendingDeletionScheduleID = nil
                }
            } message: {
                Text(
                    "Deleting removes the saved configuration and activation state. It does "
                        + "not run cleanup."
                )
            }
            .sheet(isPresented: editorIsPresented) {
                AutomationScheduleEditorSheet(settings: settings)
            }
        }
    }

    private var editorIsPresented: Binding<Bool> {
        Binding(
            get: { settings.editor != nil },
            set: { isPresented in
                if !isPresented {
                    settings.cancelEditor()
                }
            }
        )
    }

    private var deletionConfirmationIsPresented: Binding<Bool> {
        Binding(
            get: { pendingDeletionScheduleID != nil },
            set: { isPresented in
                if !isPresented {
                    pendingDeletionScheduleID = nil
                }
            }
        )
    }

    @ViewBuilder
    private func globalControls(_ overview: AutomationScheduleOverviewModel) -> some View {
        GroupBox("Global automation control") {
            VStack(alignment: .leading, spacing: 8) {
                Toggle(
                    "Save global automation consent",
                    isOn: Binding(
                        get: { overview.globalControl.enabled },
                        set: { enabled in
                            if enabled {
                                globalEnableConfirmation = ""
                                showingGlobalEnableConfirmation = true
                            } else {
                                showingGlobalEnableConfirmation = false
                                globalEnableConfirmation = ""
                                Task { await settings.setGlobalEnabled(false) }
                            }
                        }
                    )
                )
                .disabled(settings.state.isBusy || settings.requiresRefresh)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.globalToggle
                )
                .accessibilityHint(
                    "Changes saved global state only; execution remains unavailable"
                )

                if showingGlobalEnableConfirmation, !overview.globalControl.enabled {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(
                            "Type \(AutomationScheduleSettingsModel.globalEnableConfirmation) "
                                + "to save global consent. This still cannot run cleanup."
                        )
                        .font(.caption)

                        TextField(
                            AutomationScheduleSettingsModel.globalEnableConfirmation,
                            text: $globalEnableConfirmation
                        )
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.globalEnableConfirmation
                        )

                        HStack {
                            Button("Cancel") {
                                showingGlobalEnableConfirmation = false
                                globalEnableConfirmation = ""
                            }
                            .accessibilityIdentifier(
                                AutomationScheduleAccessibility.globalEnableCancel
                            )

                            Button("Enable saved automation state") {
                                let confirmation = globalEnableConfirmation
                                showingGlobalEnableConfirmation = false
                                globalEnableConfirmation = ""
                                Task {
                                    await settings.setGlobalEnabled(
                                        true,
                                        confirmation: confirmation
                                    )
                                }
                            }
                            .disabled(
                                globalEnableConfirmation
                                    != AutomationScheduleSettingsModel.globalEnableConfirmation
                            )
                            .accessibilityIdentifier(
                                AutomationScheduleAccessibility.globalEnableConfirm
                            )
                        }
                    }
                }

                Button("Reset global control", role: .destructive) {
                    Task { await settings.resetGlobalControl() }
                }
                .disabled(settings.state.isBusy || settings.requiresRefresh)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.globalReset
                )
                .accessibilityHint(
                    "Restores the disabled default without running cleanup"
                )
            }
        }
    }

    @ViewBuilder
    private func statusGrid(_ overview: AutomationScheduleOverviewModel) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 6) {
            GridRow {
                LabeledContent("Global switch") {
                    Text(overview.globalControl.enabled ? "On" : "Off")
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.globalStatus
                )

                LabeledContent("Execution") {
                    Text(overview.executionAvailable ? "Available" : "Unavailable")
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.executionStatus
                )
            }

            GridRow {
                LabeledContent("Rules approved by shipped policy") {
                    Text(verbatim: String(overview.eligibleRuleCount))
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.eligibleRuleCount
                )

                LabeledContent("Saved schedules") {
                    Text(verbatim: String(overview.schedules.count))
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.scheduleCount
                )
            }
        }
    }

    private var defaultsCard: some View {
        GroupBox("Safe schedule defaults") {
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 5) {
                GridRow {
                    LabeledContent("Cadence") {
                        Text(AutomationScheduleDefaults.cadence.displayName)
                    }
                    LabeledContent("Minimum age") {
                        Text(Self.duration(AutomationScheduleDefaults.minimumAgeSeconds))
                    }
                }
                GridRow {
                    LabeledContent("Maximum per run") {
                        Text(
                            StorageByteFormatter.string(
                                from: AutomationScheduleDefaults.maximumBytesPerRun
                            )
                        )
                    }
                    LabeledContent("Pre-run notices") {
                        Text("First \(AutomationScheduleDefaults.notificationRuns) runs")
                    }
                }
                GridRow {
                    LabeledContent("Run approval") {
                        Text(AutomationScheduleDefaults.confirmationMode.displayName)
                    }
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(AutomationScheduleAccessibility.defaults)
    }

    private var historySuggestionSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Ideas from manual cleanup history")
                    .font(.headline)
                Spacer()
                Button("Refresh history ideas") {
                    Task { await settings.loadHistorySuggestions(force: true) }
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.historySuggestionRefresh
                )
                .accessibilityHint(
                    "Reads stored manual cleanup history only and starts no cleanup"
                )
            }

            Text(
                "Based only on repeated manual cleanups and confirmed regrowth. These are "
                    + "ideas to review, not permission to schedule or run cleanup."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            if let feed = settings.historySuggestionFeed {
                Text(Self.historySuggestionCoverage(feed))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.historySuggestionCoverage
                    )

                if feed.suggestions.isEmpty {
                    ContentUnavailableView(
                        "No repeated manual patterns",
                        systemImage: "clock.badge.questionmark",
                        description: Text(
                            "No repeated manual patterns supported by current shipped "
                                + "policy were found. No schedule or cleanup was started."
                        )
                    )
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.historySuggestionEmpty
                    )
                } else {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(feed.suggestions) { suggestion in
                            historySuggestionCard(suggestion)
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.historySuggestionList
                    )
                }
            }

            if settings.historySuggestionState.isLoading {
                ProgressView("Reading stored manual cleanup history…")
                    .controlSize(.small)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.historySuggestionProgress
                    )
            }

            if case let .failed(failure) = settings.historySuggestionState {
                Label(
                    Self.historySuggestionMessage(for: failure),
                    systemImage: "exclamationmark.triangle"
                )
                .foregroundStyle(.red)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.historySuggestionError
                )
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(
            AutomationScheduleAccessibility.historySuggestionSection
        )
    }

    private var authoringCatalogSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Selectable category scopes")
                    .font(.headline)
                Spacer()
                Button("Refresh category scopes") {
                    Task { await settings.loadAuthoringCatalog(force: true) }
                }
                .disabled(settings.authoringCatalogState.isLoading)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.authoringCatalogRefresh
                )
                .accessibilityHint(
                    "Reloads the core-owned safe scope catalog without creating a schedule"
                )
            }

            Text(
                "Only categories and exact rule revisions projected by the Rust safety "
                    + "catalog can be selected. Saving still creates a disabled preference "
                    + "and never starts cleanup."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            if let catalog = settings.authoringCatalog {
                let ruleCount = catalog.staticallySelectableRuleCount
                Text(
                    "\(catalog.categories.count) selectable categories · "
                        + "\(ruleCount) exact rules"
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.authoringCatalogStatus
                )

                if settings.authoringCatalogIsStale {
                    Label(
                        "The last validated category catalog is stale. Refresh before "
                            + "creating or saving a category schedule.",
                        systemImage: "clock.badge.exclamationmark"
                    )
                    .foregroundStyle(.orange)
                }

                if catalog.categories.isEmpty {
                    ContentUnavailableView(
                        "No selectable category scopes",
                        systemImage: "calendar.badge.minus",
                        description: Text(
                            "No category contains a rule currently approved by shipped "
                                + "policy for schedule authoring."
                        )
                    )
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.authoringCatalogEmpty
                    )
                } else {
                    Menu("New disabled category schedule…") {
                        ForEach(Array(catalog.categories.enumerated()), id: \.element.id) {
                            index,
                                category in
                            Button(category.category.displayName) {
                                settings.beginCreatingCategorySchedule(from: category)
                            }
                            .accessibilityIdentifier(
                                AutomationScheduleAccessibility.authoringCatalogCategory(index)
                            )
                            .accessibilityLabel(
                                "\(category.category.displayName), "
                                    + "\(category.rules.count) exact rules"
                            )
                        }
                    }
                    .disabled(
                        settings.overview == nil || !settings.authoringCatalogIsFresh
                            || mutationControlsAreDisabled
                    )
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.authoringCatalogCreate
                    )
                    .accessibilityHint(
                        "Choose one core-projected category to review a disabled schedule"
                    )
                }
            }

            if settings.authoringCatalogState.isLoading {
                ProgressView("Loading selectable category scopes…")
                    .controlSize(.small)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.authoringCatalogLoading
                    )
            }

            if case let .failed(failure) = settings.authoringCatalogState {
                Label(
                    Self.authoringCatalogMessage(for: failure),
                    systemImage: "exclamationmark.triangle"
                )
                .foregroundStyle(.red)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.authoringCatalogError
                )
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(
            AutomationScheduleAccessibility.authoringCatalogSection
        )
    }

    private func historySuggestionCard(
        _ suggestion: AutomationScheduleHistorySuggestionModel
    ) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 6) {
                Label("Repeated manual pattern", systemImage: "clock.arrow.circlepath")
                    .font(.headline)
                    .foregroundStyle(.secondary)

                Text("\(suggestion.rule.ruleID) r\(suggestion.rule.ruleRevision)")

                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 5) {
                    GridRow {
                        LabeledContent("Successful manual runs") {
                            Text(verbatim: String(suggestion.successfulManualRunCount))
                        }
                        LabeledContent("Confirmed regrowth cycles") {
                            Text(verbatim: String(suggestion.manualRegrowthCycleCount))
                        }
                    }
                    GridRow {
                        LabeledContent("Latest manual attempt") {
                            Text(Self.historyDate(suggestion.latestManualAttemptAtUnixMilliseconds))
                        }
                        LabeledContent("Latest confirmed regrowth") {
                            Text(Self.historyDate(suggestion.latestRegrowthAtUnixMilliseconds))
                        }
                    }
                }

                Text("History only. No schedule or cleanup starts from this idea.")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Button("Review disabled schedule…") {
                    settings.beginCreatingSchedule(from: suggestion)
                }
                .disabled(mutationControlsAreDisabled)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.historySuggestionCreate(
                        rank: suggestion.rank
                    )
                )
                .accessibilityHint(
                    "Opens an inert schedule preference editor for this exact rule; no cleanup starts"
                )
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(
            AutomationScheduleAccessibility.historySuggestionRow(rank: suggestion.rank)
        )
        .accessibilityHint(
            "Advisory only; does not create a schedule, scan storage, or run cleanup"
        )
    }

    @ViewBuilder
    private func scheduleSection(_ overview: AutomationScheduleOverviewModel) -> some View {
        if overview.schedules.isEmpty {
            ContentUnavailableView(
                "No automation schedules",
                systemImage: "calendar.badge.minus",
                description: Text("There are no saved automation configurations.")
            )
        } else {
            VStack(alignment: .leading, spacing: 8) {
                Text("Saved schedules")
                    .font(.headline)
                ForEach(
                    Array(overview.schedules.enumerated()),
                    id: \.element.id
                ) { index, schedule in
                    scheduleCard(
                        schedule,
                        assessment: overview.scheduleEligibility[index],
                        index: index
                    )
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(
                AutomationScheduleAccessibility.scheduleList
            )
        }
    }

    private func scheduleCard(
        _ schedule: AutomationScheduleModel,
        assessment: AutomationScheduleEligibilityModel,
        index: Int
    ) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 5) {
                    GridRow {
                        Label(schedule.state.displayName, systemImage: stateIcon(schedule.state))
                            .font(.headline)
                            .accessibilityIdentifier(
                                AutomationScheduleAccessibility.scheduleState(index)
                            )
                        Text(schedule.scope.displayName)
                            .foregroundStyle(.secondary)
                    }
                    GridRow {
                        LabeledContent("Cadence") {
                            Text(schedule.cadence.displayName)
                        }
                        LabeledContent("Minimum age") {
                            Text(Self.duration(schedule.minimumAgeSeconds))
                        }
                    }
                    GridRow {
                        LabeledContent("Minimum reclaimable") {
                            Text(
                                StorageByteFormatter.string(
                                    from: schedule.minimumReclaimableBytes
                                )
                            )
                        }
                        LabeledContent("Maximum per run") {
                            Text(
                                StorageByteFormatter.string(
                                    from: schedule.maximumBytesPerRun
                                )
                            )
                        }
                    }
                    GridRow {
                        LabeledContent("Run approval") {
                            Text(schedule.confirmationMode.displayName)
                        }
                        LabeledContent("Schedule revision") {
                            Text(verbatim: String(schedule.revision))
                        }
                    }
                    GridRow {
                        LabeledContent("Pre-run notices") {
                            Text(Self.notificationSummary(schedule))
                        }
                        LabeledContent("Exact rule exclusions") {
                            Text(verbatim: String(schedule.exclusions.count))
                        }
                    }
                    GridRow {
                        LabeledContent("Static eligibility") {
                            Text(assessment.statusLabel)
                                .accessibilityIdentifier(
                                    AutomationScheduleAccessibility.scheduleEligibility(index)
                                )
                        }
                        LabeledContent("Included policy rules") {
                            Text(
                                verbatim: String(
                                    assessment.includedStaticallyEligibleRuleCount
                                )
                            )
                        }
                    }
                }

                if !schedule.exclusions.isEmpty {
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Excluded exact rules")
                            .font(.caption.bold())
                        ForEach(schedule.exclusions, id: \.self) { exclusion in
                            Text("\(exclusion.ruleID) r\(exclusion.ruleRevision)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                }

                if let recurrence = schedule.recurrence {
                    Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 5) {
                        GridRow {
                            LabeledContent("UTC anchor") {
                                Text(Self.historyDate(recurrence.anchorAtUnixMilliseconds))
                            }
                            LabeledContent("Next UTC occurrence") {
                                Text(Self.historyDate(recurrence.nextRunAtUnixMilliseconds))
                            }
                        }
                        GridRow {
                            LabeledContent("Occurrence ordinal") {
                                Text(verbatim: String(recurrence.nextOccurrenceOrdinal))
                            }
                            LabeledContent("Cursor revision") {
                                Text(verbatim: String(recurrence.cursorRevision))
                            }
                        }
                    }
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.scheduleRecurrence(index)
                    )
                }

                if !assessment.reasons.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(assessment.reasons, id: \.self) { reason in
                            Label(reason.explanation, systemImage: "lock.fill")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                }

                scheduleActions(schedule, assessment: assessment, index: index)

                if settings.state.isMutating(scheduleID: schedule.scheduleID) {
                    ProgressView("Updating saved schedule state…")
                        .controlSize(.small)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(AutomationScheduleAccessibility.scheduleRow(index))
        .accessibilityLabel(
            "\(schedule.state.displayName) automation schedule for "
                + schedule.scope.displayName
        )
        .accessibilityHint(
            "Manages saved state only; this build cannot execute scheduled cleanup"
        )
    }

    @ViewBuilder
    private func scheduleActions(
        _ schedule: AutomationScheduleModel,
        assessment: AutomationScheduleEligibilityModel,
        index: Int
    ) -> some View {
        HStack {
            switch schedule.state {
            case .disabled:
                Button("Edit…") {
                    settings.beginEditingSchedule(id: schedule.scheduleID)
                }
                .disabled(mutationControlsAreDisabled)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.scheduleEdit(index)
                )
                .accessibilityHint(
                    "Edits this exact disabled revision without enabling or running cleanup"
                )

                if settings.canReviewCategorySchedule(id: schedule.scheduleID) {
                    Button("Review current category membership…") {
                        settings.beginReviewingCategorySchedule(id: schedule.scheduleID)
                    }
                    .disabled(mutationControlsAreDisabled)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.scheduleCategoryReview(index)
                    )
                    .accessibilityHint(
                        "Reviews current exact catalog membership for this disabled revision"
                    )
                }

                Button("Enable saved state") {
                    Task { await settings.enableSchedule(id: schedule.scheduleID) }
                }
                .disabled(
                    mutationControlsAreDisabled
                        || assessment.status != .awaitingRuntimeEvidence
                        || schedule.cadence == .lowDiskOnly
                )
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.scheduleEnable(index)
                )
                .accessibilityHint(
                    "Runs core static preflight and saves activation state only"
                )
            case .enabled:
                Button("Pause") {
                    Task { await settings.pauseSchedule(id: schedule.scheduleID) }
                }
                .disabled(mutationControlsAreDisabled)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.schedulePause(index)
                )

                Button("Disable") {
                    Task { await settings.disableSchedule(id: schedule.scheduleID) }
                }
                .disabled(mutationControlsAreDisabled)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.scheduleDisable(index)
                )

                Text("Disable to edit configuration")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            case .paused(.user):
                Button("Resume") {
                    Task { await settings.resumeSchedule(id: schedule.scheduleID) }
                }
                .disabled(mutationControlsAreDisabled)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.scheduleResume(index)
                )

                Button("Disable") {
                    Task { await settings.disableSchedule(id: schedule.scheduleID) }
                }
                .disabled(mutationControlsAreDisabled)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.scheduleDisable(index)
                )

                Text("Disable to edit configuration")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            case .paused(.failure):
                Button("Disable") {
                    Task { await settings.disableSchedule(id: schedule.scheduleID) }
                }
                .disabled(mutationControlsAreDisabled)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.scheduleDisable(index)
                )

                Text("Disable to edit configuration")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Button("Delete…", role: .destructive) {
                pendingDeletionScheduleID = schedule.scheduleID
            }
            .disabled(mutationControlsAreDisabled)
            .accessibilityIdentifier(
                AutomationScheduleAccessibility.scheduleDelete(index)
            )
            .accessibilityHint(
                "Requires confirmation and removes saved state without running cleanup"
            )
        }

        if schedule.state == .disabled,
           settings.canReviewCategorySchedule(id: schedule.scheduleID)
        {
            Text(
                "Saving a category-membership review replaces this schedule’s prior "
                    + "category consent with the current exact catalog membership. The "
                    + "schedule remains disabled."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier(
                AutomationScheduleAccessibility.scheduleCategoryReviewDisclosure(index)
            )
        }
    }

    private var mutationControlsAreDisabled: Bool {
        settings.state.isBusy || settings.requiresRefresh || settings.editor != nil
    }

    private var progressLabel: String {
        switch settings.state {
        case .loading:
            "Loading automation state…"
        case .settingGlobalEnabled, .settingGlobalDisabled, .resettingGlobalControl:
            "Updating global automation state…"
        case .creatingSchedule:
            "Saving disabled schedule…"
        case .editingSchedule:
            "Updating disabled schedule…"
        case .enablingSchedule, .pausingSchedule, .resumingSchedule,
             .disablingSchedule, .deletingSchedule:
            "Updating saved schedule state…"
        case .idle, .ready, .failed:
            "Updating automation state…"
        }
    }

    private func stateIcon(_ state: DuxAutomationScheduleState) -> String {
        switch state {
        case .disabled: "pause.circle.fill"
        case .enabled: "checkmark.circle.fill"
        case .paused: "pause.circle"
        }
    }

    static func duration(_ seconds: UInt64) -> String {
        let day: UInt64 = 24 * 60 * 60
        if seconds.isMultiple(of: day) {
            let days = seconds / day
            return days == 1 ? "1 day" : "\(days) days"
        }
        let hour: UInt64 = 60 * 60
        if seconds.isMultiple(of: hour) {
            let hours = seconds / hour
            return hours == 1 ? "1 hour" : "\(hours) hours"
        }
        return "\(seconds) seconds"
    }

    static func historySuggestionCoverage(
        _ feed: AutomationScheduleHistorySuggestionFeedModel
    ) -> String {
        let sessionWord = feed.sourceSessionCount == 1 ? "session" : "sessions"
        var message = "Reviewed \(feed.sourceSessionCount) recent stored manual \(sessionWord)."
        if feed.hasOlderSourceSessions {
            message += " Older stored manual sessions were not inspected."
        }
        if feed.qualifyingRuleCount > UInt16(feed.suggestions.count) {
            let omitted = feed.qualifyingRuleCount - UInt16(feed.suggestions.count)
            let patternWord = omitted == 1 ? "pattern" : "patterns"
            message += " \(omitted) additional history \(patternWord) matched the "
                + "history-only checks and are not shown."
        }
        return message
    }

    static func historyDate(_ unixMilliseconds: Int64) -> String {
        Date(timeIntervalSince1970: Double(unixMilliseconds) / 1000)
            .formatted(date: .abbreviated, time: .shortened)
    }

    static func notificationSummary(_ schedule: AutomationScheduleModel) -> String {
        guard schedule.notifyBeforeRun else {
            return "Off"
        }
        let count = schedule.notifyBeforeRunsRemaining
        let noun = count == 1 ? "notice" : "notices"
        return "On; \(count) introductory \(noun) remaining"
    }

    static func historySuggestionMessage(
        for failure: AutomationScheduleSettingsFailure
    ) -> String {
        switch failure {
        case let .service(error):
            "Stored manual cleanup history is unavailable (\(serviceLabel(error))). No "
                + "scan, schedule, or cleanup was started."
        case .model:
            "An invalid manual-history idea was rejected. No cleanup was started."
        case .confirmationRequired, .deletionConfirmationRequired,
             .editorReviewRequired, .draft, .unexpected:
            "Manual-history ideas could not be loaded. No cleanup was started."
        }
    }

    static func authoringCatalogMessage(
        for failure: AutomationScheduleSettingsFailure
    ) -> String {
        switch failure {
        case let .service(error):
            "Selectable category scopes are unavailable (\(serviceLabel(error))). "
                + "Saved schedules and manual-history ideas are unchanged."
        case .model:
            "An invalid category-scope catalog was rejected. Saved schedules are unchanged."
        case .confirmationRequired, .deletionConfirmationRequired,
             .editorReviewRequired, .draft, .unexpected:
            "Selectable category scopes could not be loaded. Saved schedules are unchanged."
        }
    }

    static func message(for failure: AutomationScheduleSettingsFailure) -> String {
        switch failure {
        case .confirmationRequired:
            "The exact confirmation phrase is required. Automation state was not changed."
        case .deletionConfirmationRequired:
            "Deletion requires confirmation. The saved schedule was not changed."
        case .editorReviewRequired:
            "The reviewed schedule proposal is stale. Refresh and review current state before another save."
        case let .draft(error):
            editorDraftMessage(error)
        case let .service(error):
            serviceMessage(error)
        case .model:
            "An unsafe or invalid automation response was rejected."
        case .unexpected:
            "Automation state could not be loaded or changed. No cleanup was started."
        }
    }

    private static func serviceLabel(_ error: AutomationScheduleServiceError) -> String {
        switch error {
        case .unavailable: "unavailable"
        case .incompatibleSchema: "incompatible schema"
        case .retryable: "temporarily busy"
        case .invalidRequest: "invalid request"
        case .authoringCatalogStale: "stale category catalog"
        case .invalidAuthoringSelection: "invalid category selection"
        case .draftLimitExceeded: "schedule limit reached"
        case .notFound: "not found"
        case .revisionConflict: "revision conflict"
        case .invalidStateTransition: "invalid state transition"
        case .staticPolicyBlocked: "blocked by shipped policy"
        case .activationUnavailable: "activation unavailable"
        case .unsafeStorage: "unsafe storage"
        case .corruptData: "corrupt data"
        case .outcomeUnknown: "uncertain outcome"
        case .invalidResponse: "invalid response"
        }
    }

    private static func serviceMessage(_ error: AutomationScheduleServiceError) -> String {
        switch error {
        case .revisionConflict:
            "Automation state changed elsewhere. Refresh before trying another change."
        case .outcomeUnknown:
            "The write outcome could not be proven. Refresh the complete state before "
                + "trying another change."
        case .staticPolicyBlocked:
            "Shipped safety policy blocks activation. No state was changed."
        case .activationUnavailable:
            "The evidence needed for this activation is unavailable. No state was changed."
        case .invalidStateTransition:
            "That schedule state no longer admits this change. Refresh and review it again."
        case .notFound:
            "That schedule no longer exists. Refresh automation state."
        case .retryable:
            "Automation storage is temporarily busy. No cleanup was started."
        case .draftLimitExceeded:
            "The limit of 64 saved schedules has been reached. No schedule was created."
        case .unavailable:
            "Automation state is unavailable. No cleanup was started."
        case .incompatibleSchema:
            "This automation state format is incompatible with the current app."
        case .unsafeStorage:
            "Automation storage failed its safety checks. No state was changed."
        case .corruptData:
            "Corrupt automation state was rejected. No state was changed."
        case .invalidRequest:
            "The storage engine rejected invalid automation settings. No state was changed."
        case .authoringCatalogStale:
            "Selectable category scopes changed. Refresh and explicitly review the current scope before saving."
        case .invalidAuthoringSelection:
            "The selected category or exclusions are not in the current safe catalog. Refresh and review them again."
        case .invalidResponse:
            "The storage engine rejected an invalid automation response."
        }
    }

    static func editorDraftMessage(_ error: AutomationScheduleEditorDraftError) -> String {
        switch error {
        case let .invalidNumber(field):
            "Enter an exact non-negative number for \(editorFieldName(field))."
        case let .zero(field):
            "\(editorFieldName(field)) must be greater than zero."
        case let .outOfRange(field):
            "\(editorFieldName(field)) is outside the supported range."
        case .immutableFieldsChanged:
            "Scope, exclusions, pre-run notices, and run approval are fixed by the reviewed schedule."
        case .invalidExclusions:
            "The stored exact-rule exclusions are invalid and cannot be edited."
        case .exclusionsRequireCategoryScope:
            "Exact-rule schedules cannot contain rule exclusions."
        case .invalidCatalogSelection:
            "The selected category or exact exclusions no longer match the reviewed catalog."
        case .allRulesExcluded:
            "Include at least one exact rule in this category."
        }
    }

    private static func editorFieldName(_ field: AutomationScheduleEditorField) -> String {
        switch field {
        case .minimumAge: "Minimum age"
        case .minimumReclaimableSize: "Minimum reclaimable size"
        case .maximumBytesPerRun: "Maximum per run"
        }
    }
}

private struct AutomationScheduleEditorSheet: View {
    @Bindable var settings: AutomationScheduleSettingsModel

    var body: some View {
        if let editor = settings.editor {
            VStack(alignment: .leading, spacing: 16) {
                Text(editorTitle(editor))
                    .font(.title2.bold())

                Text(
                    "Saving stores inert preferences only. It does not enable automation, "
                        + "scan storage, select cleanup work, or run cleanup."
                )
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.editorDisclosure
                )

                if editor.isCategoryRebind {
                    Label(
                        "This explicit review replaces the saved category consent with "
                            + "the current exact rule membership and exclusions. The "
                            + "schedule remains disabled.",
                        systemImage: "arrow.triangle.2.circlepath"
                    )
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorCategoryRebindDisclosure
                    )
                }

                Form {
                    LabeledContent("Exact scope") {
                        Text(editor.draft.scope.displayName)
                    }
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorScope
                    )

                    Picker(
                        "Cadence",
                        selection: draftBinding(\.cadence, fallback: .monthly)
                    ) {
                        ForEach(DuxAutomationScheduleCadence.allCases) { cadence in
                            Text(cadence.displayName).tag(cadence)
                        }
                    }
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorCadence
                    )
                    .accessibilityHint(
                        "Stores a cadence preference only; low-disk activation remains unavailable"
                    )

                    LabeledContent("Minimum age") {
                        HStack {
                            TextField(
                                "Minimum age",
                                text: draftBinding(\.minimumAgeValue, fallback: "")
                            )
                            .frame(width: 150)
                            .accessibilityLabel("Minimum age value")
                            .accessibilityHint(
                                "Enter an exact non-negative whole number"
                            )
                            .accessibilityIdentifier(
                                AutomationScheduleAccessibility.editorMinimumAgeValue
                            )

                            Picker(
                                "Age unit",
                                selection: draftBinding(\.minimumAgeUnit, fallback: .days)
                            ) {
                                ForEach(AutomationScheduleAgeUnit.allCases) { unit in
                                    Text(unit.displayName).tag(unit)
                                }
                            }
                            .labelsHidden()
                            .frame(width: 130)
                            .accessibilityLabel("Minimum age unit")
                            .accessibilityIdentifier(
                                AutomationScheduleAccessibility.editorMinimumAgeUnit
                            )
                        }
                    }

                    LabeledContent("Minimum reclaimable size") {
                        HStack {
                            TextField(
                                "Minimum reclaimable size",
                                text: draftBinding(\.minimumReclaimableGiB, fallback: "")
                            )
                            .frame(width: 150)
                            Text("GiB")
                                .foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel("Minimum reclaimable size in GiB")
                    .accessibilityHint(
                        "Enter an exact non-negative value; zero means no minimum size threshold"
                    )
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorMinimumSize
                    )

                    LabeledContent("Maximum per run") {
                        HStack {
                            TextField(
                                "Maximum per run",
                                text: draftBinding(\.maximumBytesPerRunGiB, fallback: "")
                            )
                            .frame(width: 150)
                            Text("GiB")
                                .foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel("Maximum bytes per run in GiB")
                    .accessibilityHint("Enter an exact value greater than zero")
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorMaximumPerRun
                    )

                    LabeledContent("Pre-run notices") {
                        Text(editor.draft.notifyBeforeRun ? "On" : "Off")
                    }
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorNotify
                    )
                    .accessibilityHint(
                        "Preserved from the reviewed schedule and not editable here"
                    )

                    LabeledContent("Run approval") {
                        Text(editor.draft.confirmationMode.displayName)
                    }
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorConfirmation
                    )
                    .accessibilityHint(
                        "Preserved from the reviewed schedule and not editable here"
                    )

                    if let selection = editor.categorySelection {
                        includedRules(selection, editor: editor)
                    } else {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Preserved exact-rule exclusions")
                            if editor.draft.exclusions.isEmpty {
                                Text("None")
                                    .foregroundStyle(.secondary)
                            } else {
                                ForEach(editor.draft.exclusions, id: \.self) { exclusion in
                                    Text("\(exclusion.ruleID) r\(exclusion.ruleRevision)")
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.editorExclusions
                        )
                    }
                }
                .disabled(
                    editor.requiresReReview || settings.requiresRefresh
                        || editor.categorySelection != nil
                        && !settings.authoringCatalogIsFresh
                )

                if editor.categorySelection != nil, !settings.authoringCatalogIsFresh {
                    Label(
                        "Refresh selectable category scopes before saving. Your input is retained.",
                        systemImage: "clock.badge.exclamationmark"
                    )
                    .foregroundStyle(.orange)
                }

                if editor.draft.cadence == .lowDiskOnly {
                    Text(
                        "Low-disk preference can be stored while disabled, but activation "
                            + "remains unavailable until authoritative pressure-episode "
                            + "evidence is implemented."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }

                if case let .failed(failure) = settings.state {
                    Label(
                        AutomationScheduleSettingsView.message(for: failure),
                        systemImage: "exclamationmark.triangle"
                    )
                    .foregroundStyle(.red)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorError
                    )
                }

                if settings.state.isBusy {
                    ProgressView(
                        editor.isCreating
                            ? "Saving disabled schedule…" : "Updating disabled schedule…"
                    )
                    .controlSize(.small)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorProgress
                    )
                }

                if settings.requiresRefresh {
                    Button("Refresh complete automation state") {
                        Task { await settings.load(force: true) }
                    }
                    .disabled(settings.state.isBusy)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorReview
                    )
                    .accessibilityHint(
                        "Reads authoritative state without retrying the uncertain write"
                    )
                } else if editor.requiresReReview {
                    switch editor.mode {
                    case .createCategory where settings.canReviewCategorySelectionAgain:
                        Button("Review current category scope again") {
                            settings.reviewCategorySelectionAgain()
                        }
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.editorReview
                        )
                        .accessibilityHint(
                            "Keeps compatible input and binds it to the freshly loaded exact rules"
                        )
                    case .rebindCategory where settings.canReviewCategorySelectionAgain:
                        Button("Review current category membership again") {
                            settings.reviewCategorySelectionAgain()
                        }
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.editorReview
                        )
                        .accessibilityHint(
                            "Loads the current disabled revision and exact catalog membership"
                        )
                    case .edit where settings.canReviewEditedScheduleAgain:
                        Button("Review current schedule again") {
                            settings.reviewEditedScheduleAgain()
                        }
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.editorReview
                        )
                        .accessibilityHint(
                            "Discards unsaved fields and loads the current exact schedule revision"
                        )
                    case .create, .createCategory, .rebindCategory, .edit:
                        Button("Close and review saved schedules") {
                            settings.cancelEditor()
                        }
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.editorReview
                        )
                        .accessibilityHint(
                            editor.isCreating
                                ? "Closes this stale proposal without retrying schedule creation"
                                : "Closes this stale proposal because the current schedule cannot be edited"
                        )
                    }
                }

                HStack {
                    Spacer()
                    Button("Cancel", role: .cancel) {
                        settings.cancelEditor()
                    }
                    .disabled(settings.state.isBusy)
                    .keyboardShortcut(.cancelAction)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorCancel
                    )

                    Button(saveButtonTitle(editor)) {
                        Task { await settings.saveEditor() }
                    }
                    .disabled(
                        settings.state.isBusy || settings.requiresRefresh
                            || editor.requiresReReview
                            || editor.categorySelection != nil
                            && (!settings.authoringCatalogIsFresh
                                || !settings.categoryEditorHasIncludedRules)
                    )
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.editorSave
                    )
                    .accessibilityHint(
                        "Stores disabled preferences only and never starts cleanup"
                    )
                }
            }
            .padding(20)
            .frame(minWidth: 620, idealWidth: 680)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(AutomationScheduleAccessibility.editor)
            .interactiveDismissDisabled(settings.state.isBusy)
        }
    }

    @ViewBuilder
    private func includedRules(
        _ selection: AutomationScheduleCategoryAuthoringSelection,
        editor: AutomationScheduleEditorSession
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Included rules")
                .font(.headline)
            Text(
                "Only these exact core-projected rule revisions can be included. "
                    + "Turn a rule off to store it as an exact exclusion."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            ForEach(Array(selection.category.rules.enumerated()), id: \.element.id) {
                index,
                    rule in
                Toggle(
                    isOn: Binding(
                        get: { !editor.draft.exclusions.contains(rule.rule) },
                        set: { included in
                            settings.setCategoryRule(rule.rule, included: included)
                        }
                    )
                ) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(rule.rule.ruleID) r\(rule.rule.ruleRevision)")
                        Text(rule.titleKey)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .accessibilityLabel(
                    "Include \(rule.rule.ruleID), revision \(rule.rule.ruleRevision)"
                )
                .accessibilityValue(
                    editor.draft.exclusions.contains(rule.rule) ? "Excluded" : "Included"
                )
                .accessibilityHint(
                    "Changes an inert disabled schedule preference only"
                )
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.editorIncludedRule(index)
                )
            }

            let excludedCount = editor.draft.exclusions.count
            let includedCount = selection.category.rules.count - excludedCount
            Text("\(includedCount) included · \(excludedCount) excluded")
                .foregroundStyle(includedCount == 0 ? Color.red : Color.secondary)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.editorIncludedRuleSummary
                )
            if includedCount == 0 {
                Text("Include at least one exact rule before saving.")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(
            AutomationScheduleAccessibility.editorIncludedRules
        )
    }

    private func editorTitle(_ editor: AutomationScheduleEditorSession) -> String {
        switch editor.mode {
        case .create:
            "Review disabled schedule"
        case .createCategory:
            "New disabled category schedule"
        case .rebindCategory:
            "Review current category membership"
        case .edit:
            "Edit disabled schedule"
        }
    }

    private func saveButtonTitle(_ editor: AutomationScheduleEditorSession) -> String {
        if editor.isCategoryRebind {
            "Save reviewed membership"
        } else if editor.isCreating {
            "Save disabled schedule"
        } else {
            "Save changes"
        }
    }

    private func draftBinding<Value>(
        _ keyPath: WritableKeyPath<AutomationScheduleEditorDraft, Value>,
        fallback: Value
    ) -> Binding<Value> {
        Binding(
            get: {
                settings.editor?.draft[keyPath: keyPath] ?? fallback
            },
            set: { value in
                guard var draft = settings.editor?.draft else {
                    return
                }
                draft[keyPath: keyPath] = value
                settings.setEditorDraft(draft)
            }
        )
    }
}
