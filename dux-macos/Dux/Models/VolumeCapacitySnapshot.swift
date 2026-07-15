import Foundation

enum VolumeCapacityBasis: Equatable, Sendable {
    case importantUsage
    case filesystemAvailable
}

struct VolumeCapacitySnapshot: Equatable, Sendable {
    let displayName: String?
    let totalBytes: UInt64
    let filesystemAvailableBytes: UInt64?
    let importantAvailableBytes: UInt64?
    let effectiveAvailableBytes: UInt64
    let availabilityBasis: VolumeCapacityBasis
    let sampledAt: Date

    var usedBytes: UInt64 {
        totalBytes - (filesystemAvailableBytes ?? effectiveAvailableBytes)
    }

    var usedFraction: Double {
        guard totalBytes > 0 else {
            return 0
        }
        return Double(usedBytes) / Double(totalBytes)
    }

    var usedPercentage: Int {
        Int((usedFraction * 100).rounded())
    }
}

enum VolumeCapacityState: Equatable {
    case idle
    case loading
    case loaded(VolumeCapacitySnapshot)
    case failed
}
