import Foundation
import XCTest
@testable import DUX

private final class StubICloudProbeRequest: ICloudLocalCopyProbeRequestReading, @unchecked Sendable {
    private let lock = NSLock()
    private let version: UInt32
    private let encoding: SnapshotNameEncoding
    private var bytes: Data?
    private(set) var takeCount = 0

    init(
        version: UInt32 = 1,
        encoding: SnapshotNameEncoding = .unixBytes,
        bytes: Data = Data("/tmp/dux-icloud-probe.bin".utf8)
    ) {
        self.version = version
        self.encoding = encoding
        self.bytes = bytes
    }

    func recordVersion() throws -> UInt32 { version }
    func pathEncoding() throws -> SnapshotNameEncoding { encoding }

    func takePathBytes() throws -> Data {
        try lock.withLock {
            takeCount += 1
            guard let bytes else {
                throw ExplorerICloudLocalCopyProbeError.failed
            }
            self.bytes = nil
            return bytes
        }
    }
}

private final class StubICloudFactReader: ICloudLocalCopyRawFactReading, @unchecked Sendable {
    private let lock = NSLock()
    private let result: Result<FoundationICloudLocalCopyFacts, Error>
    private(set) var urls: [URL] = []

    init(_ result: Result<FoundationICloudLocalCopyFacts, Error>) {
        self.result = result
    }

    func read(at url: URL) throws -> FoundationICloudLocalCopyFacts {
        lock.withLock { urls.append(url) }
        return try result.get()
    }
}

final class ICloudLocalCopyMetadataDriverTests: XCTestCase {
    func testConsumesExactPathOnceAndMapsEveryFact() {
        let request = StubICloudProbeRequest()
        let reader = StubICloudFactReader(.success(facts(
            isUbiquitous: true,
            isUploaded: false,
            isUploading: nil,
            hasUnresolvedConflicts: true,
            isDownloading: false,
            downloadRequested: nil,
            isExcludedFromSync: false,
            isShared: true,
            isSyncPaused: nil,
            uploadingErrorPresence: .present,
            downloadingErrorPresence: .unknown,
            downloadStatus: .stale,
            identityCapability: FoundationICloudIdentityCapability(
                containerState: .supported,
                accountTokenStability: .changed,
                itemGenerationStability: .stable,
                fileVersionPersistentIDStability: .unavailable
            )
        )))

        let result = MacOSICloudLocalCopyMetadataAdapter(reader: reader)
            .readMetadata(request: request)

        guard case let .observed(raw) = result else {
            return XCTFail("expected observed facts")
        }
        XCTAssertEqual(request.takeCount, 1)
        XCTAssertEqual(reader.urls.map(\.path), ["/tmp/dux-icloud-probe.bin"])
        XCTAssertEqual(raw.recordVersion, 1)
        XCTAssertEqual(raw.ubiquitous, .`true`)
        XCTAssertEqual(raw.uploaded, .`false`)
        XCTAssertEqual(raw.uploading, .unknown)
        XCTAssertEqual(raw.uploadError, .present)
        XCTAssertEqual(raw.unresolvedConflicts, .`true`)
        XCTAssertEqual(raw.localCopyState, .stale)
        XCTAssertEqual(raw.downloadRequested, .unknown)
        XCTAssertEqual(raw.downloading, .`false`)
        XCTAssertEqual(raw.downloadError, .unknown)
        XCTAssertEqual(raw.excludedFromSync, .`false`)
        XCTAssertEqual(raw.accountIdentity, .changedDuringRead)
        XCTAssertEqual(raw.containerIdentity, .unsupported)
        XCTAssertEqual(raw.itemGeneration, .stable)
        XCTAssertEqual(raw.fileVersion, .unavailable)
        XCTAssertEqual(raw.shared, .`true`)
        XCTAssertEqual(raw.syncPaused, .unknown)
    }

    func testInvalidEnvelopeFailsBeforePathConsumptionOrMetadataRead() {
        for request in [
            StubICloudProbeRequest(version: 2),
            StubICloudProbeRequest(encoding: .windowsUtf16LittleEndian),
        ] {
            let reader = StubICloudFactReader(.success(facts()))
            XCTAssertEqual(
                MacOSICloudLocalCopyMetadataAdapter(reader: reader)
                    .readMetadata(request: request),
                .failed
            )
            XCTAssertEqual(request.takeCount, 0)
            XCTAssertTrue(reader.urls.isEmpty)
        }
    }

    func testChangedKindAndReaderFailureFailClosedAfterOneConsumption() {
        let changedRequest = StubICloudProbeRequest()
        let changedReader = StubICloudFactReader(.success(facts(itemKind: .symbolicLink)))
        XCTAssertEqual(
            MacOSICloudLocalCopyMetadataAdapter(reader: changedReader)
                .readMetadata(request: changedRequest),
            .failed
        )
        XCTAssertEqual(changedRequest.takeCount, 1)
        XCTAssertEqual(changedReader.urls.count, 1)

        let failedRequest = StubICloudProbeRequest()
        let failedReader = StubICloudFactReader(
            .failure(ExplorerICloudLocalCopyProbeError.failed)
        )
        XCTAssertEqual(
            MacOSICloudLocalCopyMetadataAdapter(reader: failedReader)
                .readMetadata(request: failedRequest),
            .failed
        )
        XCTAssertEqual(failedRequest.takeCount, 1)
        XCTAssertEqual(failedReader.urls.count, 1)
    }

    private func facts(
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
        downloadStatus: FoundationICloudDownloadStatus = .current,
        itemKind: FoundationICloudItemKind = .regularFile,
        identityCapability: FoundationICloudIdentityCapability =
            FoundationICloudIdentityCapability(
                containerState: .supported,
                accountTokenStability: .stable,
                itemGenerationStability: .stable,
                fileVersionPersistentIDStability: .stable
            )
    ) -> FoundationICloudLocalCopyFacts {
        FoundationICloudLocalCopyFacts(
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
            itemKind: itemKind,
            allocatedBytes: 4096,
            allocatedBytesSource: .file,
            identityCapability: identityCapability
        )
    }
}
