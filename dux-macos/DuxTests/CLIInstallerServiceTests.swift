import CryptoKit
import Darwin
@testable import DUX
import Foundation
import XCTest

final class CLIInstallationValueTests: XCTestCase {
    func testVersionParserIsStrictAndComparable() throws {
        XCTAssertEqual(CLIVersion("0.5.0")?.displayText, "0.5.0")
        XCTAssertLessThan(try XCTUnwrap(CLIVersion("1.9.9")), try XCTUnwrap(CLIVersion("2.0.0")))
        for invalid in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "+1.2.3",
            "-1.2.3",
            "1.2.beta",
        ] {
            XCTAssertNil(CLIVersion(invalid), invalid)
        }
    }

    func testPresentationPermitsOnlyTruthfulActions() throws {
        let bundled = metadata(version: "2.0.0", byte: 2)
        let cases: [
            (
                CLIInstallationDisposition,
                CLIInstallationAction?,
                uninstall: Bool,
                title: String
            )
        ] = [
            (.absent, .install, false, "Not installed"),
            (
                .managed(
                    managed(version: "1.0.0", relation: .older)
                ),
                .upgrade,
                true,
                "Upgrade available"
            ),
            (
                .managed(
                    managed(version: "2.0.0", relation: .current)
                ),
                .reinstall,
                true,
                "Up to date"
            ),
            (
                .managed(
                    managed(version: "2.0.0", relation: .sameVersionDifferentBuild)
                ),
                .reinstall,
                true,
                "Different build installed"
            ),
            (
                .managed(
                    managed(version: "3.0.0", relation: .newer)
                ),
                nil,
                true,
                "Newer CLI installed"
            ),
            (.unmanaged, nil, false, "Existing file not managed by DUX"),
            (.unsafe(.symbolicLink), nil, false, "Unsafe destination"),
        ]

        for item in cases {
            let presentation = CLIInstallationPresentation.make(
                status: CLIInstallationStatus(
                    bundled: bundled,
                    disposition: item.0,
                    pathEnvironment: .missing
                ),
                locale: Locale(identifier: "en_US")
            )
            XCTAssertEqual(presentation.primaryAction, item.1)
            XCTAssertEqual(presentation.offersUninstall, item.uninstall)
            XCTAssertEqual(presentation.statusTitle, item.title)
            XCTAssertTrue(presentation.pathDetail.contains("never edits") == false)
            XCTAssertFalse(presentation.sourceTitle.isEmpty)
        }
    }

    func testAccessibilityIdentifiersAreUniqueAndStable() {
        XCTAssertEqual(
            Set(CLIInstallationAccessibility.allIdentifiers).count,
            CLIInstallationAccessibility.allIdentifiers.count
        )
        XCTAssertEqual(
            CLIInstallationAccessibility.section,
            "cli-installation-section"
        )
        XCTAssertEqual(
            CLIInstallationStatus.destinationDisplayText,
            "~/.local/bin/dux"
        )
    }

    func testReleaseSignaturePolicyBindsCLIToRunningAppTeam() throws {
        let policy = CLIProductSignaturePolicy(
            appBundleIdentifier: "test.dux",
            permitsAdHocDevelopment: false
        )
        let app = CLIStaticSignatureIdentity(
            signingIdentifier: "test.dux",
            teamIdentifier: "TEAMAAAAAA",
            isAdHoc: false
        )
        let cli = CLIStaticSignatureIdentity(
            signingIdentifier: "test.dux.cli",
            teamIdentifier: "TEAMAAAAAA",
            isAdHoc: false
        )

        XCTAssertNoThrow(
            try policy.validateBundled(cli, runningApplication: app)
        )
        XCTAssertThrowsError(
            try policy.validateBundled(
                CLIStaticSignatureIdentity(
                    signingIdentifier: "test.dux.cli",
                    teamIdentifier: "TEAMBBBBBB",
                    isAdHoc: false
                ),
                runningApplication: app
            )
        ) { error in
            XCTAssertEqual(
                error as? CLIInstallerServiceError,
                .invalidCodeSignature
            )
        }
        XCTAssertThrowsError(
            try policy.validateBundled(
                CLIStaticSignatureIdentity(
                    signingIdentifier: "other.product.cli",
                    teamIdentifier: "TEAMAAAAAA",
                    isAdHoc: false
                ),
                runningApplication: app
            )
        )
    }

    func testAdHocPolicyIsDebugOnlyAndStillBindsIdentifiers() throws {
        let cli = CLIStaticSignatureIdentity(
            signingIdentifier: "test.dux.cli.debug",
            teamIdentifier: nil,
            isAdHoc: true
        )
        let app = CLIStaticSignatureIdentity(
            signingIdentifier: "test.dux.debug",
            teamIdentifier: nil,
            isAdHoc: true
        )
        XCTAssertNoThrow(
            try CLIProductSignaturePolicy(
                appBundleIdentifier: "test.dux",
                permitsAdHocDevelopment: true
            ).validateBundled(cli, runningApplication: app)
        )
        XCTAssertThrowsError(
            try CLIProductSignaturePolicy(
                appBundleIdentifier: "test.dux",
                permitsAdHocDevelopment: false
            ).validateBundled(cli, runningApplication: app)
        )
    }

    private func metadata(version: String, byte: UInt8) -> CLIBundledMetadata {
        CLIBundledMetadata(
            version: CLIVersion(version)!,
            databaseSchemaVersion: 16,
            snapshotFormatVersion: 1,
            sha256: Data(repeating: byte, count: 32)
        )
    }

    private func managed(
        version: String,
        relation: CLIInstalledVersionRelation
    ) -> CLIManagedInstallation {
        CLIManagedInstallation(
            version: CLIVersion(version)!,
            sha256: Data(repeating: 1, count: 32),
            relation: relation
        )
    }
}

