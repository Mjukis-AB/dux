import XCTest
@testable import DUX

@MainActor
final class DuxAppDelegateTests: XCTestCase {
    func testTerminationGateStartsExactlyOneShutdown() {
        let gate = DuxTerminationGate()

        guard case .beginShutdown = gate.begin() else {
            return XCTFail("Expected the first request to start shutdown")
        }
        guard case .waitForExistingShutdown = gate.begin() else {
            return XCTFail("Expected a duplicate request to wait")
        }

        gate.approve()
        guard case .terminateNow = gate.begin() else {
            return XCTFail("Expected approval only after shutdown completed")
        }
    }
}
