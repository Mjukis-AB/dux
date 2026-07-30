import SwiftUI

enum DuxOwnedStorageFootprintAccessibility {
  static let section = "dux-owned-storage-footprint-section"
  static let summary = "dux-owned-storage-footprint-summary"
  static let chart = "dux-owned-storage-footprint-chart"
  static let databaseSegment = "dux-owned-storage-footprint-database-segment"
  static let snapshotSegment = "dux-owned-storage-footprint-snapshot-segment"
  static let databaseRows = "dux-owned-storage-footprint-database-rows"
  static let snapshotRows = "dux-owned-storage-footprint-snapshot-rows"
  static let totalRows = "dux-owned-storage-footprint-total-rows"
  static let snapshotPolicy = "dux-owned-storage-footprint-snapshot-policy"
  static let aiContent = "dux-owned-storage-footprint-ai-content"
  static let warning = "dux-owned-storage-footprint-warning"
  static let limitations = "dux-owned-storage-footprint-limitations"
  static let refresh = "dux-owned-storage-footprint-refresh"
  static let progress = "dux-owned-storage-footprint-progress"
  static let error = "dux-owned-storage-footprint-error"

  static let allControlIdentifiers = [
    section,
    summary,
    chart,
    databaseSegment,
    snapshotSegment,
    databaseRows,
    snapshotRows,
    totalRows,
    snapshotPolicy,
    aiContent,
    warning,
    limitations,
    refresh,
    progress,
    error,
  ]
}

struct DuxOwnedStorageFootprintSettingsView: View {
  @Bindable var settings: DuxOwnedStorageFootprintSettingsModel

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      Label("DUX private storage", systemImage: "internaldrive.fill")
        .font(.headline)

      Text(
        "A point-in-time measurement of DUX’s private database and snapshot "
          + "stores. It does not inspect user files and is not a "
          + "reclaimable-space or free-space estimate."
      )
      .foregroundStyle(.secondary)

      if let observation = settings.observation {
        observationView(observation)
      } else if settings.state.isLoading {
        ProgressView("Measuring DUX private storage")
          .accessibilityIdentifier(
            DuxOwnedStorageFootprintAccessibility.progress
          )
      } else {
        Text("No storage observation has been loaded.")
          .foregroundStyle(.secondary)
      }

      if case .failed(let error) = settings.state {
        Label(Self.message(for: error), systemImage: "exclamationmark.triangle")
          .foregroundStyle(.red)
          .accessibilityIdentifier(
            DuxOwnedStorageFootprintAccessibility.error
          )
      }

      HStack {
        Button(settings.observation == nil ? "Measure now" : "Refresh") {
          Task { await settings.refresh() }
        }
        .disabled(settings.state.isLoading)
        .accessibilityIdentifier(
          DuxOwnedStorageFootprintAccessibility.refresh
        )
        .accessibilityHint(
          "Reads a new point-in-time observation without removing anything"
        )

        if settings.state.isLoading, settings.observation != nil {
          ProgressView()
            .controlSize(.small)
            .accessibilityIdentifier(
              DuxOwnedStorageFootprintAccessibility.progress
            )
            .accessibilityLabel("Refreshing DUX private storage")
        }
      }