final class CLIInstallerServiceContractTests: XCTestCase {
    func testPrepareAndPerformAreOneShotAndPassExactObservation() async throws {
        let initial = observed(.absent)
        let installed = observed(
            .managed(
                CLIManagedInstallation(
                    version: CLIVersion("2.0.0")!,
                    sha256: initial.status.bundled.sha256,
                    relation: .current
                )
            )
        )
        let platform = RecordingCLIInstallerPlatform(
            observation: initial,
            installResult: .success(installed)
        )
        let service = CLIInstallerService(platform: platform)

        let confirmation = try await service.prepare(.install)
        XCTAssertEqual(confirmation.action, .install)
        XCTAssertNil(confirmation.installedVersion)
        let update = try await service.perform(confirmation)

        XCTAssertTrue(update.changed)
        XCTAssertEqual(update.status, installed.status)
        XCTAssertEqual(platform.installExpectations(), [initial])
        await assertServiceError(.confirmationUnavailable) {
            _ = try await service.perform(confirmation)
        }
    }

    func testWrongActionUnmanagedAndNewerTargetsAreRefusedBeforeMutation() async {
        let cases: [
            (
                CLIInstallationDisposition,
                CLIInstallationAction,
                CLIInstallerServiceError
            )
        ] = [
            (.absent, .upgrade, .confirmationChanged),
            (.unmanaged, .install, .unmanagedDestination),
            (
                .unsafe(.symbolicLink),
                .install,
                .unsafeDestination(.symbolicLink)
            ),
            (
                .managed(
                    CLIManagedInstallation(
                        version: CLIVersion("3.0.0")!,
                        sha256: Data(repeating: 3, count: 32),
                        relation: .newer
                    )
                ),
                .upgrade,
                .downgradeRefused
            ),
        ]
        for item in cases {
            let platform = RecordingCLIInstallerPlatform(
                observation: observed(item.0)
            )
            let service = CLIInstallerService(platform: platform)
            await assertServiceError(item.2) {
                _ = try await service.prepare(item.1)
            }
            XCTAssertTrue(platform.installExpectations().isEmpty)
            XCTAssertTrue(platform.uninstallExpectations().isEmpty)
        }
    }

    func testDiscardAndCloseInvalidatePreparedAuthority() async throws {
        let service = CLIInstallerService(
            platform: RecordingCLIInstallerPlatform(
                observation: observed(.absent)
            )
        )
        let discarded = try await service.prepare(.install)
        await service.discard(discarded)
        await assertServiceError(.confirmationUnavailable) {
            _ = try await service.perform(discarded)
        }

        let pending = try await service.prepare(.install)
        await service.close()
        await assertServiceError(.confirmationUnavailable) {
            _ = try await service.perform(pending)
        }
        await assertServiceError(.confirmationUnavailable) {
            _ = try await service.loadStatus()
        }
    }

