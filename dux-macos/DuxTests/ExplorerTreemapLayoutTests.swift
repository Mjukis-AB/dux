import XCTest
@testable import DUX

final class ExplorerTreemapLayoutTests: XCTestCase {
    func testLayoutIsDeterministicBoundedAndConservesArea() {
        let items = [
            ExplorerTreemapLayoutItem(id: .node(1), logicalBytes: 50),
            ExplorerTreemapLayoutItem(id: .node(2), logicalBytes: 25),
            ExplorerTreemapLayoutItem(id: .node(3), logicalBytes: 15),
            ExplorerTreemapLayoutItem(id: .other, logicalBytes: 10),
        ]
        let bounds = CGRect(x: 0, y: 0, width: 800, height: 400)

        let first = ExplorerTreemapLayout.rectangles(for: items, in: bounds)
        let second = ExplorerTreemapLayout.rectangles(for: items, in: bounds)

        XCTAssertEqual(first, second)
        XCTAssertEqual(first.map(\.id), items.map(\.id))
        XCTAssertEqual(
            first.reduce(0) { $0 + $1.rect.width * $1.rect.height },
            bounds.width * bounds.height,
            accuracy: 0.01
        )
        for rectangle in first {
            XCTAssertGreaterThan(rectangle.rect.width, 0)
            XCTAssertGreaterThan(rectangle.rect.height, 0)
            XCTAssertTrue(bounds.contains(rectangle.rect))
        }
        for leftIndex in first.indices {
            for rightIndex in first.indices where rightIndex > leftIndex {
                let overlap = first[leftIndex].rect.intersection(first[rightIndex].rect)
                XCTAssertTrue(overlap.isNull || overlap.width == 0 || overlap.height == 0)
            }
        }
    }

    func testZeroWeightsAndInvalidBoundsProduceNoMisleadingArea() {
        let zero = ExplorerTreemapLayoutItem(id: .other, logicalBytes: 0)
        XCTAssertTrue(
            ExplorerTreemapLayout.rectangles(
                for: [zero],
                in: CGRect(x: 0, y: 0, width: 100, height: 100)
            ).isEmpty
        )
        XCTAssertTrue(
            ExplorerTreemapLayout.rectangles(
                for: [ExplorerTreemapLayoutItem(id: .node(1), logicalBytes: 1)],
                in: .zero
            ).isEmpty
        )
    }

    func testMaximumCellLayoutMeetsTheSixtyHertzInteractionBudget() {
        let items = (0..<64).map { index in
            ExplorerTreemapLayoutItem(
                id: .node(UInt64(index + 1)),
                logicalBytes: UInt64(64 - index)
            )
        }
        let bounds = CGRect(x: 0, y: 0, width: 1_200, height: 800)
        let repetitions = 120
        let started = Date.timeIntervalSinceReferenceDate
        for _ in 0..<repetitions {
            XCTAssertEqual(
                ExplorerTreemapLayout.rectangles(for: items, in: bounds).count,
                items.count
            )
        }
        let averageDuration =
            (Date.timeIntervalSinceReferenceDate - started) / Double(repetitions)

        XCTAssertLessThan(
            averageDuration,
            1.0 / 60.0,
            "The bounded 64-cell layout must leave one 60 Hz frame available"
        )
    }
}
