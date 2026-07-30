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

enum FoundationICloudIdentityContainerState: Equatable, Sendable {
    case supported
    case unavailable
    case unsupported
}

enum FoundationICloudIdentityStability: Equatable, Sendable {
    case stable
    case unavailable
    case changed
    case unsupported
}

/// Path-free capability evidence from two observations bracketing one
/// Foundation metadata read. Opaque identity archives never leave the reader.
struct FoundationICloudIdentityCapability: Equatable, Sendable {
    let containerState: FoundationICloudIdentityContainerState
    let accountTokenStability: FoundationICloudIdentityStability
    let itemGenerationStability: FoundationICloudIdentityStability
    let fileVersionPersistentIDStability: FoundationICloudIdentityStability
}

struct FoundationICloudLocalCopyFacts: Equatable, Sendable {
    let isUbiquitous: Bool?
    let isUploaded: Bool?
    let isUploading: Bool?
    let hasUnresolvedConflicts: Bool?
    let isDownloading: Bool?
    let downloadRequested: Bool?
    let isExcludedFromSync: Bool?
    let isShared: Bool?
    let isSyncPaused: Bool?
    let uploadingErrorPresence: FoundationICloudErrorPresence
    let downloadingErrorPresence: FoundationICloudErrorPresence
    let downloadStatus: FoundationICloudDownloadStatus
    let itemKind: FoundationICloudItemKind
    let allocatedBytes: UInt64?
    let allocatedBytesSource: FoundationICloudAllocatedBytesSource?
    let identityCapability: FoundationICloudIdentityCapability

    init(
        isUbiquitous: Bool?,
        isUploaded: Bool?,
        isUploading: Bool?,
        hasUnresolvedConflicts: Bool?,
        isDownloading: Bool?,
        downloadRequested: Bool?,
        isExcludedFromSync: Bool?,
        isShared: Bool? = nil,
        isSyncPaused: Bool? = nil,
        uploadingErrorPresence: FoundationICloudErrorPresence,
        downloadingErrorPresence: FoundationICloudErrorPresence,
        downloadStatus: FoundationICloudDownloadStatus,
        itemKind: FoundationICloudItemKind,
        allocatedBytes: UInt64?,
        allocatedBytesSource: FoundationICloudAllocatedBytesSource?,
        identityCapability: FoundationICloudIdentityCapability =
            FoundationICloudIdentityCapability(
                containerState: .unavailable,
                accountTokenStability: .unavailable,
                itemGenerationStability: .unavailable,
                fileVersionPersistentIDStability: .unavailable
            )
    ) {
        self.isUbiquitous = isUbiquitous
        self.isUploaded = isUploaded
        self.isUploading = isUploading
        self.hasUnresolvedConflicts = hasUnresolvedConflicts
        self.isDownloading = isDownloading
        self.downloadRequested = downloadRequested
        self.isExcludedFromSync = isExcludedFromSync
        self.isShared = isShared
        self.isSyncPaused = isSyncPaused
        self.uploadingErrorPresence = uploadingErrorPresence
        self.downloadingErrorPresence = downloadingErrorPresence
        self.downloadStatus = downloadStatus
        self.itemKind = itemKind
        self.allocatedBytes = allocatedBytes
        self.allocatedBytesSource = allocatedBytesSource
        self.identityCapability = identityCapability
    }
}

enum FoundationICloudLocalCopyRawFactReadError: Error, Equatable, Sendable {
    case liveFactsChanged
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
    let isShared: Bool?
    let isSyncPaused: Bool?
    let uploadingErrorPresence: FoundationICloudErrorPresence
    let downloadingErrorPresence: FoundationICloudErrorPresence
    let downloadStatus: URLUbiquitousItemDownloadingStatus?
    let isRegularFile: Bool?
    let isDirectory: Bool?
    let isSymbolicLink: Bool?
    let fileAllocatedSize: Int?
    let totalFileAllocatedSize: Int?
    let generationIdentifierArchive: Data?
}

struct FoundationICloudLocalCopyRawFactReader: ICloudLocalCopyRawFactReading, Sendable {
    typealias LoadAccountIdentity = @Sendable () throws -> Data?
    typealias LoadResourceValues = @Sendable (
        _ url: URL,
        _ keys: Set<URLResourceKey>
    ) throws -> FoundationICloudResourceValues
    typealias LoadFileVersionIdentity = @Sendable (_ url: URL) throws -> Data?