    func testPlatformUncertaintyConsumesConfirmationWithoutRetry() async throws {
        let platform = RecordingCLIInstallerPlatform(
            observation: observed(.absent),
            installResult: .failure(.outcomeUnknown)
        )
        let service = CLIInstallerService(platform: platform)
        let confirmation = try await service.prepare(.install)

        await assertServiceError(.outcomeUnknown) {
            _ = try await service.perform(confirmation)
        }
        await assertServiceError(.confirmationUnavailable) {
            _ = try await service.perform(confirmation)
        }
        XCTAssertEqual(platform.installExpectations().count, 1)
    }

    func testUninstallAcceptsEveryManagedVersionButNothingElse() async throws {
        let initial = observed(
            .managed(
                CLIManagedInstallation(
                    version: CLIVersion("3.0.0")!,
                    sha256: Data(repeating: 3, count: 32),
                    relation: .newer
                )
            )
        )
        let absent = observed(.absent)
        let platform = RecordingCLIInstallerPlatform(
            observation: initial,
            uninstallResult: .success(absent)
        )
        let service = CLIInstallerService(platform: platform)

        let confirmation = try await service.prepare(.uninstall)
        let update = try await service.perform(confirmation)

        XCTAssertEqual(update.status.disposition, .absent)
        XCTAssertEqual(platform.uninstallExpectations(), [initial])
    }

    private func observed(
        _ disposition: CLIInstallationDisposition
    ) -> CLIInstallerObservedState {
        let metadata = CLIBundledMetadata(
            version: CLIVersion("2.0.0")!,
            databaseSchemaVersion: 16,
            snapshotFormatVersion: 1,
            sha256: Data(repeating: 2, count: 32)
        )
        let file = CLIInstallerFileEvidence(
            device: 1,
            inode: 2,
            size: 96,
            modificationSeconds: 3,
            modificationNanoseconds: 4,
            mode: 0o100755,
            owner: 501,
            linkCount: 1,
            sha256: metadata.sha256
        )
        let signature = CLIStaticSignatureIdentity(
            signingIdentifier: "test.dux.cli.debug",
            teamIdentifier: nil,
            isAdHoc: true
        )
        let target: CLIInstallerTargetEvidence = switch disposition {
        case .absent:
            .absent
        case let .managed(installed):
            .managed(
                file: file,
                marker: CLIInstallerManagedMarker(
                    version: installed.version,
                    sha256: installed.sha256
                ),
                signature: signature
            )
        case .unmanaged:
            .unmanaged(file)
        case let .unsafe(reason):
            .unsafe(reason, file)
        }
        return CLIInstallerObservedState(
            status: CLIInstallationStatus(
                bundled: metadata,
                disposition: disposition,
                pathEnvironment: .included
            ),
            bundledEvidence: CLIInstallerBundledEvidence(
                file: file,
                metadata: metadata,
                signature: signature
            ),
            targetEvidence: target
        )
    }
}

final class DarwinCLIInstallerPlatformTests: XCTestCase {
    func testMissingInstallDirectoriesAreObservedAbsentWithoutCreation() throws {
        try withFixture(version: "1.0.0", seed: 1) { fixture in
            let state = try fixture.platform.observe()

            XCTAssertEqual(state.status.disposition, .absent)
            XCTAssertFalse(
                FileManager.default.fileExists(
                    atPath: fixture.home.appendingPathComponent(".local").path
                )
            )
        }
    }

    func testInstallUpgradeAndManagedOnlyUninstallRoundTrip() async throws {
        try await withAsyncFixture(version: "1.0.0", seed: 1) { fixture in
            let firstService = CLIInstallerService(platform: fixture.platform)
            let install = try await firstService.prepare(.install)
            let installed = try await firstService.perform(install)
            guard case let .managed(first) = installed.status.disposition else {
                return XCTFail("Expected managed installation")
            }
            XCTAssertEqual(first.version, CLIVersion("1.0.0"))
            XCTAssertEqual(first.relation, .current)

            try fixture.replaceResources(version: "2.0.0", seed: 2)
            let upgradedPlatform = fixture.makePlatform()
            let upgradedService = CLIInstallerService(platform: upgradedPlatform)
            let beforeUpgrade = try await upgradedService.loadStatus()
            guard case let .managed(older) = beforeUpgrade.disposition else {
                return XCTFail("Expected older managed installation")
            }
            XCTAssertEqual(older.relation, .older)

            let upgrade = try await upgradedService.prepare(.upgrade)
            let upgraded = try await upgradedService.perform(upgrade)
            guard case let .managed(current) = upgraded.status.disposition else {
                return XCTFail("Expected upgraded managed installation")
            }
            XCTAssertEqual(current.version, CLIVersion("2.0.0"))
            XCTAssertEqual(current.relation, .current)

            let uninstall = try await upgradedService.prepare(.uninstall)
            let removed = try await upgradedService.perform(uninstall)
            XCTAssertEqual(removed.status.disposition, .absent)
            XCTAssertFalse(
                FileManager.default.fileExists(atPath: fixture.destination.path)
            )
        }
    }

