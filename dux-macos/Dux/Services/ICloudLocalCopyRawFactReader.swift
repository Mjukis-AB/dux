import Foundation

enum FoundationICloudDownloadStatus: Equatable, Sendable {
    case current
    case stale
    case notDownloaded
    case unknown
}

enum FoundationICloudItemKind: Equatable, Sendable {
    case regularFile
    case directory
    case symbolicLink
    case other
    case unknown
}

enum FoundationICloudAllocatedBytesSource: Equatable, Sendable {
    case file
    case totalFallback
}

enum FoundationICloudErrorPresence: Equatable, Sendable {
    case present
    case absent
    case unknown
}

struct FoundationICloudLocalCopyFacts: Equatable, Sendable {
    let isUbiquitous: Bool?
    let isUploaded: Bool?
    let isUploading: Bool?
    let hasUnresolvedConflicts: Bool?
    let isDownloading: Bool?
    let downloadRequested: Bool?
    let isExcludedFromSync: Bool?
    let uploadingErrorPresence: FoundationICloudErrorPresence
    let downloadingErrorPresence: FoundationICloudErrorPresence
    let downloadStatus: FoundationICloudDownloadStatus
    let itemKind: FoundationICloudItemKind
    let allocatedBytes: UInt64?
    let allocatedBytesSource: FoundationICloudAllocatedBytesSource?
}

protocol ICloudLocalCopyRawFactReading: Sendable {
    /// Reads current Foundation metadata synchronously. Callers must keep this
    /// filesystem operation off the main actor.
    func read(at url: URL) throws -> FoundationICloudLocalCopyFacts
}

/// Sendable projection of the requested `URLResourceValues`. Keeping this
/// intermediate injectable avoids requiring an iCloud account in unit tests.
struct FoundationICloudResourceValues: Sendable {
    let isUbiquitous: Bool?
    let isUploaded: Bool?
    let isUploading: Bool?
    let hasUnresolvedConflicts: Bool?
    let isDownloading: Bool?
    let downloadRequested: Bool?
    let isExcludedFromSync: Bool?
    let uploadingErrorPresence: FoundationICloudErrorPresence
    let downloadingErrorPresence: FoundationICloudErrorPresence
    let downloadStatus: URLUbiquitousItemDownloadingStatus?
    let isRegularFile: Bool?
    let isDirectory: Bool?
    let isSymbolicLink: Bool?
    let fileAllocatedSize: Int?
    let totalFileAllocatedSize: Int?
}

struct FoundationICloudLocalCopyRawFactReader: ICloudLocalCopyRawFactReading, Sendable {
    typealias LoadResourceValues = @Sendable (
        _ url: URL,
        _ keys: Set<URLResourceKey>
    ) throws -> FoundationICloudResourceValues

    static let requestedKeys: Set<URLResourceKey> = [
        .isUbiquitousItemKey,
        .ubiquitousItemIsUploadedKey,
        .ubiquitousItemIsUploadingKey,
        .ubiquitousItemHasUnresolvedConflictsKey,
        .ubiquitousItemIsDownloadingKey,
        .ubiquitousItemDownloadRequestedKey,
        .ubiquitousItemIsExcludedFromSyncKey,
        .ubiquitousItemUploadingErrorKey,
        .ubiquitousItemDownloadingErrorKey,
        .ubiquitousItemDownloadingStatusKey,
        .isRegularFileKey,
        .isDirectoryKey,
        .isSymbolicLinkKey,
        .fileAllocatedSizeKey,
        .totalFileAllocatedSizeKey,
    ]

    private let loadResourceValues: LoadResourceValues

    init(
        loadResourceValues: @escaping LoadResourceValues = Self.loadFoundationResourceValues
    ) {
        self.loadResourceValues = loadResourceValues
    }

