import Dispatch
import Foundation

enum VolumeMonitorError: Error, Equatable {
    case missingTotalCapacity
    case missingAvailableCapacity
    case invalidCapacity
}

struct VolumeResourceReading: Sendable {
    let localizedName: String?
    let name: String?
    let totalCapacity: Int?
    let availableCapacity: Int?
    let importantAvailableCapacity: Int64?
}

protocol VolumeResourceProviding: Sendable {
    func readVolumeResources(at url: URL) throws -> VolumeResourceReading
}

struct FoundationVolumeResourceProvider: VolumeResourceProviding {
    func readVolumeResources(at url: URL) throws -> VolumeResourceReading {
        let values = try url.resourceValues(forKeys: [
            .volumeLocalizedNameKey,
            .volumeNameKey,
            .volumeTotalCapacityKey,
            .volumeAvailableCapacityKey,
            .volumeAvailableCapacityForImportantUsageKey,
        ])

        return VolumeResourceReading(
            localizedName: values.volumeLocalizedName,
            name: values.volumeName,
            totalCapacity: values.volumeTotalCapacity,
            availableCapacity: values.volumeAvailableCapacity,
            importantAvailableCapacity: values.volumeAvailableCapacityForImportantUsage
        )
    }
}

protocol VolumeMonitoring: Sendable {
    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot
}

struct VolumeMonitor: VolumeMonitoring, Sendable {
    private let queue = DispatchQueue(
        label: "se.mjukis.dux.volume-monitor",
        qos: .utility
    )
    private let rootURL: URL
    private let provider: any VolumeResourceProviding
    private let now: @Sendable () -> Date

    init(
        rootURL: URL = URL(fileURLWithPath: "/", isDirectory: true),
        provider: any VolumeResourceProviding = FoundationVolumeResourceProvider(),
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.rootURL = rootURL
        self.provider = provider
        self.now = now
    }

    func sampleStartupVolume() async throws -> VolumeCapacitySnapshot {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                precondition(!Thread.isMainThread, "Volume capacity sampling reached the main thread")

                do {
                    let reading = try provider.readVolumeResources(at: rootURL)
                    continuation.resume(returning: try Self.snapshot(from: reading, at: now()))
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    static func snapshot(
        from reading: VolumeResourceReading,
        at sampledAt: Date
    ) throws -> VolumeCapacitySnapshot {
        guard let totalCapacity = reading.totalCapacity else {
            throw VolumeMonitorError.missingTotalCapacity
        }
        guard totalCapacity > 0, let totalBytes = UInt64(exactly: totalCapacity) else {
            throw VolumeMonitorError.invalidCapacity
        }

        let filesystemAvailableBytes = try nonnegativeBytes(reading.availableCapacity)
        let importantAvailableBytes = try nonnegativeBytes(reading.importantAvailableCapacity)

        let effectiveAvailableBytes: UInt64
        let availabilityBasis: VolumeCapacityBasis
        if let importantAvailableBytes {
            effectiveAvailableBytes = importantAvailableBytes
            availabilityBasis = .importantUsage
        } else if let filesystemAvailableBytes {
            effectiveAvailableBytes = filesystemAvailableBytes
            availabilityBasis = .filesystemAvailable
        } else {
            throw VolumeMonitorError.missingAvailableCapacity
        }

        guard
            effectiveAvailableBytes <= totalBytes,
            filesystemAvailableBytes.map({ $0 <= totalBytes }) ?? true,
            importantAvailableBytes.map({ $0 <= totalBytes }) ?? true
        else {
            throw VolumeMonitorError.invalidCapacity
        }

        return VolumeCapacitySnapshot(
            displayName: normalizedName(reading.localizedName) ?? normalizedName(reading.name),
            totalBytes: totalBytes,
            filesystemAvailableBytes: filesystemAvailableBytes,
            importantAvailableBytes: importantAvailableBytes,
            effectiveAvailableBytes: effectiveAvailableBytes,
            availabilityBasis: availabilityBasis,
            sampledAt: sampledAt
        )
    }

    private static func nonnegativeBytes<T: BinaryInteger>(_ value: T?) throws -> UInt64? {
        guard let value else {
            return nil
        }
        guard value >= 0, let bytes = UInt64(exactly: value) else {
            throw VolumeMonitorError.invalidCapacity
        }
        return bytes
    }

    private static func normalizedName(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed?.isEmpty == false ? trimmed : nil
    }
}