    func testUnmanagedTargetIsObservedButNeverReplacedOrRemoved() async throws {
        try await withAsyncFixture(version: "1.0.0", seed: 1) { fixture in
            try fixture.createInstallDirectory()
            let bytes = fixture.fatBinary(seed: 9)
            XCTAssertTrue(
                FileManager.default.createFile(
                    atPath: fixture.destination.path,
                    contents: bytes
                )
            )
            XCTAssertEqual(chmod(fixture.destination.path, 0o755), 0)
            let original = try Data(contentsOf: fixture.destination)
            let service = CLIInstallerService(platform: fixture.platform)

            let status = try await service.loadStatus()
            XCTAssertEqual(status.disposition, .unmanaged)
            await assertServiceError(.unmanagedDestination) {
                _ = try await service.prepare(.install)
            }
            await assertServiceError(.unmanagedDestination) {
                _ = try await service.prepare(.uninstall)
            }
            XCTAssertEqual(try Data(contentsOf: fixture.destination), original)
        }
    }

    func testSymlinkAndHardLinkTargetsAreObservationOnly() async throws {
        try await withAsyncFixture(version: "1.0.0", seed: 1) { fixture in
            try fixture.createInstallDirectory()
            let outside = fixture.home.appendingPathComponent("outside")
            XCTAssertTrue(
                FileManager.default.createFile(
                    atPath: outside.path,
                    contents: fixture.fatBinary(seed: 8)
                )
            )
            XCTAssertEqual(chmod(outside.path, 0o755), 0)
            try FileManager.default.createSymbolicLink(
                at: fixture.destination,
                withDestinationURL: outside
            )
            let symlink = try await CLIInstallerService(
                platform: fixture.platform
            ).loadStatus()
            XCTAssertEqual(symlink.disposition, .unsafe(.symbolicLink))

            // DUX-DESTRUCTIVE: allow=test-cli-installer-fixture-remove -- resets only this test's temporary symlink before constructing the hard-link fixture
            try FileManager.default.removeItem(at: fixture.destination)
            XCTAssertEqual(link(outside.path, fixture.destination.path), 0)
            let hardLink = try await CLIInstallerService(
                platform: fixture.platform
            ).loadStatus()
            XCTAssertEqual(hardLink.disposition, .unsafe(.hardLinked))
        }
    }

    func testUnsafeInstallDirectoryIsRejectedAndNeverRepaired() async throws {
        try await withAsyncFixture(version: "1.0.0", seed: 1) { fixture in
            try fixture.createInstallDirectory()
            let bin = fixture.destination.deletingLastPathComponent()
            XCTAssertEqual(chmod(bin.path, 0o777), 0)

            await assertServiceError(.unsafeInstallDirectory) {
                _ = try await CLIInstallerService(
                    platform: fixture.platform
                ).loadStatus()
            }
            var value = stat()
            XCTAssertEqual(lstat(bin.path, &value), 0)
            XCTAssertNotEqual(value.st_mode & S_IWOTH, 0)
        }
    }

    func testStrictMetadataRejectsUnknownKeysAndArchitectureMismatch() async throws {
        try await withAsyncFixture(version: "1.0.0", seed: 1) { fixture in
            let original = try Data(contentsOf: fixture.metadata)
            var object = try XCTUnwrap(
                JSONSerialization.jsonObject(with: original) as? [String: Any]
            )
            object["unknown"] = true
            XCTAssertTrue(
                try FileManager.default.createFile(
                    atPath: fixture.metadata.path,
                    contents: JSONSerialization.data(withJSONObject: object)
                )
            )
            await assertServiceError(.invalidMetadata) {
                _ = try await CLIInstallerService(
                    platform: fixture.platform
                ).loadStatus()
            }

            try fixture.replaceResources(version: "1.0.0", seed: 1)
            var invalidBinary = fixture.fatBinary(seed: 1)
            invalidBinary[11] = 1
            XCTAssertTrue(
                FileManager.default.createFile(
                    atPath: fixture.binary.path,
                    contents: invalidBinary
                )
            )
            XCTAssertEqual(chmod(fixture.binary.path, 0o755), 0)
            await assertServiceError(.invalidArchitecture) {
                _ = try await CLIInstallerService(
                    platform: fixture.platform
                ).loadStatus()
            }
        }
    }

