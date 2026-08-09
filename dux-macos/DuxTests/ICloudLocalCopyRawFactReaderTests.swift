@testable import DUX
import Foundation
import XCTest

private final class ICloudResourceValueLoaderSpy: @unchecked Sendable {
    private let lock = NSLock()
    private var callCountStorage = 0
    private var requestedKeysStorage: [Set<URLResourceKey>] = []
    private var cachedSentinelStorage: Any?
    var values: FoundationICloudResourceValues

    init(values: FoundationICloudResourceValues) {
        self.values = values
    }

    func load(
        _ url: URL,
        _ keys: Set<URLResourceKey>,
        sentinelKey: URLResourceKey? = nil
    ) throws -> FoundationICloudResourceValues {
        let sentinel: Any? = if let sentinelKey {
            try url.resourceValues(forKeys: [sentinelKey]).allValues[sentinelKey]
        } else {
            nil
        }
        lock.withLock {
            callCountStorage += 1
            requestedKeysStorage.append(keys)
            cachedSentinelStorage = sentinel
        }
        return values
    }

    var callCount: Int {
        lock.withLock { callCountStorage }
    }

    var requestedKeys: [Set<URLResourceKey>] {
        lock.withLock { requestedKeysStorage }
    }

    var cachedSentinel: Any? {
        lock.withLock { cachedSentinelStorage }
    }
}

private final class ICloudIdentityCaptureLoaderSpy: @unchecked Sendable {
    enum Event: Equatable {
        case account
        case fileProvider
        case resources(Set<URLResourceKey>)
        case fileVersion
    }

    private let lock = NSLock()
    private var eventsStorage: [Event] = []
    private var accountIndex = 0
    private var fileProviderIndex = 0
    private var resourceIndex = 0
    private var fileVersionIndex = 0
    private let accounts: [Data?]
    private let fileProviderIdentities: [FoundationICloudFileProviderIdentity?]
    private let resources: [FoundationICloudResourceValues]
    private let fileVersions: [Data?]

    init(
        accounts: [Data?],
        fileProviderIdentities: [FoundationICloudFileProviderIdentity?] = [
            FoundationICloudFileProviderIdentity(
                domainIdentifier: "stable-domain",
                itemIdentifier: "stable-item"
            ),
            FoundationICloudFileProviderIdentity(
                domainIdentifier: "stable-domain",
                itemIdentifier: "stable-item"
            ),
        ],
        resources: [FoundationICloudResourceValues],
        fileVersions: [Data?]
    ) {
        precondition(accounts.count == 2)
        precondition(fileProviderIdentities.count == 2)
        precondition(resources.count == 2)
        precondition(fileVersions.count == 2)
        self.accounts = accounts
        self.fileProviderIdentities = fileProviderIdentities
        self.resources = resources
        self.fileVersions = fileVersions
    }

    func loadAccount() -> Data? {
        lock.withLock {
            defer { accountIndex += 1 }
            eventsStorage.append(.account)
            return accounts[accountIndex]
        }
    }

    func loadResources(
        _: URL,
        keys: Set<URLResourceKey>
    ) -> FoundationICloudResourceValues {
        lock.withLock {
            defer { resourceIndex += 1 }
            eventsStorage.append(.resources(keys))
            return resources[resourceIndex]
        }
    }

    func loadFileProvider(_: URL) -> FoundationICloudFileProviderIdentity? {
        lock.withLock {
            defer { fileProviderIndex += 1 }
            eventsStorage.append(.fileProvider)
            return fileProviderIdentities[fileProviderIndex]
        }
    }

    func loadFileVersion(_: URL) -> Data? {
        lock.withLock {
            defer { fileVersionIndex += 1 }
            eventsStorage.append(.fileVersion)
            return fileVersions[fileVersionIndex]
        }
    }

    var events: [Event] {
        lock.withLock { eventsStorage }
    }
}

final class ICloudLocalCopyRawFactReaderTests: XCTestCase {
    func testPreservesOptionalSyncFactsAndMapsErrorPresence() throws {
        let values = fixture(
            isUbiquitous: nil,
            isUploaded: true,
            isUploading: nil,
            hasUnresolvedConflicts: false,
            isDownloading: nil,
            downloadRequested: nil,
            isExcludedFromSync: true,
            isShared: true,
            isSyncPaused: true,
            uploadingErrorPresence: .present,
            downloadingErrorPresence: .unknown
        )
        let reader = reader(values)

        let facts = try reader.read(at: URL(fileURLWithPath: "/tmp/report"))

        XCTAssertNil(facts.isUbiquitous)
        XCTAssertEqual(facts.isUploaded, true)
        XCTAssertNil(facts.isUploading)
        XCTAssertEqual(facts.hasUnresolvedConflicts, false)
        XCTAssertNil(facts.isDownloading)
        XCTAssertNil(facts.downloadRequested)
        XCTAssertEqual(facts.isExcludedFromSync, true)
        XCTAssertEqual(facts.isShared, true)
        XCTAssertEqual(facts.isSyncPaused, true)
        XCTAssertEqual(facts.uploadingErrorPresence, .present)
        XCTAssertEqual(facts.downloadingErrorPresence, .unknown)
    }

