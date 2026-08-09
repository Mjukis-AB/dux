import Dispatch
import FileProvider
import Foundation
#if DUX_ICLOUD_READ_ONLY_QUALIFICATION
import CryptoKit
#endif

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

enum FoundationICloudIdentityStability: Equatable, Sendable {
    case stable
    case unavailable
    case changed
    case unsupported
}

/// Path-free capability evidence from two observations bracketing one
/// Foundation metadata read. Opaque identity archives never leave the reader.
struct FoundationICloudIdentityCapability: Equatable, Sendable {
    let accountTokenStability: FoundationICloudIdentityStability
    let domainIdentifierStability: FoundationICloudIdentityStability
    let providerItemIdentifierStability: FoundationICloudIdentityStability
    let itemGenerationStability: FoundationICloudIdentityStability
    let fileVersionPersistentIDStability: FoundationICloudIdentityStability
}

/// Opaque File Provider identifiers observed only inside the bracketed reader.
/// Raw values are never returned through the adapter, persisted, logged, or
/// used as cleanup authority.
struct FoundationICloudFileProviderIdentity: Equatable, Sendable {
    let domainIdentifier: String
    let itemIdentifier: String
}

private final class FoundationICloudFileProviderIdentityBox: @unchecked Sendable {
    private let lock = NSLock()
    private var value: FoundationICloudFileProviderIdentity?

    func store(_ value: FoundationICloudFileProviderIdentity?) {
        lock.withLock {
            self.value = value
        }
    }

    func load() -> FoundationICloudFileProviderIdentity? {
        lock.withLock { value }
    }
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
                accountTokenStability: .unavailable,
                domainIdentifierStability: .unavailable,
                providerItemIdentifierStability: .unavailable,
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
#if DUX_ICLOUD_READ_ONLY_QUALIFICATION
    case invalidQualificationKey
#endif
}

#if DUX_ICLOUD_READ_ONLY_QUALIFICATION
/// A comparison result safe for the redacted qualification evidence report.
/// Neither raw identities nor keyed tags are represented by this type.
enum FoundationICloudQualificationContinuity: String, Codable, Equatable, Sendable {
    case sameAsBaseline
    case changedSinceBaseline
    case changedDuringRead
    case unavailable
    case unsupported
    case noBaseline
}

struct FoundationICloudQualificationIdentityContinuity: Codable, Equatable, Sendable {
    let accountToken: FoundationICloudQualificationContinuity
    let fileProviderDomain: FoundationICloudQualificationContinuity
    let fileProviderItem: FoundationICloudQualificationContinuity
    let itemGeneration: FoundationICloudQualificationContinuity
    let fileVersion: FoundationICloudQualificationContinuity
}

/// Private qualification state. It contains only HMAC-SHA256 tags made inside
/// the production reader; raw identity values never cross the reader boundary.
/// This value must never be included in a qualification evidence report.
struct FoundationICloudQualificationIdentityReference: Codable, Equatable, Sendable {
    fileprivate let accountTokenTag: Data?
    fileprivate let fileProviderDomainTag: Data?
    fileprivate let fileProviderItemTag: Data?
    fileprivate let itemGenerationTag: Data?
    fileprivate let fileVersionTag: Data?

    var isComplete: Bool {
        accountTokenTag != nil
            && fileProviderDomainTag != nil
            && fileProviderItemTag != nil
            && itemGenerationTag != nil
            && fileVersionTag != nil
    }
}

struct FoundationICloudQualificationRead: Equatable, Sendable {
    let facts: FoundationICloudLocalCopyFacts
    let privateIdentityReference: FoundationICloudQualificationIdentityReference
    let continuity: FoundationICloudQualificationIdentityContinuity
}
#endif

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
    typealias LoadFileProviderIdentity = @Sendable (
        _ url: URL
    ) -> FoundationICloudFileProviderIdentity?

    static let maximumIdentityArchiveBytes = 4 * 1024
    static let maximumFileProviderIdentifierBytes = 4 * 1024
    static let fileProviderIdentityTimeout: DispatchTimeInterval = .seconds(5)

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
    private let loadFileProviderIdentity: LoadFileProviderIdentity

    private struct BracketedRead {
        let accountA: Data?
        let accountB: Data?
        let fileProviderIdentityA: FoundationICloudFileProviderIdentity?
        let fileProviderIdentityB: FoundationICloudFileProviderIdentity?
        let valuesA: FoundationICloudResourceValues
        let valuesB: FoundationICloudResourceValues
        let fileVersionA: Data?
        let fileVersionB: Data?
    }

    init(
        loadAccountIdentity: @escaping LoadAccountIdentity =
            Self.loadFoundationAccountIdentity,
        loadFileVersionIdentity: @escaping LoadFileVersionIdentity =
            Self.loadFoundationFileVersionIdentity,
        loadFileProviderIdentity: @escaping LoadFileProviderIdentity =
            Self.loadFoundationFileProviderIdentity,
        loadResourceValues: @escaping LoadResourceValues = Self.loadFoundationResourceValues
    ) {
        self.loadAccountIdentity = loadAccountIdentity
        self.loadResourceValues = loadResourceValues
        self.loadFileVersionIdentity = loadFileVersionIdentity
        self.loadFileProviderIdentity = loadFileProviderIdentity
    }

    func read(at url: URL) throws -> FoundationICloudLocalCopyFacts {
        Self.facts(from: try bracketedRead(at: url))
    }

