import Foundation
import XCTest

private final class QualificationIdentityLoader: @unchecked Sendable {
    private let lock = NSLock()
    private var accountIndex = 0
    private var providerIndex = 0
    private var resourceIndex = 0
    private var versionIndex = 0
    private(set) var accessCount = 0
    private let accounts: [Data?]
    private let providers: [FoundationICloudFileProviderIdentity?]
    private let resources: [FoundationICloudResourceValues]
    private let versions: [Data?]

    init(
        accounts: [Data?],
        providers: [FoundationICloudFileProviderIdentity?],
        resources: [FoundationICloudResourceValues],
        versions: [Data?]
    ) {
        precondition(accounts.count == 2)
        precondition(providers.count == 2)
        precondition(resources.count == 2)
        precondition(versions.count == 2)
        self.accounts = accounts
        self.providers = providers
        self.resources = resources
        self.versions = versions
    }

    func account() -> Data? {
        lock.withLock {
            defer { accountIndex += 1 }
            accessCount += 1
            return accounts[accountIndex]
        }
    }

    func provider(_: URL) -> FoundationICloudFileProviderIdentity? {
        lock.withLock {
            defer { providerIndex += 1 }
            accessCount += 1
            return providers[providerIndex]
        }
    }

    func resource(
        _: URL,
        _: Set<URLResourceKey>
    ) -> FoundationICloudResourceValues {
        lock.withLock {
            defer { resourceIndex += 1 }
            accessCount += 1
            return resources[resourceIndex]
        }
    }

    func version(_: URL) -> Data? {
        lock.withLock {
            defer { versionIndex += 1 }
            accessCount += 1
            return versions[versionIndex]
        }
    }
}

final class ICloudIdentityQualificationReaderTests: XCTestCase {
    private let key = Data(repeating: 0xA5, count: 32)
    private let url = URL(fileURLWithPath: "/private/qualification-fixture")

    func testRejectsInvalidKeyBeforeAnyIdentityOrTargetAccess() throws {
        let loader = stableLoader()
        let reader = reader(loader)

        for invalidCount in [0, 1, 31, 33, 4_096] {
            XCTAssertThrowsError(
                try reader.readForQualification(
                    at: url,
                    key: Data(repeating: 1, count: invalidCount),
                    baseline: nil
                )
            ) { error in
                XCTAssertEqual(
                    error as? FoundationICloudLocalCopyRawFactReadError,
                    .invalidQualificationKey
                )
            }
        }
        XCTAssertEqual(loader.accessCount, 0)
    }

    func testStableIdentitiesCaptureCompletePrivateReferenceThenMatchBaseline() throws {
        let baseline = try reader(stableLoader()).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )

        XCTAssertTrue(baseline.privateIdentityReference.isComplete)
        XCTAssertNoThrow(
            try ICloudIdentityQualificationHarness.requireUsableBaseline(
                baseline.privateIdentityReference
            )
        )
        XCTAssertEqual(
            baseline.continuity,
            continuity(repeating: .noBaseline)
        )
        XCTAssertEqual(
            ICloudIdentityQualificationHarness.policyResultsForTesting(
                baseline.facts
            ).syncEligibility,
            "eligible"
        )
        XCTAssertEqual(
            ICloudIdentityQualificationHarness.policyResultsForTesting(
                baseline.facts
            ).identityReadiness,
            "ready"
        )

        let repeated = try reader(stableLoader()).readForQualification(
            at: url,
            key: key,
            baseline: baseline.privateIdentityReference
        )

