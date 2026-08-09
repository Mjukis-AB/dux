import SwiftUI

struct ExplorerAIExplanationConsentView: View {
    let disclosure: ExplorerAIExplanationDisclosure
    let isExplaining: Bool
    let explain: () -> Void
    let cancel: () -> Void

    @State private var showsMetadata = false

    var body: some View {
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
                    privacyFact(
                        "File content",
                        included: disclosure.preview.contentIncluded
                    )
                    privacyFact(
                        "Source names",
                        included: disclosure.preview.sourceNamesIncluded
                    )
                    privacyFact(
                        "Source paths",
                        included: disclosure.preview.sourcePathsIncluded
                    )
                    Text(
                        "DUX inspected \(disclosure.preview.inspectedNodeCount) snapshot nodes, included \(disclosure.preview.includedDirectChildCount) direct children, excluded \(disclosure.preview.excludedSensitiveDirectChildCount) sensitive children, and omitted \(disclosure.preview.omittedEligibleDirectChildCount) eligible children from the bounded payload."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    Text(
                        "This single-use preview expires \(expiryText). Its SHA-256 digest begins \(disclosure.preview.inputDigestSHA256.prefix(12))."
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
                .accessibilityIdentifier(ExplorerAccessibility.snapshotAIMetadata)
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
                Button(isExplaining ? "Cancel Explanation" : "Cancel") {
                    cancel()
                }
                .keyboardShortcut(.cancelAction)
                .accessibilityIdentifier(ExplorerAccessibility.snapshotAICancel)

                if isExplaining {
                    ProgressView("Waiting for Anthropic…")
                        .controlSize(.small)
                        .accessibilityIdentifier(ExplorerAccessibility.snapshotAIProgress)
                } else {
                    Button("Explain Selection") {
                        explain()
                    }
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier(ExplorerAccessibility.snapshotAISend)
                    .accessibilityHint(
                        "Sends this exact metadata once to Anthropic; no cleanup authority is granted"
                    )
                }
            }
        }
        .padding(22)
        .frame(minWidth: 620, idealWidth: 680, minHeight: 640)
        .accessibilityIdentifier(ExplorerAccessibility.snapshotAIDisclosure)
    }

    private var expiryText: String {
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

struct ExplorerAIExplanationResultView: View {
    let result: ExplorerAIExplanationResult
    let visibleNodeIDs: Set<UInt64>
    let dismiss: () -> Void

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 3) {
                        Label(
                            "AI explanation from \(result.providerName)",
                            systemImage: "sparkles"
                        )
                        .font(.headline)
                        Text(verbatim: result.model)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button {
                        dismiss()
                    } label: {
                        Label("Dismiss AI explanation", systemImage: "xmark")
                            .labelStyle(.iconOnly)
                    }
                    .buttonStyle(.plain)
                }

                Text(verbatim: result.summary)
                    .textSelection(.enabled)

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

                if !result.groups.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("AI groups")
                            .font(.subheadline.bold())
                        ForEach(result.groups) { group in
                            HStack(alignment: .top, spacing: 8) {
                                Text(verbatim: String(group.id))
                                    .font(.caption.bold())
                                    .frame(width: 22, height: 22)
                                    .background(.purple.opacity(0.18), in: Circle())
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(verbatim: group.title)
                                        .font(.callout.bold())
                                    Text(verbatim: group.reason)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                    Text(
                                        "\(visibleCount(group)) of \(group.snapshotNodeIDs.count) grouped items are visible here"
                                    )
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                                }
                            }
                        }
                    }
                    .accessibilityIdentifier(ExplorerAccessibility.snapshotAILegend)
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
        .accessibilityIdentifier(ExplorerAccessibility.snapshotAIResult)
    }

    private func visibleCount(_ group: ExplorerAIExplanationGroup) -> Int {
        group.snapshotNodeIDs.reduce(into: 0) { count, nodeID in
            if visibleNodeIDs.contains(nodeID) { count += 1 }
        }
    }

    @ViewBuilder
    private func explanationList(_ title: String, values: [String]) -> some View {
        if !values.isEmpty {
            DisclosureGroup(title) {
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(values, id: \.self) { value in
                        Text(verbatim: "• \(value)")
                            .textSelection(.enabled)
                    }
                }
            }
        }
    }
}