    private func withFixture(
        version: String,
        seed: UInt8,
        body: (CLIInstallerFixture) throws -> Void
    ) throws {
        let fixture = try CLIInstallerFixture(version: version, seed: seed)
        defer {
            // DUX-DESTRUCTIVE: allow=test-cli-installer-fixture-cleanup -- removes only the UUID-qualified temporary home/resources fixture owned by this test
            try? FileManager.default.removeItem(at: fixture.root)
        }
        try body(fixture)
    }

    private func withAsyncFixture(
        version: String,
        seed: UInt8,
        body: (CLIInstallerFixture) async throws -> Void
    ) async throws {
        let fixture = try CLIInstallerFixture(version: version, seed: seed)
        defer {
            // DUX-DESTRUCTIVE: allow=test-cli-installer-async-fixture-cleanup -- removes only the UUID-qualified temporary home/resources fixture owned by this test
            try? FileManager.default.removeItem(at: fixture.root)
        }
        try await body(fixture)
    }
}

private final class RecordingCLIInstallerPlatform:
    CLIInstallerPlatform, @unchecked Sendable
{
    private let lock = NSLock()
    private var current: CLIInstallerObservedState
    private var installResult: Result<
        CLIInstallerObservedState,
        CLIInstallerServiceError
    >
    private var uninstallResult: Result<
        CLIInstallerObservedState,
        CLIInstallerServiceError
    >
    private var installExpected: [CLIInstallerObservedState] = []
    private var uninstallExpected: [CLIInstallerObservedState] = []

    init(
        observation: CLIInstallerObservedState,
        installResult: Result<
            CLIInstallerObservedState,
            CLIInstallerServiceError
        >? = nil,
        uninstallResult: Result<
            CLIInstallerObservedState,
            CLIInstallerServiceError
        >? = nil
    ) {
        current = observation
        self.installResult = installResult ?? .success(observation)
        self.uninstallResult = uninstallResult ?? .success(observation)
    }

    func observe() throws -> CLIInstallerObservedState {
        lock.withLock { current }
    }

    func install(
        expected: CLIInstallerObservedState
    ) throws -> CLIInstallerObservedState {
        try lock.withLock {
            installExpected.append(expected)
            let value = try installResult.get()
            current = value
            return value
        }
    }

    func uninstall(
        expected: CLIInstallerObservedState
    ) throws -> CLIInstallerObservedState {
        try lock.withLock {
            uninstallExpected.append(expected)
            let value = try uninstallResult.get()
            current = value
            return value
        }
    }

    func installExpectations() -> [CLIInstallerObservedState] {
        lock.withLock { installExpected }
    }

    func uninstallExpectations() -> [CLIInstallerObservedState] {
        lock.withLock { uninstallExpected }
    }
}

private final class CLIInstallerFixture: @unchecked Sendable {
    let root: URL
    let home: URL
    let resources: URL
    let binary: URL
    let metadata: URL
    let signature = CLIStaticSignatureIdentity(
        signingIdentifier: "test.dux.cli.debug",
        teamIdentifier: nil,
        isAdHoc: true
    )

    var destination: URL {
        home
            .appendingPathComponent(".local", isDirectory: true)
            .appendingPathComponent("bin", isDirectory: true)
            .appendingPathComponent("dux")
    }

    var platform: DarwinCLIInstallerPlatform {
        makePlatform()
    }

