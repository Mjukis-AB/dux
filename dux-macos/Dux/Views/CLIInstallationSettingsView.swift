import SwiftUI

struct CLIInstallationSettingsView: View {
    let model: CLIInstallationModel
    @State private var acceptedConfirmationToken: UUID?

    var body: some View {
        @Bindable var model = model

        Section("Command-line companion") {
            Text(
                "Install the bundled DUX command for optional Terminal use. The CLI is a "
                    + "separate process with Terminal’s permissions; it does not inherit "
                    + "this app’s Full Disk Access or other privacy permissions."
            )
            .foregroundStyle(.secondary)

            if let status = model.state.status {
                let presentation = CLIInstallationPresentation.make(status: status)

                LabeledContent("Status") {
                    Label(
                        presentation.statusTitle,
                        systemImage: statusSymbol(status.disposition)
                    )
                    .foregroundStyle(statusColor(status.disposition))
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier(CLIInstallationAccessibility.status)

                Text(verbatim: presentation.statusDetail)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                LabeledContent("Install target") {
                    Text(verbatim: CLIInstallationStatus.destinationDisplayText)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                }
                .accessibilityIdentifier(CLIInstallationAccessibility.destination)

                LabeledContent("Bundled CLI") {
                    Text(verbatim: presentation.bundledVersion)
                }
                .accessibilityIdentifier(CLIInstallationAccessibility.bundledVersion)

                LabeledContent("Storage compatibility") {
                    Text(
                        verbatim:
                            "database \(status.bundled.databaseSchemaVersion), "
                            + "snapshot \(status.bundled.snapshotFormatVersion)"
                    )
                }

                if let installedVersion = presentation.installedVersion {
                    LabeledContent("Installed CLI") {
                        Text(verbatim: installedVersion)
                    }
                    .accessibilityIdentifier(
                        CLIInstallationAccessibility.installedVersion
                    )
                }

                LabeledContent("Source") {
                    Text(verbatim: presentation.sourceTitle)
                }
                .accessibilityIdentifier(CLIInstallationAccessibility.source)

                Text(verbatim: presentation.pathDetail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier(CLIInstallationAccessibility.path)

                if status.pathEnvironment != .included {
                    Text(
                        "You can always run ~/.local/bin/dux directly. To use `dux` by "
                            + "name, add ~/.local/bin to PATH using your shell’s normal "
                            + "configuration. DUX never reads or edits shell startup files."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                }

                controls(model: model, presentation: presentation)
            } else if model.state.activity == .loading {
                ProgressView("Checking CLI installation")
                    .accessibilityIdentifier(CLIInstallationAccessibility.progress)
            } else {
                Button("Reload") {
                    Task { await model.loadStatus(force: true) }
                }
                .accessibilityIdentifier(CLIInstallationAccessibility.refresh)
                .accessibilityHint("Retries reading the fixed CLI destination")
            }

            if model.state.requiresAuthoritativeReload {
                Label(
                    "The last operation’s outcome is unverified. Reload status before "
                        + "attempting another change.",
                    systemImage: "questionmark.diamond"
                )
                .foregroundStyle(.red)
            }

            if let failure = model.state.failure {
                Label(
                    Self.message(for: failure),
                    systemImage: "exclamationmark.triangle"
                )
                .foregroundStyle(.red)
                .accessibilityIdentifier(CLIInstallationAccessibility.error)
            }

            Text(
                "Sparkle and app removal never change the installed CLI. After an app "
                    + "update, this section reports any version mismatch and waits for "
                    + "your explicit choice."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(CLIInstallationAccessibility.section)
        .confirmationDialog(
            confirmationTitle,
            isPresented: Binding(
                get: { model.confirmation != nil },
                set: { presented in
                    if !presented {
                        if acceptedConfirmationToken != nil {
                            acceptedConfirmationToken = nil
                        } else {
                            Task { await model.cancelPreparedAction() }
                        }
                    }
                }
            ),
            presenting: model.confirmation
        ) { confirmation in
            if confirmation.action == .uninstall {
                Button("Uninstall CLI", role: .destructive) {
                    acceptedConfirmationToken = confirmation.token
                    Task { await model.confirmPreparedAction() }
                }
                .accessibilityIdentifier(CLIInstallationAccessibility.confirm)
            } else {
                Button(confirmActionTitle(confirmation.action)) {
                    acceptedConfirmationToken = confirmation.token
                    Task { await model.confirmPreparedAction() }
                }
                .accessibilityIdentifier(CLIInstallationAccessibility.confirm)
            }
            Button("Cancel", role: .cancel) {
                Task { await model.cancelPreparedAction() }
            }
            .accessibilityIdentifier(CLIInstallationAccessibility.cancel)
        } message: { confirmation in
            Text(verbatim: confirmationMessage(confirmation))
        }
    }

    @ViewBuilder
    private func controls(
        model: CLIInstallationModel,
        presentation: CLIInstallationPresentation
    ) -> some View {
        HStack {
            if
                let action = presentation.primaryAction,
                let title = presentation.primaryActionTitle
            {
                Button(title) {
                    Task { await model.prepare(action) }
                }
                .disabled(model.state.isBusy || model.state.requiresAuthoritativeReload)
                .accessibilityIdentifier(accessibilityIdentifier(for: action))
                .accessibilityHint(
                    "Shows the exact version and destination before changing the CLI"
                )
            }

            if presentation.offersUninstall {
                Button("Uninstall CLI…", role: .destructive) {
                    Task { await model.prepare(.uninstall) }
                }
                .disabled(model.state.isBusy || model.state.requiresAuthoritativeReload)
                .accessibilityIdentifier(CLIInstallationAccessibility.uninstall)
                .accessibilityHint(
                    "Shows confirmation before removing the exact app-installed CLI"
                )
            }

            Button("Reload") {
                Task { await model.loadStatus(force: true) }
            }
            .disabled(model.state.isBusy)
            .accessibilityIdentifier(CLIInstallationAccessibility.refresh)
            .accessibilityHint("Reads the fixed CLI destination without changing it")

            if let activity = model.state.activity {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityIdentifier(CLIInstallationAccessibility.progress)
                    .accessibilityLabel(Text(verbatim: progressLabel(activity)))
            }
        }
    }

    private var confirmationTitle: String {
        guard let confirmation = model.confirmation else {
            return "Confirm CLI change"
        }
        return switch confirmation.action {
        case .install: "Install the DUX CLI?"
        case .upgrade: "Upgrade the DUX CLI?"
        case .reinstall: "Reinstall the DUX CLI?"
        case .uninstall: "Uninstall the DUX CLI?"
        }
    }

    private func confirmationMessage(
        _ confirmation: CLIInstallationConfirmation
    ) -> String {
        let destination = confirmation.destinationDisplayText
        return switch confirmation.action {
        case .install:
            "Install DUX \(confirmation.bundledVersion.displayText) at \(destination)? "
                + "DUX will create only the missing per-user .local/bin directories."
        case .upgrade:
            "Replace the app-installed DUX "
                + "\(confirmation.installedVersion?.displayText ?? "unknown") with "
                + "\(confirmation.bundledVersion.displayText) at \(destination)?"
        case .reinstall:
            "Replace the exact app-installed DUX "
                + "\(confirmation.installedVersion?.displayText ?? "unknown") with the "
                + "bundled \(confirmation.bundledVersion.displayText) build at \(destination)?"
        case .uninstall:
            "Remove only the verified app-installed DUX "
                + "\(confirmation.installedVersion?.displayText ?? "unknown") from "
                + "\(destination)? The directory and all other files remain unchanged."
        }
    }

    private func confirmActionTitle(_ action: CLIInstallationAction) -> String {
        switch action {
        case .install: "Install CLI"
        case .upgrade: "Upgrade CLI"
        case .reinstall: "Reinstall CLI"
        case .uninstall: "Uninstall CLI"
        }
    }

    private func accessibilityIdentifier(
        for action: CLIInstallationAction
    ) -> String {
        switch action {
        case .install: CLIInstallationAccessibility.install
        case .upgrade: CLIInstallationAccessibility.upgrade
        case .reinstall: CLIInstallationAccessibility.reinstall
        case .uninstall: CLIInstallationAccessibility.uninstall
        }
    }

    private func progressLabel(_ activity: CLIInstallationActivity) -> String {
        switch activity {
        case .loading: "Checking CLI installation"
        case .preparingInstall: "Preparing CLI installation confirmation"
        case .installing: "Installing the verified CLI"
        case .preparingUninstall: "Preparing CLI uninstall confirmation"
        case .uninstalling: "Uninstalling the verified app-installed CLI"
        }
    }

    private func statusSymbol(
        _ disposition: CLIInstallationDisposition
    ) -> String {
        switch disposition {
        case .absent: "terminal"
        case let .managed(installation):
            switch installation.relation {
            case .current: "checkmark.circle.fill"
            case .older, .sameVersionDifferentBuild: "arrow.triangle.2.circlepath.circle"
            case .newer: "arrow.up.circle"
            }
        case .unmanaged: "exclamationmark.shield"
        case .unsafe: "xmark.shield.fill"
        }
    }

    private func statusColor(
        _ disposition: CLIInstallationDisposition
    ) -> Color {
        switch disposition {
        case .absent: .secondary
        case let .managed(installation):
            installation.relation == .current ? .green : .orange
        case .unmanaged: .orange
        case .unsafe: .red
        }
    }

    private static func message(
        for failure: CLIInstallationFailure
    ) -> String {
        switch failure {
        case .unexpected:
            "DUX could not inspect or change the CLI installation."
        case let .service(error):
            switch error {
            case .busy:
                "Another DUX process is changing the CLI installation."
            case .resourceMissing:
                "This copy of DUX does not contain its bundled CLI."
            case .metadataTooLarge, .invalidMetadata:
                "The bundled CLI compatibility manifest is invalid."
            case .invalidBundledBinary, .invalidArchitecture:
                "The bundled CLI failed executable or universal-architecture validation."
            case .invalidCodeSignature:
                "The bundled or installed CLI failed code-signature validation."
            case .unsupportedAccount:
                "DUX could not determine the current macOS account safely."
            case .unsafeHome, .unsafeInstallDirectory:
                "DUX refused an unsafe home or .local/bin directory."
            case .unsafeDestination:
                "DUX refused the existing CLI destination because it is unsafe or changed."
            case .unmanagedDestination:
                "An existing file was not installed by DUX and remains unchanged."
            case .downgradeRefused:
                "DUX will not replace a newer installed CLI with an older one."
            case .confirmationUnavailable, .confirmationChanged:
                "The reviewed CLI state changed. Reload it before trying again."
            case .retryable:
                "The CLI operation did not complete. The last verified state is shown."
            case .outcomeUnknown:
                "DUX could not prove the final CLI state. Reload before trying again."
            }
        }
    }
}
