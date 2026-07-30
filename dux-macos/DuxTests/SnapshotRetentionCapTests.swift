import XCTest
@testable import DUX

final class SnapshotRetentionCapDraftTests: XCTestCase {
    func testDraftRoundTripsZeroDefaultAndMaximumWithoutRounding() throws {
        for bytes in [
            UInt64(0),
            2 * DiskPressurePolicyConfiguration.bytesPerGiB,
            UInt64.max,
        ] {
            let draft = SnapshotRetentionCapDraft(capBytes: bytes)
            XCTAssertEqual(
                try draft.capBytes(decimalSeparator: "."),
                bytes
            )
        }
    }

    func testDraftRejectsNegativeGroupedAndInexactValues() {
        for text in ["", "-1", "+1", "1e2", "1,000", "0.1"] {
            XCTAssertThrowsError(
                try SnapshotRetentionCapDraft(gib: text)
                    .capBytes(decimalSeparator: ".")
            ) { error in
                XCTAssertEqual(
                    error as? SnapshotRetentionCapDraftError,
                    .invalidNumber
                )
            }
        }
    }
}

@MainActor
final class SnapshotRetentionCapSettingsModelTests: XCTestCase {
    func testLoadSaveAndResetPublishOnlyAuthoritativeResponses() async {
        let service = SnapshotRetentionCapServiceSpy()
        let model = SnapshotRetentionCapSettingsModel(service: service)

        await model.load()
        XCTAssertEqual(model.settings, .defaultValue)
        XCTAssertEqual(model.state, .ready)
        XCTAssertEqual(model.draft.gib, "2")

        model.draft.gib = "0"
        await model.save()
        XCTAssertEqual(model.settings?.capBytes, 0)
        XCTAssertEqual(model.settings?.source, .stored)
        let setValues = await service.setValues()
        XCTAssertEqual(setValues, [0])

        await model.reset()
        XCTAssertEqual(model.settings, .defaultValue)
        let resetCount = await service.resetCount()
        XCTAssertEqual(resetCount, 1)
        XCTAssertFalse(model.requiresAuthoritativeReload)
    }

    func testOutcomeUnknownPreservesLastGoodValueAndRequiresExplicitReload() async {
        let service = SnapshotRetentionCapServiceSpy()
        let model = SnapshotRetentionCapSettingsModel(service: service)
        await model.load()
        let previous = model.settings

        await service.failNextMutation(.outcomeUnknown)
        model.draft.gib = "4"
        await model.save()

        XCTAssertEqual(model.settings, previous)
        XCTAssertEqual(
            model.state,
            .failed(.service(.outcomeUnknown))
        )
        XCTAssertTrue(model.requiresAuthoritativeReload)
        let loadCountBeforeReload = await service.loadCount()
        XCTAssertEqual(loadCountBeforeReload, 1)

        await model.load(force: true)
        let loadCountAfterReload = await service.loadCount()
        XCTAssertEqual(loadCountAfterReload, 2)
        XCTAssertEqual(model.state, .ready)
        XCTAssertFalse(model.requiresAuthoritativeReload)
    }

    func testInvalidDraftDoesNotCallTheService() async {
        let service = SnapshotRetentionCapServiceSpy()
        let model = SnapshotRetentionCapSettingsModel(service: service)
        await model.load()
        model.draft.gib = "not-a-number"

        await model.save()

        XCTAssertEqual(model.state, .failed(.draft(.invalidNumber)))
        let setValues = await service.setValues()
        XCTAssertEqual(setValues, [])
    }

    func testShutdownFencesLaterOperations() async {
        let service = SnapshotRetentionCapServiceSpy()
        let model = SnapshotRetentionCapSettingsModel(service: service)
        await model.shutdown()

        await model.load()
        model.draft.gib = "1"
        await model.save()
        await model.reset()

        let loadCount = await service.loadCount()
        let setValues = await service.setValues()
        let resetCount = await service.resetCount()
        XCTAssertEqual(loadCount, 0)
        XCTAssertEqual(setValues, [])
        XCTAssertEqual(resetCount, 0)
    }

    func testSettingsAccessibilityIdentifiersAreStableUniqueAndSpecific() {
        let identifiers = SnapshotRetentionCapAccessibility.allControlIdentifiers
        XCTAssertEqual(identifiers.count, 7)
        XCTAssertEqual(Set(identifiers).count, identifiers.count)
        XCTAssertTrue(identifiers.allSatisfy { !$0.isEmpty })
        XCTAssertTrue(
            Set(identifiers).isDisjoint(
                with: CleanupHistoryClearAccessibility.allControlIdentifiers
            )
        )
        XCTAssertNotEqual(
            SnapshotRetentionCapSettingsView.message(
                for: SnapshotRetentionCapFailure.service(.outcomeUnknown)
            ),
            SnapshotRetentionCapSettingsView.message(
                for: SnapshotRetentionCapFailure.service(.incompatibleSchema)
            )
        )
        XCTAssertFalse(
            SnapshotRetentionCapSettingsView.message(
                for: SnapshotRetentionCapFailure.unexpected
            )
                .isEmpty
        )
    }
}

private extension SnapshotRetentionCapModel {
    static let defaultValue = SnapshotRetentionCapModel(
        capBytes: 2 * DiskPressurePolicyConfiguration.bytesPerGiB,
        source: .default,
        updatedAtUnixMilliseconds: nil
    )
}

private actor SnapshotRetentionCapServiceSpy: DuxSnapshotRetentionCapServing {
    private var current = SnapshotRetentionCapModel.defaultValue
    private var loads = 0
    private var sets: [UInt64] = []
    private var resets = 0
    private var nextMutationFailure: SnapshotRetentionCapServiceError?

    func loadSnapshotRetentionCap() async throws -> SnapshotRetentionCapModel {
        loads += 1
        return current
    }

    func setSnapshotRetentionCap(
        _ capBytes: UInt64
    ) async throws -> SnapshotRetentionCapUpdateResultModel {
        sets.append(capBytes)
        if let failure = nextMutationFailure {
            nextMutationFailure = nil
            throw failure
        }
        current = SnapshotRetentionCapModel(
            capBytes: capBytes,
            source: .stored,
            updatedAtUnixMilliseconds: 1
        )
        return SnapshotRetentionCapUpdateResultModel(
            settings: current,
            changed: true
        )
    }

    func resetSnapshotRetentionCap() async throws
        -> SnapshotRetentionCapUpdateResultModel
    {
        resets += 1
        if let failure = nextMutationFailure {
            nextMutationFailure = nil
            throw failure
        }
        let changed = current != .defaultValue
        current = .defaultValue
        return SnapshotRetentionCapUpdateResultModel(
            settings: current,
            changed: changed
        )
    }

    func failNextMutation(_ failure: SnapshotRetentionCapServiceError) {
        nextMutationFailure = failure
    }

    func loadCount() -> Int {
        loads
    }

    func setValues() -> [UInt64] {
        sets
    }

    func resetCount() -> Int {
        resets
    }
}
