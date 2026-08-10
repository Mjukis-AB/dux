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

    static func historySuggestionRow(rank: UInt16) -> String {
        historySuggestionRowPrefix + String(rank)
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
        }
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
            case .paused(.failure):
                Button("Disable") {
                    Task { await settings.disableSchedule(id: schedule.scheduleID) }
                }
                .disabled(mutationControlsAreDisabled)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.scheduleDisable(index)
                )
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
    }

    private var mutationControlsAreDisabled: Bool {
        settings.state.isBusy || settings.requiresRefresh
    }

    private var progressLabel: String {
        switch settings.state {
        case .loading:
            "Loading automation state…"
        case .settingGlobalEnabled, .settingGlobalDisabled, .resettingGlobalControl:
            "Updating global automation state…"
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

    static func historySuggestionMessage(
        for failure: AutomationScheduleSettingsFailure
    ) -> String {
        switch failure {
        case let .service(error):
            "Stored manual cleanup history is unavailable (\(serviceLabel(error))). No "
                + "scan, schedule, or cleanup was started."
        case .model:
            "An invalid manual-history idea was rejected. No cleanup was started."
        case .confirmationRequired, .deletionConfirmationRequired, .unexpected:
            "Manual-history ideas could not be loaded. No cleanup was started."
        }
    }

    static func message(for failure: AutomationScheduleSettingsFailure) -> String {
        switch failure {
        case .confirmationRequired:
            "The exact confirmation phrase is required. Automation state was not changed."
        case .deletionConfirmationRequired:
            "Deletion requires confirmation. The saved schedule was not changed."
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
        case .unavailable:
            "Automation state is unavailable. No cleanup was started."
        case .incompatibleSchema:
            "This automation state format is incompatible with the current app."
        case .unsafeStorage:
            "Automation storage failed its safety checks. No state was changed."
        case .corruptData:
            "Corrupt automation state was rejected. No state was changed."
        case .invalidRequest, .invalidResponse:
            "The storage engine rejected an invalid automation response."
        }
    }
}
