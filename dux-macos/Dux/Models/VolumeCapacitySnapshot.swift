import Foundation

enum VolumeCapacityBasis: Equatable, Sendable {
    case importantUsage
    case filesystemAvailable
}

enum DiskPressureLevel: Equatable, Sendable {
    case healthy
    case warning
    case critical
    case unknown
}

enum VolumeCapacityFailure: Equatable, Sendable {
    case unavailable
    case invalidObservation
    case engineUnavailable
}

enum VolumeCapacityHistoryDisposition: Equatable, Sendable {
    case stored
    case existingExact
    case suppressedByHourlyCadence
    case notStoredMissingOrdinaryAvailability
    case notStoredMissingStableIdentity
    case notStoredIncompleteMetadata
}

struct VolumeCapacitySnapshot: Equatable, Sendable {
    let stableVolumeID: String?
    let displayName: String?
    let filesystem: String?
    let isInternal: Bool?
    let isRemovable: Bool?
    let totalBytes: UInt64
    let filesystemAvailableBytes: UInt64?
    let importantAvailableBytes: UInt64?
    let effectiveAvailableBytes: UInt64
    let availabilityBasis: VolumeCapacityBasis
    let pressure: DiskPressureLevel
    let criticalBoundaryBytes: UInt64?
    let warningBoundaryBytes: UInt64?
    let historyDisposition: VolumeCapacityHistoryDisposition?
    let sampledAt: Date

    /// Ordinary filesystem usage is unknown when only the important-use value
    /// exists; important-use capacity can include purgeable bytes and must not
    /// be subtracted from total to invent a used value.
    var usedBytes: UInt64? {
        filesystemAvailableBytes.map { totalBytes - $0 }
    }

    var usedFraction: Double? {
        usedBytes.map { Double($0) / Double(totalBytes) }
    }

    var usedPercentage: Int? {
        usedFraction.map { Int(($0 * 100).rounded()) }
    }

    var availablePercentage: Int {
        Int((Double(effectiveAvailableBytes) / Double(totalBytes) * 100).rounded())
    }

    func applying(
        stableVolumeID: String?,
        pressure: DiskPressureLevel,
        criticalBoundaryBytes: UInt64,
        warningBoundaryBytes: UInt64,
        historyDisposition: VolumeCapacityHistoryDisposition,
        sampledAt: Date
    ) -> Self {
        Self(
            stableVolumeID: stableVolumeID,
            displayName: displayName,
            filesystem: filesystem,
            isInternal: isInternal,
            isRemovable: isRemovable,
            totalBytes: totalBytes,
            filesystemAvailableBytes: filesystemAvailableBytes,
            importantAvailableBytes: importantAvailableBytes,
            effectiveAvailableBytes: effectiveAvailableBytes,
            availabilityBasis: availabilityBasis,
            pressure: pressure,
            criticalBoundaryBytes: criticalBoundaryBytes,
            warningBoundaryBytes: warningBoundaryBytes,
            historyDisposition: historyDisposition,
            sampledAt: sampledAt
        )
    }
}

enum VolumeCapacityTrendPointSource: Equatable, Sendable {
    case raw
    case dailyRollup
}

struct VolumeCapacityTrendChange: Equatable, Sendable {
    let from: Date
    let to: Date
    let totalBytes: Int64
    let availableBytes: Int64
    let importantAvailableBytes: Int64?
}

struct VolumeCapacityTrendPoint: Equatable, Sendable {
    let sampledAt: Date
    let totalBytes: UInt64
    let availableBytes: UInt64
    let importantAvailableBytes: UInt64?
    let pressure: DiskPressureLevel
    let source: VolumeCapacityTrendPointSource
}

struct VolumeCapacityTrend: Equatable, Sendable {
    let stableVolumeID: String
    let sampledAt: Date
    let totalBytes: UInt64
    let availableBytes: UInt64
    let importantAvailableBytes: UInt64?
    let pressure: DiskPressureLevel
    let change24h: VolumeCapacityTrendChange?
    let change7d: VolumeCapacityTrendChange?
    let points: [VolumeCapacityTrendPoint]
}

enum VolumeCapacityState: Equatable {
    case idle
    case loading
    case refreshing(VolumeCapacitySnapshot)
    case loaded(VolumeCapacitySnapshot)
    case stale(VolumeCapacitySnapshot, VolumeCapacityFailure)
    case failed(VolumeCapacityFailure)

    var snapshot: VolumeCapacitySnapshot? {
        switch self {
        case let .refreshing(snapshot), let .loaded(snapshot), let .stale(snapshot, _):
            snapshot
        case .idle, .loading, .failed:
            nil
        }
    }
}
