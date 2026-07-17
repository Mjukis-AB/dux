@testable import DUX
import XCTest

@MainActor
final class AppActivationTests: XCTestCase {
    func testExplorerSceneIdentifierIsStable() {
        XCTAssertEqual(DuxSceneID.explorer, "explorer")
    }

    func testExplorerOpensTheSingletonSceneBeforeActivating() {
        var events: [String] = []

        AppActivation.openExplorer(
            open: { events.append("open:\($0)") },
            activate: { events.append("activate") }
        )

        XCTAssertEqual(events, ["open:explorer", "activate"])
    }

    func testRepeatedExplorerRequestsTargetTheSameScene() {
        var identifiers: [String] = []

        for _ in 0 ..< 2 {
            AppActivation.openExplorer(
                open: { identifiers.append($0) },
                activate: {}
            )
        }

        XCTAssertEqual(identifiers, [DuxSceneID.explorer, DuxSceneID.explorer])
    }

    func testStorageAccessGuidanceUsesTheInjectedSystemSettingsAction() {
        var openCount = 0

        AppActivation.openStorageAccessSettings {
            openCount += 1
        }

        XCTAssertEqual(openCount, 1)
    }
}
