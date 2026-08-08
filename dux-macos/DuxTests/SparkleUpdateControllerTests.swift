import XCTest
@testable import DUX

final class SparkleUpdateControllerTests: XCTestCase {
    func testPlaceholderIdentityNeverEnablesUpdater() {
        XCTAssertEqual(
            SparkleUpdateConfiguration.load(
                info: completeInfo,
                bundleIdentifier: "se.mjukis.dux.spike"
            ),
            .failure(.unexpectedBundleIdentity)
        )
    }

    func testAnyOtherNonDebugIdentityNeverEnablesUpdater() {
        XCTAssertEqual(
            SparkleUpdateConfiguration.load(
                info: completeInfo,
                bundleIdentifier: "se.mjukis.dux.preview"
            ),
            .failure(.unexpectedBundleIdentity)
        )
    }

    func testUpdaterRequiresHTTPSFeed() {
        for feed in ["http://updates.example.com/appcast.xml", "not a URL"] {
            XCTAssertEqual(
                SparkleUpdateConfiguration.load(
                    info: [
                        "SUFeedURL": feed,
                        "SUPublicEDKey": validPublicKey,
                        "SURequireSignedFeed": true,
                        "SUVerifyUpdateBeforeExtraction": true,
                        "SUSignedFeedFailureExpirationInterval": 0,
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

    func testUpdaterRejectsUnexpectedPublicVerificationKey() {
        for publicKey in [
            "not-an-ed25519-public-key",
            Data(repeating: 7, count: 32).base64EncodedString(),
        ] {
            XCTAssertEqual(
                SparkleUpdateConfiguration.load(
                    info: [
                        "SUFeedURL": "https://updates.example.com/appcast.xml",
                        "SUPublicEDKey": publicKey,
                    ],
                    bundleIdentifier: "se.mjukis.dux"
                ),
                .failure(.unexpectedPublicKey)
            )
        }
    }

    func testUpdaterRequiresStrictSignedUpdatePolicy() {
        let unsafePolicies: [[String: Any]] = [
            [:],
            [
                "SURequireSignedFeed": false,
                "SUVerifyUpdateBeforeExtraction": true,
                "SUSignedFeedFailureExpirationInterval": 0,
            ],
            [
                "SURequireSignedFeed": true,
                "SUVerifyUpdateBeforeExtraction": false,
                "SUSignedFeedFailureExpirationInterval": 0,
            ],
            [
                "SURequireSignedFeed": true,
                "SUVerifyUpdateBeforeExtraction": true,
                "SUSignedFeedFailureExpirationInterval": 1,
            ],
        ]
        for policy in unsafePolicies {
            var info: [String: Any] = [
                "SUFeedURL": "https://updates.example.com/appcast.xml",
                "SUPublicEDKey": validPublicKey,
            ]
            info.merge(policy) { _, new in new }
            XCTAssertEqual(
                SparkleUpdateConfiguration.load(
                    info: info,
                    bundleIdentifier: "se.mjukis.dux"
                ),
                .failure(.unsafeVerificationPolicy)
            )
        }
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
            "SURequireSignedFeed": true,
            "SUVerifyUpdateBeforeExtraction": true,
            "SUSignedFeedFailureExpirationInterval": 0,
        ]
    }

    private var validPublicKey: String {
        "UmMI6TWBdBm2fKEmmk5xi2T+lu7K5KJl1abwIBRLSQo="
    }
}
