import Foundation
import XCTest
@testable import DUX

private final class ICloudResourceValueLoaderSpy: @unchecked Sendable {
    private let lock = NSLock()
    private var callCountStorage = 0
    private var requestedKeysStorage: Set<URLResourceKey> = []
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
        let sentinel: Any?
        if let sentinelKey {
            sentinel = try url.resourceValues(forKeys: [sentinelKey]).allValues[sentinelKey]
        } else {
            sentinel = nil
        }
        lock.withLock {
            callCountStorage += 1
            requestedKeysStorage = keys
            cachedSentinelStorage = sentinel
        }
        return values
    }

    var callCount: Int {
        lock.withLock { callCountStorage }
    }

    var requestedKeys: Set<URLResourceKey> {
        lock.withLock { requestedKeysStorage }
    }

    var cachedSentinel: Any? {
        lock.withLock { cachedSentinelStorage }
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

    func testClearsCachedValuesAndRequestsEveryKeyInOneLoad() throws {
        let sentinelKey = URLResourceKey("se.mjukis.dux.tests.cached-sentinel")
        var url = URL(fileURLWithPath: "/tmp/report")
        url.setTemporaryResourceValue("cached", forKey: sentinelKey)
        let spy = ICloudResourceValueLoaderSpy(values: fixture())
        let reader = FoundationICloudLocalCopyRawFactReader { url, keys in
            try spy.load(url, keys, sentinelKey: sentinelKey)
        }

        _ = try reader.read(at: url)

        XCTAssertEqual(spy.callCount, 1)
        XCTAssertEqual(spy.requestedKeys, FoundationICloudLocalCopyRawFactReader.requestedKeys)
        XCTAssertNil(spy.cachedSentinel)
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

        let facts = try FoundationICloudLocalCopyRawFactReader().read(at: file)

        XCTAssertEqual(facts.uploadingErrorPresence, .absent)
        XCTAssertEqual(facts.downloadingErrorPresence, .absent)
    }

    private func reader(
        _ values: FoundationICloudResourceValues
    ) -> FoundationICloudLocalCopyRawFactReader {
        FoundationICloudLocalCopyRawFactReader { _, _ in values }
    }

    private func fixture(
        isUbiquitous: Bool? = true,
        isUploaded: Bool? = true,
        isUploading: Bool? = false,
        hasUnresolvedConflicts: Bool? = false,
        isDownloading: Bool? = false,
        downloadRequested: Bool? = false,
        isExcludedFromSync: Bool? = false,
        uploadingErrorPresence: FoundationICloudErrorPresence = .absent,
        downloadingErrorPresence: FoundationICloudErrorPresence = .absent,
        downloadStatus: URLUbiquitousItemDownloadingStatus? = .current,
        isRegularFile: Bool? = true,
        isDirectory: Bool? = false,
        isSymbolicLink: Bool? = false,
        fileAllocatedSize: Int? = 100,
        totalFileAllocatedSize: Int? = 120
    ) -> FoundationICloudResourceValues {
        FoundationICloudResourceValues(
            isUbiquitous: isUbiquitous,
            isUploaded: isUploaded,
            isUploading: isUploading,
            hasUnresolvedConflicts: hasUnresolvedConflicts,
            isDownloading: isDownloading,
            downloadRequested: downloadRequested,
            isExcludedFromSync: isExcludedFromSync,
            uploadingErrorPresence: uploadingErrorPresence,
            downloadingErrorPresence: downloadingErrorPresence,
            downloadStatus: downloadStatus,
            isRegularFile: isRegularFile,
            isDirectory: isDirectory,
            isSymbolicLink: isSymbolicLink,
            fileAllocatedSize: fileAllocatedSize,
            totalFileAllocatedSize: totalFileAllocatedSize
        )
    }
}
