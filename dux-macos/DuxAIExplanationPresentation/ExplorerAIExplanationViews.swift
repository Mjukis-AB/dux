import SwiftUI

public enum ExplorerAIExplanationAccessibility {
    public static let explain = "explorer-snapshot-ai-explain"
    public static let disclosure = "explorer-snapshot-ai-disclosure"
    public static let metadata = "explorer-snapshot-ai-metadata"
    public static let send = "explorer-snapshot-ai-send"
    public static let cancel = "explorer-snapshot-ai-cancel"
    public static let progress = "explorer-snapshot-ai-progress"
    public static let result = "explorer-snapshot-ai-result"
    public static let failure = "explorer-snapshot-ai-failure"
    public static let legend = "explorer-snapshot-ai-legend"
}

/// The complete set of non-authority navigation requests a presentation view
/// may make of its host. It cannot carry paths, item identity, or actions.
public enum ExplorerAIExplanationHostIntent: Equatable, Sendable {
    case openProviderSettings
}

/// Module-owned sheet attachment. The host can render it but cannot inspect or
/// retain the private session which backs the displayed disclosure.
public struct ExplorerAIExplanationModalPresenter: View {
    @Bindable private var model: ExplorerAIExplanationModel

    public init(model: ExplorerAIExplanationModel) {
        self.model = model
    }

    public var body: some View {
        Color.clear
            .frame(width: 0, height: 0)
            .sheet(isPresented: disclosurePresented) {
                ExplorerAIExplanationConsentView(model: model)
            }
    }

    private var disclosurePresented: Binding<Bool> {
        Binding(
            get: { model.activeDisclosure != nil },
            set: { presented in
                guard !presented else { return }
                Task { await model.cancel() }
            }
        )
    }
}

public struct ExplorerAIExplanationConsentView: View {
    @Bindable private var model: ExplorerAIExplanationModel
    @State private var showsMetadata = false

    public init(model: ExplorerAIExplanationModel) {
        self.model = model
    }

    public var body: some View {
        if let disclosure = model.activeDisclosure {
            consent(disclosure)
        } else {
            EmptyView()
        }
    }

