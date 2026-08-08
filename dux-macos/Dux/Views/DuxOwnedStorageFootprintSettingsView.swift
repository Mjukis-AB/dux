import SwiftUI

enum DuxOwnedStorageFootprintAccessibility {
  static let section = "dux-owned-storage-footprint-section"
  static let summary = "dux-owned-storage-footprint-summary"
  static let chart = "dux-owned-storage-footprint-chart"
  static let databaseSegment = "dux-owned-storage-footprint-database-segment"
  static let snapshotSegment = "dux-owned-storage-footprint-snapshot-segment"
  static let managedScanCacheSegment =
    "dux-owned-storage-footprint-managed-scan-cache-segment"
  static let databaseRows = "dux-owned-storage-footprint-database-rows"
  static let snapshotRows = "dux-owned-storage-footprint-snapshot-rows"
  static let managedScanCacheRows =
    "dux-owned-storage-footprint-managed-scan-cache-rows"
  static let managedScanCacheDetails =
    "dux-owned-storage-footprint-managed-scan-cache-details"
  static let totalRows = "dux-owned-storage-footprint-total-rows"
  static let snapshotPolicy = "dux-owned-storage-footprint-snapshot-policy"
  static let legacyExternalStages =
    "dux-owned-storage-footprint-legacy-external-stages"
  static let legacyExternalStagesStatus =
    "dux-owned-storage-footprint-legacy-external-stages-status"
  static let aiContent = "dux-owned-storage-footprint-ai-content"
  static let warning = "dux-owned-storage-footprint-warning"
  static let limitations = "dux-owned-storage-footprint-limitations"
  static let refresh = "dux-owned-storage-footprint-refresh"
  static let progress = "dux-owned-storage-footprint-progress"
  static let error = "dux-owned-storage-footprint-error"
  static let clearManagedScanCache =
    "dux-owned-storage-footprint-clear-managed-scan-cache"
  static let clearManagedScanCacheConfirmation =
    "dux-owned-storage-footprint-clear-managed-scan-cache-confirmation"
  static let clearManagedScanCacheProgress =
    "dux-owned-storage-footprint-clear-managed-scan-cache-progress"
  static let clearManagedScanCacheSuccess =
    "dux-owned-storage-footprint-clear-managed-scan-cache-success"
  static let clearManagedScanCacheError =
    "dux-owned-storage-footprint-clear-managed-scan-cache-error"
  static let clearSnapshotStorage =
    "dux-owned-storage-footprint-clear-snapshot-storage"
  static let clearSnapshotStorageConfirmation =
    "dux-owned-storage-footprint-clear-snapshot-storage-confirmation"
  static let clearSnapshotStorageExclusions =
    "dux-owned-storage-footprint-clear-snapshot-storage-exclusions"
  static let clearSnapshotStorageProgress =
    "dux-owned-storage-footprint-clear-snapshot-storage-progress"
  static let clearSnapshotStorageSuccess =
    "dux-owned-storage-footprint-clear-snapshot-storage-success"
  static let clearSnapshotStorageError =
    "dux-owned-storage-footprint-clear-snapshot-storage-error"

  static let allControlIdentifiers = [
    section,
    summary,
    chart,
    databaseSegment,
    snapshotSegment,
    managedScanCacheSegment,
    databaseRows,
    snapshotRows,
    managedScanCacheRows,
    managedScanCacheDetails,
    totalRows,
    snapshotPolicy,
    legacyExternalStages,
    legacyExternalStagesStatus,
    aiContent,
    warning,
    limitations,
    refresh,
    progress,
    error,
    clearManagedScanCache,
    clearManagedScanCacheConfirmation,
    clearManagedScanCacheProgress,
    clearManagedScanCacheSuccess,
    clearManagedScanCacheError,
    clearSnapshotStorage,
    clearSnapshotStorageConfirmation,
    clearSnapshotStorageExclusions,
    clearSnapshotStorageProgress,
    clearSnapshotStorageSuccess,
    clearSnapshotStorageError,
  ]
}

struct DuxOwnedStorageFootprintSettingsView: View {
  private enum ClearDialogClaim {
    case managedScanCache
    case snapshotStorage
  }

  @Bindable var settings: DuxOwnedStorageFootprintSettingsModel
  @State private var clearDialogClaim: ClearDialogClaim?

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      Label("DUX private storage", systemImage: "internaldrive.fill")
        .font(.headline)

