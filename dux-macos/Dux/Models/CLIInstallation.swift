import Foundation

struct CLIVersion: Equatable, Comparable, Sendable {
    let major: UInt64
    let minor: UInt64
    let patch: UInt64

    init?(_ text: String) {
        guard
            text.utf8.count <= 64,
            !text.hasPrefix("+"),
            !text.hasPrefix("-")
        else {
            return nil
        }
        let components = text.split(separator: ".", omittingEmptySubsequences: false)
        guard components.count == 3 else {
            return nil
        }
        var numbers: [UInt64] = []
        numbers.reserveCapacity(3)
        for component in components {
            guard
                !component.isEmpty,
                component.allSatisfy(\.isASCIIWholeNumber),
                component.count == 1 || component.first != "0",
                let value = UInt64(component)
            else {
                return nil
            }
            numbers.append(value)
        }
        self.init(major: numbers[0], minor: numbers[1], patch: numbers[2])
    }

    init(major: UInt64, minor: UInt64, patch: UInt64) {
        self.major = major
        self.minor = minor
        self.patch = patch
    }

    var displayText: String {
        "\(major).\(minor).\(patch)"
    }

    static func < (lhs: Self, rhs: Self) -> Bool {
        if lhs.major != rhs.major {
            return lhs.major < rhs.major
        }
        if lhs.minor != rhs.minor {
            return lhs.minor < rhs.minor
        }
        return lhs.patch < rhs.patch
    }
}

private extension Character {
    var isASCIIWholeNumber: Bool {
        guard let scalar = unicodeScalars.only else {
            return false
        }
        return scalar.value >= 48 && scalar.value <= 57
    }
}

private extension String.UnicodeScalarView {
    var only: Unicode.Scalar? {
        guard count == 1 else {
            return nil
        }
        return first
    }
}

struct CLIBundledMetadata: Equatable, Sendable {
    static let recordVersion: UInt32 = 1
    static let product = "dux-cli"
    static let architectures = ["arm64", "x86_64"]

    let version: CLIVersion
    let databaseSchemaVersion: UInt32
    let snapshotFormatVersion: UInt32
    let sha256: Data
}

struct CLIStaticSignatureIdentity: Equatable, Sendable {
    let signingIdentifier: String
    let teamIdentifier: String?
    let isAdHoc: Bool
}

enum CLIPathEnvironmentStatus: Equatable, Sendable {
    case included
    case missing
    case unavailable
}

enum CLIInstalledVersionRelation: Equatable, Sendable {
    case older
    case current
    case sameVersionDifferentBuild
    case newer
}

struct CLIManagedInstallation: Equatable, Sendable {
    let version: CLIVersion
    let sha256: Data
    let relation: CLIInstalledVersionRelation
}

enum CLIUnsafeTargetReason: Equatable, Sendable {
    case symbolicLink
    case notRegularFile
    case hardLinked
    case wrongOwner
    case unsafePermissions
    case oversized
    case malformedManagedEvidence
    case managedEvidenceChanged
    case invalidCodeSignature
}

enum CLIInstallationDisposition: Equatable, Sendable {
    case absent
    case managed(CLIManagedInstallation)
    case unmanaged
    case unsafe(CLIUnsafeTargetReason)
}

struct CLIInstallationStatus: Equatable, Sendable {
    static let destinationDisplayText = "~/.local/bin/dux"

    let bundled: CLIBundledMetadata
    let disposition: CLIInstallationDisposition
    let pathEnvironment: CLIPathEnvironmentStatus
}

enum CLIInstallationAction: Equatable, Sendable {
    case install
    case upgrade
    case reinstall
    case uninstall
}

struct CLIInstallationConfirmation: Equatable, Sendable {
    let token: UUID
    let action: CLIInstallationAction
    let bundledVersion: CLIVersion
    let installedVersion: CLIVersion?
    let destinationDisplayText: String
}

struct CLIInstallationUpdate: Equatable, Sendable {
    let status: CLIInstallationStatus
    let changed: Bool
}

enum CLIInstallerServiceError: Error, Equatable, Sendable {
    case busy
    case resourceMissing
    case metadataTooLarge
    case invalidMetadata
    case invalidBundledBinary
    case invalidArchitecture
    case invalidCodeSignature
    case unsupportedAccount
    case unsafeHome
    case unsafeInstallDirectory
    case unsafeDestination(CLIUnsafeTargetReason)
    case unmanagedDestination
    case downgradeRefused
    case confirmationUnavailable
    case confirmationChanged
    case retryable
    case outcomeUnknown
}

enum CLIInstallationActivity: Equatable, Sendable {
    case loading
    case preparingInstall
    case installing
    case preparingUninstall
    case uninstalling
}

enum CLIInstallationFailure: Error, Equatable, Sendable {
    case service(CLIInstallerServiceError)
    case unexpected
}

struct CLIInstallationViewState: Equatable, Sendable {
    var status: CLIInstallationStatus?
    var activity: CLIInstallationActivity?
    var failure: CLIInstallationFailure?
    var requiresAuthoritativeReload: Bool

    static let idle = Self(
        status: nil,
        activity: nil,
        failure: nil,
        requiresAuthoritativeReload: false
    )

