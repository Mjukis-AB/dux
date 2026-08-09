import Foundation
import SwiftUI

enum AutomationScheduleAccessibility {
    static let section = "automation-schedules-section"
    static let globalStatus = "automation-schedules-global-status"
    static let executionStatus = "automation-schedules-execution-status"
    static let eligibleRuleCount = "automation-schedules-eligible-rule-count"
    static let draftCount = "automation-schedules-draft-count"
    static let defaults = "automation-schedules-defaults"
    static let safetyDisclosure = "automation-schedules-safety-disclosure"
    static let draftList = "automation-schedules-draft-list"
    static let progress = "automation-schedules-progress"
    static let error = "automation-schedules-error"
    static let reload = "automation-schedules-reload"
    static let draftRowPrefix = "automation-schedules-draft-"
    static let draftEligibilityPrefix = "automation-schedules-draft-eligibility-"
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
        executionStatus,
        eligibleRuleCount,
        draftCount,
        defaults,
        safetyDisclosure,
        draftList,
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

    static func draftRow(_ index: Int) -> String {
        draftRowPrefix + String(index)
    }

    static func draftEligibility(_ index: Int) -> String {
        draftEligibilityPrefix + String(index)
    }

    static func historySuggestionRow(rank: UInt16) -> String {
        historySuggestionRowPrefix + String(rank)
    }
}

struct AutomationScheduleSettingsView: View {
    @Bindable var settings: AutomationScheduleSettingsModel

    var body: some View {
        Section("Automations") {
            VStack(alignment: .leading, spacing: 12) {
                Label("Automations are off", systemImage: "lock.shield.fill")
                    .font(.headline)
                    .foregroundStyle(.secondary)

                Text(
                    "DUX does not schedule cleanup in this build. A disabled draft records "
                        + "preferences only: it grants no cleanup permission and cannot run "
                        + "by itself."
                )
                .foregroundStyle(.secondary)
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.safetyDisclosure
                )

                if let overview = settings.overview {
                    statusGrid(overview)

                    Text(
                        "Each draft is checked against shipped rule policy. Manual history, "
                            + "the last two runs, current age and size, activity, fresh "
                            + "evidence, and user-level execution remain separate required "
                            + "runtime gates."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)

                    defaultsCard
                }

                historySuggestionSection

                if let overview = settings.overview {
                    if overview.disabledDrafts.isEmpty {
                        ContentUnavailableView(
                            "No automation drafts",
                            systemImage: "calendar.badge.minus",
                            description: Text(
                                "There are no disabled schedule drafts. DUX currently has "
                                    + "zero shipped rules marked and structurally valid for "
                                    + "scheduling."
                            )
                        )
                    } else {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Disabled drafts")
                                .font(.headline)
                            ForEach(
                                Array(overview.disabledDrafts.enumerated()),
                                id: \.element.id
                            ) { index, draft in
                                draftCard(
                                    draft,
                                    assessment: overview.draftEligibility[index],
                                    index: index
                                )
                            }
                        }
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier(
                            AutomationScheduleAccessibility.draftList
                        )
                    }
                }

                if settings.state.isLoading {
                    ProgressView("Loading automation drafts…")
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

                    Button("Try loading drafts again") {
                        Task { await settings.load(force: true) }
                    }
                    .accessibilityIdentifier(
                        AutomationScheduleAccessibility.reload
                    )
                    .accessibilityHint(
                        "Reads disabled drafts only and never starts cleanup"
                    )
                }

                Text(
                    "A later automation release must re-plan and revalidate every run. Only "
                        + "shipped SafeRegenerable rules with repeated successful manual "
                        + "history can become eligible; AI can never approve or run cleanup."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(AutomationScheduleAccessibility.section)
        }
    }

