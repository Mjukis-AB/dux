import Foundation
import XCTest

final class ICloudV58ReadOnlyQualificationTests: XCTestCase {
    func testLiveReadOnlyQualification() throws {
        let environment = ProcessInfo.processInfo.environment
        switch try ICloudIdentityQualificationHarness.optIn(
            environment: environment
        ) {
        case .skipped:
            throw XCTSkip(
                "Read-only iCloud qualification requires the exact real-device opt-in."
            )
        case .enabled:
            try ICloudIdentityQualificationHarness.run(environment: environment)
        }
    }

    func testDefaultEnvironmentSkipsBeforeAnyFixtureConfigurationIsParsed() throws {
        XCTAssertEqual(
            try ICloudIdentityQualificationHarness.optIn(environment: [:]),
            .skipped
        )
        XCTAssertEqual(
            try ICloudIdentityQualificationHarness.optIn(
                environment: ["DUX_ICLOUD_REAL_DEVICE_TESTS": "0"]
            ),
            .skipped
        )
        XCTAssertEqual(
            try ICloudIdentityQualificationHarness.optIn(
                environment: ["DUX_ICLOUD_REAL_DEVICE_TESTS": "true"]
            ),
            .skipped
        )
    }

    func testDestructiveFlagIsAlwaysRejected() {
        for value in ["", "0", "1", "false"] {
            XCTAssertThrowsError(
                try ICloudIdentityQualificationHarness.optIn(
                    environment: [
                        "DUX_ICLOUD_REAL_DEVICE_TESTS": "1",
                        "DUX_ICLOUD_DESTRUCTIVE_TESTS": value,
                    ]
                )
            ) { error in
                XCTAssertEqual(
                    error as? ICloudIdentityQualificationHarnessError,
                    .gateRejected
                )
            }
        }
    }

    func testExactReadOnlyOptInDoesNotParseOrAccessFixturePaths() throws {
        XCTAssertEqual(
            try ICloudIdentityQualificationHarness.optIn(
                environment: ["DUX_ICLOUD_REAL_DEVICE_TESTS": "1"]
            ),
            .enabled
        )
    }

    func testEnvironmentUsesExactArchitectureAndIdentityReadinessVocabulary() throws {
        var environment = try validScalarEnvironment()
        XCTAssertNoThrow(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        )

        environment["DUX_ICLOUD_EXPECTED_ARCH"] =
            environment.removeValue(forKey: "DUX_ICLOUD_EXPECTED_ARCHITECTURE")
        XCTAssertThrowsError(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        ) { error in
            XCTAssertEqual(
                error as? ICloudIdentityQualificationHarnessError,
                .gateRejected
            )
        }

        environment = try validScalarEnvironment()
        environment["DUX_ICLOUD_EXPECTED_IDENTITY_READINESS"] = "eligible"
        XCTAssertThrowsError(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        ) { error in
            XCTAssertEqual(
                error as? ICloudIdentityQualificationHarnessError,
                .gateRejected
            )
        }
    }

    func testPhaseSpecificNetworkAndAccountExpectationsFailClosed() throws {
        var environment = try validScalarEnvironment()
        environment["DUX_ICLOUD_PHASE"] = "network_offline"
        XCTAssertThrowsError(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        )
        environment["DUX_ICLOUD_NETWORK"] = "offline"
        XCTAssertNoThrow(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        )

        environment = try validScalarEnvironment()
        environment["DUX_ICLOUD_PHASE"] = "network_restored"
        XCTAssertThrowsError(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        )
        environment["DUX_ICLOUD_NETWORK"] = "restored"
        XCTAssertNoThrow(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        )

        environment = try validScalarEnvironment()
        environment["DUX_ICLOUD_PHASE"] = "account_changed"
        XCTAssertThrowsError(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        )
        environment["DUX_ICLOUD_EXPECTED_IDENTITY_READINESS"] = "blocked"
        XCTAssertNoThrow(
            try ICloudIdentityQualificationHarness.validateEnvironmentForTesting(
                environment
            )
        )
    }

    private func validScalarEnvironment() throws -> [String: String] {
        var environment = try ICloudIdentityQualificationHarness
            .currentPlatformEnvironmentForTesting()
        environment.merge([
            "DUX_ICLOUD_REAL_DEVICE_TESTS": "1",
            "DUX_ICLOUD_DISPOSABLE_ACCOUNT_CONFIRMED": "YES",
            "DUX_ICLOUD_DISPOSABLE_FIXTURE_CONFIRMED": "YES",
            "DUX_ICLOUD_EXTERNAL_CONTENT_REFERENCE_CONFIRMED": "YES",
            "DUX_ICLOUD_EXCLUSIVE_SERIALIZATION": "1",
            "DUX_ICLOUD_SOURCE_COMMIT": String(repeating: "a", count: 40),
            "DUX_ICLOUD_EVIDENCE_SCHEMA_SHA256": String(repeating: "b", count: 64),
            "DUX_ICLOUD_RUN_STARTED_AT_UNIX_NS": "1",
            "DUX_ICLOUD_ACCOUNT_LABEL": "acct-AAAAAAAAAAAA",
            "DUX_ICLOUD_FIXTURE_LABEL": "fixture-AAAAAAAAAAAA",
            "DUX_ICLOUD_PHASE": "baseline",
            "DUX_ICLOUD_NETWORK": "online",
            "DUX_ICLOUD_EXPECTED_SYNC_ELIGIBILITY": "eligible",
            "DUX_ICLOUD_EXPECTED_IDENTITY_READINESS": "ready",
            "DUX_ICLOUD_FIXTURE_ROOT": "/private/redacted-root",
            "DUX_ICLOUD_FIXTURE_PATH": "/private/redacted-root/item",
            "DUX_ICLOUD_PRIVATE_STATE_DIRECTORY": "/private/redacted-state",
            "DUX_ICLOUD_EVIDENCE_OUTPUT": "/private/redacted-evidence/report.json",
        ]) { _, new in new }
        return environment
    }
}