        XCTAssertEqual(
            repeated.privateIdentityReference,
            baseline.privateIdentityReference
        )
        XCTAssertEqual(
            repeated.continuity,
            continuity(repeating: .sameAsBaseline)
        )
    }

    func testDifferentQualificationKeyCannotMatchPrivateBaseline() throws {
        let baseline = try reader(stableLoader()).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )

        let repeated = try reader(stableLoader()).readForQualification(
            at: url,
            key: Data(repeating: 0x5A, count: 32),
            baseline: baseline.privateIdentityReference
        )

        XCTAssertEqual(
            repeated.continuity,
            continuity(repeating: .changedSinceBaseline)
        )
    }

    func testCrossPhaseDriftIsReportedWithoutReturningRawIdentity() throws {
        let baseline = try reader(stableLoader()).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )
        let changed = stableLoader(suffix: "phase-b")

        let result = try reader(changed).readForQualification(
            at: url,
            key: key,
            baseline: baseline.privateIdentityReference
        )

        XCTAssertTrue(result.privateIdentityReference.isComplete)
        XCTAssertEqual(
            result.continuity,
            continuity(repeating: .changedSinceBaseline)
        )
    }

    func testWithinReadIdentityDriftIsNotMintedIntoPrivateState() throws {
        let first = qualificationValues(generation: identity("generation-a"))
        let second = qualificationValues(generation: identity("generation-b"))
        let loader = QualificationIdentityLoader(
            accounts: [identity("account-a"), identity("account-b")],
            providers: [
                provider(domain: "domain-a", item: "item-a"),
                provider(domain: "domain-b", item: "item-b"),
            ],
            resources: [first, second],
            versions: [identity("version-a"), identity("version-b")]
        )

        let result = try reader(loader).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )

        XCTAssertFalse(result.privateIdentityReference.isComplete)
        XCTAssertEqual(
            result.continuity,
            continuity(repeating: .changedDuringRead)
        )
    }

    func testUnavailableIdentitiesProduceNoPrivateTags() throws {
        let values = qualificationValues(generation: nil)
        let loader = QualificationIdentityLoader(
            accounts: [nil, nil],
            providers: [nil, nil],
            resources: [values, values],
            versions: [nil, nil]
        )

        let result = try reader(loader).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )

        XCTAssertFalse(result.privateIdentityReference.isComplete)
        XCTAssertEqual(result.continuity, continuity(repeating: .unavailable))
        XCTAssertEqual(
            ICloudIdentityQualificationHarness.policyResultsForTesting(
                result.facts
            ).syncEligibility,
            "eligible"
        )
        XCTAssertEqual(
            ICloudIdentityQualificationHarness.policyResultsForTesting(
                result.facts
            ).identityReadiness,
            "blocked"
        )
    }

    func testPartialBaselineRecordsStableSlotsAndKeepsMissingSlotUnproven() throws {
        let values = qualificationValues()
        let partialLoader = QualificationIdentityLoader(
            accounts: [nil, nil],
            providers: [
                provider(domain: "domain-stable", item: "item-stable"),
                provider(domain: "domain-stable", item: "item-stable"),
            ],
            resources: [values, values],
            versions: [identity("version-stable"), identity("version-stable")]
        )
        let baseline = try reader(partialLoader).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )

        XCTAssertFalse(baseline.privateIdentityReference.isComplete)
        XCTAssertThrowsError(
            try ICloudIdentityQualificationHarness.requireUsableBaseline(
                baseline.privateIdentityReference
            )
        ) { error in
            XCTAssertEqual(
                error as? ICloudIdentityQualificationHarnessError,
                .observationChanged
            )
        }
        XCTAssertEqual(
            baseline.continuity,
            FoundationICloudQualificationIdentityContinuity(
                accountToken: .unavailable,
                fileProviderDomain: .noBaseline,
                fileProviderItem: .noBaseline,
                itemGeneration: .noBaseline,
                fileVersion: .noBaseline
            )
        )

        let later = try reader(stableLoader()).readForQualification(
            at: url,
            key: key,
            baseline: baseline.privateIdentityReference
        )
        XCTAssertEqual(
            later.continuity,
            FoundationICloudQualificationIdentityContinuity(
                accountToken: .noBaseline,
                fileProviderDomain: .sameAsBaseline,
                fileProviderItem: .sameAsBaseline,
                itemGeneration: .sameAsBaseline,
                fileVersion: .sameAsBaseline
            )
        )
    }

    func testNonUbiquitousItemIsUnsupportedAndProducesNoPrivateTags() throws {
        let values = qualificationValues(isUbiquitous: false)
        let loader = stableLoader(values: values)

        let result = try reader(loader).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )

        XCTAssertFalse(result.privateIdentityReference.isComplete)
        XCTAssertEqual(result.continuity, continuity(repeating: .unsupported))
    }

    func testOversizedIdentitiesAreUnavailableAndNeverHashed() throws {
        let oversizedData = Data(
            repeating: 0x41,
            count: FoundationICloudLocalCopyRawFactReader.maximumIdentityArchiveBytes + 1
        )
        let oversizedString = String(
            repeating: "x",
            count: FoundationICloudLocalCopyRawFactReader
                .maximumFileProviderIdentifierBytes + 1
        )
        let values = qualificationValues(generation: oversizedData)
        let loader = QualificationIdentityLoader(
            accounts: [oversizedData, oversizedData],
            providers: [
                provider(domain: oversizedString, item: oversizedString),
                provider(domain: oversizedString, item: oversizedString),
            ],
            resources: [values, values],
            versions: [oversizedData, oversizedData]
        )

        let result = try reader(loader).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )

        XCTAssertFalse(result.privateIdentityReference.isComplete)
        XCTAssertEqual(result.continuity, continuity(repeating: .unavailable))
    }

    func testPrivateReferenceIsCodableDomainSeparatedAndContainsNoRawIdentity() throws {
        let result = try reader(stableLoader()).readForQualification(
            at: url,
            key: key,
            baseline: nil
        )

        let encoded = try JSONEncoder().encode(result.privateIdentityReference)
        let decoded = try JSONDecoder().decode(
            FoundationICloudQualificationIdentityReference.self,
            from: encoded
        )
        XCTAssertEqual(decoded, result.privateIdentityReference)

        let encodedText = try XCTUnwrap(String(data: encoded, encoding: .utf8))
        for rawIdentity in ["account-stable", "domain-stable", "item-stable",
                            "generation-stable", "version-stable"] {
            XCTAssertFalse(encodedText.contains(rawIdentity))
        }

        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: encoded) as? [String: String]
        )
        let tags = try object.values.map { value in
            try XCTUnwrap(Data(base64Encoded: value))
        }
        XCTAssertEqual(tags.count, 5)
        XCTAssertEqual(Set(tags).count, 5)
        XCTAssertTrue(tags.allSatisfy { $0.count == 32 })
    }

    private func stableLoader(
        suffix: String = "stable",
        values: FoundationICloudResourceValues? = nil
    ) -> QualificationIdentityLoader {
        let values = values ?? qualificationValues(
            generation: identity("generation-\(suffix)")
        )
        return QualificationIdentityLoader(
            accounts: [identity("account-\(suffix)"), identity("account-\(suffix)")],
            providers: [
                provider(domain: "domain-\(suffix)", item: "item-\(suffix)"),
                provider(domain: "domain-\(suffix)", item: "item-\(suffix)"),
            ],
            resources: [values, values],
            versions: [identity("version-\(suffix)"), identity("version-\(suffix)")]
        )
    }

    private func reader(
        _ loader: QualificationIdentityLoader
    ) -> FoundationICloudLocalCopyRawFactReader {
        FoundationICloudLocalCopyRawFactReader(
            loadAccountIdentity: { loader.account() },
            loadFileVersionIdentity: { loader.version($0) },
            loadFileProviderIdentity: { loader.provider($0) },
            loadResourceValues: { loader.resource($0, $1) }
        )
    }

    private func provider(
        domain: String,
        item: String
    ) -> FoundationICloudFileProviderIdentity {
        FoundationICloudFileProviderIdentity(
            domainIdentifier: domain,
            itemIdentifier: item
        )
    }

    private func identity(_ value: String) -> Data {
        Data(value.utf8)
    }

    private func continuity(
        repeating value: FoundationICloudQualificationContinuity
    ) -> FoundationICloudQualificationIdentityContinuity {
        FoundationICloudQualificationIdentityContinuity(
            accountToken: value,
            fileProviderDomain: value,
            fileProviderItem: value,
            itemGeneration: value,
            fileVersion: value
        )
    }

    private func qualificationValues(
        isUbiquitous: Bool? = true,
        generation: Data? = Data("generation-stable".utf8)
    ) -> FoundationICloudResourceValues {
        FoundationICloudResourceValues(
            isUbiquitous: isUbiquitous,
            isUploaded: true,
            isUploading: false,
            hasUnresolvedConflicts: false,
            isDownloading: false,
            downloadRequested: false,
            isExcludedFromSync: false,
            isShared: false,
            isSyncPaused: false,
            uploadingErrorPresence: .absent,
            downloadingErrorPresence: .absent,
            downloadStatus: .current,
            isRegularFile: true,
            isDirectory: false,
            isSymbolicLink: false,
            fileAllocatedSize: 4_096,
            totalFileAllocatedSize: 4_096,
            generationIdentifierArchive: generation
        )
    }
}
