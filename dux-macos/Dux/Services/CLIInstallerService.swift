import CryptoKit
import Darwin
import Foundation
import Security

actor CLIInstallerService: CLIInstallerServing {
    private let platform: any CLIInstallerPlatform
    private var pending: CLIInstallerPendingConfirmation?
    private var isClosed = false

    init(platform: any CLIInstallerPlatform = DarwinCLIInstallerPlatform()) {
        self.platform = platform
    }

    func loadStatus() async throws -> CLIInstallationStatus {
        guard !isClosed else {
            throw CLIInstallerServiceError.confirmationUnavailable
        }
        return try platform.observe().status
    }

    func prepare(
        _ action: CLIInstallationAction
    ) async throws -> CLIInstallationConfirmation {
        guard !isClosed else {
            throw CLIInstallerServiceError.confirmationUnavailable
        }
        guard pending == nil else {
            throw CLIInstallerServiceError.busy
        }
        let observation = try platform.observe()
        guard Self.action(action, isAvailableFor: observation.status.disposition) else {
            throw Self.unavailableActionError(
                action,
                disposition: observation.status.disposition
            )
        }
        let confirmation = CLIInstallationConfirmation(
            token: UUID(),
            action: action,
            bundledVersion: observation.status.bundled.version,
            installedVersion: observation.status.disposition.managedVersion,
            destinationDisplayText: CLIInstallationStatus.destinationDisplayText
        )
        pending = CLIInstallerPendingConfirmation(
            confirmation: confirmation,
            observation: observation
        )
        return confirmation
    }

    func perform(
        _ confirmation: CLIInstallationConfirmation
    ) async throws -> CLIInstallationUpdate {
        guard !isClosed else {
            throw CLIInstallerServiceError.confirmationUnavailable
        }
        guard
            let pending,
            pending.confirmation == confirmation
        else {
            throw CLIInstallerServiceError.confirmationUnavailable
        }
        // Every confirmed mutation is one-shot, including uncertain outcomes.
        self.pending = nil
        let observation: CLIInstallerObservedState = switch confirmation.action {
        case .install, .upgrade, .reinstall:
            try platform.install(expected: pending.observation)
        case .uninstall:
            try platform.uninstall(expected: pending.observation)
        }
        return CLIInstallationUpdate(status: observation.status, changed: true)
    }

    func discard(_ confirmation: CLIInstallationConfirmation) async {
        guard pending?.confirmation == confirmation else {
            return
        }
        pending = nil
    }

    func close() async {
        isClosed = true
        pending = nil
    }

    private static func action(
        _ action: CLIInstallationAction,
        isAvailableFor disposition: CLIInstallationDisposition
    ) -> Bool {
        switch (action, disposition) {
        case (.install, .absent):
            true
        case let (.upgrade, .managed(installed)):
            installed.relation == .older
        case let (.reinstall, .managed(installed)):
            installed.relation == .current
                || installed.relation == .sameVersionDifferentBuild
        case (.uninstall, .managed):
            true
        case (.install, _), (.upgrade, _), (.reinstall, _), (.uninstall, _):
            false
        }
    }

    private static func unavailableActionError(
        _ action: CLIInstallationAction,
        disposition: CLIInstallationDisposition
    ) -> CLIInstallerServiceError {
        switch disposition {
        case .unmanaged:
            .unmanagedDestination
        case let .unsafe(reason):
            .unsafeDestination(reason)
        case let .managed(installed)
            where action != .uninstall && installed.relation == .newer:
            .downgradeRefused
        case .absent, .managed:
            .confirmationChanged
        }
    }
}

private extension CLIInstallationDisposition {
    var managedVersion: CLIVersion? {
        guard case let .managed(installed) = self else {
            return nil
        }
        return installed.version
    }
}

struct CLIInstallerPendingConfirmation: Equatable, Sendable {
    let confirmation: CLIInstallationConfirmation
    let observation: CLIInstallerObservedState
}

protocol CLIInstallerPlatform: Sendable {
    func observe() throws -> CLIInstallerObservedState
    func install(expected: CLIInstallerObservedState) throws -> CLIInstallerObservedState
    func uninstall(expected: CLIInstallerObservedState) throws -> CLIInstallerObservedState
}

struct CLIInstallerObservedState: Equatable, Sendable {
    let status: CLIInstallationStatus
    let bundledEvidence: CLIInstallerBundledEvidence
    let targetEvidence: CLIInstallerTargetEvidence
}

struct CLIInstallerBundledEvidence: Equatable, Sendable {
    let file: CLIInstallerFileEvidence
    let metadata: CLIBundledMetadata
    let signature: CLIStaticSignatureIdentity
}

enum CLIInstallerTargetEvidence: Equatable, Sendable {
    case absent
    case managed(
        file: CLIInstallerFileEvidence,
        marker: CLIInstallerManagedMarker,
        signature: CLIStaticSignatureIdentity
    )
    case unmanaged(CLIInstallerFileEvidence)
    case unsafe(CLIUnsafeTargetReason, CLIInstallerFileEvidence?)
}

struct CLIInstallerFileEvidence: Equatable, Sendable {
    let device: UInt64
    let inode: UInt64
    let size: UInt64
    let modificationSeconds: Int64
    let modificationNanoseconds: Int64
    let mode: UInt16
    let owner: UInt32
    let linkCount: UInt64
    let sha256: Data?
}

struct CLIInstallerManagedMarker: Equatable, Sendable {
    static let recordVersion: UInt32 = 1
    static let product = "dux-cli"
    static let source = "dux-macos-app"

    let version: CLIVersion
    let sha256: Data
}

protocol CLIInstallerResourceProviding: Sendable {
    func bundledBinaryURL() -> URL?
    func bundledMetadataURL() -> URL?
    func runningApplicationURL() -> URL
}

struct BundleCLIInstallerResources: CLIInstallerResourceProviding, @unchecked Sendable {
    private let bundle: Bundle

    init(bundle: Bundle = .main) {
        self.bundle = bundle
    }

    func bundledBinaryURL() -> URL? {
        bundle.url(forResource: "dux-cli-bundled", withExtension: nil)
    }

    func bundledMetadataURL() -> URL? {
        bundle.url(
            forResource: "dux-cli-bundled-metadata",
            withExtension: "json"
        )
    }

    func runningApplicationURL() -> URL {
        bundle.bundleURL
    }
}

protocol CLIInstallerAccountProviding: Sendable {
    func currentHomeDirectory() throws -> URL
    func currentUserID() -> uid_t
    func inheritedPath() -> String?
}

