import SwiftUI

enum MenuBarLabelAccessibility {
    static let picker = "menu-bar-label-mode"
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
    static let error = "permanent-cleanup-error"
    static let progress = "permanent-cleanup-progress"
    static let status = "permanent-cleanup-status"

    static let allControlIdentifiers = [
        toggle,
        reset,
        confirmation,
        confirm,
        error,
        progress,
        status,
    ]
}

private enum PermanentCleanupConfirmationAction {
    case enable
    case reset
}

struct DuxSettingsView: View {
    @Environment(\.scenePhase) private var scenePhase
    @State private var permanentCleanupConfirmation = ""
    @State private var showingPermanentCleanupConfirmation = false
    @State private var permanentCleanupConfirmationAction:
        PermanentCleanupConfirmationAction = .enable

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
                    "The DUX default permits reviewed permanent-cleanup effects to run. "
                        + "This switch is only a global safety gate; it never selects or "
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
                                permanentCleanupConfirmationAction = .enable
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
                    "Disabling is immediate; re-enabling requires typing the confirmation phrase"
                )

                if model.permanentCleanupPolicy?.enabled == false {
                    Label(
                        "Permanent-cleanup effects are currently blocked",
                        systemImage: "hand.raised.fill"
                    )
                    .foregroundStyle(.orange)
                    .accessibilityElement(children: .combine)
                }

                if showingPermanentCleanupConfirmation {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(
                            "To continue, type “"
                                + AppModel.permanentCleanupReenableConfirmation
                                + "” exactly."
                        )
                        .font(.callout)
                        TextField(
                            AppModel.permanentCleanupReenableConfirmation,
                            text: $permanentCleanupConfirmation
                        )
                        .textFieldStyle(.roundedBorder)
                        .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.confirmation)
                        HStack {
                            Button("Cancel") {
                                showingPermanentCleanupConfirmation = false
                                permanentCleanupConfirmation = ""
                            }
                            Button(
                                permanentCleanupConfirmationAction == .reset
                                    ? "Restore default"
                                    : "Confirm re-enable"
                            ) {
                                let phrase = permanentCleanupConfirmation
                                showingPermanentCleanupConfirmation = false
                                permanentCleanupConfirmation = ""
                                Task {
                                    switch permanentCleanupConfirmationAction {
                                    case .enable:
                                        await model.setPermanentCleanupEnabled(
                                            true,
                                            confirmation: phrase
                                        )
                                    case .reset:
                                        await model.resetPermanentCleanup(confirmation: phrase)
                                    }
                                }
                            }
                            .disabled(
                                permanentCleanupConfirmation
                                    != AppModel.permanentCleanupReenableConfirmation
                            )
                            .keyboardShortcut(.defaultAction)
                            .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.confirm)
                        }
                    }
                    .padding(.vertical, 4)
                }

                HStack {
                    Button("Restore DUX default") {
                        if model.permanentCleanupPolicy?.enabled == false {
                            permanentCleanupConfirmationAction = .reset
                            permanentCleanupConfirmation = ""
                            showingPermanentCleanupConfirmation = true
                        } else {
                            Task { await model.resetPermanentCleanup() }
                        }
                    }
                    .disabled(
                        model.permanentCleanupPolicy == nil
                            || model.permanentCleanupPolicyState.isBusy
                    )
                    .accessibilityIdentifier(PermanentCleanupPolicyAccessibility.reset)
                    .accessibilityHint(
                        "Restores the enabled DUX default and may require typed confirmation"
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
            await model.refreshLoginItemState()
            await model.refreshNotificationAuthorizationState()
            await model.loadDiskPressurePolicy()
            await model.loadPermanentCleanupPolicy()
            await model.loadInitialState()
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
            String(localized: "Type ENABLE PERMANENT CLEANUP exactly to re-enable this setting.")
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