    func testPreservesEverySyncErrorPresenceState() throws {
        let cases: [FoundationICloudErrorPresence] = [.present, .absent, .unknown]

        for presence in cases {
            let facts = try reader(
                fixture(
                    uploadingErrorPresence: presence,
                    downloadingErrorPresence: presence
                )
            ).read(at: URL(fileURLWithPath: "/tmp/report"))
            XCTAssertEqual(facts.uploadingErrorPresence, presence)
            XCTAssertEqual(facts.downloadingErrorPresence, presence)
        }
    }

    func testMapsEveryDownloadStatusWithoutUsingDeprecatedDownloadedBoolean() throws {
        let cases: [(URLUbiquitousItemDownloadingStatus?, FoundationICloudDownloadStatus)] = [
            (.current, .current),
            (.downloaded, .stale),
            (.notDownloaded, .notDownloaded),
            (URLUbiquitousItemDownloadingStatus(rawValue: "future-status"), .unknown),
            (nil, .unknown),
        ]

        for (status, expected) in cases {
            let facts = try reader(fixture(downloadStatus: status))
                .read(at: URL(fileURLWithPath: "/tmp/report"))
            XCTAssertEqual(facts.downloadStatus, expected)
        }
    }

    func testMapsKindOnlyWhenFoundationFactsAreUnambiguous() throws {
        let cases: [(Bool?, Bool?, Bool?, FoundationICloudItemKind)] = [
            (true, false, false, .regularFile),
            (false, true, false, .directory),
            (false, false, true, .symbolicLink),
            (false, false, false, .other),
            (nil, false, false, .unknown),
            (true, true, false, .unknown),
        ]

        for (regular, directory, symlink, expected) in cases {
            let facts = try reader(
                fixture(
                    isRegularFile: regular,
                    isDirectory: directory,
                    isSymbolicLink: symlink
                )
            ).read(at: URL(fileURLWithPath: "/tmp/report"))
            XCTAssertEqual(facts.itemKind, expected)
        }
    }

    func testUsesFileAllocationAndOnlyFallsBackToTotalAllocation() throws {
        let filePreferred = try reader(
            fixture(fileAllocatedSize: 100, totalFileAllocatedSize: 120)
        ).read(at: URL(fileURLWithPath: "/tmp/report"))
        XCTAssertEqual(filePreferred.allocatedBytes, 100)
        XCTAssertEqual(filePreferred.allocatedBytesSource, .file)

        let totalFallback = try reader(
            fixture(fileAllocatedSize: nil, totalFileAllocatedSize: 120)
        ).read(at: URL(fileURLWithPath: "/tmp/report"))
        XCTAssertEqual(totalFallback.allocatedBytes, 120)
        XCTAssertEqual(totalFallback.allocatedBytesSource, .totalFallback)

        let invalidFileFallsBack = try reader(
            fixture(fileAllocatedSize: -1, totalFileAllocatedSize: 120)
        ).read(at: URL(fileURLWithPath: "/tmp/report"))
        XCTAssertEqual(invalidFileFallsBack.allocatedBytes, 120)
        XCTAssertEqual(invalidFileFallsBack.allocatedBytesSource, .totalFallback)

        let unknown = try reader(
            fixture(fileAllocatedSize: -1, totalFileAllocatedSize: -1)
        ).read(at: URL(fileURLWithPath: "/tmp/report"))
        XCTAssertNil(unknown.allocatedBytes)
        XCTAssertNil(unknown.allocatedBytesSource)
    }