    func read(at url: URL) throws -> FoundationICloudLocalCopyFacts {
        var freshURL = url
        freshURL.removeAllCachedResourceValues()
        let values = try loadResourceValues(freshURL, Self.requestedKeys)
        let allocation = Self.allocatedBytes(
            file: values.fileAllocatedSize,
            total: values.totalFileAllocatedSize
        )
        return FoundationICloudLocalCopyFacts(
            isUbiquitous: values.isUbiquitous,
            isUploaded: values.isUploaded,
            isUploading: values.isUploading,
            hasUnresolvedConflicts: values.hasUnresolvedConflicts,
            isDownloading: values.isDownloading,
            downloadRequested: values.downloadRequested,
            isExcludedFromSync: values.isExcludedFromSync,
            uploadingErrorPresence: values.uploadingErrorPresence,
            downloadingErrorPresence: values.downloadingErrorPresence,
            downloadStatus: Self.downloadStatus(values.downloadStatus),
            itemKind: Self.itemKind(
                isRegularFile: values.isRegularFile,
                isDirectory: values.isDirectory,
                isSymbolicLink: values.isSymbolicLink
            ),
            allocatedBytes: allocation.bytes,
            allocatedBytesSource: allocation.source
        )
    }

    private static func loadFoundationResourceValues(
        _ url: URL,
        _ keys: Set<URLResourceKey>
    ) throws -> FoundationICloudResourceValues {
        let values = try url.resourceValues(forKeys: keys)
        return FoundationICloudResourceValues(
            isUbiquitous: values.isUbiquitousItem,
            isUploaded: values.ubiquitousItemIsUploaded,
            isUploading: values.ubiquitousItemIsUploading,
            hasUnresolvedConflicts: values.ubiquitousItemHasUnresolvedConflicts,
            isDownloading: values.ubiquitousItemIsDownloading,
            downloadRequested: values.ubiquitousItemDownloadRequested,
            isExcludedFromSync: values.ubiquitousItemIsExcludedFromSync,
            uploadingErrorPresence: errorPresence(values.ubiquitousItemUploadingError),
            downloadingErrorPresence: errorPresence(values.ubiquitousItemDownloadingError),
            downloadStatus: values.ubiquitousItemDownloadingStatus,
            isRegularFile: values.isRegularFile,
            isDirectory: values.isDirectory,
            isSymbolicLink: values.isSymbolicLink,
            fileAllocatedSize: values.fileAllocatedSize,
            totalFileAllocatedSize: values.totalFileAllocatedSize
        )
    }

    private static func errorPresence(
        _ error: NSError?
    ) -> FoundationICloudErrorPresence {
        error == nil ? .absent : .present
    }

    private static func downloadStatus(
        _ status: URLUbiquitousItemDownloadingStatus?
    ) -> FoundationICloudDownloadStatus {
        switch status {
        case .current: .current
        case .downloaded: .stale
        case .notDownloaded: .notDownloaded
        default: .unknown
        }
    }

    private static func itemKind(
        isRegularFile: Bool?,
        isDirectory: Bool?,
        isSymbolicLink: Bool?
    ) -> FoundationICloudItemKind {
        let trueCount = [isRegularFile, isDirectory, isSymbolicLink].count { $0 == true }
        guard trueCount <= 1 else {
            return .unknown
        }
        if isSymbolicLink == true {
            return .symbolicLink
        }
        if isRegularFile == true {
            return .regularFile
        }
        if isDirectory == true {
            return .directory
        }
        return [isRegularFile, isDirectory, isSymbolicLink].allSatisfy { $0 != nil }
            ? .other
            : .unknown
    }

    private static func allocatedBytes(
        file: Int?,
        total: Int?
    ) -> (bytes: UInt64?, source: FoundationICloudAllocatedBytesSource?) {
        // File allocation excludes some metadata and is therefore the more
        // conservative reclaim estimate. Total allocation is only a fallback
        // when Foundation does not expose file allocation.
        if let file, let bytes = UInt64(exactly: file) {
            return (bytes, .file)
        }
        if let total, let bytes = UInt64(exactly: total) {
            return (bytes, .totalFallback)
        }
        return (nil, nil)
    }
}
