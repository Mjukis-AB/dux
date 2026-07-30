import Foundation

/// Exact logical, allocated, and conservative charged usage for one
/// DUX-owned storage component.
struct DuxOwnedStorageUsageModel: Equatable, Sendable {
  let logicalBytes: UInt64
  let allocatedBytes: UInt64
  let chargedBytes: UInt64
}

/// Point-in-time physical and policy accounting for DUX snapshot storage.
///
/// Eligibility is descriptive. It is not cleanup authority or a promise that
/// the charged bytes can be reclaimed.
struct DuxSnapshotStorageFootprintModel: Equatable, Sendable {
  static let maximumObjectCount: UInt32 = 2_048
  static let maximumResidualTemporaryLeaseCount: UInt32 = 64
  static let maximumPinRowCount: UInt32 = 1_024

  let capBytes: UInt64
  let capExcessBytes: UInt64
  let controls: DuxOwnedStorageUsageModel
  let available: DuxOwnedStorageUsageModel
  let protected: DuxOwnedStorageUsageModel
  let retentionEligible: DuxOwnedStorageUsageModel
  let tombstonedResidual: DuxOwnedStorageUsageModel
  let orphan: DuxOwnedStorageUsageModel
  let temporaryActive: DuxOwnedStorageUsageModel
  let temporaryQuiescent: DuxOwnedStorageUsageModel
  let temporaryUnleased: DuxOwnedStorageUsageModel
  let total: DuxOwnedStorageUsageModel
  let availableCount: UInt32
  let protectedCount: UInt32
  let retentionEligibleCount: UInt32
  let tombstonedResidualCount: UInt32
  let orphanCount: UInt32
  let activeTemporaryCount: UInt32
  let quiescentTemporaryCount: UInt32
  let unleasedTemporaryCount: UInt32
  let residualTemporaryLeaseCount: UInt32
  let activePinRows: UInt32
  let expiredPinRows: UInt32
  let nonEvictableOverCap: Bool
  let accountingUnstable: Bool

  var maintenanceDebt: DuxOwnedStorageUsageModel? {
    Self.sum(
      [
        tombstonedResidual,
        orphan,
        temporaryActive,
        temporaryQuiescent,
        temporaryUnleased,
      ]
    )
  }

  var maintenanceDebtCount: UInt32? {
    var total = UInt32(0)
    for count in [
      tombstonedResidualCount,
      orphanCount,
      activeTemporaryCount,
      quiescentTemporaryCount,
      unleasedTemporaryCount,
    ] {
      let next = total.addingReportingOverflow(count)
      guard !next.overflow else {
        return nil
      }
      total = next.partialValue
    }
    return total
  }

  private static func sum(
    _ values: [DuxOwnedStorageUsageModel]
  ) -> DuxOwnedStorageUsageModel? {
    var total = DuxOwnedStorageUsageModel(
      logicalBytes: 0,
      allocatedBytes: 0,
      chargedBytes: 0
    )
    for usage in values {
      guard
        let logicalBytes = total.logicalBytes.addingExactly(usage.logicalBytes),
        let allocatedBytes = total.allocatedBytes.addingExactly(usage.allocatedBytes),
        let chargedBytes = total.chargedBytes.addingExactly(usage.chargedBytes)
      else {
        return nil
      }
      total = DuxOwnedStorageUsageModel(
        logicalBytes: logicalBytes,
        allocatedBytes: allocatedBytes,
        chargedBytes: chargedBytes
      )
    }
    return total
  }
}

/// Logical variable-length AI insight content embedded in the DUX database.
/// These bytes are non-additive and must never be added to physical totals.
struct DuxEmbeddedAiCacheFootprintModel: Equatable, Sendable {
  static let minimumContentBytesPerRecord: UInt64 = 36
  static let maximumContentBytesPerRecord: UInt64 = 16_777_888

  let recordCount: UInt32
  let logicalContentBytes: UInt64
  let expiredRecordCount: UInt32
  let expiredLogicalContentBytes: UInt64
}

/// A bounded, path-free observation of fixed marker-owned DUX storage.
///
/// It excludes the legacy caller-selected CLI cache and is neither free-space
/// telemetry nor an estimate of bytes cleanup would reclaim.
struct DuxOwnedStorageFootprintModel: Equatable, Sendable {
  let observedAt: Date
  let database: DuxOwnedStorageUsageModel
  let snapshots: DuxSnapshotStorageFootprintModel
  let embeddedAiCache: DuxEmbeddedAiCacheFootprintModel
  let physicalTotal: DuxOwnedStorageUsageModel
}

enum DuxOwnedStorageFootprintServiceError: Error, Equatable, Sendable {
  case closed
  case invalidClock
  case incompatibleSchema
  case retryable
  case unsafeStorage
  case budgetExceeded
  case corruptData
  case unavailable
  case internalState
  case invalidResponse
}

enum DuxOwnedStorageFootprintLoadState: Equatable, Sendable {
  case idle
  case loading
  case ready
  case failed(DuxOwnedStorageFootprintServiceError)

  var isLoading: Bool {
    self == .loading
  }
}

enum DuxOwnedStorageChartMath {
  struct Shares: Equatable, Sendable {
    let database: Double
    let snapshots: Double

    var isEmpty: Bool {
      database == 0 && snapshots == 0
    }
  }

  /// Normalizes without adding the two UInt64 inputs, so zero and
  /// UInt64.max are both safe.
  static func shares(database: UInt64, snapshots: UInt64) -> Shares {
    let scale = max(database, snapshots)
    guard scale > 0 else {
      return Shares(database: 0, snapshots: 0)
    }
    let scaledDatabase = Double(database) / Double(scale)
    let scaledSnapshots = Double(snapshots) / Double(scale)
    let scaledTotal = scaledDatabase + scaledSnapshots
    guard scaledTotal.isFinite, scaledTotal > 0 else {
      return Shares(database: 0, snapshots: 0)
    }
    return Shares(
      database: scaledDatabase / scaledTotal,
      snapshots: scaledSnapshots / scaledTotal
    )
  }
}

enum DuxOwnedStorageByteFormatter {
  private static let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"]

  /// A full-range UInt64 formatter. It never narrows or clamps through
  /// Int64, and always includes the exact byte count.
  static func string(from bytes: UInt64, locale: Locale = .current) -> String {
    let exact = bytes.formatted(
      .number.grouping(.automatic).locale(locale)
    )
    guard bytes >= 1_024 else {
      return "\(exact) B"
    }

    var unitIndex = 0
    var factor = UInt64(1)
    while unitIndex + 1 < units.count, bytes / factor >= 1_024 {
      factor *= 1_024
      unitIndex += 1
    }

    var whole = bytes / factor
    let remainder = bytes % factor
    var tenth = (remainder * 10 + factor / 2) / factor
    if tenth == 10 {
      whole += 1
      tenth = 0
    }
    return "\(whole).\(tenth) \(units[unitIndex]) · \(exact) B"
  }
}

extension UInt64 {
  fileprivate func addingExactly(_ other: UInt64) -> UInt64? {
    let result = addingReportingOverflow(other)
    return result.overflow ? nil : result.partialValue
  }
}