    func testStableIdentitiesBracketOneCompleteMetadataOperation() throws {
        let account = identity("account-a")
        let generation = identity("generation-a")
        let fileVersion = identity("file-version-a")
        let values = fixture(generationIdentifierArchive: generation)
        let spy = ICloudIdentityCaptureLoaderSpy(
            accounts: [account, account],
            resources: [values, values],
            fileVersions: [fileVersion, fileVersion]
        )

        let facts = try reader(spy).read(
            at: URL(fileURLWithPath: "/private/redacted-item")
        )

        XCTAssertEqual(facts.identityCapability, FoundationICloudIdentityCapability(
            accountTokenStability: .stable,
            domainIdentifierStability: .stable,
            providerItemIdentifierStability: .stable,
            itemGenerationStability: .stable,
            fileVersionPersistentIDStability: .stable
        ))
        XCTAssertEqual(spy.events, [
            .account,
            .fileProvider,
            .resources(FoundationICloudLocalCopyRawFactReader.requestedKeys),
            .fileVersion,
            .resources(FoundationICloudLocalCopyRawFactReader.requestedKeys),
            .fileVersion,
            .fileProvider,
            .account,
        ])
    }

    func testChangedAccountTokenIsReportedWithoutExposingEitherToken() throws {
        let values = fixture()
        let spy = ICloudIdentityCaptureLoaderSpy(
            accounts: [identity("account-a"), identity("account-b")],
            resources: [values, values],
            fileVersions: [identity("version"), identity("version")]
        )

        let facts = try reader(spy).read(at: URL(fileURLWithPath: "/private/item"))

        XCTAssertEqual(facts.identityCapability.accountTokenStability, .changed)
        XCTAssertEqual(facts.identityCapability.itemGenerationStability, .stable)
        XCTAssertEqual(
            facts.identityCapability.fileVersionPersistentIDStability,
            .stable
        )
    }

    func testNilIdentitiesAreUnavailable() throws {
        let values = fixture(generationIdentifierArchive: nil)
        let spy = ICloudIdentityCaptureLoaderSpy(
            accounts: [nil, nil],
            fileProviderIdentities: [nil, nil],
            resources: [values, values],
            fileVersions: [nil, nil]
        )

        let facts = try reader(spy).read(at: URL(fileURLWithPath: "/private/item"))

        XCTAssertEqual(facts.identityCapability, FoundationICloudIdentityCapability(
            accountTokenStability: .unavailable,
            domainIdentifierStability: .unavailable,
            providerItemIdentifierStability: .unavailable,
            itemGenerationStability: .unavailable,
            fileVersionPersistentIDStability: .unavailable
        ))
    }

    func testFileProviderDomainAndItemDriftAreReportedIndependently() throws {
        let values = fixture()
        let cases: [(
            first: FoundationICloudFileProviderIdentity,
            second: FoundationICloudFileProviderIdentity,
            domain: FoundationICloudIdentityStability,
            item: FoundationICloudIdentityStability
        )] = [
            (
                FoundationICloudFileProviderIdentity(
                    domainIdentifier: "domain-a",
                    itemIdentifier: "item"
                ),
                FoundationICloudFileProviderIdentity(
                    domainIdentifier: "domain-b",
                    itemIdentifier: "item"
                ),
                .changed,
                .stable
            ),
            (
                FoundationICloudFileProviderIdentity(
                    domainIdentifier: "domain",
                    itemIdentifier: "item-a"
                ),
                FoundationICloudFileProviderIdentity(
                    domainIdentifier: "domain",
                    itemIdentifier: "item-b"
                ),
                .stable,
                .changed
            ),
        ]

        for testCase in cases {
            let spy = ICloudIdentityCaptureLoaderSpy(
                accounts: [identity("account"), identity("account")],
                fileProviderIdentities: [testCase.first, testCase.second],
                resources: [values, values],
                fileVersions: [identity("version"), identity("version")]
            )

            let facts = try reader(spy).read(at: URL(fileURLWithPath: "/private/item"))

            XCTAssertEqual(
                facts.identityCapability.domainIdentifierStability,
                testCase.domain
            )
            XCTAssertEqual(
                facts.identityCapability.providerItemIdentifierStability,
                testCase.item
            )
            XCTAssertEqual(facts.identityCapability.accountTokenStability, .stable)
            XCTAssertEqual(facts.identityCapability.itemGenerationStability, .stable)
        }
    }