    private func consent(_ disclosure: ExplorerAIExplanationDisclosure) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 5) {
                Label("Preview AI explanation", systemImage: "sparkles")
                    .font(.title2.bold())
                Text(
                    "Review the exact path-free metadata before choosing whether to send it once."
                )
                .foregroundStyle(.secondary)
            }

            GroupBox("Provider") {
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
                    disclosureRow("Provider", disclosure.providerName)
                    disclosureRow("Model", disclosure.model)
                    disclosureRow(
                        "Adapter",
                        "\(disclosure.adapterID) · revision \(disclosure.adapterRevision)"
                    )
                    disclosureRow(
                        "Metadata limit",
                        byteCount(disclosure.maximumMetadataInputBytes)
                    )
                    disclosureRow(
                        "Encoded request limit",
                        byteCount(disclosure.maximumEncodedRequestBytes)
                    )
                    disclosureRow(
                        "Response limit",
                        byteCount(disclosure.maximumResponseBytes)
                    )
                    disclosureRow(
                        "Maximum output tokens",
                        String(disclosure.maximumOutputTokens)
                    )
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            GroupBox("What leaves this Mac") {
                VStack(alignment: .leading, spacing: 8) {
                    Label(
                        "Path-free storage sizes, kinds, age buckets, and opaque item labels",
                        systemImage: "checkmark.shield.fill"
                    )
                    privacyFact("File content", included: disclosure.preview.contentIncluded)
                    privacyFact("Source names", included: disclosure.preview.sourceNamesIncluded)
                    privacyFact("Source paths", included: disclosure.preview.sourcePathsIncluded)
                    Text(
                        "DUX inspected \(disclosure.preview.inspectedNodeCount) snapshot nodes, included \(disclosure.preview.includedDirectChildCount) direct children, excluded \(disclosure.preview.excludedSensitiveDirectChildCount) sensitive children, and omitted \(disclosure.preview.omittedEligibleDirectChildCount) eligible children from the bounded payload."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    Text(
                        "This single-use preview expires \(expiryText(disclosure)). Its SHA-256 digest begins \(disclosure.preview.inputDigestSHA256.prefix(12))."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            DisclosureGroup("View metadata sent", isExpanded: $showsMetadata) {
                ScrollView([.horizontal, .vertical]) {
                    Text(verbatim: disclosure.preview.encodedInputJSON)
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                }
                .frame(minHeight: 120, maxHeight: 220)
                .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
                .accessibilityIdentifier(ExplorerAIExplanationAccessibility.metadata)
            }

            GroupBox("Billing and data handling") {
                VStack(alignment: .leading, spacing: 7) {
                    Text(
                        "Your Anthropic account may be billed. DUX does not infer Zero Data Retention from an API key."
                    )
                    Text(
                        "The reviewed policy says API inputs and outputs are normally deleted within \(disclosure.standardAPIDeletionWithinDays) days. Flagged inputs or outputs may be retained for up to \(disclosure.flaggedInputOutputRetentionYears) years, and safety-classification scores for up to \(disclosure.safetyScoreRetentionYears) years. Legal or abuse-prevention needs may retain data longer. The fixed structured-output grammar may be cached for up to \(disclosure.structuredOutputGrammarMayBeCachedHours) hours; it contains no DUX metadata."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    Link(
                        "Read Anthropic's reviewed data-retention policy",
                        destination: disclosure.providerPolicyURL
                    )
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            Label(
                "AI can only explain and group this saved metadata. It cannot create cleanup plans, approve actions, or remove anything.",
                systemImage: "hand.raised.fill"
            )
            .font(.callout.weight(.semibold))

            HStack {
                Spacer()
                Button(model.isExplaining ? "Cancel Explanation" : "Cancel") {
                    Task { await model.cancel() }
                }
                .keyboardShortcut(.cancelAction)
                .accessibilityIdentifier(ExplorerAIExplanationAccessibility.cancel)

                if model.isExplaining {
                    ProgressView("Waiting for Anthropic…")
                        .controlSize(.small)
                        .accessibilityIdentifier(ExplorerAIExplanationAccessibility.progress)
                } else {
                    Button("Explain Selection") {
                        Task { await model.explain(disclosureID: disclosure.id) }
                    }
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier(ExplorerAIExplanationAccessibility.send)
                    .accessibilityHint(
                        "Sends this exact metadata once to Anthropic; no cleanup authority is granted"
                    )
                }
            }
        }
        .padding(22)
        .frame(minWidth: 620, idealWidth: 680, minHeight: 640)
        .accessibilityIdentifier(ExplorerAIExplanationAccessibility.disclosure)
    }

    private func expiryText(_ disclosure: ExplorerAIExplanationDisclosure) -> String {
        Date(
            timeIntervalSince1970:
            Double(disclosure.preview.expiresAtUnixMilliseconds) / 1000
        ).formatted(date: .omitted, time: .standard)
    }

    private func disclosureRow(_ label: String, _ value: String) -> some View {
        GridRow {
            Text(verbatim: label)
                .foregroundStyle(.secondary)
            Text(verbatim: value)
                .textSelection(.enabled)
        }
    }

    private func byteCount(_ value: Int) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(value), countStyle: .binary)
    }

    private func privacyFact(_ title: String, included: Bool) -> some View {
        Label(
            "\(title): \(included ? "included" : "not included")",
            systemImage: included ? "exclamationmark.triangle.fill" : "xmark.circle.fill"
        )
        .foregroundStyle(included ? Color.orange : Color.secondary)
    }
}

public struct ExplorerAIExplanationStatusView: View {
    @Bindable private var model: ExplorerAIExplanationModel
    private let sendIntent: @MainActor (ExplorerAIExplanationHostIntent) -> Void

    public init(
        model: ExplorerAIExplanationModel,
        sendIntent: @escaping @MainActor (ExplorerAIExplanationHostIntent) -> Void
    ) {
        self.model = model
        self.sendIntent = sendIntent
    }

    public var body: some View {
        switch model.phase {
        case .idle, .awaitingConsent:
            EmptyView()
        case .preparing:
            HStack(spacing: 10) {
                ProgressView().controlSize(.small)
                Text("Preparing the exact path-free metadata preview…")
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") { Task { await model.cancel() } }
            }
            .accessibilityIdentifier(ExplorerAIExplanationAccessibility.progress)
        case let .explaining(disclosure):
            Label(
                "Waiting for \(disclosure.providerName). The deterministic snapshot remains unchanged.",
                systemImage: "sparkles"
            )
            .foregroundStyle(.secondary)
            .accessibilityIdentifier(ExplorerAIExplanationAccessibility.progress)
        case .ready:
            if model.resultForCurrentDirectory != nil {
                ExplorerAIExplanationResultView(model: model)
            } else {
                HStack(spacing: 10) {
                    Label("AI explanation ready for the selected folder", systemImage: "sparkles")
                    Text("Open that folder to see its inert group overlays.")
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Dismiss") { Task { await model.dismiss() } }
                }
                .padding(10)
                .background(.purple.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
            }
        case let .failed(failure):
            failureView(failure)
        }
    }

    private func failureView(_ failure: ExplorerAIExplanationFailure) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 3) {
                Text(verbatim: failure.title).font(.headline)
                Text(verbatim: failure.detail).foregroundStyle(.secondary)
            }
            Spacer()
            if failure == .missingCredential {
                Button("Open Settings") { sendIntent(.openProviderSettings) }
            }
            if model.canPreview {
                Button("Preview Again") { Task { await model.previewSelection() } }
                    .help("Creates a fresh local preview; this is not a network retry")
            }
            Button("Dismiss") { Task { await model.dismiss() } }
        }
        .padding(10)
        .background(.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityIdentifier(ExplorerAIExplanationAccessibility.failure)
    }
}

public struct ExplorerAIExplanationResultView: View {
    @Bindable private var model: ExplorerAIExplanationModel

    public init(model: ExplorerAIExplanationModel) {
        self.model = model
    }

    public var body: some View {
        if let result = model.resultForCurrentDirectory {
            resultView(result)
        }
    }

    private func resultView(_ result: ExplorerAIExplanationResult) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 3) {
                        Label("AI explanation from \(result.providerName)", systemImage: "sparkles")
                            .font(.headline)
                        Text(verbatim: result.model)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button { Task { await model.dismiss() } } label: {
                        Label("Dismiss AI explanation", systemImage: "xmark")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.plain)
                }

                Text(verbatim: result.summary).textSelection(.enabled)

                if !result.labels.isEmpty {
                    HStack(spacing: 6) {
                        ForEach(result.labels, id: \.self) { label in
                            Text(verbatim: label)
                                .font(.caption.weight(.semibold))
                                .padding(.horizontal, 7)
                                .padding(.vertical, 3)
                                .background(.purple.opacity(0.12), in: Capsule())
                        }
                    }
                }

                let groups = result.groupPresentations()
                if !groups.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("AI groups").font(.subheadline.bold())
                        ForEach(groups, id: \.ordinal) { group in
                            HStack(alignment: .top, spacing: 8) {
                                Text(verbatim: String(group.ordinal))
                                    .font(.caption.bold())
                                    .frame(width: 22, height: 22)
                                    .background(.purple.opacity(0.18), in: Circle())
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(verbatim: group.title).font(.callout.bold())
                                    Text(verbatim: group.reason)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                    Text(
                                        "\(result.visibleCount(groupOrdinal: group.ordinal, observedNodeIDs: model.context.visibleObservedNodeIDs)) of \(result.itemCount(groupOrdinal: group.ordinal)) grouped items are visible here"
                                    )
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                                }
                            }
                        }
                    }
                    .accessibilityIdentifier(ExplorerAIExplanationAccessibility.legend)
                }

                explanationList("Questions", values: result.questions)
                explanationList("Uncertainties", values: result.uncertainties)
                explanationList("Ideas for future rule research", values: result.researchSuggestions)

                Label(
                    "AI grouping only — not a safety or cleanup judgment. No text or group is actionable.",
                    systemImage: "hand.raised.fill"
                )
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            }
        } label: {
            Label("Optional AI insight", systemImage: "sparkles")
        }
        .accessibilityIdentifier(ExplorerAIExplanationAccessibility.result)
    }

    @ViewBuilder
    private func explanationList(_ title: String, values: [String]) -> some View {
        if !values.isEmpty {
            DisclosureGroup(title) {
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(values, id: \.self) { value in
                        Text(verbatim: "• \(value)").textSelection(.enabled)
                    }
                }
            }
        }
    }
}

