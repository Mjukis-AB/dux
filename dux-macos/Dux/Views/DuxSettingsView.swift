import AppKit
import SwiftUI

enum MenuBarLabelAccessibility {
    static let picker = "menu-bar-label-mode"
}

enum ApplicationUpdateAccessibility {
    static let check = "application-update-check"
    static let status = "application-update-status"
}

enum DiskPressurePolicyAccessibility {
    static let criticalGiB = "pressure-policy-critical-gib"
    static let criticalPercent = "pressure-policy-critical-percent"
    static let warningGiB = "pressure-policy-warning-gib"
    static let warningPercent = "pressure-policy-warning-percent"
    static let recoveryGiB = "pressure-policy-recovery-gib"
    static let recoveryPercent = "pressure-policy-recovery-percent"
    static let save = "pressure-policy-save"
    static let reset = "pressure-policy-reset"
    static let error = "pressure-policy-error"
    static let progress = "pressure-policy-progress"

    static let fieldIdentifiers = [
        criticalGiB,
        criticalPercent,
        warningGiB,
        warningPercent,
        recoveryGiB,
        recoveryPercent,
    ]

    static let allControlIdentifiers = fieldIdentifiers + [save, reset, error, progress]
}

enum PermanentCleanupPolicyAccessibility {
    static let toggle = "permanent-cleanup-toggle"
    static let reset = "permanent-cleanup-reset"
    static let confirmation = "permanent-cleanup-confirmation"
    static let confirm = "permanent-cleanup-confirm"
    static let cancel = "permanent-cleanup-cancel"
    static let error = "permanent-cleanup-error"
    static let progress = "permanent-cleanup-progress"
    static let status = "permanent-cleanup-status"

    static let allControlIdentifiers = [
        toggle,
        reset,
        confirmation,
        confirm,
        cancel,
        error,
        progress,
        status,
    ]
}

enum CleanupExclusionsAccessibility {
    static let add = "cleanup-exclusions-add"
    static let reset = "cleanup-exclusions-reset"
    static let list = "cleanup-exclusions-list"
    static let removePrefix = "cleanup-exclusions-remove-"
    static let error = "cleanup-exclusions-error"
    static let progress = "cleanup-exclusions-progress"
    static let status = "cleanup-exclusions-status"

    static let allStaticControlIdentifiers = [
        add,
        reset,
        list,
        error,
        progress,
        status,
    ]

    static func remove(_ index: Int) -> String {
        removePrefix + String(index)
    }
}

enum ProjectDiscoveryRootsAccessibility {
    static let section = "project-discovery-roots-section"
    static let add = "project-discovery-roots-add"
    static let reset = "project-discovery-roots-reset"
    static let list = "project-discovery-roots-list"
    static let removePrefix = "project-discovery-roots-remove-"
    static let error = "project-discovery-roots-error"
    static let progress = "project-discovery-roots-progress"
    static let status = "project-discovery-roots-status"

    static let allStaticControlIdentifiers = [
        section,
        add,
        reset,
        list,
        error,
        progress,
        status,
    ]

    static func remove(_ index: Int) -> String {
        removePrefix + String(index)
    }
}

enum DirectCargoEnrollmentAccessibility {
    static let section = "direct-cargo-section"
    static let choose = "direct-cargo-choose"
    static let status = "direct-cargo-status"
    static let preview = "direct-cargo-preview"
    static let enroll = "direct-cargo-enroll"
    static let discard = "direct-cargo-discard"
    static let revoke = "direct-cargo-revoke"
    static let confirmEnroll = "direct-cargo-confirm-enroll"
    static let confirmRevoke = "direct-cargo-confirm-revoke"
    static let confirmCancel = "direct-cargo-confirm-cancel"
    static let reload = "direct-cargo-reload"
    static let progress = "direct-cargo-progress"
    static let error = "direct-cargo-error"

    static let allControlIdentifiers = [
        section,
        choose,
        status,
        preview,
        enroll,
        discard,
        revoke,
        confirmEnroll,
        confirmRevoke,
        confirmCancel,
        reload,
        progress,
        error,
    ]
}

enum CleanupHistoryClearAccessibility {
    static let section = "cleanup-history-clear-section"
    static let prepare = "cleanup-history-clear-prepare"
    static let status = "cleanup-history-clear-status"
    static let confirmation = "cleanup-history-clear-confirmation"
    static let confirm = "cleanup-history-clear-confirm"
    static let cancel = "cleanup-history-clear-cancel"
    static let progress = "cleanup-history-clear-progress"
    static let error = "cleanup-history-clear-error"
    static let dismiss = "cleanup-history-clear-dismiss"

    static let allControlIdentifiers = [
        section,
        prepare,
        status,
        confirmation,
        confirm,
        cancel,
        progress,
        error,
        dismiss,
    ]
}

enum PersistentRecoveryDebtAccessibility {
    static let section = "persistent-recovery-debt-section"
    static let status = "persistent-recovery-debt-status"
    static let count = "persistent-recovery-debt-count"
    static let details = "persistent-recovery-debt-details"
    static let limitations = "persistent-recovery-debt-limitations"
    static let refresh = "persistent-recovery-debt-refresh"
    static let progress = "persistent-recovery-debt-progress"
    static let error = "persistent-recovery-debt-error"

    static let allControlIdentifiers = [
        section,
        status,
        count,
        details,
        limitations,
        refresh,
        progress,
        error,
    ]
}

enum ClaimedRunningScanProvenanceAccessibility {
    static let section = "claimed-running-scan-provenance-section"
    static let status = "claimed-running-scan-provenance-status"
    static let count = "claimed-running-scan-provenance-count"
    static let chart = "claimed-running-scan-provenance-chart"
    static let details = "claimed-running-scan-provenance-details"
    static let currentBoot = "claimed-running-scan-provenance-current-boot"
    static let priorBoot = "claimed-running-scan-provenance-prior-boot"
    static let foreignHost = "claimed-running-scan-provenance-foreign-host"
    static let storedUnproven = "claimed-running-scan-provenance-stored-unproven"
    static let currentContextUnavailable =
        "claimed-running-scan-provenance-current-context-unavailable"
    static let limitations = "claimed-running-scan-provenance-limitations"
    static let refresh = "claimed-running-scan-provenance-refresh"
    static let progress = "claimed-running-scan-provenance-progress"
    static let error = "claimed-running-scan-provenance-error"

    static let allControlIdentifiers = [
        section,
        status,
        count,
        chart,
        details,
        currentBoot,
        priorBoot,
        foreignHost,
        storedUnproven,
        currentContextUnavailable,
        limitations,
        refresh,
        progress,
        error,
    ]
}

private enum CleanupExclusionConfirmationAction {
    case remove(CleanupExclusionPathObservation)
    case reset
}

private enum DirectCargoConfirmationAction: Equatable {
    case enroll(DirectCargoEnrollmentConfirmation)
    case revoke
}

private enum DirectCargoExecutablePickerResult {
    case cancelled
    case invalid
    case selected(DirectCargoExecutableSelection)
}

private enum ProjectDiscoveryRootPickerResult {
    case cancelled
    case invalid
    case selected(ProjectDiscoveryRoot)
}

struct DuxSettingsView: View {
    @Environment(\.scenePhase) private var scenePhase
    @State private var permanentCleanupConfirmation = ""
    @State private var showingPermanentCleanupConfirmation = false
    @State private var cleanupExclusionConfirmationAction:
        CleanupExclusionConfirmationAction?
    @State private var directCargoConfirmationAction:
        DirectCargoConfirmationAction?
    @State private var cleanupHistoryClearConfirmationAction:
        CleanupHistoryClearConfirmation?
    @State private var showingPersistentRecoveryDebtDetails = false
    @State private var showingClaimedRunningScanProvenanceDetails = false

    let model: AppModel

