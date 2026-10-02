import XCTest
@testable import DUX

final class ExplorerDiskMapLayoutTests: XCTestCase {
    func testCirclePackIsDeterministicBoundedNonOverlappingAndAreaProportional() {
        let items = [
            ExplorerDiskMapLayoutItem(id: .node(1), allocatedBytes: 100),
            ExplorerDiskMapLayoutItem(id: .node(2), allocatedBytes: 64),
            ExplorerDiskMapLayoutItem(id: .node(3), allocatedBytes: 25),
            ExplorerDiskMapLayoutItem(id: .other, allocatedBytes: 9),
        ]
        let bounds = CGRect(x: 0, y: 0, width: 700, height: 600)

        let first = ExplorerDiskMapLayout.circles(for: items, in: bounds)
        let second = ExplorerDiskMapLayout.circles(for: items, in: bounds)

        XCTAssertEqual(first, second)
        XCTAssertEqual(first.map(\.id), items.map(\.id))
        for circle in first {
            XCTAssertGreaterThan(circle.radius, 0)
            XCTAssertGreaterThanOrEqual(circle.center.x - circle.radius, bounds.minX)
            XCTAssertLessThanOrEqual(circle.center.x + circle.radius, bounds.maxX)
            XCTAssertGreaterThanOrEqual(circle.center.y - circle.radius, bounds.minY)
            XCTAssertLessThanOrEqual(circle.center.y + circle.radius, bounds.maxY)
        }
        for leftIndex in first.indices {
            for rightIndex in first.indices where rightIndex > leftIndex {
                let left = first[leftIndex]
                let right = first[rightIndex]
                XCTAssertGreaterThanOrEqual(
                    hypot(left.center.x - right.center.x, left.center.y - right.center.y) + 0.001,
                    left.radius + right.radius
                )
            }
        }
        XCTAssertEqual(
            pow(first[0].radius, 2) / pow(first[1].radius, 2),
            100.0 / 64.0,
            accuracy: 0.000_001
        )
        XCTAssertEqual(
            pow(first[0].radius, 2) / pow(first[2].radius, 2),
            100.0 / 25.0,
            accuracy: 0.000_001
        )
    }

    func testZeroWeightsAndInvalidBoundsDoNotInventArea() {
        XCTAssertTrue(
            ExplorerDiskMapLayout.circles(
                for: [.init(id: .unmapped, allocatedBytes: 0)],
                in: CGRect(x: 0, y: 0, width: 100, height: 100)
            ).isEmpty
        )
        XCTAssertTrue(
            ExplorerDiskMapLayout.circles(
                for: [.init(id: .available, allocatedBytes: 1)],
                in: .zero
            ).isEmpty
        )
    }

    func testEqualAreasPreserveTheRequestedVisualOrder() throws {
        let circles = ExplorerDiskMapLayout.circles(
            for: [
                .init(id: .used, allocatedBytes: 50),
                .init(id: .available, allocatedBytes: 50),
            ],
            in: CGRect(x: 0, y: 0, width: 720, height: 520)
        )

        XCTAssertEqual(circles.map(\.id), [.used, .available])
        XCTAssertLessThan(try XCTUnwrap(circles.first).center.x, try XCTUnwrap(circles.last).center.x)
    }

    func testVolumeEnvelopeSeparatesMeasuredUnmappedAndAvailableExactly() throws {
        let capacity = makeCapacity(total: 1_000, ordinaryAvailable: 250)
        let envelope = try XCTUnwrap(
            ExplorerDiskMapVolumeEnvelope.make(
                capacity: capacity,
                measuredAllocatedBytes: 400
            )
        )

        XCTAssertEqual(envelope.usedBytes, 750)
        XCTAssertEqual(envelope.measuredBytes, 400)
        XCTAssertEqual(envelope.unmappedBytes, 350)
        XCTAssertEqual(envelope.availableBytes, 250)
        XCTAssertEqual(
            envelope.layoutItems.reduce(UInt64(0)) { $0 + $1.allocatedBytes },
            envelope.totalBytes
        )
        XCTAssertEqual(envelope.overviewLayoutItems.map(\.id), [.used, .available])
        XCTAssertEqual(
            envelope.usedLayoutItems.reduce(UInt64(0)) { $0 + $1.allocatedBytes },
            envelope.usedBytes
        )
    }