      Text(
        "A point-in-time measurement of DUX’s private database, snapshot, "
          + "and marker-owned scan-cache stores. It does not inspect user "
          + "files and is not a "
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

      snapshotStorageAction
      managedScanCacheAction

      Text(
        "This bounded observation excludes directory metadata, the legacy "
          + "caller-selected CLI cache, and the contents and bytes of "
          + "unattributable interrupted setup stages. Charged bytes are "
          + "conservative physical accounting—not "
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
    .confirmationDialog(
      "Clear DUX scan cache?",
      isPresented: Binding(
        get: { settings.managedScanCacheClearConfirmation != nil },
        set: { presented in
          guard
            !presented,
            clearDialogClaim != .managedScanCache,
            let confirmation =
              settings.managedScanCacheClearConfirmation
          else {
            return
          }
          Task {
            await settings.dismissManagedScanCacheClear(confirmation)
          }
        }
      ),
      titleVisibility: .visible
    ) {
      if let confirmation = settings.managedScanCacheClearConfirmation {
        Button("Clear scan cache", role: .destructive) {
          // Claim synchronously before the system implicitly dismisses its
          // dialog. The model then consumes the exact engine-owned lease.
          clearDialogClaim = .managedScanCache
          Task {
            await settings.confirmManagedScanCacheClear(confirmation)
            clearDialogClaim = nil
          }
        }
        Button("Cancel", role: .cancel) {
          Task {
            await settings.cancelManagedScanCacheClear(confirmation)
          }
        }
      }
    } message: {
      if let confirmation = settings.managedScanCacheClearConfirmation {
        Text(Self.confirmationMessage(for: confirmation.preview))
          .accessibilityIdentifier(
            DuxOwnedStorageFootprintAccessibility
              .clearManagedScanCacheConfirmation
          )
      }
    }
    .confirmationDialog(
      "Clear older DUX snapshots?",
      isPresented: Binding(
        get: { settings.snapshotStorageClearConfirmation != nil },
        set: { presented in
          guard
            !presented,
            clearDialogClaim != .snapshotStorage,
            let confirmation = settings.snapshotStorageClearConfirmation
          else {
            return
          }
          Task {
            await settings.dismissSnapshotStorageClear(confirmation)
          }
        }
      ),
      titleVisibility: .visible
    ) {
      if let confirmation = settings.snapshotStorageClearConfirmation {
        Button("Clear older snapshots", role: .destructive) {
          clearDialogClaim = .snapshotStorage
          Task {
            await settings.confirmSnapshotStorageClear(confirmation)
            clearDialogClaim = nil
          }
        }
        Button("Cancel", role: .cancel) {
          Task {
            await settings.cancelSnapshotStorageClear(confirmation)
          }
        }
      }
    } message: {
      if let confirmation = settings.snapshotStorageClearConfirmation {
        Text(Self.snapshotConfirmationMessage(for: confirmation.preview))
          .accessibilityIdentifier(
            DuxOwnedStorageFootprintAccessibility
              .clearSnapshotStorageConfirmation
          )
      }
    }
    .onDisappear {
      Task {
        await settings.dismissManagedScanCacheClear()
        await settings.dismissSnapshotStorageClear()
      }
    }
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
      snapshotBytes: observation.snapshots.total.chargedBytes,
      managedScanCacheBytes:
        observation.managedScanCache.total.chargedBytes
    )

    HStack(spacing: 18) {
      Label("Database & history", systemImage: "cylinder.fill")
      Label("Snapshots (striped)", systemImage: "doc.on.doc.fill")
      Label("Scan cache (dotted)", systemImage: "internaldrive")
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
      title: "DUX scan cache",
      usage: observation.managedScanCache.total,
      identifier:
        DuxOwnedStorageFootprintAccessibility.managedScanCacheRows
    )
    storageRows(
      title: "Combined physical storage",
      usage: observation.physicalTotal,
      identifier: DuxOwnedStorageFootprintAccessibility.totalRows
    )

    snapshotPolicy(observation.snapshots)
    legacyExternalStageDiagnostic(
      observation.legacyExternalSnapshotStages
    )
    managedScanCacheDetails(observation.managedScanCache)
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
        LabeledContent("Protected snapshots") {
          Text(verbatim: String(snapshots.protectedCount))
            .monospacedDigit()
        }
        LabeledContent("Active snapshot reviews") {
          Text(verbatim: String(snapshots.activePinRows))
            .monospacedDigit()
        }
        byteRow(
          "Retention-eligible",
          snapshots.retentionEligible.chargedBytes
        )
        LabeledContent("Retention-eligible snapshots") {
          Text(verbatim: String(snapshots.retentionEligibleCount))
            .monospacedDigit()
        }
        if let maintenanceDebt = snapshots.maintenanceDebt {
          byteRow("Maintenance debt", maintenanceDebt.chargedBytes)
        }
        if let maintenanceDebtCount = snapshots.maintenanceDebtCount {
          LabeledContent("Excluded maintenance objects") {
            Text(verbatim: String(maintenanceDebtCount))
              .monospacedDigit()
          }
          .accessibilityIdentifier(
            DuxOwnedStorageFootprintAccessibility
              .clearSnapshotStorageExclusions
          )
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

  private func managedScanCacheDetails(
    _ cache: DuxManagedScanCacheFootprintModel
  ) -> some View {
    GroupBox("DUX scan-cache accounting") {
      VStack(alignment: .leading, spacing: 5) {
        LabeledContent("Published entries") {
          Text(verbatim: String(cache.entryCount))
            .monospacedDigit()
        }
        byteRow("Published entry charged usage", cache.entries.chargedBytes)
        LabeledContent("Quiescent temporary remnants") {
          Text(verbatim: String(cache.temporaryCount))
            .monospacedDigit()
        }
        byteRow(
          "Temporary remnant charged usage",
          cache.temporary.chargedBytes
        )
        byteRow("Ownership controls", cache.controls.chargedBytes)
        if let clearableCount = cache.clearableCount,
          let clearable = cache.clearable
        {
          LabeledContent("Clearable objects") {
            Text(verbatim: String(clearableCount))
              .monospacedDigit()
          }
          byteRow("Clearable charged usage", clearable.chargedBytes)
        }
        Text(
          "Only DUX’s fixed marker-owned private scan cache is counted here. "
            + "The outer legacy CLI cache and embedded AI content are excluded."
        )
        .font(.caption)
        .foregroundStyle(.secondary)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.managedScanCacheDetails
    )
  }

  private func legacyExternalStageDiagnostic(
    _ census: DuxLegacyExternalSnapshotStageCensusModel
  ) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      Label(
        Self.legacyExternalStageStatus(census),
        systemImage: "folder.badge.questionmark"
      )
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility
          .legacyExternalStagesStatus
      )

      Text(
        "These are only direct sibling names shaped like an older DUX setup "
          + "stage. Ownership and size are unknown. They are excluded from "
          + "every storage total and are not cleanup eligible. DUX offers no "
          + "removal action for them."
      )
      .font(.caption)
      .foregroundStyle(.secondary)
    }
    .padding(10)
    .frame(maxWidth: .infinity, alignment: .leading)
    .overlay {
      RoundedRectangle(cornerRadius: 8)
        .stroke(
          .secondary.opacity(0.55),
          style: StrokeStyle(lineWidth: 1, dash: [5, 4])
        )
    }
    .accessibilityElement(children: .combine)
    .accessibilityLabel("Possible older setup remnants")
    .accessibilityValue(
      Self.legacyExternalStageAccessibilityValue(census)
    )
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.legacyExternalStages
    )
  }

  static func legacyExternalStageStatus(
    _ census: DuxLegacyExternalSnapshotStageCensusModel
  ) -> String {
    if census.inspectionComplete {
      if census.stageShapedEntryCount == 0 {
        return "No stage-shaped older setup entries observed."
      }
      return "\(census.stageShapedEntryCount) possible older setup remnants observed."
    }
    return "At least \(census.stageShapedEntryCount) possible older setup "
      + "remnants observed; the bounded parent check was incomplete after "
      + "\(census.inspectedParentEntryCount) entries."
  }

  static func legacyExternalStageAccessibilityValue(
    _ census: DuxLegacyExternalSnapshotStageCensusModel
  ) -> String {
    legacyExternalStageStatus(census)
      + " Ownership is unknown. Byte size is unknown. Excluded from totals. "
      + "Not cleanup eligible; no removal action is available."
  }

  @ViewBuilder
  private var snapshotStorageAction: some View {
    let snapshots = settings.observation?.snapshots
    let observedClearableCount: UInt32 = {
      guard let snapshots else {
        return 0
      }
      let sum = snapshots.retentionEligibleCount.addingReportingOverflow(
        snapshots.tombstonedResidualCount
      )
      return sum.overflow ? 0 : sum.partialValue
    }()
    Button("Clear older snapshots…") {
      Task {
        await settings.prepareSnapshotStorageClear()
      }
    }
    .disabled(
      observedClearableCount == 0
        || snapshots?.accountingUnstable == true
        || settings.state.isLoading
        || settings.snapshotStorageClearState.isBusy
        || settings.snapshotStorageClearConfirmation != nil
        || settings.managedScanCacheClearState.isBusy
        || settings.managedScanCacheClearConfirmation != nil
    )
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.clearSnapshotStorage
    )
    .accessibilityHint(
      "Prepares an exact confirmation for clearing only older eligible DUX "
        + "snapshots and retired residuals; protected snapshots and user files remain"
    )

    switch settings.snapshotStorageClearState {
    case .preparing:
      ProgressView("Preparing exact snapshot confirmation")
        .accessibilityIdentifier(
          DuxOwnedStorageFootprintAccessibility
            .clearSnapshotStorageProgress
        )
    case .clearing:
      ProgressView("Clearing older DUX snapshots")
        .accessibilityIdentifier(
          DuxOwnedStorageFootprintAccessibility
            .clearSnapshotStorageProgress
        )
    case .completed(let result):
      Label(
        "Cleared \(result.clearedCount) snapshot objects "
          + "(\(DuxOwnedStorageByteFormatter.string(from: result.clearedUsage.chargedBytes)) charged).",
        systemImage: "checkmark.circle"
      )
      .foregroundStyle(.green)
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility
          .clearSnapshotStorageSuccess
      )
    case .failed(let error):
      Label(
        Self.snapshotClearMessage(for: error),
        systemImage: "exclamationmark.triangle"
      )
      .foregroundStyle(.red)
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility
          .clearSnapshotStorageError
      )
    case .outcomeUnknown:
      Label(
        "DUX could not prove whether snapshot clearing completed. DUX measured "
          + "private storage once, did not retry deletion, and preserved no "
          + "stale pre-clear measurement.",
        systemImage: "questionmark.diamond"
      )
      .foregroundStyle(.orange)
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility
          .clearSnapshotStorageError
      )
    case .idle, .awaitingConfirmation:
      EmptyView()
    }
  }

  @ViewBuilder
  private var managedScanCacheAction: some View {
    let clearableCount =
      settings.observation?.managedScanCache.clearableCount ?? 0
    Button("Clear DUX scan cache…") {
      Task {
        await settings.prepareManagedScanCacheClear()
      }
    }
    .disabled(
      clearableCount == 0
        || settings.state.isLoading
        || settings.managedScanCacheClearState.isBusy
        || settings.managedScanCacheClearConfirmation != nil
        || settings.snapshotStorageClearState.isBusy
        || settings.snapshotStorageClearConfirmation != nil
    )
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.clearManagedScanCache
    )
    .accessibilityHint(
      "Prepares an exact confirmation for clearing only DUX’s "
        + "marker-owned private scan cache; it does not remove user files"
    )

    switch settings.managedScanCacheClearState {
    case .preparing:
      ProgressView("Preparing exact scan-cache confirmation")
        .accessibilityIdentifier(
          DuxOwnedStorageFootprintAccessibility
            .clearManagedScanCacheProgress
        )
    case .clearing:
      ProgressView("Clearing DUX scan cache")
        .accessibilityIdentifier(
          DuxOwnedStorageFootprintAccessibility
            .clearManagedScanCacheProgress
        )
    case .completed(let result):
      Label(
        "Cleared \(result.clearedCount) scan-cache objects "
          + "(\(DuxOwnedStorageByteFormatter.string(from: result.clearedUsage.chargedBytes)) charged).",
        systemImage: "checkmark.circle"
      )
      .foregroundStyle(.green)
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility
          .clearManagedScanCacheSuccess
      )
    case .failed(let error):
      Label(Self.clearMessage(for: error), systemImage: "exclamationmark.triangle")
        .foregroundStyle(.red)
        .accessibilityIdentifier(
          DuxOwnedStorageFootprintAccessibility
            .clearManagedScanCacheError
        )
    case .outcomeUnknown:
      Label(
        "The clear result is uncertain. DUX measured storage again and "
          + "did not retry the deletion.",
        systemImage: "questionmark.diamond"
      )
      .foregroundStyle(.orange)
      .accessibilityIdentifier(
        DuxOwnedStorageFootprintAccessibility
          .clearManagedScanCacheError
      )
    case .idle, .awaitingConfirmation:
      EmptyView()
    }
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

  static func confirmationMessage(
    for preview: DuxManagedScanCacheClearPreviewModel
  ) -> String {
    "Clear exactly \(preview.clearableCount) objects "
      + "(\(preview.entryCount) published entries and "
      + "\(preview.temporaryCount) temporary remnants) using "
      + "\(DuxOwnedStorageByteFormatter.string(from: preview.clearable.chargedBytes)) "
      + "of charged storage? This affects only DUX’s marker-owned private "
      + "scan cache. It excludes the outer legacy cache, embedded AI, "
      + "database and history, snapshots, settings, and user files. The next "
      + "CLI scan may be slower. Charged bytes are not a promise of the "
      + "free-space change. This confirmation expires "
      + preview.expiresAt.formatted(date: .omitted, time: .standard)
      + "."
  }

  static func snapshotConfirmationMessage(
    for preview: DuxSnapshotStorageClearPreviewModel
  ) -> String {
    let eligible = counted(
      preview.eligibleSnapshotCount,
      singular: "retention-eligible snapshot",
      plural: "retention-eligible snapshots"
    )
    let residuals = counted(
      preview.tombstonedResidualCount,
      singular: "retired residual",
      plural: "retired residuals"
    )
    let protected = counted(
      preview.protectedSnapshotCount,
      singular: "protected snapshot",
      plural: "protected snapshots"
    )
    let reviews = counted(
      preview.activeReviewCount,
      singular: "active review",
      plural: "active reviews"
    )
    let maintenance = counted(
      preview.excludedMaintenanceObjectCount,
      singular: "excluded maintenance object",
      plural: "excluded maintenance objects"
    )
    return "Clear exactly \(eligible) and \(residuals) "
      + "(\(preview.clearableCount) objects total) using "
      + "\(DuxOwnedStorageByteFormatter.string(from: preview.clearable.chargedBytes)) "
      + "of charged storage? DUX will keep \(protected) using "
      + "\(DuxOwnedStorageByteFormatter.string(from: preview.protected.chargedBytes)). "
      + "Protected snapshots, when present, include the latest two snapshots for "
      + "each scanned root and snapshots held by \(reviews). It also excludes \(maintenance) using "
      + "\(DuxOwnedStorageByteFormatter.string(from: preview.excludedMaintenance.chargedBytes)), "
      + "including orphaned snapshots and active, quiescent, or unleased temporary "
      + "data. Snapshot-store controls, database and cleanup history, scan and "
      + "candidate history, scan cache, AI content, settings, exclusions, legacy "
      + "cache data, and user files are not removed. Cleared snapshots can no "
      + "longer be opened or compared in Explorer, although their history records "
      + "remain. Charged storage is not a promise of the free-space change. This "
      + "confirmation expires "
      + preview.expiresAt.formatted(date: .omitted, time: .standard)
      + "."
  }

  static func snapshotClearMessage(
    for error: DuxSnapshotStorageClearServiceError
  ) -> String {
    switch error {
    case .nothingToClear:
      String(
        localized:
          "No older snapshots are currently clearable. Protected snapshots, including current per-root snapshots and active reviews, remain available."
      )
    case .changedSincePreview:
      String(
        localized:
          "Snapshot storage changed after confirmation. DUX measured it again and did not retry deletion. Prepare a new confirmation."
      )
    case .previewExpired, .previewUnavailable:
      String(
        localized:
          "The snapshot-clear confirmation expired or is no longer available."
      )
    case .readOnlyStore:
      String(localized: "The DUX snapshot store is read-only.")
    case .incompatibleSchema:
      String(
        localized:
          "This DUX snapshot format is incompatible with the current app."
      )
    case .retryable:
      String(
        localized:
          "Snapshot storage is busy or still changing. Refresh after current work settles."
      )
    case .unsafeStorage:
      String(
        localized:
          "Snapshot storage failed its ownership or permission checks."
      )
    case .budgetExceeded:
      String(
        localized:
          "The bounded snapshot-clear operation exceeded its safety budget."
      )
    case .corruptData:
      String(
        localized:
          "DUX snapshot accounting is inconsistent or corrupt. Nothing was selected by the app."
      )
    case .closed:
      String(localized: "The storage engine session is closed.")
    case .wrongEngine, .invalidResponse, .unavailable, .internalState:
      String(localized: "DUX snapshot clearing is unavailable.")
    case .outcomeUnknown:
      String(
        localized:
          "DUX could not prove whether snapshot clearing completed. DUX will measure private storage once without retrying deletion."
      )
    }
  }

  private static func counted(
    _ count: UInt32,
    singular: String,
    plural: String
  ) -> String {
    "\(count) \(count == 1 ? singular : plural)"
  }

  static func clearMessage(
    for error: DuxManagedScanCacheClearServiceError
  ) -> String {
    switch error {
    case .nothingToClear:
      String(localized: "There is no DUX scan cache to clear.")
    case .changedSincePreview:
      String(
        localized:
          "The DUX scan cache changed after confirmation. Prepare a new confirmation."
      )
    case .previewExpired, .previewUnavailable:
      String(
        localized:
          "The scan-cache confirmation expired or is no longer available."
      )
    case .readOnlyStore:
      String(localized: "The DUX scan-cache store is read-only.")
    case .unsafeStorage:
      String(
        localized:
          "The DUX scan cache failed its ownership or permission checks."
      )
    case .budgetExceeded:
      String(localized: "The bounded scan-cache operation exceeded its safety budget.")
    case .corruptData:
      String(localized: "DUX scan-cache accounting is inconsistent or corrupt.")
    case .closed:
      String(localized: "The storage engine session is closed.")
    case .retryable:
      String(localized: "The DUX scan cache is busy. Try again after current work settles.")
    case .wrongEngine, .invalidResponse, .unavailable, .internalState:
      String(localized: "The DUX scan cache is unavailable.")
    case .outcomeUnknown:
      String(
        localized:
          "The clear result is uncertain. DUX will measure storage again without retrying."
      )
    }
  }
}