    var body: some View {
        @Bindable var model = model

        Form {
            Section("Application") {
                LabeledContent("App mode") {
                    Text("Menu bar helper")
                }
                Picker("Menu bar label", selection: $model.menuBarLabelMode) {
                    ForEach(MenuBarLabelMode.allCases) { mode in
                        Text(verbatim: mode.localizedTitle())
                            .tag(mode)
                    }
                }
                .accessibilityIdentifier(MenuBarLabelAccessibility.picker)
                .accessibilityHint("Changes the menu bar status immediately")
                Picker(
                    "Menu bar visibility",
                    selection: Binding(
                        get: { model.menuBarVisibilityPreference.mode },
                        set: { model.setMenuBarVisibilityMode($0) }
                    )
                ) {
                    ForEach(MenuBarVisibilityMode.allCases) { mode in
                        Text(verbatim: mode.localizedTitle())
                            .tag(mode)
                    }
                }
                .accessibilityIdentifier(MenuBarVisibilityAccessibility.mode)
                .accessibilityHint("Controls when DUX appears in the menu bar")

                if model.menuBarVisibilityPreference.mode == .belowFreePercent {
                    Stepper(
                        value: Binding(
                            get: { model.menuBarVisibilityPreference.thresholdPercent },
                            set: { model.setMenuBarVisibilityThresholdPercent($0) }
                        ),
                        in: 1 ... 100
                    ) {
                        LabeledContent("Show at or below") {
                            Text(
                                verbatim:
                                    "\(model.menuBarVisibilityPreference.thresholdPercent)% free"
                            )
                        }
                    }
                    .accessibilityIdentifier(MenuBarVisibilityAccessibility.threshold)
                    .accessibilityHint(
                        "Sets the free-space percentage that makes DUX visible"
                    )

                    Text(
                        "DUX uses its cached startup-disk capacity and does not start a scan. "
                            + "It stays visible when capacity is unknown, and reopening the app "
                            + "reveals it for the rest of this session."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                Text(
                    "Free-space labels use startup-disk capacity available for important use, "
                        + "with filesystem availability as a clearly disclosed fallback."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                Text("Closing Explorer keeps DUX available from the menu bar.")
                    .foregroundStyle(.secondary)

                Divider()

                loginItemSettings(model: model)
            }

            applicationUpdateSettings()

            CLIInstallationSettingsView(model: model.cliInstallation)

            Section("Notifications") {
                notificationPermissionSettings(model: model)
            }

            Section("Disk pressure") {
                Text(
                    "DUX enters each pressure level when available storage reaches the "
                        + "smaller of its GiB and percentage limits."
                )
                .foregroundStyle(.secondary)

                if let policy = model.diskPressurePolicy {
                    LabeledContent("Policy source") {
                        Text(policy.source == .default ? "DUX defaults" : "Customized")
                    }
                    LabeledContent("Policy revision") {
                        Text(verbatim: String(policy.revision))
                    }
                    if let milliseconds = policy.updatedAtUnixMilliseconds {
                        LabeledContent("Policy updated") {
                            Text(
                                Date(timeIntervalSince1970: Double(milliseconds) / 1_000),
                                format: .dateTime
                            )
                        }
                    }
                }

                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                    GridRow {
                        Text(verbatim: "")
                        Text("GiB")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text("Percent")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    policyRow(
                        "Critical",
                        gib: $model.diskPressurePolicyDraft.criticalGiB,
                        percent: $model.diskPressurePolicyDraft.criticalPercent,
                        gibIdentifier: DiskPressurePolicyAccessibility.criticalGiB,
                        percentIdentifier: DiskPressurePolicyAccessibility.criticalPercent
                    )
                    policyRow(
                        "Warning",
                        gib: $model.diskPressurePolicyDraft.warningGiB,
                        percent: $model.diskPressurePolicyDraft.warningPercent,
                        gibIdentifier: DiskPressurePolicyAccessibility.warningGiB,
                        percentIdentifier: DiskPressurePolicyAccessibility.warningPercent
                    )
                    policyRow(
                        "Recovery margin",
                        gib: $model.diskPressurePolicyDraft.recoveryGiB,
                        percent: $model.diskPressurePolicyDraft.recoveryPercent,
                        gibIdentifier: DiskPressurePolicyAccessibility.recoveryGiB,
                        percentIdentifier: DiskPressurePolicyAccessibility.recoveryPercent
                    )
                }
                .accessibilityElement(children: .contain)
                .accessibilityLabel("Disk pressure thresholds")

                Text(
                    "Recovery requires available storage to clear the active boundary by "
                        + "both recovery margins. Values are stored exactly; DUX never rounds them."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                Text(
                    "These settings change classification only. They do not grant cleanup, "
                        + "notification, scan, or scheduling authority."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                if case let .failed(failure) = model.diskPressurePolicyState {
                    Label(Self.message(for: failure), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .accessibilityIdentifier(DiskPressurePolicyAccessibility.error)
                }

                HStack {
                    Button("Save thresholds") {
                        Task { await model.saveDiskPressurePolicy() }
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(
                        model.diskPressurePolicyState.isBusy
                            || model.diskPressurePolicy == nil
                    )
                    .accessibilityIdentifier(DiskPressurePolicyAccessibility.save)
                    .accessibilityHint("Validates and stores these exact thresholds")

                    Button("Restore DUX defaults") {
                        Task { await model.resetDiskPressurePolicy() }
                    }
                    .disabled(
                        model.diskPressurePolicyState.isBusy
                            || model.diskPressurePolicy == nil
                    )
                    .accessibilityIdentifier(DiskPressurePolicyAccessibility.reset)
                    .accessibilityHint("Restores the thresholds supplied by this DUX version")

                    if model.diskPressurePolicyState.isBusy {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityIdentifier(DiskPressurePolicyAccessibility.progress)
                            .accessibilityLabel(
                                model.diskPressurePolicyState == .loading
                                    ? "Loading disk pressure thresholds"
                                    : model.diskPressurePolicyState == .resetting
                                        ? "Restoring disk pressure thresholds"
                                        : "Saving disk pressure thresholds"
                            )
                    }
                }
            }

            Section("Cleanup safety") {
                Text(
                    "DUX blocks permanent-cleanup effects until you explicitly enable them. "
                        + "This opt-in is only a global safety gate; it never selects or "
                        + "approves a target, and DUX never lets AI approve or execute cleanup."
                )
                .foregroundStyle(.secondary)

                if let policy = model.permanentCleanupPolicy {
                    LabeledContent("Policy source") {
                        Text(policy.source == .default ? "DUX default" : "Stored choice")
                    }
                    LabeledContent("Policy revision") {
                        Text(verbatim: String(policy.revision))
                    }
                    if let milliseconds = policy.updatedAtUnixMilliseconds {
                        LabeledContent("Policy updated") {
                            Text(
                                Date(timeIntervalSince1970: Double(milliseconds) / 1_000),
                                format: .dateTime
                            )
                        }
                    }
                }

                Toggle(
                    "Allow permanent-cleanup effects",
                    isOn: Binding(
                        get: { model.permanentCleanupPolicy?.enabled ?? false },
                        set: { requested in
                            if requested {
                                permanentCleanupConfirmation = ""
                                showingPermanentCleanupConfirmation = true
                            } else {
                                Task { await model.setPermanentCleanupEnabled(false) }
                            }
                        }
                    )
                )
                .disabled(
                    model.permanentCleanupPolicy == nil
                        || model.permanentCleanupPolicyState.isBusy
                )
                .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.toggle)
                .accessibilityHint(
                    "Disabling is immediate; enabling requires typing the confirmation phrase"
                )

                if model.permanentCleanupPolicy?.enabled == false {
                    Label(
                        "Permanent cleanup is off (safe default)",
                        systemImage: "lock.shield.fill"
                    )
                    .foregroundStyle(.secondary)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.status)
                }

                if showingPermanentCleanupConfirmation {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(
                            "To continue, type “"
                                + AppModel.permanentCleanupEnableConfirmation
                                + "” exactly."
                        )
                        .font(.callout)
                        TextField(
                            AppModel.permanentCleanupEnableConfirmation,
                            text: $permanentCleanupConfirmation
                        )
                        .textFieldStyle(.roundedBorder)
                        .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.confirmation)
                        .accessibilityLabel("Permanent cleanup confirmation")
                        .accessibilityHint(
                            "Enter the exact phrase ENABLE PERMANENT CLEANUP"
                        )
                        HStack {
                            Button("Cancel") {
                                showingPermanentCleanupConfirmation = false
                                permanentCleanupConfirmation = ""
                            }
                            .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.cancel)
                            Button("Confirm enable") {
                                let phrase = permanentCleanupConfirmation
                                showingPermanentCleanupConfirmation = false
                                permanentCleanupConfirmation = ""
                                Task {
                                    await model.setPermanentCleanupEnabled(
                                        true,
                                        confirmation: phrase
                                    )
                                }
                            }
                            .disabled(
                                permanentCleanupConfirmation
                                    != AppModel.permanentCleanupEnableConfirmation
                            )
                            .keyboardShortcut(.defaultAction)
                            .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.confirm)
                        }
                    }
                    .padding(.vertical, 4)
                }

                HStack {
                    Button("Restore DUX default") {
                        Task { await model.resetPermanentCleanup() }
                    }
                    .disabled(
                        model.permanentCleanupPolicy == nil
                            || model.permanentCleanupPolicyState.isBusy
                    )
                    .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.reset)
                    .accessibilityHint(
                        "Restores the disabled DUX default"
                    )

                    if model.permanentCleanupPolicyState.isBusy {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.progress)
                            .accessibilityLabel(
                                model.permanentCleanupPolicyState == .loading
                                    ? "Loading cleanup safety setting"
                                    : "Updating cleanup safety setting"
                            )
                    }
                }
                .accessibilityElement(children: .contain)

                if case let .failed(failure) = model.permanentCleanupPolicyState {
                    Label(Self.message(for: failure), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                        .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.error)
                }
            }

            projectDiscoveryRootSettings(model: model)

            directCargoEnrollmentSettings(model: model)

            cleanupExclusionSettings(model: model)

            cleanupHistoryClearSettings(model: model)

            Section("Engine") {
                switch model.engineState {
                case .idle, .loading:
                    LabeledContent("Status") {
                        Text("Connecting…")
                    }
                case let .loaded(result):
                    LabeledContent("Status") {
                        Text("Ready")
                    }
                    LabeledContent("Library version") {
                        Text(verbatim: result.libraryVersion)
                    }
                    LabeledContent("FFI contract") {
                        Text(verbatim: String(result.ffiContractVersion))
                    }
                    LabeledContent("Database schema") {
                        Text(verbatim: String(result.databaseSchemaVersion))
                    }
                    LabeledContent("Snapshot format") {
                        Text(verbatim: String(result.snapshotFormatVersion))
                    }
                case .failed:
                    LabeledContent("Status") {
                        Text("Unavailable")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 620, height: 800)
        .task {
            await model.cliInstallation.loadStatus()
            await model.refreshLoginItemState()
            await model.refreshNotificationAuthorizationState()
            await model.loadDiskPressurePolicy()
            await model.ownedStorageFootprintSettings.load()
            await model.snapshotRetentionCapSettings.load()
            await model.loadPermanentCleanupPolicy()
            await model.loadCleanupExclusions()
            await model.loadProjectDiscoveryRoots()
            await model.loadDirectCargoEnrollmentStatus()
            await model.loadInitialState()
            await model.loadPersistentRecoveryDebt()
            await model.loadClaimedRunningScanProvenance()
        }
        .onDisappear {
            Task {
                await model.cliInstallation.cancelPreparedAction()
            }
        }
        .confirmationDialog(
            cleanupExclusionConfirmationTitle,
            isPresented: Binding(
                get: { cleanupExclusionConfirmationAction != nil },
                set: { presented in
                    if !presented {
                        cleanupExclusionConfirmationAction = nil
                    }
                }
            ),
            presenting: cleanupExclusionConfirmationAction
        ) { action in
            switch action {
            case let .remove(path):
                Button("Remove exclusion", role: .destructive) {
                    cleanupExclusionConfirmationAction = nil
                    Task {
                        await model.removeCleanupExclusion(path, confirmed: true)
                    }
                }
            case .reset:
                Button("Remove all exclusions", role: .destructive) {
                    cleanupExclusionConfirmationAction = nil
                    Task { await model.resetCleanupExclusions(confirmed: true) }
                }
            }
            Button("Cancel", role: .cancel) {
                cleanupExclusionConfirmationAction = nil
            }
        } message: { action in
            switch action {
            case let .remove(path):
                Text(
                    "Removing this deny-only prefix may allow a future reviewed plan to "
                        + "consider items under it:\n\(path.displayText)"
                )
            case .reset:
                Text(
                    "This removes every user exclusion and restores the empty DUX default. "
                        + "It does not start cleanup."
                )
            }
        }
        .confirmationDialog(
            directCargoConfirmationTitle,
            isPresented: Binding(
                get: { directCargoConfirmationAction != nil },
                set: { presented in
                    if !presented {
                        directCargoConfirmationAction = nil
                    }
                }
            ),
            presenting: directCargoConfirmationAction
        ) { action in
            switch action {
            case let .enroll(confirmation):
                Button("Run once and enroll") {
                    directCargoConfirmationAction = nil
                    Task {
                        await model.enrollInspectedDirectCargo(
                            confirmation: confirmation
                        )
                    }
                }
                .keyboardShortcut(.defaultAction)
                .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.confirmEnroll)
            case .revoke:
                Button("Revoke Cargo enrollment", role: .destructive) {
                    directCargoConfirmationAction = nil
                    Task { await model.revokeDirectCargoEnrollment(confirmed: true) }
                }
                .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.confirmRevoke)
            }
            Button("Cancel", role: .cancel) {
                directCargoConfirmationAction = nil
                if case let .enroll(confirmation) = action {
                    Task {
                        await model.discardDirectCargoEnrollmentPreview(
                            matching: confirmation
                        )
                    }
                }
            }
            .keyboardShortcut(.cancelAction)
            .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.confirmCancel)
        } message: { action in
            switch action {
            case let .enroll(confirmation):
                Text(directCargoEnrollmentConfirmationMessage(confirmation))
            case .revoke:
                Text(
                    "DUX will stop trusting the enrolled Cargo executable for discovery. "
                        + "This does not remove Cargo, project files, build output, or any "
                        + "other data."
                )
            }
        }
        .confirmationDialog(
            cleanupHistoryClearConfirmationTitle,
            isPresented: Binding(
                get: { cleanupHistoryClearConfirmationAction != nil },
                set: { presented in
                    guard !presented,
                          let confirmation = cleanupHistoryClearConfirmationAction else {
                        return
                    }
                    cleanupHistoryClearConfirmationAction = nil
                    Task { await model.cancelCleanupHistoryClear(confirmation) }
                }
            ),
            presenting: cleanupHistoryClearConfirmationAction
        ) { confirmation in
            Button("Clear cleanup history", role: .destructive) {
                cleanupHistoryClearConfirmationAction = nil
                Task { await model.confirmCleanupHistoryClear(confirmation) }
            }
            .keyboardShortcut(.defaultAction)
            .accessibilityIdentifier(CleanupHistoryClearAccessibility.confirm)

            Button("Cancel", role: .cancel) {
                cleanupHistoryClearConfirmationAction = nil
                Task { await model.cancelCleanupHistoryClear(confirmation) }
            }
            .keyboardShortcut(.cancelAction)
            .accessibilityIdentifier(CleanupHistoryClearAccessibility.cancel)
        } message: { confirmation in
            Text(cleanupHistoryClearConfirmationMessage(confirmation.preview))
                .accessibilityIdentifier(CleanupHistoryClearAccessibility.confirmation)
        }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else {
                return
            }
            Task {
                await model.refreshLoginItemState()
                await model.refreshNotificationAuthorizationState()
            }
        }
        .onDisappear {
            directCargoConfirmationAction = nil
            cleanupHistoryClearConfirmationAction = nil
            Task {
                await model.dismissDirectCargoEnrollmentPresentation()
                await model.dismissCleanupHistoryClearPresentation()
            }
        }
    }

    @ViewBuilder
    private func applicationUpdateSettings() -> some View {
        let updater = SparkleUpdateController.shared

        Section("Updates") {
            switch updater.availability {
            case .ready:
                Button("Check for Updates…") {
                    updater.checkForUpdates()
                }
                .accessibilityIdentifier(ApplicationUpdateAccessibility.check)
                Text(
                    "Sparkle verifies both the signed update feed and the downloaded app. "
                        + "The separately installed DUX CLI is never changed."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            case let .unavailable(message):
                LabeledContent("Automatic updates") {
                    Text("Not configured")
                }
                .accessibilityIdentifier(ApplicationUpdateAccessibility.status)
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func projectDiscoveryRootSettings(model: AppModel) -> some View {
        Section("Project discovery roots") {
            Text(
                "Save project folders for bounded, read-only recommendation scans while the "
                    + "startup disk is in Warning or Critical pressure. These roots are "
                    + "discovery scope only: adding one does not start a scan and never "
                    + "approves or performs cleanup."
            )
            .foregroundStyle(.secondary)

            Text(
                "DUX stores at most \(ProjectDiscoveryRoot.maximumCount) exact local paths. "
                    + "Nested or overlapping roots are rejected so a targeted run does not "
                    + "scan the same files twice."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            if let roots = model.projectDiscoveryRoots {
                LabeledContent("Root source") {
                    Text(roots.source == .default ? "Empty DUX default" : "Stored choice")
                }
                LabeledContent("Root revision") {
                    Text(verbatim: String(roots.revision))
                }
                if let milliseconds = roots.updatedAtUnixMilliseconds {
                    LabeledContent("Roots updated") {
                        Text(
                            Date(timeIntervalSince1970: Double(milliseconds) / 1_000),
                            format: .dateTime
                        )
                    }
                }

                if roots.roots.isEmpty {
                    Label("No project roots configured", systemImage: "folder.badge.questionmark")
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier(ProjectDiscoveryRootsAccessibility.status)
                } else {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(Array(roots.roots.enumerated()), id: \.element) { index, root in
                            HStack(alignment: .firstTextBaseline, spacing: 10) {
                                Text(verbatim: root.displayText)
                                    .font(.system(.body, design: .monospaced))
                                    .textSelection(.enabled)
                                    .lineLimit(2)
                                    .truncationMode(.middle)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                Button("Remove") {
                                    Task { await model.removeProjectDiscoveryRoot(root) }
                                }
                                .disabled(
                                    model.projectDiscoveryRootsState.isBusy
                                        || model
                                            .projectDiscoveryRootsRequiresAuthoritativeReload
                                )
                                .accessibilityIdentifier(
                                    ProjectDiscoveryRootsAccessibility.remove(index)
                                )
                                .accessibilityLabel("Remove project discovery root")
                                .accessibilityHint(
                                    "Stops later low-space discovery from scanning \(root.displayText)"
                                )
                            }
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier(ProjectDiscoveryRootsAccessibility.list)
                }

                HStack {
                    Button("Add project folder…") {
                        switch selectProjectDiscoveryRoot() {
                        case .cancelled:
                            break
                        case .invalid:
                            model.rejectProjectDiscoveryRootSelection()
                        case let .selected(root):
                            Task { await model.addProjectDiscoveryRoot(root) }
                        }
                    }
                    .disabled(
                        model.projectDiscoveryRootsState.isBusy
                            || model.projectDiscoveryRootsRequiresAuthoritativeReload
                            || roots.roots.count >= ProjectDiscoveryRoot.maximumCount
                    )
                    .accessibilityIdentifier(ProjectDiscoveryRootsAccessibility.add)
                    .accessibilityHint(
                        "Selects one local folder for bounded read-only low-space scans"
                    )

                    Button(
                        roots.roots.isEmpty ? "Restore DUX default" : "Remove all roots"
                    ) {
                        Task { await model.resetProjectDiscoveryRoots() }
                    }
                    .disabled(
                        model.projectDiscoveryRootsState.isBusy
                            || model.projectDiscoveryRootsRequiresAuthoritativeReload
                            || roots.source == .default
                    )
                    .accessibilityIdentifier(ProjectDiscoveryRootsAccessibility.reset)
                    .accessibilityHint(
                        "Restores the empty project-root default without deleting files"
                    )

                    if model.projectDiscoveryRootsState.isBusy {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityIdentifier(ProjectDiscoveryRootsAccessibility.progress)
                            .accessibilityLabel("Updating project discovery roots")
                    }
                }
            } else if model.projectDiscoveryRootsState.isBusy {
                ProgressView("Loading project discovery roots")
                    .accessibilityIdentifier(ProjectDiscoveryRootsAccessibility.progress)
            }

            if case let .failed(failure) = model.projectDiscoveryRootsState {
                Label(Self.message(for: failure), systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
                    .accessibilityIdentifier(ProjectDiscoveryRootsAccessibility.error)
                if model.projectDiscoveryRootsRequiresAuthoritativeReload {
                    Button("Reload saved roots") {
                        Task { await model.loadProjectDiscoveryRoots() }
                    }
                    .accessibilityHint(
                        "Reloads authoritative project roots before another change"
                    )
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(ProjectDiscoveryRootsAccessibility.section)
    }

    private func selectProjectDiscoveryRoot() -> ProjectDiscoveryRootPickerResult {
        let panel = NSOpenPanel()
        panel.title = "Choose a project folder for read-only discovery"
        panel.message =
            "DUX will store this exact folder as scan scope. This does not start cleanup."
        panel.prompt = "Add Root"
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = false
        panel.resolvesAliases = false
        guard panel.runModal() == .OK, let url = panel.url else {
            return .cancelled
        }
        let values = try? url.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        guard
            values?.isDirectory == true,
            values?.isSymbolicLink != true,
            let root = ProjectDiscoveryRoot(fileURL: url)
        else {
            return .invalid
        }
        return .selected(root)
    }

    @ViewBuilder
    private func directCargoEnrollmentSettings(model: AppModel) -> some View {
        Section("Rust project discovery") {
            Text(
                "Enroll one exact Cargo executable so DUX can understand recognized Rust "
                    + "target directories. Enrollment enables discovery only: it cannot "
                    + "select, plan, approve, schedule, or perform cleanup, and AI gains no "
                    + "authority from it."
            )
            .foregroundStyle(.secondary)

            Text(
                "Choose a direct toolchain Cargo executable. The common "
                    + "~/.cargo/bin/cargo rustup proxy symlink is intentionally rejected. "
                    + "DUX does not install or update Cargo and never accepts arbitrary commands."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            if let status = model.directCargoEnrollmentStatus {
                LabeledContent("Enrollment revision") {
                    Text(verbatim: String(status.revision))
                }
                if let milliseconds = status.updatedAtUnixMilliseconds {
                    LabeledContent("Updated") {
                        Text(
                            Date(timeIntervalSince1970: Double(milliseconds) / 1_000),
                            format: .dateTime
                        )
                    }
                }
                switch status.disposition {
                case .notEnrolled:
                    Label("No Cargo executable enrolled", systemImage: "circle.dashed")
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.status)
                case .revoked:
                    Label("Cargo enrollment revoked", systemImage: "xmark.shield")
                        .foregroundStyle(.orange)
                        .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.status)
                case let .enrolled(identity):
                    Label("Cargo identity enrolled", systemImage: "checkmark.shield")
                        .foregroundStyle(.green)
                        .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.status)
                    Text(
                        "The exact executable identity is revalidated before every use."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    directCargoIdentityDetails(identity)
                    Button("Revoke enrollment…", role: .destructive) {
                        directCargoConfirmationAction = .revoke
                    }
                    .disabled(
                        model.directCargoEnrollmentState.isBusy
                            || model.directCargoEnrollmentNeedsStatusReload
                    )
                    .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.revoke)
                    .accessibilityHint(
                        "Asks for confirmation before disabling Cargo-backed discovery"
                    )
                }
            } else if model.directCargoEnrollmentState == .loading {
                ProgressView("Loading Cargo enrollment")
                    .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.progress)
            }

            if model.directCargoEnrollmentNeedsStatusReload {
                Label(
                    "Current enrollment state is unverified",
                    systemImage: "questionmark.diamond"
                )
                .foregroundStyle(.red)
                Button("Reload Cargo status") {
                    Task { await model.loadDirectCargoEnrollmentStatus() }
                }
                .disabled(model.directCargoEnrollmentState.isBusy)
                .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.reload)
                .accessibilityHint(
                    "Reads authoritative enrollment state without retrying the prior mutation"
                )
            }

            if let preview = model.directCargoEnrollmentPreview {
                GroupBox("Inspected executable") {
                    VStack(alignment: .leading, spacing: 8) {
                        directCargoPreviewDetails(preview)
                        Text(
                            "Inspection performed static file and code-signature validation only. "
                                + "The selected executable has not been run."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.preview)

                HStack {
                    Button("Review and enroll exact Cargo…") {
                        if let confirmation = model.directCargoEnrollmentConfirmation {
                            directCargoConfirmationAction = .enroll(confirmation)
                        }
                    }
                    .disabled(model.directCargoEnrollmentState.isBusy)
                    .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.enroll)
                    .accessibilityHint(
                        "Shows exact identity evidence before Cargo is run once for version verification"
                    )

                    Button("Discard inspection") {
                        Task { await model.discardDirectCargoEnrollmentPreview() }
                    }
                    .disabled(model.directCargoEnrollmentState.isBusy)
                    .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.discard)
                    .accessibilityHint("Releases the inspected preview without enrolling Cargo")
                }
            }

            HStack {
                Button("Choose Cargo executable…") {
                    switch selectDirectCargoExecutable() {
                    case .cancelled:
                        break
                    case .invalid:
                        model.rejectDirectCargoExecutableSelection()
                    case let .selected(selection):
                        Task {
                            await model.inspectDirectCargoExecutable(selection)
                            if let confirmation =
                                model.directCargoEnrollmentConfirmation
                            {
                                directCargoConfirmationAction = .enroll(confirmation)
                            }
                        }
                    }
                }
                .disabled(
                    model.directCargoEnrollmentState.isBusy
                        || model.directCargoEnrollmentNeedsStatusReload
                )
                .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.choose)
                .accessibilityHint(
                    "Selects one exact file named cargo for static inspection"
                )

                if model.directCargoEnrollmentState.isBusy {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.progress)
                        .accessibilityLabel(directCargoProgressLabel(model))
                }
            }

            if case let .failed(failure) = model.directCargoEnrollmentState {
                Label(
                    Self.message(
                        for: failure,
                        authoritativeReloadRequired:
                            model.directCargoEnrollmentNeedsStatusReload
                    ),
                    systemImage: "exclamationmark.triangle"
                )
                    .foregroundStyle(.red)
                    .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.error)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(DirectCargoEnrollmentAccessibility.section)
    }

    @ViewBuilder
    private func directCargoPreviewDetails(
        _ preview: DirectCargoEnrollmentPreviewModel
    ) -> some View {
        LabeledContent("Path") {
            selectableTechnicalText(preview.executable.displayPath)
        }
        LabeledContent("Executable SHA-256") {
            selectableTechnicalText(preview.executableSHA256.duxLowercaseHex)
        }
        directCargoSignatureDetails(preview.signature)
        LabeledContent("Required Cargo version") {
            Text("1.96.0")
        }
    }

    @ViewBuilder
    private func directCargoIdentityDetails(
        _ identity: DirectCargoEnrollmentIdentityModel
    ) -> some View {
        LabeledContent("Cargo path") {
            selectableTechnicalText(identity.executable.displayPath)
        }
        LabeledContent("Cargo version") {
            Text(verbatim: identity.version.displayText)
        }
        LabeledContent("Executable SHA-256") {
            selectableTechnicalText(identity.executableSHA256.duxLowercaseHex)
        }
        LabeledContent("Version evidence SHA-256") {
            selectableTechnicalText(identity.versionSHA256.duxLowercaseHex)
        }
        directCargoSignatureDetails(identity.signature)
    }

    @ViewBuilder
    private func directCargoSignatureDetails(
        _ signature: DirectCargoSignatureEvidence
    ) -> some View {
        LabeledContent("Signature") {
            Text(verbatim: signature.kindLabel)
        }
        Text(
            signature.kind == .adHoc
                ? "Ad-hoc signing proves local byte integrity only."
                : "CMS evidence is displayed for identity review; DUX does not treat it "
                    + "as a publisher allowlist."
        )
        .font(.caption)
        .foregroundStyle(.secondary)
        LabeledContent("Signing identifier") {
            selectableTechnicalText(signature.signingIdentifier)
        }
        if let teamIdentifier = signature.teamIdentifier {
            LabeledContent("Team identifier") {
                selectableTechnicalText(teamIdentifier)
            }
        }
        ForEach(Array(signature.codeDirectoryHashes.enumerated()), id: \.offset) { index, hash in
            LabeledContent(
                signature.codeDirectoryHashes.count == 1
                    ? "Code Directory hash"
                    : "Code Directory hash \(index + 1)"
            ) {
                selectableTechnicalText(hash.duxLowercaseHex)
            }
        }
        if let digest = signature.designatedRequirementSHA256 {
            LabeledContent("Requirement SHA-256") {
                selectableTechnicalText(digest.duxLowercaseHex)
            }
        }
    }

    private func selectableTechnicalText(_ text: String) -> some View {
        Text(verbatim: text)
            .font(.system(.caption, design: .monospaced))
            .textSelection(.enabled)
            .lineLimit(3)
            .truncationMode(.middle)
    }

    private func selectDirectCargoExecutable() -> DirectCargoExecutablePickerResult {
        let panel = NSOpenPanel()
        panel.title = "Choose the exact Cargo executable DUX may inspect"
        panel.message =
            "Inspection validates the selected file and its signature without running it."
        panel.prompt = "Inspect"
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.canCreateDirectories = false
        panel.resolvesAliases = false
        guard panel.runModal() == .OK, let url = panel.url else {
            return .cancelled
        }
        guard let selection = DirectCargoExecutableSelection(fileURL: url) else {
            return .invalid
        }
        return .selected(selection)
    }

    private var directCargoConfirmationTitle: String {
        switch directCargoConfirmationAction {
        case .enroll(_): "Run and enroll this exact Cargo executable?"
        case .revoke: "Revoke Cargo enrollment?"
        case nil: "Confirm Cargo enrollment change"
        }
    }

    private func directCargoEnrollmentConfirmationMessage(
        _ confirmation: DirectCargoEnrollmentConfirmation
    ) -> String {
        let preview = confirmation.preview
        let team = preview.signature.teamIdentifier.map { ", team \($0)" } ?? ""
        let codeDirectoryHashes = preview.signature.codeDirectoryHashes
            .map(\.duxLowercaseHex)
            .joined(separator: ", ")
        return "Path: \(preview.executable.displayPath)\n"
            + "Executable SHA-256: \(preview.executableSHA256.duxLowercaseHex)\n"
            + "Signature: \(preview.signature.kindLabel), "
            + "\(preview.signature.signingIdentifier)\(team)\n"
            + "Code Directory hash"
            + (preview.signature.codeDirectoryHashes.count == 1 ? ": " : "es: ")
            + "\(codeDirectoryHashes)\n\n"
            + "DUX will now run these exact inspected bytes once with a fixed, bounded "
            + "verbose-version check to verify Cargo 1.96.0 "
            + "and bind the version-output hash before storing discovery trust. "
            + "This does not start or authorize cleanup."
    }

    private func directCargoProgressLabel(_ model: AppModel) -> String {
        switch model.directCargoEnrollmentState {
        case .loading: "Loading Cargo enrollment"
        case .inspecting: "Inspecting Cargo without running it"
        case .enrolling:
            "Finishing Cargo enrollment; the exact inspected executable may be running once"
        case .revoking: "Revoking Cargo enrollment"
        case .idle, .ready, .awaitingEnrollmentConfirmation, .failed:
            "Updating Cargo enrollment"
        }
    }

    @ViewBuilder
    private func cleanupExclusionSettings(model: AppModel) -> some View {
        Section("Cleanup exclusions") {
            Text(
                "These exact path prefixes can only block cleanup. They never approve a target, "
                    + "start an effect, or let AI remove anything."
            )
            .foregroundStyle(.secondary)

            if let exclusions = model.cleanupExclusions {
                LabeledContent("Exclusion source") {
                    Text(exclusions.source == .default ? "Empty DUX default" : "Stored choice")
                }
                LabeledContent("Exclusion revision") {
                    Text(verbatim: String(exclusions.revision))
                }
                if let milliseconds = exclusions.updatedAtUnixMilliseconds {
                    LabeledContent("Exclusions updated") {
                        Text(
                            Date(timeIntervalSince1970: Double(milliseconds) / 1_000),
                            format: .dateTime
                        )
                    }
                }

                if exclusions.paths.isEmpty {
                    Label("No user cleanup exclusions", systemImage: "checkmark.shield")
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier(CleanupExclusionsAccessibility.status)
                } else {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(Array(exclusions.paths.enumerated()), id: \.element) { index, path in
                            HStack(alignment: .firstTextBaseline, spacing: 10) {
                                Text(verbatim: path.displayText)
                                    .font(.system(.body, design: .monospaced))
                                    .textSelection(.enabled)
                                    .lineLimit(2)
                                    .truncationMode(.middle)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                Button("Remove…", role: .destructive) {
                                    cleanupExclusionConfirmationAction = .remove(path)
                                }
                                .accessibilityIdentifier(CleanupExclusionsAccessibility.remove(index))
                                .accessibilityLabel("Remove cleanup exclusion")
                                .accessibilityHint(
                                    "Asks for confirmation before removing \(path.displayText)"
                                )
                            }
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier(CleanupExclusionsAccessibility.list)
                }

                HStack {
                    Button("Add path…") {
                        if let path = selectCleanupExclusionPath() {
                            Task { await model.addCleanupExclusion(path) }
                        }
                    }
                    .disabled(model.cleanupExclusionsState.isBusy || exclusions.paths.count >= 64)
                    .accessibilityIdentifier(CleanupExclusionsAccessibility.add)
                    .accessibilityHint("Selects a file or folder prefix that cleanup must avoid")

                    Button("Restore empty default…") {
                        cleanupExclusionConfirmationAction = .reset
                    }
                    .disabled(model.cleanupExclusionsState.isBusy || exclusions.paths.isEmpty)
                    .accessibilityIdentifier(CleanupExclusionsAccessibility.reset)
                    .accessibilityHint("Asks for confirmation before removing every exclusion")

                    if model.cleanupExclusionsState.isBusy {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityIdentifier(CleanupExclusionsAccessibility.progress)
                            .accessibilityLabel("Updating cleanup exclusions")
                    }
                }
            } else if model.cleanupExclusionsState.isBusy {
                ProgressView("Loading cleanup exclusions")
                    .accessibilityIdentifier(CleanupExclusionsAccessibility.progress)
            }

            if case let .failed(failure) = model.cleanupExclusionsState {
                Label(Self.message(for: failure), systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
                    .accessibilityIdentifier(CleanupExclusionsAccessibility.error)
            }
        }
    }

    private var cleanupExclusionConfirmationTitle: String {
        switch cleanupExclusionConfirmationAction {
        case .remove: "Remove this cleanup exclusion?"
        case .reset: "Remove all cleanup exclusions?"
        case nil: "Confirm cleanup exclusion change"
        }
    }

    private func selectCleanupExclusionPath() -> CleanupExclusionPathObservation? {
        let panel = NSOpenPanel()
        panel.title = "Choose a path DUX cleanup must avoid"
        panel.prompt = "Exclude"
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = true
        panel.canChooseFiles = true
        panel.canCreateDirectories = false
        panel.resolvesAliases = false
        guard panel.runModal() == .OK, let url = panel.url else {
            return nil
        }
        return CleanupExclusionPathObservation(fileURL: url)
    }

    @ViewBuilder
    private func cleanupHistoryClearSettings(model: AppModel) -> some View {
        Section("Storage & Privacy") {
            DuxOwnedStorageFootprintSettingsView(
                settings: model.ownedStorageFootprintSettings
            )

            Divider()

            SnapshotRetentionCapSettingsView(
                settings: model.snapshotRetentionCapSettings
            )

            Divider()

            persistentRecoveryDebtSettings(model)

            Divider()

            claimedRunningScanProvenanceSettings(model)

            Divider()

            VStack(alignment: .leading, spacing: 10) {
                Text(
                    "Cleanup history is DUX’s local activity log. Clearing it does not delete "
                        + "files, snapshots, scan or candidate history, settings, exclusions, "
                        + "capacity samples, or AI insights."
                )
                .foregroundStyle(.secondary)

                Text(
                    "This privacy action does not run cleanup, compact the database, resample "
                        + "capacity, or promise to free disk space. Active or uncertain cleanup "
                        + "evidence is preserved automatically."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                cleanupHistoryClearStateDetails(model)

                HStack {
                    Button("Clear cleanup history…", role: .destructive) {
                        Task {
                            await model.prepareCleanupHistoryClear()
                            if let confirmation = model.cleanupHistoryClearConfirmation {
                                cleanupHistoryClearConfirmationAction = confirmation
                            }
                        }
                    }
                    .disabled(
                        model.cleanupHistoryClearState.isBusy
                            || model.cleanupHistoryClearConfirmation != nil
                    )
                    .accessibilityIdentifier(CleanupHistoryClearAccessibility.prepare)
                    .accessibilityHint(
                        "Prepares an exact expiring preview before any history can be removed"
                    )

                    if model.cleanupHistoryClearState.isBusy {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityIdentifier(
                                CleanupHistoryClearAccessibility.progress
                            )
                            .accessibilityLabel(cleanupHistoryClearProgressLabel(model))
                    }

                    switch model.cleanupHistoryClearState {
                    case .completed, .failed, .outcomeUnknown:
                        Button("Dismiss") {
                            model.dismissCleanupHistoryClearNotice()
                        }
                        .accessibilityIdentifier(
                            CleanupHistoryClearAccessibility.dismiss
                        )
                    case .idle, .preparing, .awaitingConfirmation, .clearing:
                        EmptyView()
                    }
                }

                switch model.cleanupHistoryClearState {
                case let .failed(failure):
                    Label(
                        Self.message(for: failure),
                        systemImage: failure == .nothingToClear
                            ? "checkmark.circle"
                            : "exclamationmark.triangle"
                    )
                    .foregroundStyle(
                        failure == .nothingToClear ? Color.secondary : Color.red
                    )
                    .accessibilityIdentifier(CleanupHistoryClearAccessibility.error)
                case .outcomeUnknown:
                    Label(
                        Self.cleanupHistoryClearOutcomeUnknownMessage,
                        systemImage: "questionmark.diamond"
                    )
                    .foregroundStyle(.red)
                    .accessibilityIdentifier(CleanupHistoryClearAccessibility.error)
                case .idle, .preparing, .awaitingConfirmation, .clearing, .completed:
                    EmptyView()
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(CleanupHistoryClearAccessibility.section)
        }
    }

    @ViewBuilder
    private func persistentRecoveryDebtSettings(_ model: AppModel) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Label(
                    "Unclaimed running records",
                    systemImage: persistentRecoveryDebtSystemImage(model)
                )
                .accessibilityIdentifier(PersistentRecoveryDebtAccessibility.status)

                Spacer()

                Text(model.persistentRecoveryDebt?.displayedCount ?? "—")
                    .font(.title2.weight(.semibold))
                    .monospacedDigit()
                    .accessibilityIdentifier(PersistentRecoveryDebtAccessibility.count)
                    .accessibilityLabel(
                        model.persistentRecoveryDebt?.accessibilityCount
                            ?? "Retained recovery record count unavailable"
                    )
            }

            if let observation = model.persistentRecoveryDebt {
                Text(
                    observation.inspectedUnclaimedCount == 0
                        ? "No unclaimed running scan records were observed."
                        : "DUX observed unclaimed running scan bookkeeping in its local database."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                if let readAt = model.persistentRecoveryDebtReadAt {
                    Text(
                        "Checked "
                            + readAt.formatted(date: .abbreviated, time: .shortened)
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }

                DisclosureGroup(
                    "Why DUX keeps these records",
                    isExpanded: $showingPersistentRecoveryDebtDetails
                ) {
                    Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                        GridRow {
                            Text("Older-format shape")
                            Text(verbatim: String(observation.pristineUnclaimedCount))
                                .monospacedDigit()
                        }
                        GridRow {
                            Text("Unexpected running shape")
                            Text(verbatim: String(observation.unexplainedUnclaimedCount))
                                .monospacedDigit()
                        }
                    }
                    .padding(.top, 4)

                    Text(
                        observation.hasMore
                            ? "The bounded inspection stops after 64 records, so more may exist."
                            : "The bounded inspection reached the end of the unclaimed records."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)

                    Text(
                        "A record being unclaimed does not prove that a process is dead or "
                            + "that recovery is safe. DUX does not infer ownership from age or PID."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                .accessibilityIdentifier(PersistentRecoveryDebtAccessibility.details)
            }

            Text(
                "This is bookkeeping, not disk usage or reclaimable space. This "
                    + "unclaimed-record check cannot recover or delete anything and reads "
                    + "only DUX’s database; it does not search temporary folders or "
                    + "attribute older external snapshot stages."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier(PersistentRecoveryDebtAccessibility.limitations)

            HStack {
                Button("Refresh unclaimed diagnostics") {
                    Task {
                        await model.refreshPersistentRecoveryDebt()
                    }
                }
                .disabled(model.persistentRecoveryDebtState.isLoading)
                .accessibilityIdentifier(PersistentRecoveryDebtAccessibility.refresh)
                .accessibilityHint(
                    "Reads the bounded DUX database census again without recovering or deleting"
                )

                if model.persistentRecoveryDebtState.isLoading {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityIdentifier(PersistentRecoveryDebtAccessibility.progress)
                        .accessibilityLabel("Inspecting retained recovery records")
                }
            }

            if case let .failed(failure) = model.persistentRecoveryDebtState {
                Label(
                    persistentRecoveryDebtMessage(
                        failure,
                        showingEarlierResult: model.persistentRecoveryDebt != nil
                    ),
                    systemImage: "exclamationmark.triangle"
                )
                .foregroundStyle(.red)
                .accessibilityIdentifier(PersistentRecoveryDebtAccessibility.error)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(PersistentRecoveryDebtAccessibility.section)
    }

    @ViewBuilder
    private func claimedRunningScanProvenanceSettings(_ model: AppModel) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Label(
                    "Claimed running records",
                    systemImage: claimedRunningScanProvenanceSystemImage(model)
                )
                .accessibilityIdentifier(ClaimedRunningScanProvenanceAccessibility.status)

                Spacer()

                Text(model.claimedRunningScanProvenance?.displayedCount ?? "—")
                    .font(.title2.weight(.semibold))
                    .monospacedDigit()
                    .accessibilityIdentifier(ClaimedRunningScanProvenanceAccessibility.count)
                    .accessibilityLabel(
                        model.claimedRunningScanProvenance?.accessibilityCount
                            ?? "Claimed running scan record count unavailable"
                    )
            }

            if let observation = model.claimedRunningScanProvenance {
                Text(
                    observation.inspectedClaimedCount == 0
                        ? "No claimed running scan records were observed."
                        : "DUX compared claimed bookkeeping with current execution provenance."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                if observation.inspectedClaimedCount > 0 {
                    GeometryReader { proxy in
                        let nonemptySegmentCount = [
                            observation.sameHostCurrentBootCount,
                            observation.sameHostPriorBootCount,
                            observation.foreignHostCount,
                            observation.storedUnprovenCount,
                            observation.currentContextUnavailableCount,
                        ].filter { $0 > 0 }.count
                        let availableWidth = max(
                            0,
                            proxy.size.width
                                - CGFloat(max(0, nonemptySegmentCount - 1)) * 2
                        )
                        HStack(spacing: 2) {
                            claimedProvenanceSegment(
                                count: observation.sameHostCurrentBootCount,
                                total: observation.inspectedClaimedCount,
                                width: availableWidth,
                                color: .blue
                            )
                            claimedProvenanceSegment(
                                count: observation.sameHostPriorBootCount,
                                total: observation.inspectedClaimedCount,
                                width: availableWidth,
                                color: .purple
                            )
                            claimedProvenanceSegment(
                                count: observation.foreignHostCount,
                                total: observation.inspectedClaimedCount,
                                width: availableWidth,
                                color: .orange
                            )
                            claimedProvenanceSegment(
                                count: observation.storedUnprovenCount,
                                total: observation.inspectedClaimedCount,
                                width: availableWidth,
                                color: .gray
                            )
                            claimedProvenanceSegment(
                                count: observation.currentContextUnavailableCount,
                                total: observation.inspectedClaimedCount,
                                width: availableWidth,
                                color: .pink
                            )
                        }
                    }
                    .frame(height: 9)
                    .clipShape(Capsule())
                    .accessibilityElement(children: .ignore)
                    .accessibilityIdentifier(
                        ClaimedRunningScanProvenanceAccessibility.chart
                    )
                    .accessibilityLabel("Claimed record provenance distribution")
                    .accessibilityValue(observation.accessibilityDistribution)
                }

                if let readAt = model.claimedRunningScanProvenanceReadAt {
                    Text(
                        "Checked "
                            + readAt.formatted(date: .abbreviated, time: .shortened)
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }

                DisclosureGroup(
                    "Provenance categories",
                    isExpanded: $showingClaimedRunningScanProvenanceDetails
                ) {
                    Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                        claimedProvenanceRow(
                            "Current startup session",
                            systemImage: "power.circle.fill",
                            color: .blue,
                            count: observation.sameHostCurrentBootCount,
                            accessibilityIdentifier:
                            ClaimedRunningScanProvenanceAccessibility.currentBoot
                        )
                        claimedProvenanceRow(
                            "Earlier startup session on this Mac",
                            systemImage: "clock.arrow.circlepath",
                            color: .purple,
                            count: observation.sameHostPriorBootCount,
                            accessibilityIdentifier:
                            ClaimedRunningScanProvenanceAccessibility.priorBoot
                        )
                        claimedProvenanceRow(
                            "Different host",
                            systemImage: "desktopcomputer",
                            color: .orange,
                            count: observation.foreignHostCount,
                            accessibilityIdentifier:
                            ClaimedRunningScanProvenanceAccessibility.foreignHost
                        )
                        claimedProvenanceRow(
                            "Identity not stored",
                            systemImage: "questionmark.circle",
                            color: .gray,
                            count: observation.storedUnprovenCount,
                            accessibilityIdentifier:
                            ClaimedRunningScanProvenanceAccessibility.storedUnproven
                        )
                        claimedProvenanceRow(
                            "Current identity unavailable",
                            systemImage: "exclamationmark.circle",
                            color: .pink,
                            count: observation.currentContextUnavailableCount,
                            accessibilityIdentifier:
                            ClaimedRunningScanProvenanceAccessibility
                                .currentContextUnavailable
                        )
                    }
                    .padding(.top, 4)

                    Text(
                        observation.hasMore
                            ? "The bounded inspection stops after 64 records, so more may exist."
                            : "The bounded inspection reached the end of the claimed records."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                .accessibilityIdentifier(ClaimedRunningScanProvenanceAccessibility.details)
            }

            Text(
                "These categories compare private local execution provenance. They do not "
                    + "prove that a process is alive, that a record can be interrupted, or "
                    + "that disk space can be reclaimed. This screen cannot recover or "
                    + "delete anything. It reads bounded DUX bookkeeping and current OS "
                    + "identity evidence; it never searches user or temporary files."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier(ClaimedRunningScanProvenanceAccessibility.limitations)

            HStack {
                Button("Refresh claimed diagnostics") {
                    Task {
                        await model.refreshClaimedRunningScanProvenance()
                    }
                }
                .disabled(model.claimedRunningScanProvenanceState.isLoading)
                .accessibilityIdentifier(ClaimedRunningScanProvenanceAccessibility.refresh)
                .accessibilityHint(
                    "Repeats the bounded provenance comparison without probing processes, "
                        + "recovering, or deleting"
                )

                if model.claimedRunningScanProvenanceState.isLoading {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityIdentifier(
                            ClaimedRunningScanProvenanceAccessibility.progress
                        )
                        .accessibilityLabel("Comparing claimed running record provenance")
                }
            }

            if case let .failed(failure) = model.claimedRunningScanProvenanceState {
                Label(
                    claimedRunningScanProvenanceMessage(
                        failure,
                        showingEarlierResult: model.claimedRunningScanProvenance != nil
                    ),
                    systemImage: "exclamationmark.triangle"
                )
                .foregroundStyle(.red)
                .accessibilityIdentifier(ClaimedRunningScanProvenanceAccessibility.error)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(ClaimedRunningScanProvenanceAccessibility.section)
    }

    @ViewBuilder
    private func claimedProvenanceSegment(
        count: UInt16,
        total: UInt16,
        width: CGFloat,
        color: Color
    ) -> some View {
        if count > 0, total > 0 {
            color.frame(
                width: max(2, width * CGFloat(count) / CGFloat(total))
            )
        }
    }

    @ViewBuilder
    private func claimedProvenanceRow(
        _ title: String,
        systemImage: String,
        color: Color,
        count: UInt16,
        accessibilityIdentifier: String
    ) -> some View {
        GridRow {
            Label(title, systemImage: systemImage)
                .symbolRenderingMode(.monochrome)
                .foregroundStyle(color)
            Text(verbatim: String(count))
                .monospacedDigit()
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(accessibilityIdentifier)
    }

    private func claimedRunningScanProvenanceSystemImage(_ model: AppModel) -> String {
        guard let observation = model.claimedRunningScanProvenance else {
            return "questionmark.folder"
        }
        if observation.foreignHostCount > 0
            || observation.storedUnprovenCount > 0
            || observation.currentContextUnavailableCount > 0
            || observation.hasMore
        {
            return "info.circle"
        }
        return observation.inspectedClaimedCount == 0
            ? "checkmark.circle"
            : "point.3.connected.trianglepath.dotted"
    }

    private func claimedRunningScanProvenanceMessage(
        _ failure: ClaimedRunningScanProvenanceServiceError,
        showingEarlierResult: Bool
    ) -> String {
        let prefix = showingEarlierResult
            ? "Refresh failed; the earlier bounded result remains visible. "
            : ""
        let detail = switch failure {
        case .closed: "The storage engine is closed."
        case .incompatibleSchema: "The DUX database is newer than this app."
        case .retryable: "The DUX database is busy. Try again."
        case .unsafeStorage: "The DUX database location failed its safety checks."
        case .budgetExceeded: "The bounded inspection reached its resource limit."
        case .corruptData: "The claimed running records are inconsistent."
        case .unavailable: "The claimed running records are unavailable."
        case .internalState: "The diagnostics service is unavailable."
        case .invalidResponse: "The diagnostics response was invalid."
        }
        return prefix + detail
    }

    private func persistentRecoveryDebtSystemImage(_ model: AppModel) -> String {
        guard let observation = model.persistentRecoveryDebt else {
            return "questionmark.folder"
        }
        if observation.unexplainedUnclaimedCount > 0 || observation.hasMore {
            return "exclamationmark.triangle"
        }
        return observation.inspectedUnclaimedCount == 0
            ? "checkmark.circle"
            : "clock.arrow.circlepath"
    }

    private func persistentRecoveryDebtMessage(
        _ failure: PersistentRecoveryDebtServiceError,
        showingEarlierResult: Bool
    ) -> String {
        let prefix = showingEarlierResult
            ? "Refresh failed; the earlier bounded result remains visible. "
            : ""
        let detail = switch failure {
        case .closed: "The storage engine is closed."
        case .incompatibleSchema: "The DUX database is newer than this app."
        case .retryable: "The DUX database is busy. Try again."
        case .unsafeStorage: "The DUX database location failed its safety checks."
        case .budgetExceeded: "The bounded inspection reached its resource limit."
        case .corruptData: "The retained recovery records are inconsistent."
        case .unavailable: "The retained recovery records are unavailable."
        case .internalState: "The diagnostics service is unavailable."
        case .invalidResponse: "The diagnostics response was invalid."
        }
        return prefix + detail
    }

    @ViewBuilder
    private func cleanupHistoryClearStateDetails(_ model: AppModel) -> some View {
        switch model.cleanupHistoryClearState {
        case .idle:
            EmptyView()
        case .preparing:
            Label("Preparing an exact history preview…", systemImage: "clock")
                .accessibilityIdentifier(CleanupHistoryClearAccessibility.status)
        case let .awaitingConfirmation(confirmation):
            Label(
                cleanupHistorySessionCount(
                    confirmation.preview.sessionCount,
                    suffix: "ready for review"
                ),
                systemImage: "doc.text.magnifyingglass"
            )
            .accessibilityIdentifier(CleanupHistoryClearAccessibility.status)
        case let .clearing(preview):
            Label(
                cleanupHistorySessionCount(
                    preview.sessionCount,
                    suffix: "being removed"
                ),
                systemImage: "trash"
            )
            .accessibilityIdentifier(CleanupHistoryClearAccessibility.status)
        case let .completed(result):
            Label(
                cleanupHistorySessionCount(
                    result.clearedSessionCount,
                    suffix: "removed from DUX"
                ),
                systemImage: "checkmark.circle.fill"
            )
            .foregroundStyle(.green)
            .accessibilityIdentifier(CleanupHistoryClearAccessibility.status)
        case .failed, .outcomeUnknown:
            EmptyView()
        }
    }

    private func cleanupHistoryClearProgressLabel(_ model: AppModel) -> String {
        switch model.cleanupHistoryClearState {
        case .preparing:
            "Preparing cleanup history preview"
        case .clearing:
            "Clearing cleanup history"
        case .idle, .awaitingConfirmation, .completed, .failed, .outcomeUnknown:
            "Updating cleanup history"
        }
    }

    private var cleanupHistoryClearConfirmationTitle: String {
        guard let confirmation = cleanupHistoryClearConfirmationAction else {
            return "Clear cleanup history?"
        }
        return cleanupHistorySessionCount(
            confirmation.preview.sessionCount,
            suffix: "will be removed"
        )
    }

    private func cleanupHistoryClearConfirmationMessage(
        _ preview: CleanupHistoryClearPreviewModel
    ) -> String {
        let oldest = preview.oldestStartedAt.formatted(
            date: .abbreviated,
            time: .shortened
        )
        let newest = preview.newestStartedAt.formatted(
            date: .abbreviated,
            time: .shortened
        )
        return
            "Clear \(cleanupHistorySessionCount(preview.sessionCount)) from \(oldest) "
            + "through \(newest)? This removes DUX’s local activity log only. It does "
            + "not delete files, snapshots, scan or candidate history, settings, "
            + "exclusions, capacity samples, or AI insights. It does not promise to "
            + "free disk space and cannot be undone."
    }

    private func cleanupHistorySessionCount(
        _ count: UInt64,
        suffix: String? = nil
    ) -> String {
        let noun = count == 1 ? "cleanup history session" : "cleanup history sessions"
        let base = "\(count.formatted()) \(noun)"
        return suffix.map { base + " " + $0 } ?? base
    }

    @ViewBuilder
    private func loginItemSettings(model: AppModel) -> some View {
        let presentation = LoginItemPresentation.make(state: model.loginItemState)
        Toggle(
            "Launch at login",
            isOn: Binding(
                get: { presentation.toggleOn },
                set: { requested in
                    Task { await model.setLaunchAtLogin(requested) }
                }
            )
        )
        .disabled(!presentation.toggleEnabled)
        .accessibilityIdentifier(LoginItemAccessibility.toggle)
        .accessibilityHint(
            "Uses the current macOS Login Items setting; separate approval may be required"
        )

        LabeledContent("Login item status") {
            HStack(spacing: 8) {
                if let progressLabel = presentation.progressLabel {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityIdentifier(LoginItemAccessibility.progress)
                        .accessibilityLabel(Text(verbatim: progressLabel))
                }
                if model.loginItemState.status == .requiresApproval {
                    Image(systemName: "exclamationmark.triangle")
                }
                Text(verbatim: presentation.statusTitle)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(LoginItemAccessibility.status)

        Text(verbatim: presentation.statusDetail)
            .font(.caption)
            .foregroundStyle(.secondary)

        if presentation.showsApprovalAction {
            Button("Open Login Items Settings…") {
                AppActivation.openLoginItemsSettings()
            }
            .accessibilityIdentifier(LoginItemAccessibility.openSystemSettings)
            .accessibilityHint("Opens macOS System Settings so you can approve DUX")
        }

        if presentation.showsRefreshAction {
            Button("Check again") {
                Task { await model.refreshLoginItemState() }
            }
            .accessibilityIdentifier(LoginItemAccessibility.refresh)
            .accessibilityHint("Reads the Login Items status from macOS again")
        }

        if let message = presentation.errorMessage {
            Label {
                Text(verbatim: message)
            } icon: {
                Image(systemName: "exclamationmark.triangle")
            }
            .foregroundStyle(.red)
            .accessibilityIdentifier(LoginItemAccessibility.error)
        }

        Text("Uses macOS Login Items. DUX installs no daemon or privileged helper.")
            .font(.caption)
            .foregroundStyle(.secondary)
    }

    @ViewBuilder
    private func notificationPermissionSettings(model: AppModel) -> some View {
        let presentation = NotificationAuthorizationPresentation.make(
            state: model.notificationAuthorizationState
        )

        LabeledContent("Permission") {
            HStack(spacing: 8) {
                if let progressLabel = presentation.progressLabel {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityIdentifier(
                            NotificationAuthorizationAccessibility.progress
                        )
                        .accessibilityLabel(Text(verbatim: progressLabel))
                }
                Text(verbatim: presentation.statusTitle)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(NotificationAuthorizationAccessibility.status)

        Text(verbatim: presentation.statusDetail)
            .font(.caption)
            .foregroundStyle(.secondary)

        HStack {
            if presentation.showsRequestAction {
                Button("Allow notifications…") {
                    Task { await model.requestNotificationAuthorization() }
                }
                .accessibilityIdentifier(
                    NotificationAuthorizationAccessibility.request
                )
                .accessibilityHint(
                    "Asks macOS for permission only; DUX does not schedule an alert"
                )
            }

            if presentation.showsSystemSettingsAction {
                Button("Open System Settings…") {
                    AppActivation.openNotificationSettings()
                }
                .accessibilityIdentifier(
                    NotificationAuthorizationAccessibility.openSystemSettings
                )
                .accessibilityHint(
                    "Opens System Settings; choose Notifications, then DUX"
                )
            }

            if presentation.showsRefreshAction {
                Button("Check again") {
                    Task { await model.refreshNotificationAuthorizationState() }
                }
                .accessibilityIdentifier(
                    NotificationAuthorizationAccessibility.refresh
                )
                .accessibilityHint("Reads the notification permission from macOS again")
            }
        }

        if let message = presentation.errorMessage {
            Label {
                Text(verbatim: message)
            } icon: {
                Image(systemName: "exclamationmark.triangle")
            }
            .foregroundStyle(.red)
            .accessibilityIdentifier(NotificationAuthorizationAccessibility.error)
        }

        Text(
            String(
                localized: "Permission is optional and reserved for future low-disk pressure alerts. It grants no scan, scheduling, or cleanup authority."
            )
        )
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    private func policyRow(
        _ title: LocalizedStringKey,
        gib: Binding<String>,
        percent: Binding<String>,
        gibIdentifier: String,
        percentIdentifier: String
    ) -> some View {
        GridRow {
            Text(title)
            TextField("GiB", text: gib)
                .frame(width: 120)
                .accessibilityLabel(Text(title) + Text(" available GiB"))
                .accessibilityHint("Enter an exact value greater than zero")
                .accessibilityIdentifier(gibIdentifier)
            TextField("Percent", text: percent)
                .frame(width: 120)
                .accessibilityLabel(Text(title) + Text(" available percent"))
                .accessibilityHint(
                    "Enter an exact value greater than zero and no more than 100 percent"
                )
                .accessibilityIdentifier(percentIdentifier)
        }
    }

    static func message(for failure: DiskPressurePolicyFailure) -> String {
        switch failure {
        case let .draft(error):
            switch error {
            case let .invalidNumber(field):
                String(
                    localized: "Enter an exact non-negative number for \(fieldName(field))."
                )
            case let .zero(field):
                String(localized: "\(fieldName(field)) must be greater than zero.")
            case let .outOfRange(field):
                String(localized: "\(fieldName(field)) must be no more than 100 percent.")
            }
        case let .service(error):
            switch error {
            case .warningBytesBelowCritical, .warningBasisPointsBelowCritical:
                String(
                    localized: "Warning limits must be greater than or equal to Critical limits."
                )
            case .warningThresholdMatchesCritical:
                String(localized: "Warning and Critical limits cannot both be identical.")
            case .thresholdBytesZero, .recoveryBytesZero:
                String(localized: "GiB values must be greater than zero.")
            case .thresholdBasisPointsOutOfRange, .recoveryBasisPointsOutOfRange:
                String(
                    localized: "Percentage values must be greater than zero and no more than 100."
                )
            case .revisionExhausted:
                String(
                    localized: "The policy revision limit has been reached. Restarting will not change it."
                )
            case .closed:
                String(localized: "The storage engine session is closed.")
            case .invalidRecordVersion, .incompatibleSchema:
                String(localized: "This DUX policy format is incompatible with the current app.")
            case .retryable, .invalidClock, .outcomeUnknown:
                String(localized: "The policy could not be saved safely. Try again.")
            case .unsafeStorage, .corruptData, .unavailable, .internalState,
                 .invalidResponse:
                String(
                    localized: "Disk pressure settings are unavailable. Your last saved policy is unchanged."
                )
            }
        case .unexpected:
            String(
                localized: "Disk pressure settings are unavailable. Your last saved policy is unchanged."
            )
        }
    }

    static func message(for failure: PermanentCleanupPolicyFailure) -> String {
        switch failure {
        case .confirmationRequired:
            String(localized: "Type ENABLE PERMANENT CLEANUP exactly to enable this setting.")
        case let .service(error):
            switch error {
            case .closed:
                String(localized: "The storage engine session is closed.")
            case .retryable, .invalidClock, .outcomeUnknown:
                String(localized: "The setting could not be changed safely. Try again.")
            case .incompatibleSchema:
                String(localized: "This cleanup-safety setting is incompatible with this app.")
            case .unsafeStorage, .corruptData, .unavailable, .budgetExceeded, .internalState,
                 .invalidResponse, .revisionExhausted:
                String(localized: "Cleanup-safety settings are unavailable. Your last choice is unchanged.")
            }
        case .unexpected:
            String(localized: "Cleanup-safety settings are unavailable. Your last choice is unchanged.")
        }
    }

    static func message(for failure: CleanupExclusionsFailure) -> String {
        switch failure {
        case .confirmationRequired:
            String(localized: "Confirm the removal before weakening cleanup protection.")
        case .invalidSelection:
            String(localized: "Choose an absolute local file or folder path.")
        case let .service(error):
            switch error {
            case .closed:
                String(localized: "The storage engine session is closed.")
            case .invalidPath:
                String(localized: "That path cannot be stored as a cleanup exclusion.")
            case .tooManyPaths:
                String(localized: "DUX supports at most 64 cleanup exclusions.")
            case .invalidRecordVersion, .incompatibleSchema:
                String(localized: "This cleanup-exclusion format is incompatible with this app.")
            case .retryable, .invalidClock, .outcomeUnknown:
                String(localized: "Cleanup exclusions could not be changed safely. Try again.")
            case .revisionExhausted, .unsafeStorage, .budgetExceeded, .corruptData,
                 .unavailable, .internalState, .invalidResponse:
                String(
                    localized: "Cleanup exclusions are unavailable. Your last saved exclusions are unchanged."
                )
            }
        case .unexpected:
            String(
                localized: "Cleanup exclusions are unavailable. Your last saved exclusions are unchanged."
            )
        }
    }

    static func message(for failure: ProjectDiscoveryRootsFailure) -> String {
        switch failure {
        case .invalidSelection:
            String(localized: "Choose an absolute local folder that is not a symbolic link.")
        case .overlappingSelection:
            String(
                localized:
                    "That folder overlaps a configured project root. Choose one non-nested root."
            )
        case let .service(error):
            switch error {
            case .closed:
                String(localized: "The storage engine session is closed.")
            case .invalidPath:
                String(localized: "That folder cannot be stored as a project discovery root.")
            case .tooManyPaths:
                String(
                    localized:
                        "DUX supports at most \(ProjectDiscoveryRoot.maximumCount) project roots."
                )
            case .overlappingPaths:
                String(
                    localized:
                        "Project discovery roots must be unique and must not contain one another."
                )
            case .invalidRecordVersion, .incompatibleSchema:
                String(localized: "This project-root format is incompatible with this app.")
            case .outcomeUnknown, .internalState, .invalidResponse:
                String(
                    localized:
                        "DUX could not confirm the saved roots. Reload them before another change."
                )
            case .retryable, .invalidClock:
                String(localized: "Project roots could not be changed safely. Try again.")
            case .revisionExhausted, .unsafeStorage, .budgetExceeded, .corruptData,
                 .unavailable:
                String(
                    localized:
                        "Project roots are unavailable. Your last saved roots are unchanged."
                )
            }
        case .unexpected:
            String(
                localized: "Project roots are unavailable. Your last saved roots are unchanged."
            )
        }
    }

    static func message(
        for failure: DirectCargoEnrollmentFailure,
        authoritativeReloadRequired: Bool = true
    ) -> String {
        switch failure {
        case .invalidSelection:
            String(
                localized:
                    "Choose an absolute canonical UTF-8 file path named cargo with no control characters."
            )
        case .enrollmentConfirmationRequired:
            String(localized: "Review the inspected identity before enrolling Cargo.")
        case .enrollmentPreviewChanged:
            String(
                localized:
                    "The inspected Cargo identity changed after confirmation opened. Review the current identity again."
            )
        case .revocationConfirmationRequired:
            String(localized: "Confirm before revoking Cargo-backed discovery.")
        case let .service(error):
            switch error {
            case .closed:
                String(localized: "The storage engine session is closed.")
            case .unsupportedPlatform:
                String(localized: "Direct Cargo enrollment is available only on macOS.")
            case .invalidRecordVersion, .incompatibleSchema:
                String(localized: "This Cargo enrollment format is incompatible with this app.")
            case .invalidExecutablePath:
                String(
                    localized:
                        "Choose one canonical local executable file named cargo; aliases and PATH lookup are not accepted."
                )
            case .executableNotRegular:
                String(localized: "The selected Cargo path is not a regular file.")
            case .changedDuringInspection:
                String(
                    localized:
                        "Cargo or its resolution environment changed. Inspect the executable again."
                )
            case .inspectionUnavailable:
                String(localized: "DUX could not inspect the selected Cargo executable.")
            case .inspectionLimitExceeded:
                String(localized: "Cargo inspection exceeded its fixed safety budget.")
            case .invalidResolutionEnvironment:
                String(localized: "Cargo's executable-resolution directories are not safe to use.")
            case .invalidCargoVersion:
                String(localized: "The exact executable is not supported Cargo 1.96.0.")
            case .invalidCodeSignature:
                String(
                    localized:
                        "The selected Cargo executable lacks acceptable bounded macOS signing evidence."
                )
            case .previewUnavailable, .wrongEngine:
                String(localized: "That inspection expired. Inspect the Cargo executable again.")
            case .revisionExhausted:
                String(localized: "The Cargo enrollment revision limit has been reached.")
            case .retryable, .invalidClock:
                String(localized: "Cargo enrollment is temporarily unavailable. Try again.")
            case .outcomeUnknown:
                if authoritativeReloadRequired {
                    String(
                        localized:
                            "DUX could not prove whether enrollment changed. Reload status before trying again."
                    )
                } else {
                    String(
                        localized:
                            "DUX could not prove the mutation response. Authoritative status was reloaded and no operation was retried."
                    )
                }
            case .unsafeStorage, .budgetExceeded, .corruptData, .unavailable,
                 .internalState, .invalidResponse:
                String(
                    localized:
                        "Cargo enrollment is unavailable. No cleanup action was performed."
                )
            }
        case .unexpected:
            String(localized: "Cargo enrollment is unavailable. No cleanup action was performed.")
        }
    }

    static let cleanupHistoryClearOutcomeUnknownMessage = String(
        localized:
            "DUX could not prove whether cleanup history was cleared. DUX attempted one read-only history refresh, retried nothing, and ran no file cleanup."
    )

    static func message(for failure: CleanupHistoryClearServiceError) -> String {
        switch failure {
        case .closed:
            String(localized: "The storage engine session is closed.")
        case .nothingToClear:
            String(localized: "There is no terminal cleanup history to clear.")
        case .activeCleanup:
            String(
                localized:
                    "Cleanup history includes unfinished work. DUX will not erase recovery evidence."
            )
        case .changedSincePreview:
            String(
                localized:
                    "Cleanup history changed after review. Nothing was removed; prepare a new preview."
            )
        case .previewExpired:
            String(
                localized:
                    "The cleanup history preview expired. Nothing was removed; prepare a new preview."
            )
        case .wrongEngine, .previewUnavailable:
            String(
                localized:
                    "That cleanup history preview is no longer available. Nothing was removed."
            )
        case .incompatibleSchema:
            String(
                localized:
                    "This cleanup history format is incompatible with the current app."
            )
        case .retryable:
            String(
                localized:
                    "Cleanup history is temporarily busy. Nothing was removed; try again."
            )
        case .unsafeStorage:
            String(
                localized:
                    "DUX cannot verify safe access to its history store. Nothing was removed."
            )
        case .budgetExceeded:
            String(
                localized:
                    "Cleanup history exceeded the fixed review budget. Nothing was removed."
            )
        case .corruptData:
            String(
                localized:
                    "Cleanup history is inconsistent. DUX preserved it for diagnosis."
            )
        case .outcomeUnknown:
            cleanupHistoryClearOutcomeUnknownMessage
        case .unavailable, .internalState, .invalidResponse:
            String(
                localized:
                    "Cleanup history clearing is unavailable. No file cleanup was performed."
            )
        }
    }

    private static func fieldName(_ field: DiskPressurePolicyField) -> String {
        switch field {
        case .criticalGiB: String(localized: "Critical GiB")
        case .criticalPercent: String(localized: "Critical percent")
        case .warningGiB: String(localized: "Warning GiB")
        case .warningPercent: String(localized: "Warning percent")
        case .recoveryGiB: String(localized: "Recovery GiB")
        case .recoveryPercent: String(localized: "Recovery percent")
        }
    }
}
