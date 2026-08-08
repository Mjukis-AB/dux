import XCTest
@testable import DUX

final class SparkleUpdateControllerTests: XCTestCase {
    func testPlaceholderIdentityNeverEnablesUpdater() {
        XCTAssertEqual(
            SparkleUpdateConfiguration.load(
                info: completeInfo,
                bundleIdentifier: "se.mjukis.dux.spike"
            ),
            .failure(.placeholderBundleIdentity)
        )
    }

    func testUpdaterRequiresHTTPSFeed() {
        for feed in ["http://updates.example.com/appcast.xml", "not a URL"] {
            XCTAssertEqual(
                SparkleUpdateConfiguration.load(
                    info: [
                        "SUFeedURL": feed,
                        "SUPublicEDKey": validPublicKey,
                    ],
                    bundleIdentifier: "se.mjukis.dux"
                ),
                .failure(.missingSecureFeed)
            )
        }
    }

    func testUpdaterRequiresPublicVerificationKey() {
        XCTAssertEqual(
            SparkleUpdateConfiguration.load(
                info: ["SUFeedURL": "https://updates.example.com/appcast.xml"],
                bundleIdentifier: "se.mjukis.dux"
            ),
            .failure(.missingPublicKey)
        )
    }

    func testUpdaterRejectsMalformedPublicVerificationKey() {
        XCTAssertEqual(
            SparkleUpdateConfiguration.load(
                info: [
                    "SUFeedURL": "https://updates.example.com/appcast.xml",
                    "SUPublicEDKey": "not-an-ed25519-public-key",
                ],
                bundleIdentifier: "se.mjukis.dux"
            ),
            .failure(.invalidPublicKey)
        )
    }

    func testCompleteProductionConfigurationEnablesUpdater() throws {
        XCTAssertEqual(
            SparkleUpdateConfiguration.load(
                info: completeInfo,
                bundleIdentifier: "se.mjukis.dux"
            ),
            .success(
                SparkleUpdateConfiguration(
                    feedURL: try XCTUnwrap(
                        URL(string: "https://updates.example.com/appcast.xml")
                    ),
                    publicEdKey: validPublicKey
                )
            )
        )
    }

    private var completeInfo: [String: Any] {
        [
            "SUFeedURL": "https://updates.example.com/appcast.xml",
            "SUPublicEDKey": validPublicKey,
        ]
    }

    private var validPublicKey: String {
        "UmMI6TWBdBm2fKEmmk5xi2T+lu7K5KJl1abwIBRLSQo="
    }
}