    static let maximumIdentityArchiveBytes = 4 * 1024

    static var requestedKeys: Set<URLResourceKey> {
        var keys: Set<URLResourceKey> = [
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
            .ubiquitousItemIsSharedKey,
            .generationIdentifierKey,
            .isRegularFileKey,
            .isDirectoryKey,
            .isSymbolicLinkKey,
            .fileAllocatedSizeKey,
            .totalFileAllocatedSizeKey,
        ]
        if #available(macOS 26.0, *) {
            keys.insert(.ubiquitousItemIsSyncPausedKey)
        }
        return keys
    }

    private let loadAccountIdentity: LoadAccountIdentity
    private let loadResourceValues: LoadResourceValues
    private let loadFileVersionIdentity: LoadFileVersionIdentity

    init(
        loadAccountIdentity: @escaping LoadAccountIdentity =
            Self.loadFoundationAccountIdentity,
        loadFileVersionIdentity: @escaping LoadFileVersionIdentity =
            Self.loadFoundationFileVersionIdentity,
        loadResourceValues: @escaping LoadResourceValues = Self.loadFoundationResourceValues
    ) {
        self.loadAccountIdentity = loadAccountIdentity
        self.loadResourceValues = loadResourceValues
        self.loadFileVersionIdentity = loadFileVersionIdentity
    }

    func read(at url: URL) throws -> FoundationICloudLocalCopyFacts {
        var freshURL = url
        let accountA = try loadAccountIdentity()
        freshURL.removeAllCachedResourceValues()
        let valuesA = try loadResourceValues(freshURL, Self.requestedKeys)
        let fileVersionA = try loadFileVersionIdentity(freshURL)
        freshURL.removeAllCachedResourceValues()
        let valuesB = try loadResourceValues(freshURL, Self.requestedKeys)
        let fileVersionB = try loadFileVersionIdentity(freshURL)
        let accountB = try loadAccountIdentity()

        guard Self.haveEqualLiveFacts(valuesA, valuesB) else {
            throw FoundationICloudLocalCopyRawFactReadError.liveFactsChanged
        }

        let allocation = Self.allocatedBytes(
            file: valuesA.fileAllocatedSize,
            total: valuesA.totalFileAllocatedSize
        )
        let containerState = Self.containerState(valuesA.isUbiquitous)
        return FoundationICloudLocalCopyFacts(
            isUbiquitous: valuesA.isUbiquitous,
            isUploaded: valuesA.isUploaded,
            isUploading: valuesA.isUploading,
            hasUnresolvedConflicts: valuesA.hasUnresolvedConflicts,
            isDownloading: valuesA.isDownloading,
            downloadRequested: valuesA.downloadRequested,
            isExcludedFromSync: valuesA.isExcludedFromSync,
            isShared: valuesA.isShared,
            isSyncPaused: valuesA.isSyncPaused,
            uploadingErrorPresence: valuesA.uploadingErrorPresence,
            downloadingErrorPresence: valuesA.downloadingErrorPresence,
            downloadStatus: Self.downloadStatus(valuesA.downloadStatus),
            itemKind: Self.itemKind(
                isRegularFile: valuesA.isRegularFile,
                isDirectory: valuesA.isDirectory,
                isSymbolicLink: valuesA.isSymbolicLink
            ),
            allocatedBytes: allocation.bytes,
            allocatedBytesSource: allocation.source,
            identityCapability: FoundationICloudIdentityCapability(
                containerState: containerState,
                accountTokenStability: Self.identityStability(
                    accountA,
                    accountB,
                    containerState: containerState
                ),
                itemGenerationStability: Self.identityStability(
                    valuesA.generationIdentifierArchive,
                    valuesB.generationIdentifierArchive,
                    containerState: containerState
                ),
                fileVersionPersistentIDStability: Self.identityStability(
                    fileVersionA,
                    fileVersionB,
                    containerState: containerState
                )
            )
        )
    }

    private static func loadFoundationAccountIdentity() throws -> Data? {
        try boundedArchive(FileManager.default.ubiquityIdentityToken)
    }

    private static func loadFoundationFileVersionIdentity(
        _ url: URL
    ) throws -> Data? {
        try boundedArchive(
            NSFileVersion.currentVersionOfItem(at: url)?.persistentIdentifier
        )
    }

    private static func loadFoundationResourceValues(
        _ url: URL,
        _ keys: Set<URLResourceKey>
    ) throws -> FoundationICloudResourceValues {
        let values = try url.resourceValues(forKeys: keys)
        let isSyncPaused: Bool? = if #available(macOS 26.0, *) {
            values.ubiquitousItemIsSyncPaused
        } else {
            nil
        }
        return try FoundationICloudResourceValues(
            isUbiquitous: values.isUbiquitousItem,
            isUploaded: values.ubiquitousItemIsUploaded,
            isUploading: values.ubiquitousItemIsUploading,
            hasUnresolvedConflicts: values.ubiquitousItemHasUnresolvedConflicts,
            isDownloading: values.ubiquitousItemIsDownloading,
            downloadRequested: values.ubiquitousItemDownloadRequested,
            isExcludedFromSync: values.ubiquitousItemIsExcludedFromSync,
            isShared: values.ubiquitousItemIsShared,
            isSyncPaused: isSyncPaused,
            uploadingErrorPresence: errorPresence(values.ubiquitousItemUploadingError),
            downloadingErrorPresence: errorPresence(values.ubiquitousItemDownloadingError),
            downloadStatus: values.ubiquitousItemDownloadingStatus,
            isRegularFile: values.isRegularFile,
            isDirectory: values.isDirectory,
            isSymbolicLink: values.isSymbolicLink,
            fileAllocatedSize: values.fileAllocatedSize,
            totalFileAllocatedSize: values.totalFileAllocatedSize,
            generationIdentifierArchive: boundedArchive(values.generationIdentifier)
        )
    }

    private static func boundedArchive(_ value: Any?) throws -> Data? {
        guard let value else {
            return nil
        }
        let archive = try NSKeyedArchiver.archivedData(
            withRootObject: value,
            requiringSecureCoding: false
        )
        guard archive.count <= maximumIdentityArchiveBytes else {
            return nil
        }
        return archive
    }

    private static func containerState(
        _ isUbiquitous: Bool?
    ) -> FoundationICloudIdentityContainerState {
        switch isUbiquitous {
        case true: .supported
        case false: .unsupported
        case nil: .unavailable
        }
    }

    private static func identityStability(
        _ first: Data?,
        _ second: Data?,
        containerState: FoundationICloudIdentityContainerState
    ) -> FoundationICloudIdentityStability {
        switch containerState {
        case .unsupported:
            return .unsupported
        case .unavailable:
            return .unavailable
        case .supported:
            guard
                let first,
                let second,
                first.count <= maximumIdentityArchiveBytes,
                second.count <= maximumIdentityArchiveBytes
            else {
                return .unavailable
            }
            // Equality is meaningful only inside this bracketed read. Apple
            // does not document keyed archives as canonical durable identity.
            return first == second ? .stable : .changed
        }
    }

    private static func haveEqualLiveFacts(
        _ first: FoundationICloudResourceValues,
        _ second: FoundationICloudResourceValues
    ) -> Bool {
        first.isUbiquitous == second.isUbiquitous
            && first.isUploaded == second.isUploaded
            && first.isUploading == second.isUploading
            && first.hasUnresolvedConflicts == second.hasUnresolvedConflicts
            && first.isDownloading == second.isDownloading
            && first.downloadRequested == second.downloadRequested
            && first.isExcludedFromSync == second.isExcludedFromSync
            && first.isShared == second.isShared
            && first.isSyncPaused == second.isSyncPaused
            && first.uploadingErrorPresence == second.uploadingErrorPresence
            && first.downloadingErrorPresence == second.downloadingErrorPresence
            && first.downloadStatus?.rawValue == second.downloadStatus?.rawValue
            && first.isRegularFile == second.isRegularFile
            && first.isDirectory == second.isDirectory
            && first.isSymbolicLink == second.isSymbolicLink
            && first.fileAllocatedSize == second.fileAllocatedSize
            && first.totalFileAllocatedSize == second.totalFileAllocatedSize
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