    func testInvalidFileProviderIdentifiersAreUnavailable() throws {
        let oversized = String(
            repeating: "x",
            count: FoundationICloudLocalCopyRawFactReader
                .maximumFileProviderIdentifierBytes + 1
        )
        let values = fixture()
        let spy = ICloudIdentityCaptureLoaderSpy(
            accounts: [identity("account"), identity("account")],
            fileProviderIdentities: [
                FoundationICloudFileProviderIdentity(
                    domainIdentifier: "",
                    itemIdentifier: oversized
                ),
                FoundationICloudFileProviderIdentity(
                    domainIdentifier: "",
                    itemIdentifier: oversized
                ),
            ],
            resources: [values, values],
            fileVersions: [identity("version"), identity("version")]
        )

        let facts = try reader(spy).read(at: URL(fileURLWithPath: "/private/item"))

        XCTAssertEqual(facts.identityCapability.domainIdentifierStability, .unavailable)
        XCTAssertEqual(
            facts.identityCapability.providerItemIdentifierStability,
            .unavailable
        )
    }

    func testGenerationAndFileVersionDriftAreReportedIndependently() throws {
        let first = fixture(generationIdentifierArchive: identity("generation-a"))
        let second = fixture(generationIdentifierArchive: identity("generation-b"))
        let spy = ICloudIdentityCaptureLoaderSpy(
            accounts: [identity("account"), identity("account")],
            resources: [first, second],
            fileVersions: [identity("version-a"), identity("version-b")]
        )

        let facts = try reader(spy).read(at: URL(fileURLWithPath: "/private/item"))

        XCTAssertEqual(facts.identityCapability.accountTokenStability, .stable)
        XCTAssertEqual(facts.identityCapability.itemGenerationStability, .changed)
        XCTAssertEqual(
            facts.identityCapability.fileVersionPersistentIDStability,
            .changed
        )
    }

    func testLiveFactDriftFailsWholeReadWithoutPartialFacts() {
        let first = fixture(isUploaded: true)
        let second = fixture(isUploaded: false)
        let spy = ICloudIdentityCaptureLoaderSpy(
            accounts: [identity("account"), identity("account")],
            resources: [first, second],
            fileVersions: [identity("version"), identity("version")]
        )

        XCTAssertThrowsError(
            try reader(spy).read(at: URL(fileURLWithPath: "/private/item"))
        ) { error in
            XCTAssertEqual(
                error as? FoundationICloudLocalCopyRawFactReadError,
                .liveFactsChanged
            )
        }
        XCTAssertEqual(spy.events.count, 8)
    }

    func testNonUbiquitousContainerMarksEveryIdentityUnsupported() throws {
        let values = fixture(isUbiquitous: false)
        let spy = ICloudIdentityCaptureLoaderSpy(
            accounts: [identity("account"), identity("account")],
            resources: [values, values],
            fileVersions: [identity("version"), identity("version")]
        )

        let facts = try reader(spy).read(at: URL(fileURLWithPath: "/private/item"))

        XCTAssertEqual(facts.identityCapability, FoundationICloudIdentityCapability(
            accountTokenStability: .unsupported,
            domainIdentifierStability: .unsupported,
            providerItemIdentifierStability: .unsupported,
            itemGenerationStability: .unsupported,
            fileVersionPersistentIDStability: .unsupported
        ))
    }

    func testOversizedIdentityArchivesAreUnavailableAndNeverCompared() throws {
        let oversized = Data(
            repeating: 0x41,
            count: FoundationICloudLocalCopyRawFactReader.maximumIdentityArchiveBytes + 1
        )
        let values = fixture(generationIdentifierArchive: oversized)
        let spy = ICloudIdentityCaptureLoaderSpy(
            accounts: [oversized, oversized],
            resources: [values, values],
            fileVersions: [oversized, oversized]
        )

        let facts = try reader(spy).read(at: URL(fileURLWithPath: "/private/item"))

        XCTAssertEqual(facts.identityCapability.accountTokenStability, .unavailable)
        XCTAssertEqual(facts.identityCapability.itemGenerationStability, .unavailable)
        XCTAssertEqual(
            facts.identityCapability.fileVersionPersistentIDStability,
            .unavailable
        )
    }