public struct ExplorerAIExplanationDecorationView: View {
    @Bindable private var model: ExplorerAIExplanationModel
    private let observedNodeID: UInt64

    public init(model: ExplorerAIExplanationModel, observedNodeID: UInt64) {
        self.model = model
        self.observedNodeID = observedNodeID
    }

    public var body: some View {
        if let group = model.decoration(forObservedNodeID: observedNodeID) {
            Label {
                Text(verbatim: "AI \(group.ordinal)")
            } icon: {
                Image(systemName: "sparkles")
            }
            .font(.caption2.bold())
            .padding(.horizontal, 5)
            .padding(.vertical, 2)
            .background(.purple.opacity(0.14), in: Capsule())
            .help("AI group \(group.ordinal): \(group.title). Not a safety or cleanup judgment.")
        }
    }
}

public struct ExplorerAIExplanationInspectorControl: View {
    @Bindable private var model: ExplorerAIExplanationModel

    public init(model: ExplorerAIExplanationModel) {
        self.model = model
    }

    public var body: some View {
        Button {
            Task { await model.previewSelection() }
        } label: {
            Label("Preview AI Explanation…", systemImage: "sparkles")
        }
        .disabled(!model.canPreview)
        .accessibilityIdentifier(ExplorerAIExplanationAccessibility.explain)
        .accessibilityHint(
            "Creates a local path-free metadata preview before anything can be sent"
        )
    }
}
