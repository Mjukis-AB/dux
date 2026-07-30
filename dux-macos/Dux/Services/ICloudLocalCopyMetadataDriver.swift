import Foundation

/// Testable projection of the generated consume-once request. Production
/// receives only Rust-issued request objects.
protocol ICloudLocalCopyProbeRequestReading: Sendable {
    func recordVersion() throws -> UInt32
    func pathEncoding() throws -> SnapshotNameEncoding
    func takePathBytes() throws -> Data
}

extension ICloudLocalCopyProbeRequest: ICloudLocalCopyProbeRequestReading {}

struct MacOSICloudLocalCopyMetadataAdapter: Sendable {
    private let reader: any ICloudLocalCopyRawFactReading

    init(
        reader: any ICloudLocalCopyRawFactReading = FoundationICloudLocalCopyRawFactReader()
    ) {
        self.reader = reader
    }

    /// Consume one Rust-selected path, read Foundation metadata once, and
    /// return only bounded raw facts. No path or eligibility decision returns.
    func readMetadata(
        request: any ICloudLocalCopyProbeRequestReading
    ) -> ICloudLocalCopyMetadataResult {
        do {
            guard
                try request.recordVersion() == 1,
                try request.pathEncoding() == .unixBytes
            else {
                return .failed
            }
            let bytes = [UInt8](try request.takePathBytes())
            let url = try ExplorerSnapshotLivePathAdapter.url(
                fromValidatedUnixPathBytes: bytes,
                isDirectory: false
            )
            let facts = try reader.read(at: url)
            guard facts.itemKind == .regularFile else {
                return .failed
            }
            return .observed(
                facts: ICloudLocalCopyRawFacts(
                    recordVersion: 1,
                    ubiquitous: boolean(facts.isUbiquitous),
                    uploaded: boolean(facts.isUploaded),
                    uploading: boolean(facts.isUploading),
                    uploadError: transferError(facts.uploadingErrorPresence),
                    unresolvedConflicts: boolean(facts.hasUnresolvedConflicts),
                    localCopyState: localCopy(facts.downloadStatus),
                    downloadRequested: boolean(facts.downloadRequested),
                    downloading: boolean(facts.isDownloading),
                    downloadError: transferError(facts.downloadingErrorPresence),
                    excludedFromSync: boolean(facts.isExcludedFromSync)
                )
            )
        } catch {
            return .failed
        }
    }

    private func boolean(_ value: Bool?) -> ICloudBooleanState {
        switch value {
        case true: .`true`
        case false: .`false`
        case nil: .unknown
        }
    }

    private func transferError(
        _ value: FoundationICloudErrorPresence
    ) -> ICloudErrorState {
        switch value {
        case .absent: .absent
        case .present: .present
        case .unknown: .unknown
        }
    }

    private func localCopy(
        _ value: FoundationICloudDownloadStatus
    ) -> ICloudLocalCopyState {
        switch value {
        case .current: .current
        case .stale: .stale
        case .notDownloaded: .notDownloaded
        case .unknown: .unknown
        }
    }
}

/// Generated UniFFI callback implementation. The call remains synchronous on
/// EngineService's dedicated utility queue.
final class MacOSICloudLocalCopyMetadataDriver:
    ICloudLocalCopyMetadataDriver, @unchecked Sendable
{
    private let adapter: MacOSICloudLocalCopyMetadataAdapter

    init(
        adapter: MacOSICloudLocalCopyMetadataAdapter = MacOSICloudLocalCopyMetadataAdapter()
    ) {
        self.adapter = adapter
    }

    func readMetadata(
        request: ICloudLocalCopyProbeRequest
    ) -> ICloudLocalCopyMetadataResult {
        adapter.readMetadata(request: request)
    }
}