      Text(
        "This bounded observation excludes directory metadata, the legacy "
          + "caller-selected CLI cache, and unattributable interrupted setup "
          + "stages. Charged bytes are conservative physical accounting—not "
          + "free space, reclaimable space, or permission to clean."
      )
      .font(.caption)
      .foregroundStyle(.secondary)
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility.limitations
      )
    }
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.section
    )
  }

  @ViewBuilder
  private func observationView(
    _ observation: DuxOwnedStorageFootprintModel
  ) -> some View {
    LabeledContent("Conservative physical total") {
      Text(
        verbatim: DuxOwnedStorageByteFormatter.string(
          from: observation.physicalTotal.chargedBytes
        )
      )
      .monospacedDigit()
    }
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.summary
    )

    DuxOwnedStorageStackedBar(
      databaseBytes: observation.database.chargedBytes,
      snapshotBytes: observation.snapshots.total.chargedBytes
    )

    HStack(spacing: 18) {
      Label("Database & history", systemImage: "cylinder.fill")
      Label("Snapshots (striped)", systemImage: "doc.on.doc.fill")
    }
    .font(.caption)
    .foregroundStyle(.secondary)

    storageRows(
      title: "Database & history",
      usage: observation.database,
      identifier: DuxOwnedStorageFootprintAccessibility.databaseRows
    )
    storageRows(
      title: "Snapshots",
      usage: observation.snapshots.total,
      identifier: DuxOwnedStorageFootprintAccessibility.snapshotRows
    )
    storageRows(
      title: "Combined physical storage",
      usage: observation.physicalTotal,
      identifier: DuxOwnedStorageFootprintAccessibility.totalRows
    )

    snapshotPolicy(observation.snapshots)
    aiContent(observation.embeddedAiCache)

    if observation.snapshots.accountingUnstable {
      Label(
        "Snapshot accounting is changing because an active or unleased "
          + "temporary object was observed. Refresh after current work settles.",
        systemImage: "arrow.triangle.2.circlepath"
      )
      .foregroundStyle(.orange)
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility.warning
      )
    } else if observation.snapshots.nonEvictableOverCap {
      Label(
        "Non-evictable snapshot storage is above the configured limit. "
          + "The limit cannot override protection or safety evidence.",
        systemImage: "exclamationmark.shield"
      )
      .foregroundStyle(.orange)
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility.warning
      )
    }

    Text(
      "Observed "
        + observation.observedAt.formatted(
          date: .abbreviated,
          time: .standard
        )
    )
    .font(.caption)
    .foregroundStyle(.secondary)
  }

  private func storageRows(
    title: String,
    usage: DuxOwnedStorageUsageModel,
    identifier: String
  ) -> some View {
    GroupBox(title) {
      VStack(alignment: .leading, spacing: 5) {
        byteRow("Logical", usage.logicalBytes)
        byteRow("Allocated", usage.allocatedBytes)
        byteRow("Charged", usage.chargedBytes)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier(identifier)
  }

  private func byteRow(_ label: String, _ bytes: UInt64) -> some View {
    LabeledContent(label) {
      Text(verbatim: DuxOwnedStorageByteFormatter.string(from: bytes))
        .font(.system(.caption, design: .monospaced))
        .monospacedDigit()
        .textSelection(.enabled)
    }
  }

  private func snapshotPolicy(
    _ snapshots: DuxSnapshotStorageFootprintModel
  ) -> some View {
    GroupBox("Snapshot policy context") {
      VStack(alignment: .leading, spacing: 5) {
        byteRow("Configured limit", snapshots.capBytes)
        byteRow("Above limit", snapshots.capExcessBytes)
        byteRow("Protected", snapshots.protected.chargedBytes)
        byteRow(
          "Retention-eligible",
          snapshots.retentionEligible.chargedBytes
        )
        if let maintenanceDebt = snapshots.maintenanceDebt {
          byteRow("Maintenance debt", maintenanceDebt.chargedBytes)
        }
        LabeledContent("Observed snapshot objects") {
          Text(verbatim: String(snapshots.availableCount))
            .monospacedDigit()
        }
        Text(
          "Retention eligibility is descriptive only. It does not grant "
            + "cleanup authority or promise that those bytes are reclaimable."
        )
        .font(.caption)
        .foregroundStyle(.secondary)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.snapshotPolicy
    )
  }

  private func aiContent(
    _ ai: DuxEmbeddedAiCacheFootprintModel
  ) -> some View {
    GroupBox("Embedded AI insight content") {
      VStack(alignment: .leading, spacing: 5) {
        LabeledContent("Cached records") {
          Text(verbatim: String(ai.recordCount))
            .monospacedDigit()
        }
        byteRow("Logical content", ai.logicalContentBytes)
        LabeledContent("Expired records") {
          Text(verbatim: String(ai.expiredRecordCount))
            .monospacedDigit()
        }
        byteRow(
          "Expired logical content",
          ai.expiredLogicalContentBytes
        )
        Text(
          "These logical content bytes are already included inside the "
            + "database above. They are not added to the chart or physical "
            + "total, and exclude SQLite page, index, and fragmentation overhead."
        )
        .font(.caption)
        .foregroundStyle(.secondary)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.aiContent
    )
  }

  static func message(
    for error: DuxOwnedStorageFootprintServiceError
  ) -> String {
    switch error {
    case .closed:
      String(localized: "The storage engine session is closed.")
    case .invalidClock, .retryable:
      String(
        localized:
          "DUX private storage could not be measured safely. Try refreshing."
      )
    case .incompatibleSchema:
      String(
        localized:
          "This DUX storage format is incompatible with the current app."
      )
    case .unsafeStorage:
      String(
        localized:
          "DUX private storage failed its ownership or permission checks."
      )
    case .budgetExceeded:
      String(
        localized:
          "The bounded storage observation exceeded its safety budget."
      )
    case .corruptData:
      String(
        localized:
          "DUX private storage accounting is inconsistent or corrupt."
      )
    case .unavailable, .internalState, .invalidResponse:
      String(
        localized:
          "DUX private storage is unavailable. The last complete observation remains displayed."
      )
    }
  }
}

struct DuxOwnedStorageStackedBar: View {
  let databaseBytes: UInt64
  let snapshotBytes: UInt64

  private var shares: DuxOwnedStorageChartMath.Shares {
    DuxOwnedStorageChartMath.shares(
      database: databaseBytes,
      snapshots: snapshotBytes
    )
  }

  var body: some View {
    GeometryReader { geometry in
      if shares.isEmpty {
        Capsule()
          .fill(.quaternary)
      } else {
        HStack(spacing: 0) {
          Rectangle()
            .fill(Color.accentColor)
            .frame(
              width: geometry.size.width * shares.database
            )
            .overlay {
              Image(systemName: "cylinder.fill")
                .font(.caption2)
                .foregroundStyle(.white.opacity(0.8))
            }
            .accessibilityElement()
            .accessibilityLabel("Database and history")
            .accessibilityValue(
              DuxOwnedStorageByteFormatter.string(
                from: databaseBytes
              )
            )
            .accessibilityIdentifier(
              DuxOwnedStorageFootprintAccessibility.databaseSegment
            )

          Rectangle()
            .fill(Color.orange.opacity(0.72))
            .frame(
              width: geometry.size.width * shares.snapshots
            )
            .overlay {
              DuxOwnedStorageDiagonalHatch()
                .stroke(
                  Color.primary.opacity(0.42),
                  lineWidth: 1
                )
            }
            .accessibilityElement()
            .accessibilityLabel("Snapshots, striped")
            .accessibilityValue(
              DuxOwnedStorageByteFormatter.string(
                from: snapshotBytes
              )
            )
            .accessibilityIdentifier(
              DuxOwnedStorageFootprintAccessibility.snapshotSegment
            )
        }
        .clipShape(Capsule())
      }
    }
    .frame(height: 14)
    .accessibilityElement(children: .contain)
    .accessibilityLabel("DUX private physical storage distribution")
    .accessibilityValue(
      Self.accessibilityValue(
        databaseBytes: databaseBytes,
        snapshotBytes: snapshotBytes
      )
    )
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.chart
    )
  }

  static func accessibilityValue(
    databaseBytes: UInt64,
    snapshotBytes: UInt64
  ) -> String {
    "Database and history "
      + DuxOwnedStorageByteFormatter.string(from: databaseBytes)
      + "; snapshots "
      + DuxOwnedStorageByteFormatter.string(from: snapshotBytes)
  }
}

private struct DuxOwnedStorageDiagonalHatch: Shape {
  func path(in rect: CGRect) -> Path {
    var path = Path()
    let spacing: CGFloat = 8
    var offset = -rect.height
    while offset < rect.width {
      path.move(to: CGPoint(x: offset, y: rect.maxY))
      path.addLine(
        to: CGPoint(x: offset + rect.height, y: rect.minY)
      )
      offset += spacing
    }
    return path
  }
}