struct SystemCLIInstallerAccount: CLIInstallerAccountProviding {
    func currentHomeDirectory() throws -> URL {
        let userID = getuid()
        let initialCapacity = max(Int(sysconf(_SC_GETPW_R_SIZE_MAX)), 4096)
        guard initialCapacity <= 1_048_576 else {
            throw CLIInstallerServiceError.unsupportedAccount
        }
        var buffer = [CChar](repeating: 0, count: initialCapacity)
        var record = passwd()
        var result: UnsafeMutablePointer<passwd>?
        let status = getpwuid_r(
            userID,
            &record,
            &buffer,
            buffer.count,
            &result
        )
        guard
            status == 0,
            result != nil,
            let directory = record.pw_dir,
            let path = String(validatingCString: directory),
            path.utf8.count <= 4096,
            path.hasPrefix("/")
        else {
            throw CLIInstallerServiceError.unsupportedAccount
        }
        return URL(fileURLWithPath: path, isDirectory: true)
    }

    func currentUserID() -> uid_t {
        getuid()
    }

    func inheritedPath() -> String? {
        ProcessInfo.processInfo.environment["PATH"]
    }
}

protocol CLIStaticSignatureInspecting: Sendable {
    func inspect(at url: URL) throws -> CLIStaticSignatureIdentity
}

struct SystemCLIStaticSignatureInspector: CLIStaticSignatureInspecting {
    func inspect(at url: URL) throws -> CLIStaticSignatureIdentity {
        var staticCode: SecStaticCode?
        guard
            SecStaticCodeCreateWithPath(
                url as CFURL,
                SecCSFlags(),
                &staticCode
            ) == errSecSuccess,
            let staticCode
        else {
            throw CLIInstallerServiceError.invalidCodeSignature
        }
        let validationFlags = SecCSFlags(
            rawValue: UInt32(kSecCSStrictValidate) | UInt32(kSecCSCheckAllArchitectures)
        )
        guard SecStaticCodeCheckValidity(staticCode, validationFlags, nil) == errSecSuccess else {
            throw CLIInstallerServiceError.invalidCodeSignature
        }
        var copiedInformation: CFDictionary?
        guard
            SecCodeCopySigningInformation(
                staticCode,
                SecCSFlags(rawValue: UInt32(kSecCSSigningInformation)),
                &copiedInformation
            ) == errSecSuccess,
            let information = copiedInformation as? [CFString: Any],
            let identifier = information[kSecCodeInfoIdentifier] as? String,
            Self.isSafeIdentifier(identifier, maximumBytes: 512),
            let rawFlags = information[kSecCodeInfoFlags] as? NSNumber
        else {
            throw CLIInstallerServiceError.invalidCodeSignature
        }
        let teamIdentifier = information[kSecCodeInfoTeamIdentifier] as? String
        guard teamIdentifier.map({
            Self.isSafeIdentifier($0, maximumBytes: 128)
        }) ?? true else {
            throw CLIInstallerServiceError.invalidCodeSignature
        }
        return CLIStaticSignatureIdentity(
            signingIdentifier: identifier,
            teamIdentifier: teamIdentifier,
            isAdHoc: rawFlags.uint32Value & 0x2 != 0
        )
    }

    private static func isSafeIdentifier(
        _ value: String,
        maximumBytes: Int
    ) -> Bool {
        !value.isEmpty
            && value.utf8.count <= maximumBytes
            && value.unicodeScalars.allSatisfy {
                $0.value >= 0x20 && $0.value != 0x7F
            }
    }
}

struct CLIProductSignaturePolicy: Sendable {
    private let appBundleIdentifier: String
    private let permitsAdHocDevelopment: Bool

    init(
        appBundleIdentifier: String =
            Bundle.main.bundleIdentifier ?? "se.mjukis.dux.spike",
        permitsAdHocDevelopment: Bool = _isDebugAssertConfiguration()
    ) {
        self.appBundleIdentifier = appBundleIdentifier
        self.permitsAdHocDevelopment = permitsAdHocDevelopment
    }

    func validateBundled(
        _ identity: CLIStaticSignatureIdentity,
        runningApplication: CLIStaticSignatureIdentity
    ) throws {
        let applicationIdentifier = appBundleIdentifier
        let releaseIdentifier = appBundleIdentifier + ".cli"
        if
            !identity.isAdHoc,
            !runningApplication.isAdHoc,
            identity.signingIdentifier == releaseIdentifier,
            runningApplication.signingIdentifier == applicationIdentifier,
            let teamIdentifier = identity.teamIdentifier,
            !teamIdentifier.isEmpty,
            runningApplication.teamIdentifier == teamIdentifier
        {
            return
        }
        guard
            permitsAdHocDevelopment,
            identity.isAdHoc,
            runningApplication.isAdHoc,
            identity.teamIdentifier == nil,
            runningApplication.teamIdentifier == nil,
            runningApplication.signingIdentifier == applicationIdentifier
            || runningApplication.signingIdentifier
            == applicationIdentifier + ".debug",
            identity.signingIdentifier == releaseIdentifier
            || identity.signingIdentifier == releaseIdentifier + ".debug"
        else {
            throw CLIInstallerServiceError.invalidCodeSignature
        }
    }

    func validateInstalled(
        _ installed: CLIStaticSignatureIdentity,
        matches bundled: CLIStaticSignatureIdentity
    ) throws {
        guard
            installed.signingIdentifier == bundled.signingIdentifier,
            installed.teamIdentifier == bundled.teamIdentifier,
            installed.isAdHoc == bundled.isAdHoc
        else {
            throw CLIInstallerServiceError.invalidCodeSignature
        }
    }
}

struct DarwinCLIInstallerPlatform: CLIInstallerPlatform, @unchecked Sendable {
    static let maximumMetadataBytes = 4096
    static let maximumMarkerBytes = 1024
    static let maximumBinaryBytes: UInt64 = 256 * 1024 * 1024
    static let markerName = "com.mjukis.dux.cli-installation"

    private let resources: any CLIInstallerResourceProviding
    private let account: any CLIInstallerAccountProviding
    private let signatureInspector: any CLIStaticSignatureInspecting
    private let signaturePolicy: CLIProductSignaturePolicy

    init(
        resources: any CLIInstallerResourceProviding = BundleCLIInstallerResources(),
        account: any CLIInstallerAccountProviding = SystemCLIInstallerAccount(),
        signatureInspector: any CLIStaticSignatureInspecting =
            SystemCLIStaticSignatureInspector(),
        signaturePolicy: CLIProductSignaturePolicy = CLIProductSignaturePolicy()
    ) {
        self.resources = resources
        self.account = account
        self.signatureInspector = signatureInspector
        self.signaturePolicy = signaturePolicy
    }

