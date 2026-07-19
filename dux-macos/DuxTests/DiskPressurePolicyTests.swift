import XCTest
@testable import DUX

final class DiskPressurePolicyTests: XCTestCase {
    func testEverySelectedByteValueRoundTripsThroughExactGiBText() {
        let gib = DiskPressurePolicyConfiguration.bytesPerGiB
        var values: [UInt64] = [
            0,
            1,
            2,
            gib - 1,
            gib,
            gib + 1,
            10 * gib,
            UInt64.max - 1,
            UInt64.max,
        ]
        var value: UInt64 = 0x9e37_79b9_7f4a_7c15
        for _ in 0 ..< 1_024 {
            value = value &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            values.append(value)
        }

        for bytes in values {
            let text = ExactPolicyDecimal.formatGiB(bytes)
            XCTAssertEqual(
                ExactPolicyDecimal.parseGiB(text, decimalSeparator: "."),
                bytes,
                "Failed exact round trip for \(bytes) via \(text)"
            )
        }
    }

    func testGiBParserAcceptsCommaLocaleAndRejectsInexactOrAmbiguousInput() {
        let gib = DiskPressurePolicyConfiguration.bytesPerGiB
        XCTAssertEqual(
            ExactPolicyDecimal.parseGiB("1,5", decimalSeparator: ","),
            gib + gib / 2
        )
        XCTAssertEqual(
            ExactPolicyDecimal.parseGiB(" 1.5 ", decimalSeparator: ","),
            gib + gib / 2
        )

        for invalid in [
            "", "-1", "+1", "1e2", "1,000", "1.2,3", "1.", ".5", "0.1",
            "17179869184", "0.000000000000000000000000000001",
            "0.0000000009313225746154785156251",
        ] {
            XCTAssertNil(
                ExactPolicyDecimal.parseGiB(invalid, decimalSeparator: "."),
                "Unexpectedly accepted \(invalid)"
            )
        }
    }

    func testEveryValidBasisPointValueRoundTrips() {
        for basisPoints in UInt16(1) ... UInt16(10_000) {
            let text = ExactPolicyDecimal.formatPercent(basisPoints)
            XCTAssertEqual(
                ExactPolicyDecimal.parsePercent(text, decimalSeparator: "."),
                basisPoints
            )
        }
        XCTAssertEqual(ExactPolicyDecimal.parsePercent("5,01", decimalSeparator: ","), 501)
        XCTAssertEqual(ExactPolicyDecimal.parsePercent("5.0100", decimalSeparator: "."), 501)
    }

    func testPercentParserRejectsRoundingGroupingSignsAndOverflow() {
        for invalid in ["", "-1", "+1", "1e2", "1,000", "1.2.3", "1.", ".5", "5.001", "655.36"] {
            XCTAssertNil(
                ExactPolicyDecimal.parsePercent(invalid, decimalSeparator: "."),
                "Unexpectedly accepted \(invalid)"
            )
        }
    }

    func testDraftPerformsRepresentationValidationButLeavesPolicyOrderingToRust() throws {
        let semanticInvalid = DiskPressurePolicyDraft(
            criticalGiB: "10",
            criticalPercent: "5",
            warningGiB: "8",
            warningPercent: "4",
            recoveryGiB: "2",
            recoveryPercent: "1"
        )
        let configuration = try semanticInvalid.configuration(decimalSeparator: ".")
        XCTAssertEqual(configuration.warningAvailableBytes, UInt64(8) << 30)

        var invalid = semanticInvalid
        invalid.recoveryPercent = "100.01"
        XCTAssertThrowsError(try invalid.configuration(decimalSeparator: ".")) { error in
            XCTAssertEqual(
                error as? DiskPressurePolicyDraftError,
                .outOfRange(.recoveryPercent)
            )
        }
    }

    @MainActor
    func testSettingsAccessibilityControlIdentifiersAreStableAndUnique() {
        let identifiers = DiskPressurePolicyAccessibility.allControlIdentifiers
        XCTAssertEqual(identifiers.count, 10)
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
        XCTAssertNotEqual(
            DuxSettingsView.message(for: .service(.warningBytesBelowCritical)),
            DuxSettingsView.message(
                for: DiskPressurePolicyFailure.service(.revisionExhausted)
            )
        )
        XCTAssertFalse(
            DuxSettingsView.message(for: DiskPressurePolicyFailure.unexpected).isEmpty
        )
    }
}