    @ViewBuilder
    private func statusGrid(_ overview: AutomationScheduleOverviewModel) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 6) {
            GridRow {
                LabeledContent("Global switch") {
                    Text(overview.globalEnabled ? "On" : "Off")
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

                LabeledContent("Disabled drafts") {
                    Text(verbatim: String(overview.disabledDrafts.count))
                }
                .accessibilityIdentifier(
                    AutomationScheduleAccessibility.draftCount
                )
            }
        }
    }

    private var defaultsCard: some View {
        GroupBox("Safe draft defaults") {
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 5) {
                GridRow {
                    LabeledContent("Cadence") {
                        Text(AutomationScheduleDefaults.cadence.displayName)
                    }
                    LabeledContent("Minimum age") {
                        Text(
                            Self.duration(
                                AutomationScheduleDefaults.minimumAgeSeconds
                            )
                        )
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
                        Text(
                            "First \(AutomationScheduleDefaults.notificationRuns) runs"
                        )
                    }
                }
                GridRow {
                    LabeledContent("Run approval") {
                        Text(
                            AutomationScheduleDefaults.confirmationMode.displayName
                        )
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
                    "Reads stored manual cleanup history only; does not scan, schedule, "
                        + "or run cleanup"
                )
            }

            Text(
                "Based only on repeated manual cleanups and confirmed regrowth. These are "
                    + "ideas to review, not permission to schedule or run cleanup. "
                    + "Refreshing reads stored history only; it starts no scan or cleanup."
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
                                + "policy were found in the recent bounded history window. "
                                + "No schedule was created, and no scan or cleanup was started."
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

                Text(
                    "History only. No schedule was created or enabled, and no cleanup "
                        + "will run from this idea."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(
            AutomationScheduleAccessibility.historySuggestionRow(rank: suggestion.rank)
        )
        .accessibilityLabel(
            "History idea rank \(suggestion.rank) for \(suggestion.rule.ruleID), "
                + "revision \(suggestion.rule.ruleRevision)"
        )
        .accessibilityHint(
            "Advisory only; does not create a schedule, scan storage, or run cleanup"
        )
    }

    private func draftCard(
        _ draft: AutomationScheduleDraftModel,
        assessment: AutomationScheduleDraftEligibilityModel,
        index: Int
    ) -> some View {
        GroupBox {
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 5) {
                GridRow {
                    Label("Disabled draft", systemImage: "pause.circle.fill")
                        .font(.headline)
                    Text(draft.scope.displayName)
                        .foregroundStyle(.secondary)
                }
                GridRow {
                    LabeledContent("Cadence") {
                        Text(draft.cadence.displayName)
                    }
                    LabeledContent("Minimum age") {
                        Text(Self.duration(draft.minimumAgeSeconds))
                    }
                }
                GridRow {
                    LabeledContent("Minimum reclaimable") {
                        Text(
                            StorageByteFormatter.string(
                                from: draft.minimumReclaimableBytes
                            )
                        )
                    }
                    LabeledContent("Maximum per run") {
                        Text(
                            StorageByteFormatter.string(
                                from: draft.maximumBytesPerRun
                            )
                        )
                    }
                }
                GridRow {
                    LabeledContent("Pre-run notices left") {
                        Text(
                            draft.notifyBeforeRun
                                ? String(draft.notifyBeforeRunsRemaining)
                                : "Off"
                        )
                    }
                    LabeledContent("Run approval") {
                        Text(draft.confirmationMode.displayName)
                    }
                }
                GridRow {
                    LabeledContent("Rule exclusions") {
                        Text(verbatim: String(draft.exclusions.count))
                    }
                    LabeledContent("Draft revision") {
                        Text(verbatim: String(draft.revision))
                    }
                }
                GridRow {
                    LabeledContent("Static eligibility") {
                        Text(assessment.statusLabel)
                            .accessibilityIdentifier(
                                AutomationScheduleAccessibility
                                    .draftEligibility(index)
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

            if !assessment.reasons.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(assessment.reasons, id: \.self) { reason in
                        Label(reason.explanation, systemImage: "lock.fill")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.top, 6)
                .accessibilityElement(children: .combine)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(AutomationScheduleAccessibility.draftRow(index))
        .accessibilityLabel(
            "Disabled automation draft for \(draft.scope.displayName)"
        )
        .accessibilityHint(
            "This read-only draft cannot schedule or execute cleanup"
        )
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
            switch error {
            case .unavailable:
                "Stored manual cleanup history is unavailable. No scan, schedule, or "
                    + "cleanup was started."
            case .incompatibleSchema:
                "This manual-history idea format is incompatible with the current app. "
                    + "No scan, schedule, or cleanup was started."
            case .invalidResponse:
                "The storage engine returned an invalid manual-history idea response. "
                    + "No scan, schedule, or cleanup was started."
            }
        case .model:
            "An invalid manual-history idea was rejected. No scan, schedule, or cleanup "
                + "was started."
        case .unexpected:
            "Manual-history ideas could not be loaded. No scan, schedule, or cleanup was "
                + "started."
        }
    }

    static func message(for failure: AutomationScheduleSettingsFailure) -> String {
        switch failure {
        case let .service(error):
            switch error {
            case .unavailable:
                "Automation drafts are unavailable. No cleanup was scheduled."
            case .incompatibleSchema:
                "This automation-draft format is incompatible with the current app."
            case .invalidResponse:
                "The storage engine returned an invalid automation-draft response."
            }
        case .model:
            "An unsafe or invalid automation draft was rejected."
        case .unexpected:
            "Automation drafts could not be loaded. No cleanup was scheduled."
        }
    }
}