    var isBusy: Bool {
        activity != nil
    }
}

struct CLIInstallationPresentation: Equatable, Sendable {
    let statusTitle: String
    let statusDetail: String
    let bundledVersion: String
    let installedVersion: String?
    let sourceTitle: String
    let pathDetail: String
    let primaryAction: CLIInstallationAction?
    let primaryActionTitle: String?
    let offersUninstall: Bool
}

extension CLIInstallationPresentation {
    static func make(
        status: CLIInstallationStatus,
        locale: Locale = .current
    ) -> Self {
        let statusCopy: (
            title: String,
            detail: String,
            source: String,
            action: CLIInstallationAction?,
            actionTitle: String?,
            uninstall: Bool,
            installedVersion: String?
        ) = switch status.disposition {
        case .absent:
            (
                String(localized: "Not installed", locale: locale),
                String(
                    localized: "DUX can install its bundled command-line companion.",
                    locale: locale
                ),
                String(localized: "No file", locale: locale),
                .install,
                String(localized: "Install CLI…", locale: locale),
                false,
                nil
            )
        case let .managed(installation):
            switch installation.relation {
            case .older:
                (
                    String(localized: "Upgrade available", locale: locale),
                    String(
                        localized: "The app-installed CLI is older than this DUX app.",
                        locale: locale
                    ),
                    String(localized: "Installed by DUX", locale: locale),
                    .upgrade,
                    String(localized: "Upgrade CLI…", locale: locale),
                    true,
                    installation.version.displayText
                )
            case .current:
                (
                    String(localized: "Up to date", locale: locale),
                    String(
                        localized: "The app-installed CLI matches this DUX app.",
                        locale: locale
                    ),
                    String(localized: "Installed by DUX", locale: locale),
                    .reinstall,
                    String(localized: "Reinstall CLI…", locale: locale),
                    true,
                    installation.version.displayText
                )
            case .sameVersionDifferentBuild:
                (
                    String(localized: "Different build installed", locale: locale),
                    String(
                        localized: "The app-installed CLI has the same version but different verified bytes.",
                        locale: locale
                    ),
                    String(localized: "Installed by DUX", locale: locale),
                    .reinstall,
                    String(localized: "Reinstall CLI…", locale: locale),
                    true,
                    installation.version.displayText
                )
            case .newer:
                (
                    String(localized: "Newer CLI installed", locale: locale),
                    String(
                        localized: "DUX will not replace a newer app-installed CLI with an older one.",
                        locale: locale
                    ),
                    String(localized: "Installed by DUX", locale: locale),
                    nil,
                    nil,
                    true,
                    installation.version.displayText
                )
            }
        case .unmanaged:
            (
                String(localized: "Existing file not managed by DUX", locale: locale),
                String(
                    localized: "DUX will not execute, replace, or remove this file. Resolve it manually before installing from the app.",
                    locale: locale
                ),
                String(localized: "External or unrecognized", locale: locale),
                nil,
                nil,
                false,
                nil
            )
        case .unsafe:
            (
                String(localized: "Unsafe destination", locale: locale),
                String(
                    localized: "DUX refused the destination because its filesystem evidence is unsafe or changed.",
                    locale: locale
                ),
                String(localized: "Unverified", locale: locale),
                nil,
                nil,
                false,
                nil
            )
        }

        let pathDetail = switch status.pathEnvironment {
        case .included:
            String(
                localized: "~/.local/bin is present in this app’s inherited PATH.",
                locale: locale
            )
        case .missing:
            String(
                localized: "~/.local/bin is not present in this app’s inherited PATH. Add it to your shell configuration manually.",
                locale: locale
            )
        case .unavailable:
            String(
                localized: "DUX could not inspect the inherited PATH. It never edits shell configuration.",
                locale: locale
            )
        }

        return Self(
            statusTitle: statusCopy.title,
            statusDetail: statusCopy.detail,
            bundledVersion: status.bundled.version.displayText,
            installedVersion: statusCopy.installedVersion,
            sourceTitle: statusCopy.source,
            pathDetail: pathDetail,
            primaryAction: statusCopy.action,
            primaryActionTitle: statusCopy.actionTitle,
            offersUninstall: statusCopy.uninstall
        )
    }
}

enum CLIInstallationAccessibility {
    static let section = "cli-installation-section"
    static let status = "cli-installation-status"
    static let destination = "cli-installation-destination"
    static let bundledVersion = "cli-installation-bundled-version"
    static let installedVersion = "cli-installation-installed-version"
    static let source = "cli-installation-source"
    static let path = "cli-installation-path"
    static let install = "cli-installation-install"
    static let upgrade = "cli-installation-upgrade"
    static let reinstall = "cli-installation-reinstall"
    static let uninstall = "cli-installation-uninstall"
    static let confirm = "cli-installation-confirm"
    static let cancel = "cli-installation-cancel"
    static let refresh = "cli-installation-refresh"
    static let progress = "cli-installation-progress"
    static let error = "cli-installation-error"

    static let allIdentifiers = [
        section,
        status,
        destination,
        bundledVersion,
        installedVersion,
        source,
        path,
        install,
        upgrade,
        reinstall,
        uninstall,
        confirm,
        cancel,
        refresh,
        progress,
        error,
    ]
}