#if DUX_ICLOUD_READ_ONLY_QUALIFICATION
    /// Performs the same bracketed read as production and derives only keyed,
    /// domain-separated HMAC tags for private cross-phase comparison state.
    /// The key must be exactly 32 bytes and is rejected before any URL access.
    func readForQualification(
        at url: URL,
        key: Data,
        baseline: FoundationICloudQualificationIdentityReference?
    ) throws -> FoundationICloudQualificationRead {
        guard key.count == 32 else {
            throw FoundationICloudLocalCopyRawFactReadError.invalidQualificationKey
        }
        let observation = try bracketedRead(at: url)
        let facts = Self.facts(from: observation)
        let current = Self.qualificationReference(
            from: observation,
            key: SymmetricKey(data: key)
        )
        return FoundationICloudQualificationRead(
            facts: facts,
            privateIdentityReference: current,
            continuity: Self.qualificationContinuity(
                facts: facts,
                current: current,
                baseline: baseline
            )
        )
    }
#endif

    private func bracketedRead(at url: URL) throws -> BracketedRead {
        var freshURL = url
        let accountA = try loadAccountIdentity()
        let fileProviderIdentityA = loadFileProviderIdentity(freshURL)
        freshURL.removeAllCachedResourceValues()
        let valuesA = try loadResourceValues(freshURL, Self.requestedKeys)
        let fileVersionA = try loadFileVersionIdentity(freshURL)
        freshURL.removeAllCachedResourceValues()
        let valuesB = try loadResourceValues(freshURL, Self.requestedKeys)
        let fileVersionB = try loadFileVersionIdentity(freshURL)
        let fileProviderIdentityB = loadFileProviderIdentity(freshURL)
        let accountB = try loadAccountIdentity()

        guard Self.haveEqualLiveFacts(valuesA, valuesB) else {
            throw FoundationICloudLocalCopyRawFactReadError.liveFactsChanged
        }

        return BracketedRead(
            accountA: accountA,
            accountB: accountB,
            fileProviderIdentityA: fileProviderIdentityA,
            fileProviderIdentityB: fileProviderIdentityB,
            valuesA: valuesA,
            valuesB: valuesB,
            fileVersionA: fileVersionA,
            fileVersionB: fileVersionB
        )
    }

    private static func facts(from observation: BracketedRead) -> FoundationICloudLocalCopyFacts {
        let valuesA = observation.valuesA
        let allocation = Self.allocatedBytes(
            file: valuesA.fileAllocatedSize,
            total: valuesA.totalFileAllocatedSize
        )
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
                accountTokenStability: Self.identityStability(
                    observation.accountA,
                    observation.accountB,
                    isUbiquitous: valuesA.isUbiquitous
                ),
                domainIdentifierStability: Self.identityStability(
                    Self.boundedFileProviderIdentifier(
                        observation.fileProviderIdentityA?.domainIdentifier
                    ),
                    Self.boundedFileProviderIdentifier(
                        observation.fileProviderIdentityB?.domainIdentifier
                    ),
                    isUbiquitous: valuesA.isUbiquitous
                ),
                providerItemIdentifierStability: Self.identityStability(
                    Self.boundedFileProviderIdentifier(
                        observation.fileProviderIdentityA?.itemIdentifier
                    ),
                    Self.boundedFileProviderIdentifier(
                        observation.fileProviderIdentityB?.itemIdentifier
                    ),
                    isUbiquitous: valuesA.isUbiquitous
                ),
                itemGenerationStability: Self.identityStability(
                    valuesA.generationIdentifierArchive,
                    observation.valuesB.generationIdentifierArchive,
                    isUbiquitous: valuesA.isUbiquitous
                ),
                fileVersionPersistentIDStability: Self.identityStability(
                    observation.fileVersionA,
                    observation.fileVersionB,
                    isUbiquitous: valuesA.isUbiquitous
                )
            )
        )
    }

