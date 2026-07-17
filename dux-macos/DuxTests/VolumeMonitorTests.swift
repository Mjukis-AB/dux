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

    func testImportantOnlyCapacityDoesNotInventUsedStorage() throws {
        let snapshot = try VolumeMonitor.snapshot(
            from: VolumeResourceReading(
                localizedName: "Disk",
                name: nil,
                totalCapacity: 100,
                availableCapacity: nil,
                importantAvailableCapacity: 25
            ),
            at: Date(timeIntervalSince1970: 1)
        )

        XCTAssertEqual(snapshot.effectiveAvailableBytes, 25)
        XCTAssertEqual(snapshot.availabilityBasis, .importantUsage)
        XCTAssertNil(snapshot.usedBytes)
        XCTAssertNil(snapshot.usedPercentage)
        XCTAssertEqual(
            CapacityBar.accessibilityValue(
                for: snapshot,
                locale: Locale(identifier: "en")
            ),
            "Unavailable"
        )
    }

    func testPressurePresentationHasLocalizedVisibleAndAccessibilityText() {
        let locale = Locale(identifier: "en")
        let expected: [(DiskPressureLevel, String)] = [
            (.healthy, "Healthy"),
            (.warning, "Low space"),
            (.critical, "Critically low space"),
            (.unknown, "Pressure unknown"),
        ]

        for (pressure, title) in expected {
            XCTAssertEqual(
                DiskPressureBadge.localizedTitle(for: pressure, locale: locale),
                title
            )
            XCTAssertEqual(
                DiskPressureBadge.localizedAccessibilityLabel(
                    for: pressure,
                    locale: locale
                ),
                "Disk pressure: \(title)"
            )
        }
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
        guard let usedPercentage = snapshot.usedPercentage else {
            return XCTFail("Foundation supplied no ordinary filesystem capacity")
        }
        XCTAssertGreaterThanOrEqual(usedPercentage, 0)
        XCTAssertLessThanOrEqual(usedPercentage, 100)
    }

    func testFoundationCapacitySampleMedianMeetsNormalBudget() async throws {
        let monitor = VolumeMonitor()
        var durations: [TimeInterval] = []
        for _ in 0 ..< 5 {
            let started = ProcessInfo.processInfo.systemUptime
            _ = try await monitor.sampleStartupVolume()
            durations.append(ProcessInfo.processInfo.systemUptime - started)
        }
        durations.sort()
        XCTAssertLessThan(
            durations[durations.count / 2],
            0.5,
            "Median startup-volume capacity sampling exceeded the 500 ms normal budget"
        )
    }
}

private struct StubVolumeResourceProvider: VolumeResourceProviding {
    let reading: VolumeResourceReading

    func readVolumeResources(at _: URL) throws -> VolumeResourceReading {
        precondition(!Thread.isMainThread, "Stub provider was called on the main thread")
        return reading
    }
}