    func testClearsCachedValuesAndRequestsEveryKeyInBothCompleteLoads() throws {
        let sentinelKey = URLResourceKey("se.mjukis.dux.tests.cached-sentinel")
        var url = URL(fileURLWithPath: "/tmp/report")
        url.setTemporaryResourceValue("cached", forKey: sentinelKey)
        let spy = ICloudResourceValueLoaderSpy(values: fixture())
        let reader = FoundationICloudLocalCopyRawFactReader(
            loadAccountIdentity: { nil },
            loadFileVersionIdentity: { _ in nil },
            loadFileProviderIdentity: { _ in nil },
            loadResourceValues: { url, keys in
                try spy.load(url, keys, sentinelKey: sentinelKey)
            }
        )

        _ = try reader.read(at: url)

        XCTAssertEqual(spy.callCount, 2)
        XCTAssertEqual(spy.requestedKeys, [
            FoundationICloudLocalCopyRawFactReader.requestedKeys,
            FoundationICloudLocalCopyRawFactReader.requestedKeys,
        ])
        XCTAssertNil(spy.cachedSentinel)
        XCTAssertTrue(
            FoundationICloudLocalCopyRawFactReader.requestedKeys.contains(
                .generationIdentifierKey
            )
        )
        XCTAssertTrue(
            FoundationICloudLocalCopyRawFactReader.requestedKeys.contains(
                .ubiquitousItemIsSharedKey
            )
        )
        if #available(macOS 26.0, *) {
            XCTAssertTrue(
                FoundationICloudLocalCopyRawFactReader.requestedKeys.contains(
                    .ubiquitousItemIsSyncPausedKey
                )
            )
        }
    }

    func testSuccessfulFoundationReadTreatsNilTransferErrorsAsAbsent() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: false
        )
        // DUX-DESTRUCTIVE: allow=test-swift-icloud-reader-fixture-remove -- remove only the UUID-named temporary directory created by this test
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("ordinary.bin")
        XCTAssertTrue(FileManager.default.createFile(atPath: file.path, contents: Data([1])))

        let facts = try FoundationICloudLocalCopyRawFactReader(
            loadFileProviderIdentity: { _ in nil }
        ).read(at: file)

        XCTAssertEqual(facts.uploadingErrorPresence, .absent)
        XCTAssertEqual(facts.downloadingErrorPresence, .absent)
    }

    private func reader(
        _ values: FoundationICloudResourceValues
    ) -> FoundationICloudLocalCopyRawFactReader {
        FoundationICloudLocalCopyRawFactReader(
            loadAccountIdentity: {
                Data("stable-account".utf8)
            },
            loadFileVersionIdentity: { _ in
                Data("stable-file-version".utf8)
            },
            loadFileProviderIdentity: { _ in
                FoundationICloudFileProviderIdentity(
                    domainIdentifier: "stable-domain",
                    itemIdentifier: "stable-item"
                )
            },
            loadResourceValues: { _, _ in values }
        )
    }

    private func reader(
        _ spy: ICloudIdentityCaptureLoaderSpy
    ) -> FoundationICloudLocalCopyRawFactReader {
        FoundationICloudLocalCopyRawFactReader(
            loadAccountIdentity: spy.loadAccount,
            loadFileVersionIdentity: spy.loadFileVersion,
            loadFileProviderIdentity: spy.loadFileProvider,
            loadResourceValues: spy.loadResources
        )
    }

    private func identity(_ value: String) -> Data {
        Data(value.utf8)
    }

    private func fixture(
        isUbiquitous: Bool? = true,
        isUploaded: Bool? = true,
        isUploading: Bool? = false,
        hasUnresolvedConflicts: Bool? = false,
        isDownloading: Bool? = false,
        downloadRequested: Bool? = false,
        isExcludedFromSync: Bool? = false,
        isShared: Bool? = false,
        isSyncPaused: Bool? = false,
        uploadingErrorPresence: FoundationICloudErrorPresence = .absent,
        downloadingErrorPresence: FoundationICloudErrorPresence = .absent,
        downloadStatus: URLUbiquitousItemDownloadingStatus? = .current,
        isRegularFile: Bool? = true,
        isDirectory: Bool? = false,
        isSymbolicLink: Bool? = false,
        fileAllocatedSize: Int? = 100,
        totalFileAllocatedSize: Int? = 120,
        generationIdentifierArchive: Data? = Data("stable-generation".utf8)
    ) -> FoundationICloudResourceValues {
        FoundationICloudResourceValues(
            isUbiquitous: isUbiquitous,
            isUploaded: isUploaded,
            isUploading: isUploading,
            hasUnresolvedConflicts: hasUnresolvedConflicts,
            isDownloading: isDownloading,
            downloadRequested: downloadRequested,
            isExcludedFromSync: isExcludedFromSync,
            isShared: isShared,
            isSyncPaused: isSyncPaused,
            uploadingErrorPresence: uploadingErrorPresence,
            downloadingErrorPresence: downloadingErrorPresence,
            downloadStatus: downloadStatus,
            isRegularFile: isRegularFile,
            isDirectory: isDirectory,
            isSymbolicLink: isSymbolicLink,
            fileAllocatedSize: fileAllocatedSize,
            totalFileAllocatedSize: totalFileAllocatedSize,
            generationIdentifierArchive: generationIdentifierArchive
        )
    }
}
