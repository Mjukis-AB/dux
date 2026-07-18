import XCTest
@testable import DUX

final class ExplorerStorageCategoryPresentationTests: XCTestCase {
    func testEveryCategoryHasStableDistinctTextSymbolPaletteAndLegendIdentifier() {
        let categories = ExplorerStorageCategory.allCases
        XCTAssertEqual(categories.count, 11)

        let presentations = categories.map(\.presentation)
        XCTAssertEqual(Set(presentations.map(\.title)).count, categories.count)
        XCTAssertEqual(Set(presentations.map(\.accessibilityPhrase)).count, categories.count)
        XCTAssertEqual(Set(presentations.map(\.symbol)).count, categories.count)
        XCTAssertEqual(Set(presentations.map(\.palette)).count, categories.count)
        XCTAssertTrue(presentations.allSatisfy { !$0.title.isEmpty })
        XCTAssertTrue(presentations.allSatisfy { !$0.accessibilityPhrase.isEmpty })
        XCTAssertTrue(presentations.allSatisfy { !$0.symbol.isEmpty })
        XCTAssertEqual(
            Set(presentations.map { $0.palette.strokeDash }).count,
            Set(presentations.map(\.palette)).count
        )
        XCTAssertTrue(presentations.allSatisfy { !$0.palette.strokeDash.isEmpty })

        let identifiers = categories.map(ExplorerAccessibility.snapshotCategoryLegend(category:))
        XCTAssertEqual(Set(identifiers).count, categories.count)
    }

    func testUnclassifiedAndUnknownStorageRemainDifferentClaims() {
        XCTAssertEqual(ExplorerStorageCategory.unclassified.presentation.title, "Unclassified")
        XCTAssertEqual(
            ExplorerStorageCategory.unclassified.presentation.accessibilityPhrase,
            "not classified"
        )
        XCTAssertEqual(
            ExplorerStorageCategory.unknownStorage.presentation.title,
            "Unknown storage"
        )
        XCTAssertNotEqual(
            ExplorerStorageCategory.unclassified.presentation.palette,
            ExplorerStorageCategory.unknownStorage.presentation.palette
        )
    }

    func testTreemapCellIdentifiersAreStableAndNodeSpecific() {
        XCTAssertEqual(
            ExplorerAccessibility.snapshotTreemapCell(nodeID: 42),
            "explorer-snapshot-treemap-cell-42"
        )
        XCTAssertNotEqual(
            ExplorerAccessibility.snapshotTreemapCell(nodeID: 1),
            ExplorerAccessibility.snapshotTreemapCell(nodeID: 2)
        )
    }
}