struct DuxOwnedStorageStackedBar: View {
  let databaseBytes: UInt64
  let snapshotBytes: UInt64
  let managedScanCacheBytes: UInt64

  private var shares: DuxOwnedStorageChartMath.Shares {
    DuxOwnedStorageChartMath.shares(
      database: databaseBytes,
      snapshots: snapshotBytes,
      managedScanCache: managedScanCacheBytes
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

          Rectangle()
            .fill(Color.purple.opacity(0.72))
            .frame(
              width: geometry.size.width * shares.managedScanCache
            )
            .overlay {
              DuxOwnedStorageDotPattern()
                .fill(Color.primary.opacity(0.48))
            }
            .overlay {
              Image(systemName: "internaldrive")
                .font(.caption2)
                .foregroundStyle(.white.opacity(0.85))
            }
            .accessibilityElement()
            .accessibilityLabel("DUX scan cache, dotted")
            .accessibilityValue(
              DuxOwnedStorageByteFormatter.string(
                from: managedScanCacheBytes
              )
            )
            .accessibilityIdentifier(
              DuxOwnedStorageFootprintAccessibility
                .managedScanCacheSegment
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
        snapshotBytes: snapshotBytes,
        managedScanCacheBytes: managedScanCacheBytes
      )
    )
    .accessibilityIdentifier(
      DuxOwnedStorageFootprintAccessibility.chart
    )
  }

  static func accessibilityValue(
    databaseBytes: UInt64,
    snapshotBytes: UInt64,
    managedScanCacheBytes: UInt64
  ) -> String {
    "Database and history "
      + DuxOwnedStorageByteFormatter.string(from: databaseBytes)
      + "; snapshots "
      + DuxOwnedStorageByteFormatter.string(from: snapshotBytes)
      + "; DUX scan cache "
      + DuxOwnedStorageByteFormatter.string(from: managedScanCacheBytes)
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

private struct DuxOwnedStorageDotPattern: Shape {
  func path(in rect: CGRect) -> Path {
    var path = Path()
    let spacing: CGFloat = 7
    let diameter: CGFloat = 2
    var y = rect.minY + spacing / 2
    while y < rect.maxY {
      var x = rect.minX + spacing / 2
      while x < rect.maxX {
        path.addEllipse(
          in: CGRect(
            x: x - diameter / 2,
            y: y - diameter / 2,
            width: diameter,
            height: diameter
          )
        )
        x += spacing
      }
      y += spacing
    }
    return path
  }
}