    init(version: String, seed: UInt8) throws {
        root = FileManager.default.temporaryDirectory
            .appendingPathComponent(
                "dux-cli-installer-tests-\(UUID().uuidString)",
                isDirectory: true
            )
        home = root.appendingPathComponent("home", isDirectory: true)
        resources = root.appendingPathComponent("resources", isDirectory: true)
        binary = resources.appendingPathComponent("dux-cli-bundled")
        metadata = resources.appendingPathComponent(
            "dux-cli-bundled-metadata.json"
        )
        try FileManager.default.createDirectory(
            at: home,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        try FileManager.default.createDirectory(
            at: resources,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        try replaceResources(version: version, seed: seed)
    }

    func replaceResources(version: String, seed: UInt8) throws {
        let bytes = fatBinary(seed: seed)
        XCTAssertTrue(
            FileManager.default.createFile(
                atPath: binary.path,
                contents: bytes,
                attributes: [.posixPermissions: 0o755]
            )
        )
        XCTAssertEqual(chmod(binary.path, 0o755), 0)
        let digest = Data(SHA256.hash(data: bytes))
        let object: [String: Any] = [
            "record_version": 1,
            "product": "dux-cli",
            "version": version,
            "database_schema_version": 16,
            "snapshot_format_version": 1,
            "sha256": digest.map { String(format: "%02x", $0) }.joined(),
            "architectures": ["arm64", "x86_64"],
        ]
        XCTAssertTrue(
            try FileManager.default.createFile(
                atPath: metadata.path,
                contents: JSONSerialization.data(
                    withJSONObject: object,
                    options: [.sortedKeys]
                ),
                attributes: [.posixPermissions: 0o600]
            )
        )
    }

    func createInstallDirectory() throws {
        try FileManager.default.createDirectory(
            at: destination.deletingLastPathComponent(),
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
    }

    func makePlatform() -> DarwinCLIInstallerPlatform {
        DarwinCLIInstallerPlatform(
            resources: FixtureResources(
                binary: binary,
                metadata: metadata,
                application: resources
            ),
            account: FixtureAccount(
                home: home,
                userID: getuid(),
                path: home
                    .appendingPathComponent(".local/bin")
                    .path + ":/usr/bin"
            ),
            signatureInspector: FixtureSignatureInspector(
                applicationURL: resources,
                applicationIdentity: CLIStaticSignatureIdentity(
                    signingIdentifier: "test.dux.debug",
                    teamIdentifier: nil,
                    isAdHoc: true
                ),
                cliIdentity: signature
            ),
            signaturePolicy: CLIProductSignaturePolicy(
                appBundleIdentifier: "test.dux",
                permitsAdHocDevelopment: true
            )
        )
    }

    func fatBinary(seed: UInt8) -> Data {
        var bytes: [UInt8] = []
        appendUInt32(0xCAFE_BABE, to: &bytes)
        appendUInt32(2, to: &bytes)
        appendUInt32(0x0100_000C, to: &bytes)
        appendUInt32(0, to: &bytes)
        appendUInt32(64, to: &bytes)
        appendUInt32(16, to: &bytes)
        appendUInt32(0, to: &bytes)
        appendUInt32(0x0100_0007, to: &bytes)
        appendUInt32(0, to: &bytes)
        appendUInt32(80, to: &bytes)
        appendUInt32(16, to: &bytes)
        appendUInt32(0, to: &bytes)
        bytes.append(contentsOf: repeatElement(0, count: 16))
        bytes.append(contentsOf: repeatElement(seed, count: 16))
        bytes.append(contentsOf: repeatElement(seed &+ 1, count: 16))
        return Data(bytes)
    }

    private func appendUInt32(
        _ value: UInt32,
        to bytes: inout [UInt8]
    ) {
        bytes.append(UInt8((value >> 24) & 0xFF))
        bytes.append(UInt8((value >> 16) & 0xFF))
        bytes.append(UInt8((value >> 8) & 0xFF))
        bytes.append(UInt8(value & 0xFF))
    }
}

private struct FixtureResources: CLIInstallerResourceProviding {
    let binary: URL
    let metadata: URL
    let application: URL

    func bundledBinaryURL() -> URL? { binary }
    func bundledMetadataURL() -> URL? { metadata }
    func runningApplicationURL() -> URL { application }
}

private struct FixtureAccount: CLIInstallerAccountProviding {
    let home: URL
    let userID: uid_t
    let path: String?

    func currentHomeDirectory() throws -> URL { home }
    func currentUserID() -> uid_t { userID }
    func inheritedPath() -> String? { path }
}

private struct FixtureSignatureInspector: CLIStaticSignatureInspecting {
    let applicationURL: URL
    let applicationIdentity: CLIStaticSignatureIdentity
    let cliIdentity: CLIStaticSignatureIdentity

    func inspect(at url: URL) throws -> CLIStaticSignatureIdentity {
        url == applicationURL ? applicationIdentity : cliIdentity
    }
}

private func assertServiceError(
    _ expected: CLIInstallerServiceError,
    operation: () async throws -> some Any,
    file: StaticString = #filePath,
    line: UInt = #line
) async {
    do {
        _ = try await operation()
        XCTFail("Expected \(expected)", file: file, line: line)
    } catch let error as CLIInstallerServiceError {
        XCTAssertEqual(error, expected, file: file, line: line)
    } catch {
        XCTFail("Unexpected error: \(error)", file: file, line: line)
    }
}