    func observe() throws -> CLIInstallerObservedState {
        let bundled = try observeBundled()
        let pathEnvironment = try Self.pathEnvironment(
            account.inheritedPath(),
            home: account.currentHomeDirectory()
        )
        guard let directory = try openInstallDirectory(create: false) else {
            return Self.observedState(
                bundled: bundled,
                target: .absent,
                pathEnvironment: pathEnvironment
            )
        }
        defer { directory.close() }
        let target = try observeTarget(in: directory, bundled: bundled)
        return Self.observedState(
            bundled: bundled,
            target: target,
            pathEnvironment: pathEnvironment
        )
    }

    func install(
        expected: CLIInstallerObservedState
    ) throws -> CLIInstallerObservedState {
        let bundled = try observeBundled()
        guard bundled == expected.bundledEvidence else {
            throw CLIInstallerServiceError.confirmationChanged
        }
        let pathEnvironment = try Self.pathEnvironment(
            account.inheritedPath(),
            home: account.currentHomeDirectory()
        )
        guard let directory = try openInstallDirectory(create: true) else {
            throw CLIInstallerServiceError.unsafeInstallDirectory
        }
        defer { directory.close() }
        try directory.lock(exclusive: true)
        defer { directory.unlock() }
        let current = try observeTarget(in: directory, bundled: bundled)
        guard current == expected.targetEvidence else {
            throw CLIInstallerServiceError.confirmationChanged
        }
        switch current {
        case .absent, .managed:
            break
        case .unmanaged:
            throw CLIInstallerServiceError.unmanagedDestination
        case let .unsafe(reason, _):
            throw CLIInstallerServiceError.unsafeDestination(reason)
        }

        let stageName = ".dux.install-\(UUID().uuidString.lowercased())"
        var published = false
        do {
            try copyBundledCLI(
                bundled,
                into: stageName,
                directory: directory
            )
            let stage = try observeNamedTarget(
                stageName,
                in: directory,
                bundled: bundled
            )
            guard
                case let .managed(file, marker, signature) = stage,
                marker.version == bundled.metadata.version,
                marker.sha256 == bundled.metadata.sha256,
                file.sha256 == bundled.metadata.sha256,
                signature == bundled.signature
            else {
                throw CLIInstallerServiceError.invalidBundledBinary
            }

            switch current {
            case .absent:
                // DUX-DESTRUCTIVE: allow=macos-cli-installer-publish-new -- atomically publishes only the verified private stage to the absent fixed ~/.local/bin/dux destination
                guard renameatx_np(
                    directory.fileDescriptor,
                    stageName,
                    directory.fileDescriptor,
                    "dux",
                    UInt32(RENAME_EXCL)
                ) == 0 else {
                    if errno == EEXIST {
                        throw CLIInstallerServiceError.confirmationChanged
                    }
                    throw CLIInstallerServiceError.retryable
                }
            case .managed:
                // Swap preserves the old inode under our create-new stage name.
                // DUX validates that displaced inode before it is unlinked, so
                // an uncooperative same-user race cannot make DUX delete an
                // unmanaged replacement.
                // DUX-DESTRUCTIVE: allow=macos-cli-installer-publish-upgrade -- atomically swaps the verified private stage with the exactly revalidated app-managed fixed ~/.local/bin/dux destination
                guard renameatx_np(
                    directory.fileDescriptor,
                    stageName,
                    directory.fileDescriptor,
                    "dux",
                    UInt32(RENAME_SWAP)
                ) == 0 else {
                    throw CLIInstallerServiceError.retryable
                }
                published = true
                let displaced: CLIInstallerTargetEvidence
                do {
                    displaced = try observeNamedTarget(
                        stageName,
                        in: directory,
                        bundled: bundled
                    )
                } catch {
                    try restoreSwappedTarget(
                        stageName,
                        directory: directory
                    )
                    published = false
                    throw CLIInstallerServiceError.confirmationChanged
                }
                guard displaced == current else {
                    try restoreSwappedTarget(
                        stageName,
                        directory: directory
                    )
                    published = false
                    throw CLIInstallerServiceError.confirmationChanged
                }
                // DUX-DESTRUCTIVE: allow=macos-cli-installer-displaced-unlink -- removes only the exact app-managed inode atomically displaced under this invocation's private stage name
                guard unlinkat(directory.fileDescriptor, stageName, 0) == 0 else {
                    throw CLIInstallerServiceError.outcomeUnknown
                }
            case .unmanaged, .unsafe:
                preconditionFailure("target changed after exact revalidation")
            }
            published = true
            guard fsync(directory.fileDescriptor) == 0 else {
                throw CLIInstallerServiceError.outcomeUnknown
            }
        } catch {
            if !published {
                // DUX-DESTRUCTIVE: allow=macos-cli-installer-stage-unlink -- removes only this invocation's create-new private sibling stage before it has been published
                _ = unlinkat(directory.fileDescriptor, stageName, 0)
            }
            throw error
        }

        let finalTarget: CLIInstallerTargetEvidence
        do {
            finalTarget = try observeTarget(in: directory, bundled: bundled)
        } catch {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        guard
            case let .managed(file, marker, signature) = finalTarget,
            file.sha256 == bundled.metadata.sha256,
            marker.version == bundled.metadata.version,
            marker.sha256 == bundled.metadata.sha256,
            signature == bundled.signature
        else {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        return Self.observedState(
            bundled: bundled,
            target: finalTarget,
            pathEnvironment: pathEnvironment
        )
    }

    private func restoreSwappedTarget(
        _ stageName: String,
        directory: CLIInstallerDirectory
    ) throws {
        guard
            // DUX-DESTRUCTIVE: allow=macos-cli-installer-upgrade-rollback -- atomically restores the two entries after the displaced inode fails exact managed-target comparison
            renameatx_np(
                directory.fileDescriptor,
                stageName,
                directory.fileDescriptor,
                "dux",
                UInt32(RENAME_SWAP)
            ) == 0,
            fsync(directory.fileDescriptor) == 0
        else {
            throw CLIInstallerServiceError.outcomeUnknown
        }
    }

    func uninstall(
        expected: CLIInstallerObservedState
    ) throws -> CLIInstallerObservedState {
        let bundled = try observeBundled()
        guard bundled == expected.bundledEvidence else {
            throw CLIInstallerServiceError.confirmationChanged
        }
        let pathEnvironment = try Self.pathEnvironment(
            account.inheritedPath(),
            home: account.currentHomeDirectory()
        )
        guard let directory = try openInstallDirectory(create: false) else {
            throw CLIInstallerServiceError.confirmationChanged
        }
        defer { directory.close() }
        try directory.lock(exclusive: true)
        defer { directory.unlock() }
        let current = try observeTarget(in: directory, bundled: bundled)
        guard current == expected.targetEvidence else {
            throw CLIInstallerServiceError.confirmationChanged
        }
        guard case .managed = current else {
            switch current {
            case .unmanaged:
                throw CLIInstallerServiceError.unmanagedDestination
            case let .unsafe(reason, _):
                throw CLIInstallerServiceError.unsafeDestination(reason)
            case .absent, .managed:
                throw CLIInstallerServiceError.confirmationChanged
            }
        }
        let sentinelName = ".dux.uninstall-\(UUID().uuidString.lowercased())"
        let sentinel = try createUninstallSentinel(
            sentinelName,
            directory: directory
        )
        guard
            // DUX-DESTRUCTIVE: allow=macos-cli-installer-uninstall-swap -- atomically displaces the fixed revalidated target under this invocation's private sentinel name
            renameatx_np(
                directory.fileDescriptor,
                sentinelName,
                directory.fileDescriptor,
                "dux",
                UInt32(RENAME_SWAP)
            ) == 0
        else {
            // DUX-DESTRUCTIVE: allow=macos-cli-installer-uninstall-sentinel-failure-cleanup -- removes only this invocation's create-new UUID sentinel after its target exchange fails
            _ = unlinkat(directory.fileDescriptor, sentinelName, 0)
            throw CLIInstallerServiceError.retryable
        }

        let displaced: CLIInstallerTargetEvidence
        let publishedSentinel: CLIInstallerFileEvidence
        do {
            displaced = try observeNamedTarget(
                sentinelName,
                in: directory,
                bundled: bundled
            )
            publishedSentinel = try observePlainFile(
                "dux",
                in: directory,
                requireExecutable: false
            )
        } catch {
            try restoreUninstallSwap(
                sentinelName,
                sentinel: sentinel,
                directory: directory
            )
            throw CLIInstallerServiceError.confirmationChanged
        }
        guard displaced == current, publishedSentinel == sentinel else {
            try restoreUninstallSwap(
                sentinelName,
                sentinel: sentinel,
                directory: directory
            )
            throw CLIInstallerServiceError.confirmationChanged
        }

        // DUX-DESTRUCTIVE: allow=macos-cli-installer-uninstall-displaced -- removes only the exact app-managed inode displaced under this invocation's private sentinel name
        guard unlinkat(directory.fileDescriptor, sentinelName, 0) == 0 else {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        let finalSentinel: CLIInstallerFileEvidence
        do {
            finalSentinel = try observePlainFile(
                "dux",
                in: directory,
                requireExecutable: false
            )
        } catch {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        guard finalSentinel == sentinel else {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        // DUX-DESTRUCTIVE: allow=macos-cli-installer-uninstall -- unlinks only this invocation's exact revalidated sentinel from the fixed destination after the managed target is safely displaced
        guard unlinkat(directory.fileDescriptor, "dux", 0) == 0 else {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        guard fsync(directory.fileDescriptor) == 0 else {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        let finalTarget: CLIInstallerTargetEvidence
        do {
            finalTarget = try observeTarget(in: directory, bundled: bundled)
        } catch {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        guard finalTarget == .absent else {
            throw CLIInstallerServiceError.outcomeUnknown
        }
        return Self.observedState(
            bundled: bundled,
            target: finalTarget,
            pathEnvironment: pathEnvironment
        )
    }

    private func restoreUninstallSwap(
        _ sentinelName: String,
        sentinel: CLIInstallerFileEvidence,
        directory: CLIInstallerDirectory
    ) throws {
        do {
            try restoreSwappedTarget(
                sentinelName,
                directory: directory
            )
            let restoredSentinel = try observePlainFile(
                sentinelName,
                in: directory,
                requireExecutable: false
            )
            guard restoredSentinel == sentinel else {
                throw CLIInstallerServiceError.outcomeUnknown
            }
            guard
                // DUX-DESTRUCTIVE: allow=macos-cli-installer-uninstall-sentinel-cleanup -- removes only this invocation's revalidated UUID sentinel after restoring the original target
                unlinkat(directory.fileDescriptor, sentinelName, 0) == 0,
                fsync(directory.fileDescriptor) == 0
            else {
                throw CLIInstallerServiceError.outcomeUnknown
            }
        } catch {
            throw CLIInstallerServiceError.outcomeUnknown
        }
    }

    private func createUninstallSentinel(
        _ name: String,
        directory: CLIInstallerDirectory
    ) throws -> CLIInstallerFileEvidence {
        let descriptor = openat(
            directory.fileDescriptor,
            name,
            O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
            mode_t(S_IRUSR | S_IWUSR)
        )
        guard descriptor >= 0 else {
            throw CLIInstallerServiceError.retryable
        }
        defer { close(descriptor) }
        do {
            var bytes = [UInt8](repeating: 0, count: 32)
            guard
                SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
                == errSecSuccess
            else {
                throw CLIInstallerServiceError.retryable
            }
            let written = bytes.withUnsafeBytes { buffer in
                write(descriptor, buffer.baseAddress, buffer.count)
            }
            guard
                written == bytes.count,
                fsync(descriptor) == 0
            else {
                throw CLIInstallerServiceError.retryable
            }
            return try Self.observeRegularFile(
                descriptor,
                expectedOwner: account.currentUserID(),
                requireExecutable: false,
                includeHash: true
            )
        } catch {
            // DUX-DESTRUCTIVE: allow=macos-cli-installer-uninstall-sentinel-create-cleanup -- removes only this invocation's create-new UUID sentinel after its private initialization fails
            _ = unlinkat(directory.fileDescriptor, name, 0)
            throw error
        }
    }

    private func observeBundled() throws -> CLIInstallerBundledEvidence {
        guard
            let binaryURL = resources.bundledBinaryURL(),
            let metadataURL = resources.bundledMetadataURL()
        else {
            throw CLIInstallerServiceError.resourceMissing
        }
        let metadata = try Self.loadMetadata(at: metadataURL)
        let descriptor = open(binaryURL.path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW)
        guard descriptor >= 0 else {
            throw CLIInstallerServiceError.invalidBundledBinary
        }
        defer { close(descriptor) }
        let file = try Self.observeRegularFile(
            descriptor,
            expectedOwner: nil,
            requireExecutable: true,
            includeHash: true
        )
        guard
            file.sha256 == metadata.sha256,
            try Self.machOArchitectures(descriptor, size: file.size)
            == CLIBundledMetadata.architectures
        else {
            throw CLIInstallerServiceError.invalidArchitecture
        }
        let signature = try signatureInspector.inspect(at: binaryURL)
        let runningApplicationSignature = try signatureInspector.inspect(
            at: resources.runningApplicationURL()
        )
        try signaturePolicy.validateBundled(
            signature,
            runningApplication: runningApplicationSignature
        )
        try Self.revalidatePath(binaryURL.path, matches: file)
        return CLIInstallerBundledEvidence(
            file: file,
            metadata: metadata,
            signature: signature
        )
    }

    private func openInstallDirectory(
        create: Bool
    ) throws -> CLIInstallerDirectory? {
        let home = try account.currentHomeDirectory()
        let userID = account.currentUserID()
        let homeDescriptor = open(
            home.path,
            O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
        )
        guard homeDescriptor >= 0 else {
            throw CLIInstallerServiceError.unsafeHome
        }
        var current = CLIInstallerDirectory(fileDescriptor: homeDescriptor, url: home)
        do {
            try current.validate(owner: userID, home: true)
            guard let local = try current.openChild(
                ".local",
                create: create,
                owner: userID
            ) else {
                return nil
            }
            current = local
            guard let bin = try current.openChild(
                "bin",
                create: create,
                owner: userID
            ) else {
                return nil
            }
            return bin
        } catch {
            current.close()
            throw error
        }
    }

    private func observeTarget(
        in directory: CLIInstallerDirectory,
        bundled: CLIInstallerBundledEvidence
    ) throws -> CLIInstallerTargetEvidence {
        try observeNamedTarget("dux", in: directory, bundled: bundled)
    }

    private func observeNamedTarget(
        _ name: String,
        in directory: CLIInstallerDirectory,
        bundled: CLIInstallerBundledEvidence
    ) throws -> CLIInstallerTargetEvidence {
        var pathStat = stat()
        guard
            fstatat(
                directory.fileDescriptor,
                name,
                &pathStat,
                AT_SYMLINK_NOFOLLOW
            ) == 0
        else {
            if errno == ENOENT {
                return .absent
            }
            throw CLIInstallerServiceError.retryable
        }
        let preliminary = Self.fileEvidence(pathStat, sha256: nil)
        guard pathStat.st_mode & S_IFMT != S_IFLNK else {
            return .unsafe(.symbolicLink, preliminary)
        }
        guard pathStat.st_mode & S_IFMT == S_IFREG else {
            return .unsafe(.notRegularFile, preliminary)
        }
        guard pathStat.st_nlink == 1 else {
            return .unsafe(.hardLinked, preliminary)
        }
        guard pathStat.st_uid == account.currentUserID() else {
            return .unsafe(.wrongOwner, preliminary)
        }
        guard pathStat.st_mode & (S_ISUID | S_ISGID | S_IWGRP | S_IWOTH) == 0 else {
            return .unsafe(.unsafePermissions, preliminary)
        }
        guard pathStat.st_size > 0,
              UInt64(pathStat.st_size) <= Self.maximumBinaryBytes
        else {
            return .unsafe(.oversized, preliminary)
        }

        let descriptor = openat(
            directory.fileDescriptor,
            name,
            O_RDONLY | O_CLOEXEC | O_NOFOLLOW
        )
        guard descriptor >= 0 else {
            throw CLIInstallerServiceError.retryable
        }
        defer { close(descriptor) }
        let file: CLIInstallerFileEvidence
        do {
            file = try Self.observeRegularFile(
                descriptor,
                expectedOwner: account.currentUserID(),
                requireExecutable: true,
                includeHash: true
            )
        } catch let error as CLIInstallerServiceError {
            switch error {
            case let .unsafeDestination(reason):
                return .unsafe(reason, preliminary)
            default:
                throw error
            }
        }
        guard Self.sameIdentity(file, preliminary) else {
            throw CLIInstallerServiceError.retryable
        }
        do {
            guard
                try Self.machOArchitectures(descriptor, size: file.size)
                == CLIBundledMetadata.architectures
            else {
                return .unsafe(.managedEvidenceChanged, file)
            }
        } catch {
            return .unsafe(.managedEvidenceChanged, file)
        }
        let markerData: Data
        do {
            markerData = try Self.readMarker(descriptor)
        } catch CLIInstallerMarkerReadError.missing {
            return .unmanaged(file)
        } catch {
            return .unsafe(.malformedManagedEvidence, file)
        }
        let marker: CLIInstallerManagedMarker
        do {
            marker = try Self.decodeMarker(markerData)
        } catch {
            return .unsafe(.malformedManagedEvidence, file)
        }
        guard marker.sha256 == file.sha256 else {
            return .unsafe(.managedEvidenceChanged, file)
        }
        let fileURL = directory.url.appending(path: name, directoryHint: .notDirectory)
        let signature: CLIStaticSignatureIdentity
        do {
            signature = try signatureInspector.inspect(at: fileURL)
            try signaturePolicy.validateInstalled(signature, matches: bundled.signature)
            try Self.revalidateDescriptorPath(
                descriptor,
                directory: directory,
                name: name,
                matches: file
            )
        } catch {
            return .unsafe(.invalidCodeSignature, file)
        }
        return .managed(file: file, marker: marker, signature: signature)
    }

    private func observePlainFile(
        _ name: String,
        in directory: CLIInstallerDirectory,
        requireExecutable: Bool
    ) throws -> CLIInstallerFileEvidence {
        let descriptor = openat(
            directory.fileDescriptor,
            name,
            O_RDONLY | O_CLOEXEC | O_NOFOLLOW
        )
        guard descriptor >= 0 else {
            throw CLIInstallerServiceError.retryable
        }
        defer { close(descriptor) }
        let file = try Self.observeRegularFile(
            descriptor,
            expectedOwner: account.currentUserID(),
            requireExecutable: requireExecutable,
            includeHash: true
        )
        try Self.revalidateDescriptorPath(
            descriptor,
            directory: directory,
            name: name,
            matches: file
        )
        return file
    }

    private func copyBundledCLI(
        _ bundled: CLIInstallerBundledEvidence,
        into name: String,
        directory: CLIInstallerDirectory
    ) throws {
        guard let binaryURL = resources.bundledBinaryURL() else {
            throw CLIInstallerServiceError.resourceMissing
        }
        let source = open(binaryURL.path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW)
        guard source >= 0 else {
            throw CLIInstallerServiceError.invalidBundledBinary
        }
        defer { close(source) }
        let sourceEvidence = try Self.observeRegularFile(
            source,
            expectedOwner: nil,
            requireExecutable: true,
            includeHash: true
        )
        guard
            sourceEvidence == bundled.file,
            lseek(source, 0, SEEK_SET) == 0
        else {
            throw CLIInstallerServiceError.confirmationChanged
        }

        let destination = openat(
            directory.fileDescriptor,
            name,
            O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
            mode_t(S_IRUSR | S_IWUSR)
        )
        guard destination >= 0 else {
            throw CLIInstallerServiceError.retryable
        }
        defer { close(destination) }
        var buffer = [UInt8](repeating: 0, count: 128 * 1024)
        while true {
            let count = read(source, &buffer, buffer.count)
            guard count >= 0 else {
                if errno == EINTR {
                    continue
                }
                throw CLIInstallerServiceError.retryable
            }
            if count == 0 {
                break
            }
            var offset = 0
            while offset < count {
                let written = buffer.withUnsafeBytes { bytes in
                    write(
                        destination,
                        bytes.baseAddress!.advanced(by: offset),
                        count - offset
                    )
                }
                if written < 0 {
                    if errno == EINTR {
                        continue
                    }
                    throw CLIInstallerServiceError.retryable
                }
                guard written > 0 else {
                    throw CLIInstallerServiceError.retryable
                }
                offset += written
            }
        }
        guard
            fchmod(
                destination,
                mode_t(S_IRUSR | S_IWUSR | S_IXUSR | S_IRGRP | S_IXGRP | S_IROTH | S_IXOTH)
            ) == 0
        else {
            throw CLIInstallerServiceError.retryable
        }
        let marker = try Self.encodeMarker(
            CLIInstallerManagedMarker(
                version: bundled.metadata.version,
                sha256: bundled.metadata.sha256
            )
        )
        let markerResult = marker.withUnsafeBytes { bytes in
            fsetxattr(
                destination,
                Self.markerName,
                bytes.baseAddress,
                bytes.count,
                0,
                XATTR_CREATE
            )
        }
        guard markerResult == 0, fsync(destination) == 0 else {
            throw CLIInstallerServiceError.retryable
        }
    }

    private static func observedState(
        bundled: CLIInstallerBundledEvidence,
        target: CLIInstallerTargetEvidence,
        pathEnvironment: CLIPathEnvironmentStatus
    ) -> CLIInstallerObservedState {
        let disposition: CLIInstallationDisposition = switch target {
        case .absent:
            .absent
        case let .managed(file, marker, _):
            .managed(
                CLIManagedInstallation(
                    version: marker.version,
                    sha256: marker.sha256,
                    relation: relation(
                        installed: marker,
                        file: file,
                        bundled: bundled.metadata
                    )
                )
            )
        case .unmanaged:
            .unmanaged
        case let .unsafe(reason, _):
            .unsafe(reason)
        }
        return CLIInstallerObservedState(
            status: CLIInstallationStatus(
                bundled: bundled.metadata,
                disposition: disposition,
                pathEnvironment: pathEnvironment
            ),
            bundledEvidence: bundled,
            targetEvidence: target
        )
    }

    private static func relation(
        installed: CLIInstallerManagedMarker,
        file: CLIInstallerFileEvidence,
        bundled: CLIBundledMetadata
    ) -> CLIInstalledVersionRelation {
        if installed.version < bundled.version {
            return .older
        }
        if installed.version > bundled.version {
            return .newer
        }
        return file.sha256 == bundled.sha256 ? .current : .sameVersionDifferentBuild
    }

    private static func pathEnvironment(
        _ inheritedPath: String?,
        home: URL
    ) -> CLIPathEnvironmentStatus {
        guard let inheritedPath else {
            return .unavailable
        }
        let expected = home
            .appending(path: ".local", directoryHint: .isDirectory)
            .appending(path: "bin", directoryHint: .isDirectory)
            .standardizedFileURL.path
        return inheritedPath.split(separator: ":", omittingEmptySubsequences: false)
            .contains { component in
                guard !component.isEmpty else {
                    return false
                }
                return URL(fileURLWithPath: String(component), isDirectory: true)
                    .standardizedFileURL.path == expected
            }
            ? .included
            : .missing
    }
}

private enum CLIInstallerMarkerReadError: Error {
    case missing
    case malformed
}

private final class CLIInstallerDirectory: @unchecked Sendable {
    let fileDescriptor: Int32
    let url: URL
    private var ownsDescriptor = true

    init(fileDescriptor: Int32, url: URL) {
        self.fileDescriptor = fileDescriptor
        self.url = url
    }

    func close() {
        guard ownsDescriptor else {
            return
        }
        Darwin.close(fileDescriptor)
        ownsDescriptor = false
    }

    func validate(owner: uid_t, home: Bool) throws {
        var value = stat()
        guard
            fstat(fileDescriptor, &value) == 0,
            value.st_mode & S_IFMT == S_IFDIR,
            value.st_uid == owner,
            value.st_mode & (S_IWGRP | S_IWOTH) == 0
        else {
            throw home
                ? CLIInstallerServiceError.unsafeHome
                : CLIInstallerServiceError.unsafeInstallDirectory
        }
    }

    func openChild(
        _ name: String,
        create: Bool,
        owner: uid_t
    ) throws -> CLIInstallerDirectory? {
        var descriptor = openat(
            fileDescriptor,
            name,
            O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
        )
        if descriptor < 0, errno == ENOENT, create {
            guard mkdirat(fileDescriptor, name, mode_t(S_IRWXU)) == 0 || errno == EEXIST else {
                throw CLIInstallerServiceError.unsafeInstallDirectory
            }
            descriptor = openat(
                fileDescriptor,
                name,
                O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
            )
        }
        guard descriptor >= 0 else {
            if errno == ENOENT, !create {
                close()
                return nil
            }
            throw CLIInstallerServiceError.unsafeInstallDirectory
        }
        let child = CLIInstallerDirectory(
            fileDescriptor: descriptor,
            url: url.appending(path: name, directoryHint: .isDirectory)
        )
        do {
            try child.validate(owner: owner, home: false)
        } catch {
            child.close()
            throw error
        }
        close()
        return child
    }

    func lock(exclusive: Bool) throws {
        let operation = exclusive ? LOCK_EX : LOCK_SH
        guard flock(fileDescriptor, operation | LOCK_NB) == 0 else {
            throw CLIInstallerServiceError.busy
        }
    }

    func unlock() {
        _ = flock(fileDescriptor, LOCK_UN)
    }
}

private extension DarwinCLIInstallerPlatform {
    static func loadMetadata(
        at url: URL
    ) throws -> CLIBundledMetadata {
        let data: Data
        do {
            let values = try url.resourceValues(forKeys: [
                .isRegularFileKey,
                .isSymbolicLinkKey,
                .fileSizeKey,
            ])
            guard
                values.isRegularFile == true,
                values.isSymbolicLink != true,
                let size = values.fileSize,
                size > 0,
                size <= maximumMetadataBytes
            else {
                throw CLIInstallerServiceError.invalidMetadata
            }
            data = try Data(contentsOf: url, options: [.mappedIfSafe])
        } catch let error as CLIInstallerServiceError {
            throw error
        } catch {
            throw CLIInstallerServiceError.resourceMissing
        }
        guard data.count <= maximumMetadataBytes else {
            throw CLIInstallerServiceError.metadataTooLarge
        }
        let object: [String: Any]
        do {
            guard
                let decoded = try JSONSerialization.jsonObject(
                    with: data,
                    options: []
                ) as? [String: Any]
            else {
                throw CLIInstallerServiceError.invalidMetadata
            }
            object = decoded
        } catch let error as CLIInstallerServiceError {
            throw error
        } catch {
            throw CLIInstallerServiceError.invalidMetadata
        }
        let exactKeys: Set<String> = [
            "record_version",
            "product",
            "version",
            "database_schema_version",
            "snapshot_format_version",
            "sha256",
            "architectures",
        ]
        guard
            Set(object.keys) == exactKeys,
            uint32(object["record_version"]) == CLIBundledMetadata.recordVersion,
            object["product"] as? String == CLIBundledMetadata.product,
            let versionText = object["version"] as? String,
            let version = CLIVersion(versionText),
            let databaseSchemaVersion = uint32(object["database_schema_version"]),
            databaseSchemaVersion > 0,
            let snapshotFormatVersion = uint32(object["snapshot_format_version"]),
            snapshotFormatVersion > 0,
            let hashText = object["sha256"] as? String,
            let hash = decodeLowercaseSHA256(hashText),
            object["architectures"] as? [String] == CLIBundledMetadata.architectures
        else {
            throw CLIInstallerServiceError.invalidMetadata
        }
        return CLIBundledMetadata(
            version: version,
            databaseSchemaVersion: databaseSchemaVersion,
            snapshotFormatVersion: snapshotFormatVersion,
            sha256: hash
        )
    }

    static func encodeMarker(
        _ marker: CLIInstallerManagedMarker
    ) throws -> Data {
        let object: [String: Any] = [
            "record_version": CLIInstallerManagedMarker.recordVersion,
            "product": CLIInstallerManagedMarker.product,
            "source": CLIInstallerManagedMarker.source,
            "version": marker.version.displayText,
            "sha256": marker.sha256.lowercaseHex,
        ]
        let data = try JSONSerialization.data(
            withJSONObject: object,
            options: [.sortedKeys]
        )
        guard data.count <= maximumMarkerBytes else {
            throw CLIInstallerServiceError.invalidBundledBinary
        }
        return data
    }

    static func decodeMarker(
        _ data: Data
    ) throws -> CLIInstallerManagedMarker {
        guard
            !data.isEmpty,
            data.count <= maximumMarkerBytes,
            let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
            Set(object.keys) == [
                "record_version",
                "product",
                "source",
                "version",
                "sha256",
            ],
            uint32(object["record_version"]) == CLIInstallerManagedMarker.recordVersion,
            object["product"] as? String == CLIInstallerManagedMarker.product,
            object["source"] as? String == CLIInstallerManagedMarker.source,
            let versionText = object["version"] as? String,
            let version = CLIVersion(versionText),
            let hashText = object["sha256"] as? String,
            let hash = decodeLowercaseSHA256(hashText)
        else {
            throw CLIInstallerMarkerReadError.malformed
        }
        return CLIInstallerManagedMarker(version: version, sha256: hash)
    }

    static func readMarker(_ descriptor: Int32) throws -> Data {
        errno = 0
        let size = fgetxattr(
            descriptor,
            markerName,
            nil,
            0,
            0,
            0
        )
        guard size >= 0 else {
            if errno == ENOATTR {
                throw CLIInstallerMarkerReadError.missing
            }
            throw CLIInstallerMarkerReadError.malformed
        }
        guard size > 0, size <= maximumMarkerBytes else {
            throw CLIInstallerMarkerReadError.malformed
        }
        var data = Data(count: size)
        let readCount = data.withUnsafeMutableBytes { bytes in
            fgetxattr(
                descriptor,
                markerName,
                bytes.baseAddress,
                bytes.count,
                0,
                0
            )
        }
        guard readCount == size else {
            throw CLIInstallerMarkerReadError.malformed
        }
        return data
    }

    static func observeRegularFile(
        _ descriptor: Int32,
        expectedOwner: uid_t?,
        requireExecutable: Bool,
        includeHash: Bool
    ) throws -> CLIInstallerFileEvidence {
        var before = stat()
        guard fstat(descriptor, &before) == 0 else {
            throw CLIInstallerServiceError.retryable
        }
        guard before.st_mode & S_IFMT == S_IFREG else {
            throw CLIInstallerServiceError.unsafeDestination(.notRegularFile)
        }
        guard before.st_nlink == 1 else {
            throw CLIInstallerServiceError.unsafeDestination(.hardLinked)
        }
        if let expectedOwner, before.st_uid != expectedOwner {
            throw CLIInstallerServiceError.unsafeDestination(.wrongOwner)
        }
        guard before.st_mode & (S_ISUID | S_ISGID | S_IWGRP | S_IWOTH) == 0 else {
            throw CLIInstallerServiceError.unsafeDestination(.unsafePermissions)
        }
        if requireExecutable {
            guard before.st_mode & S_IXUSR != 0 else {
                throw CLIInstallerServiceError.unsafeDestination(.unsafePermissions)
            }
        }
        guard before.st_size > 0,
              UInt64(before.st_size) <= maximumBinaryBytes
        else {
            throw CLIInstallerServiceError.unsafeDestination(.oversized)
        }
        let hash = includeHash ? try sha256(descriptor, size: UInt64(before.st_size)) : nil
        var after = stat()
        guard
            fstat(descriptor, &after) == 0,
            fileEvidence(before, sha256: nil) == fileEvidence(after, sha256: nil)
        else {
            throw CLIInstallerServiceError.retryable
        }
        return fileEvidence(after, sha256: hash)
    }

    static func sha256(
        _ descriptor: Int32,
        size: UInt64
    ) throws -> Data {
        guard lseek(descriptor, 0, SEEK_SET) == 0 else {
            throw CLIInstallerServiceError.retryable
        }
        var hasher = SHA256()
        var remaining = size
        var buffer = [UInt8](repeating: 0, count: 128 * 1024)
        while remaining > 0 {
            let requested = min(buffer.count, Int(remaining))
            let count = read(descriptor, &buffer, requested)
            guard count > 0 else {
                if count < 0, errno == EINTR {
                    continue
                }
                throw CLIInstallerServiceError.retryable
            }
            hasher.update(data: Data(buffer.prefix(count)))
            remaining -= UInt64(count)
        }
        var trailingByte: UInt8 = 0
        guard read(descriptor, &trailingByte, 1) == 0 else {
            throw CLIInstallerServiceError.retryable
        }
        return Data(hasher.finalize())
    }

    static func machOArchitectures(
        _ descriptor: Int32,
        size: UInt64
    ) throws -> [String] {
        guard size >= 8 else {
            throw CLIInstallerServiceError.invalidArchitecture
        }
        var header = [UInt8](repeating: 0, count: 8)
        guard pread(descriptor, &header, header.count, 0) == header.count else {
            throw CLIInstallerServiceError.invalidArchitecture
        }
        let magic = bigEndianUInt32(header[0 ..< 4])
        let entrySize: Int
        switch magic {
        case 0xCAFE_BABE:
            entrySize = 20
        case 0xCAFE_BABF:
            entrySize = 32
        default:
            throw CLIInstallerServiceError.invalidArchitecture
        }
        let count = Int(bigEndianUInt32(header[4 ..< 8]))
        guard count == 2 else {
            throw CLIInstallerServiceError.invalidArchitecture
        }
        var entries = [UInt8](repeating: 0, count: count * entrySize)
        guard
            pread(descriptor, &entries, entries.count, 8) == entries.count
        else {
            throw CLIInstallerServiceError.invalidArchitecture
        }
        var architectures: [String] = []
        var ranges: [Range<UInt64>] = []
        for index in 0 ..< count {
            let base = index * entrySize
            let cpuType = bigEndianUInt32(entries[base ..< base + 4])
            let architecture: String = switch cpuType {
            case 0x0100_000C: "arm64"
            case 0x0100_0007: "x86_64"
            default:
                throw CLIInstallerServiceError.invalidArchitecture
            }
            let offset: UInt64
            let sliceSize: UInt64
            if entrySize == 20 {
                offset = UInt64(bigEndianUInt32(entries[base + 8 ..< base + 12]))
                sliceSize = UInt64(bigEndianUInt32(entries[base + 12 ..< base + 16]))
            } else {
                offset = bigEndianUInt64(entries[base + 8 ..< base + 16])
                sliceSize = bigEndianUInt64(entries[base + 16 ..< base + 24])
            }
            guard
                sliceSize > 0,
                offset <= size,
                sliceSize <= size - offset
            else {
                throw CLIInstallerServiceError.invalidArchitecture
            }
            let range = offset ..< offset + sliceSize
            guard ranges.allSatisfy({ !$0.overlaps(range) }) else {
                throw CLIInstallerServiceError.invalidArchitecture
            }
            ranges.append(range)
            architectures.append(architecture)
        }
        guard Set(architectures).count == 2 else {
            throw CLIInstallerServiceError.invalidArchitecture
        }
        return architectures.sorted {
            CLIBundledMetadata.architectures.firstIndex(of: $0)!
                < CLIBundledMetadata.architectures.firstIndex(of: $1)!
        }
    }

    static func bigEndianUInt32(
        _ bytes: ArraySlice<UInt8>
    ) -> UInt32 {
        bytes.reduce(0) { ($0 << 8) | UInt32($1) }
    }

    static func bigEndianUInt64(
        _ bytes: ArraySlice<UInt8>
    ) -> UInt64 {
        bytes.reduce(0) { ($0 << 8) | UInt64($1) }
    }

    static func uint32(_ value: Any?) -> UInt32? {
        guard
            let number = value as? NSNumber,
            CFGetTypeID(number) != CFBooleanGetTypeID()
        else {
            return nil
        }
        let decimal = number.decimalValue
        guard
            decimal >= 0,
            decimal <= Decimal(UInt32.max),
            decimal == Decimal(number.uint32Value)
        else {
            return nil
        }
        return number.uint32Value
    }

    static func decodeLowercaseSHA256(
        _ text: String
    ) -> Data? {
        guard
            text.utf8.count == 64,
            text.utf8.allSatisfy({
                ($0 >= 48 && $0 <= 57) || ($0 >= 97 && $0 <= 102)
            })
        else {
            return nil
        }
        var bytes: [UInt8] = []
        bytes.reserveCapacity(32)
        var index = text.startIndex
        for _ in 0 ..< 32 {
            let next = text.index(index, offsetBy: 2)
            guard let byte = UInt8(text[index ..< next], radix: 16) else {
                return nil
            }
            bytes.append(byte)
            index = next
        }
        return Data(bytes)
    }

    static func fileEvidence(
        _ value: stat,
        sha256: Data?
    ) -> CLIInstallerFileEvidence {
        CLIInstallerFileEvidence(
            device: UInt64(value.st_dev),
            inode: UInt64(value.st_ino),
            size: value.st_size >= 0 ? UInt64(value.st_size) : 0,
            modificationSeconds: Int64(value.st_mtimespec.tv_sec),
            modificationNanoseconds: Int64(value.st_mtimespec.tv_nsec),
            mode: UInt16(value.st_mode & 0xFFFF),
            owner: value.st_uid,
            linkCount: UInt64(value.st_nlink),
            sha256: sha256
        )
    }

    static func sameIdentity(
        _ lhs: CLIInstallerFileEvidence,
        _ rhs: CLIInstallerFileEvidence
    ) -> Bool {
        lhs.device == rhs.device
            && lhs.inode == rhs.inode
            && lhs.size == rhs.size
            && lhs.modificationSeconds == rhs.modificationSeconds
            && lhs.modificationNanoseconds == rhs.modificationNanoseconds
            && lhs.mode == rhs.mode
            && lhs.owner == rhs.owner
            && lhs.linkCount == rhs.linkCount
    }

    static func revalidatePath(
        _ path: String,
        matches expected: CLIInstallerFileEvidence
    ) throws {
        var value = stat()
        guard lstat(path, &value) == 0 else {
            throw CLIInstallerServiceError.retryable
        }
        let observed = fileEvidence(value, sha256: nil)
        guard sameIdentity(observed, expected) else {
            throw CLIInstallerServiceError.retryable
        }
    }

    static func revalidateDescriptorPath(
        _ descriptor: Int32,
        directory: CLIInstallerDirectory,
        name: String,
        matches expected: CLIInstallerFileEvidence
    ) throws {
        var descriptorStat = stat()
        var pathStat = stat()
        guard
            fstat(descriptor, &descriptorStat) == 0,
            fstatat(
                directory.fileDescriptor,
                name,
                &pathStat,
                AT_SYMLINK_NOFOLLOW
            ) == 0
        else {
            throw CLIInstallerServiceError.retryable
        }
        let descriptorEvidence = fileEvidence(descriptorStat, sha256: nil)
        let pathEvidence = fileEvidence(pathStat, sha256: nil)
        guard
            sameIdentity(descriptorEvidence, expected),
            sameIdentity(pathEvidence, expected)
        else {
            throw CLIInstallerServiceError.retryable
        }
    }
}

private extension Data {
    var lowercaseHex: String {
        map { String(format: "%02x", $0) }.joined()
    }
}