    func testVolumeEnvelopeClampsStaleScanAndRejectsImportantOnlyAvailability() throws {
        let ordinary = makeCapacity(total: 1_000, ordinaryAvailable: 250)
        let clamped = try XCTUnwrap(
            ExplorerDiskMapVolumeEnvelope.make(
                capacity: ordinary,
                measuredAllocatedBytes: 900
            )
        )
        XCTAssertEqual(clamped.measuredBytes, 750)
        XCTAssertEqual(clamped.unmappedBytes, 0)

        let importantOnly = makeCapacity(total: 1_000, ordinaryAvailable: nil)
        XCTAssertNil(
            ExplorerDiskMapVolumeEnvelope.make(
                capacity: importantOnly,
                measuredAllocatedBytes: 400
            )
        )
    }

    func testMaximumCellPackMeetsInteractionBudget() {
        let items = (0 ..< 64).map {
            ExplorerDiskMapLayoutItem(
                id: .node(UInt64($0 + 1)),
                allocatedBytes: UInt64(64 - $0)
            )
        }
        let bounds = CGRect(x: 0, y: 0, width: 1_000, height: 700)
        let repetitions = 20
        let started = Date.timeIntervalSinceReferenceDate
        for _ in 0 ..< repetitions {
            XCTAssertEqual(
                ExplorerDiskMapLayout.circles(for: items, in: bounds).count,
                items.count
            )
        }
        let average = (Date.timeIntervalSinceReferenceDate - started) / Double(repetitions)
        XCTAssertLessThan(average, 1.0 / 60.0)
    }

    func testCompactPackUsesTheStageWithoutCollapsingIntoOneRing() {
        var items: [ExplorerDiskMapLayoutItem] = []
        for index in 0 ..< 24 {
            let bytes = UInt64(576 - index * 19)
            items.append(
                ExplorerDiskMapLayoutItem(
                    id: .node(UInt64(index + 1)),
                    allocatedBytes: bytes
                )
            )
        }
        let bounds = CGRect(x: 0, y: 0, width: 720, height: 720)
        let circles = ExplorerDiskMapLayout.circles(for: items, in: bounds)
        let center = CGPoint(x: bounds.midX, y: bounds.midY)
        let enclosingRadius = circles.map {
            hypot($0.center.x - center.x, $0.center.y - center.y) + $0.radius
        }.max() ?? 1
        let areaDensity = circles.reduce(CGFloat(0)) { $0 + pow($1.radius, 2) }
            / pow(enclosingRadius, 2)

        XCTAssertTrue(circles.contains { $0.center.x < center.x })
        XCTAssertTrue(circles.contains { $0.center.x > center.x })
        XCTAssertTrue(circles.contains { $0.center.y < center.y })
        XCTAssertTrue(circles.contains { $0.center.y > center.y })
        XCTAssertGreaterThan(areaDensity, 0.3)
    }

    private func makeCapacity(
        total: UInt64,
        ordinaryAvailable: UInt64?
    ) -> VolumeCapacitySnapshot {
        VolumeCapacitySnapshot(
            stableVolumeID: "volume",
            displayName: "Data",
            filesystem: "apfs",
            isInternal: true,
            isRemovable: false,
            totalBytes: total,
            filesystemAvailableBytes: ordinaryAvailable,
            importantAvailableBytes: 300,
            effectiveAvailableBytes: ordinaryAvailable ?? 300,
            availabilityBasis: ordinaryAvailable == nil ? .importantUsage : .filesystemAvailable,
            pressure: .healthy,
            criticalBoundaryBytes: 100,
            warningBoundaryBytes: 200,
            historyDisposition: nil,
            sampledAt: Date(timeIntervalSince1970: 1)
        )
    }
}
