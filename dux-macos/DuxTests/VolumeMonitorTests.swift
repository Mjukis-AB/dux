import Foundation
import XCTest
@testable import DUX

final class VolumeMonitorTests: XCTestCase {
    func testPrefersImportantUsageCapacityAndNormalizesName() throws {
        let sampledAt = Date(timeIntervalSince1970: 42)
        let snapshot = try VolumeMonitor.snapshot(
            from: VolumeResourceReading(
                localizedName: "  Startup  ",
                name: "Ignored",
                totalCapacity: 100,
                availableCapacity: 20,
                importantAvailableCapacity: 30
            ),
            at: sampledAt
        )

        XCTAssertEqual(snapshot.displayName, "Startup")
        XCTAssertEqual(snapshot.totalBytes, 100)
        XCTAssertEqual(snapshot.filesystemAvailableBytes, 20)
        XCTAssertEqual(snapshot.importantAvailableBytes, 30)
        XCTAssertEqual(snapshot.effectiveAvailableBytes, 30)
        XCTAssertEqual(snapshot.availabilityBasis, .importantUsage)
        XCTAssertEqual(snapshot.usedBytes, 80)
        XCTAssertEqual(snapshot.usedPercentage, 80)
        XCTAssertEqual(snapshot.sampledAt, sampledAt)
    }

    func testFallsBackToFilesystemCapacityWhenImportantUsageIsUnavailable() throws {
        let snapshot = try VolumeMonitor.snapshot(
            from: VolumeResourceReading(
                localizedName: nil,
                name: "Disk",
                totalCapacity: 100,
                availableCapacity: 25,
                importantAvailableCapacity: nil
            ),
            at: Date(timeIntervalSince1970: 1)
        )

        XCTAssertEqual(snapshot.effectiveAvailableBytes, 25)
        XCTAssertEqual(snapshot.availabilityBasis, .filesystemAvailable)
    }

    func testRejectsMissingAndInvalidCapacity() {
        XCTAssertThrowsError(
            try VolumeMonitor.snapshot(
                from: VolumeResourceReading(
                    localizedName: nil,
                    name: nil,
                    totalCapacity: nil,
                    availableCapacity: 1,
                    importantAvailableCapacity: nil
                ),
                at: Date()
            )
        ) { error in
            XCTAssertEqual(error as? VolumeMonitorError, .missingTotalCapacity)
        }

        XCTAssertThrowsError(
            try VolumeMonitor.snapshot(
                from: VolumeResourceReading(
                    localizedName: nil,
                    name: nil,
                    totalCapacity: -1,
                    availableCapacity: 1,
                    importantAvailableCapacity: nil
                ),
                at: Date()
            )
        ) { error in
            XCTAssertEqual(error as? VolumeMonitorError, .invalidCapacity)
        }

        XCTAssertThrowsError(
            try VolumeMonitor.snapshot(
                from: VolumeResourceReading(
                    localizedName: nil,
                    name: nil,
                    totalCapacity: 100,
                    availableCapacity: 101,
                    importantAvailableCapacity: nil
                ),
                at: Date()
            )
        ) { error in
            XCTAssertEqual(error as? VolumeMonitorError, .invalidCapacity)
        }

        XCTAssertThrowsError(
            try VolumeMonitor.snapshot(
                from: VolumeResourceReading(
                    localizedName: nil,
                    name: nil,
                    totalCapacity: 100,
                    availableCapacity: nil,
                    importantAvailableCapacity: nil
                ),
                at: Date()
            )
        ) { error in
            XCTAssertEqual(error as? VolumeMonitorError, .missingAvailableCapacity)
        }
    }

    func testSamplingRunsProviderOffMainAndUsesInjectedTime() async throws {
        let sampledAt = Date(timeIntervalSince1970: 99)
        let monitor = VolumeMonitor(
            provider: StubVolumeResourceProvider(
                reading: VolumeResourceReading(
                    localizedName: "Test",
                    name: nil,
                    totalCapacity: 200,
                    availableCapacity: 40,
                    importantAvailableCapacity: 50
                )
            ),
            now: { sampledAt }
        )

        let snapshot = try await monitor.sampleStartupVolume()

        XCTAssertEqual(snapshot.totalBytes, 200)
        XCTAssertEqual(snapshot.effectiveAvailableBytes, 50)
        XCTAssertEqual(snapshot.sampledAt, sampledAt)
    }

    func testFoundationSamplesTheStartupVolume() async throws {
        let snapshot = try await VolumeMonitor().sampleStartupVolume()

        XCTAssertGreaterThan(snapshot.totalBytes, 0)
        XCTAssertLessThanOrEqual(snapshot.effectiveAvailableBytes, snapshot.totalBytes)
        XCTAssertGreaterThanOrEqual(snapshot.usedPercentage, 0)
        XCTAssertLessThanOrEqual(snapshot.usedPercentage, 100)
    }
}

private struct StubVolumeResourceProvider: VolumeResourceProviding {
    let reading: VolumeResourceReading

    func readVolumeResources(at _: URL) throws -> VolumeResourceReading {
        precondition(!Thread.isMainThread, "Stub provider was called on the main thread")
        return reading
    }
}