#if DUX_ICLOUD_READ_ONLY_QUALIFICATION
    private enum QualificationIdentitySlot: String {
        case accountToken = "account-token"
        case fileProviderDomain = "file-provider-domain"
        case fileProviderItem = "file-provider-item"
        case itemGeneration = "item-generation"
        case fileVersion = "file-version"
    }

    private static func qualificationReference(
        from observation: BracketedRead,
        key: SymmetricKey
    ) -> FoundationICloudQualificationIdentityReference {
        let isUbiquitous = observation.valuesA.isUbiquitous
        return FoundationICloudQualificationIdentityReference(
            accountTokenTag: qualificationTag(
                first: observation.accountA,
                second: observation.accountB,
                isUbiquitous: isUbiquitous,
                slot: .accountToken,
                key: key
            ),
            fileProviderDomainTag: qualificationTag(
                first: boundedFileProviderIdentifier(
                    observation.fileProviderIdentityA?.domainIdentifier
                ),
                second: boundedFileProviderIdentifier(
                    observation.fileProviderIdentityB?.domainIdentifier
                ),
                isUbiquitous: isUbiquitous,
                slot: .fileProviderDomain,
                key: key
            ),
            fileProviderItemTag: qualificationTag(
                first: boundedFileProviderIdentifier(
                    observation.fileProviderIdentityA?.itemIdentifier
                ),
                second: boundedFileProviderIdentifier(
                    observation.fileProviderIdentityB?.itemIdentifier
                ),
                isUbiquitous: isUbiquitous,
                slot: .fileProviderItem,
                key: key
            ),
            itemGenerationTag: qualificationTag(
                first: observation.valuesA.generationIdentifierArchive,
                second: observation.valuesB.generationIdentifierArchive,
                isUbiquitous: isUbiquitous,
                slot: .itemGeneration,
                key: key
            ),
            fileVersionTag: qualificationTag(
                first: observation.fileVersionA,
                second: observation.fileVersionB,
                isUbiquitous: isUbiquitous,
                slot: .fileVersion,
                key: key
            )
        )
    }

    private static func qualificationTag(
        first: Data?,
        second: Data?,
        isUbiquitous: Bool?,
        slot: QualificationIdentitySlot,
        key: SymmetricKey
    ) -> Data? {
        guard identityStability(first, second, isUbiquitous: isUbiquitous) == .stable,
              let first
        else {
            return nil
        }
        var message = Data("dux-icloud-read-only-qualification-v1\0".utf8)
        message.append(contentsOf: slot.rawValue.utf8)
        message.append(0)
        message.append(first)
        return Data(HMAC<SHA256>.authenticationCode(for: message, using: key))
    }

    private static func qualificationContinuity(
        facts: FoundationICloudLocalCopyFacts,
        current: FoundationICloudQualificationIdentityReference,
        baseline: FoundationICloudQualificationIdentityReference?
    ) -> FoundationICloudQualificationIdentityContinuity {
        FoundationICloudQualificationIdentityContinuity(
            accountToken: qualificationContinuity(
                stability: facts.identityCapability.accountTokenStability,
                current: current.accountTokenTag,
                baseline: baseline?.accountTokenTag
            ),
            fileProviderDomain: qualificationContinuity(
                stability: facts.identityCapability.domainIdentifierStability,
                current: current.fileProviderDomainTag,
                baseline: baseline?.fileProviderDomainTag
            ),
            fileProviderItem: qualificationContinuity(
                stability: facts.identityCapability.providerItemIdentifierStability,
                current: current.fileProviderItemTag,
                baseline: baseline?.fileProviderItemTag
            ),
            itemGeneration: qualificationContinuity(
                stability: facts.identityCapability.itemGenerationStability,
                current: current.itemGenerationTag,
                baseline: baseline?.itemGenerationTag
            ),
            fileVersion: qualificationContinuity(
                stability: facts.identityCapability.fileVersionPersistentIDStability,
                current: current.fileVersionTag,
                baseline: baseline?.fileVersionTag
            )
        )
    }

    private static func qualificationContinuity(
        stability: FoundationICloudIdentityStability,
        current: Data?,
        baseline: Data?
    ) -> FoundationICloudQualificationContinuity {
        switch stability {
        case .changed:
            return .changedDuringRead
        case .unavailable:
            return .unavailable
        case .unsupported:
            return .unsupported
        case .stable:
            guard let current, let baseline else {
                return .noBaseline
            }
            return current == baseline ? .sameAsBaseline : .changedSinceBaseline
        }
    }
#endif

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

    private static func loadFoundationFileProviderIdentity(
        _ url: URL
    ) -> FoundationICloudFileProviderIdentity? {
        let result = FoundationICloudFileProviderIdentityBox()
        let semaphore = DispatchSemaphore(value: 0)
        NSFileProviderManager.getIdentifierForUserVisibleFile(at: url) {
            itemIdentifier,
            domainIdentifier,
            error in
            defer { semaphore.signal() }
            guard
                error == nil,
                let itemIdentifier,
                let domainIdentifier
            else {
                result.store(nil)
                return
            }
            result.store(FoundationICloudFileProviderIdentity(
                domainIdentifier: domainIdentifier.rawValue,
                itemIdentifier: itemIdentifier.rawValue
            ))
        }
        guard semaphore.wait(timeout: .now() + fileProviderIdentityTimeout) == .success else {
            return nil
        }
        return result.load()
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

    private static func identityStability(
        _ first: Data?,
        _ second: Data?,
        isUbiquitous: Bool?
    ) -> FoundationICloudIdentityStability {
        switch isUbiquitous {
        case false:
            return .unsupported
        case nil:
            return .unavailable
        case true:
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

    private static func boundedFileProviderIdentifier(
        _ value: String?
    ) -> Data? {
        guard let value, !value.isEmpty else {
            return nil
        }
        let bytes = Data(value.utf8)
        guard bytes.count <= maximumFileProviderIdentifierBytes else {
            return nil
        }
        return bytes
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
