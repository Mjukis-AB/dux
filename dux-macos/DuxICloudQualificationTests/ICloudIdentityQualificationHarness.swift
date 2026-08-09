import Darwin
import Foundation
import Security

enum ICloudIdentityQualificationHarnessError: Error, Equatable {
    case gateRejected
    case unsafeFixture
    case unsafePrivateState
    case stateUnavailable
    case observationChanged
    case fixtureChanged
    case evidenceUnavailable
    case expectationMismatch
}

enum ICloudIdentityQualificationOptIn: Equatable {
    case skipped
    case enabled
}

struct ICloudIdentityQualificationHarness {
    static let observationCount = 3

    static func optIn(
        environment: [String: String]
    ) throws -> ICloudIdentityQualificationOptIn {
        guard environment["DUX_ICLOUD_DESTRUCTIVE_TESTS"] == nil else {
            throw ICloudIdentityQualificationHarnessError.gateRejected
        }
        return environment["DUX_ICLOUD_REAL_DEVICE_TESTS"] == "1"
            ? .enabled
            : .skipped
    }

    static func run(environment: [String: String]) throws {
        guard try optIn(environment: environment) == .enabled else {
            throw ICloudIdentityQualificationHarnessError.gateRejected
        }
        let configuration = try Configuration(environment: environment)
        let fixture = try FixtureBoundary(configuration: configuration)
        let privateState = try PrivateStateStore(
            path: configuration.privateStateDirectory,
            fixtureRoot: fixture.root,
            accountLabel: configuration.accountLabel,
            fixtureLabel: configuration.fixtureLabel
        )
        let key = try privateState.loadOrCreateKey()
        let baseline: FoundationICloudQualificationIdentityReference? =
            if configuration.phase == "baseline" {
                nil
            } else {
                try privateState.loadReference(required: true)
            }

        let before = try fixture.snapshot()
        let reader = FoundationICloudLocalCopyRawFactReader()
        var reads: [FoundationICloudQualificationRead] = []
        reads.reserveCapacity(observationCount)
        for _ in 0..<observationCount {
            reads.append(
                try reader.readForQualification(
                    at: fixture.url,
                    key: key,
                    baseline: baseline
                )
            )
        }
        guard reads.dropFirst().allSatisfy({ $0 == reads[0] }) else {
            throw ICloudIdentityQualificationHarnessError.observationChanged
        }
        if configuration.phase == "baseline" {
            try requireUsableBaseline(reads[0].privateIdentityReference)
        }
        let after = try fixture.snapshot()
        guard before.statIdentity == after.statIdentity else {
            throw ICloudIdentityQualificationHarnessError.fixtureChanged
        }
        guard before.allocatedBytes == after.allocatedBytes else {
            throw ICloudIdentityQualificationHarnessError.fixtureChanged
        }

        let evidence = Evidence(
            configuration: configuration,
            reads: reads
        )
        guard evidence.matches(
            expectedSyncEligibility: configuration.expectedSyncEligibility,
            expectedIdentityReadiness: configuration.expectedIdentityReadiness
        ) else {
            throw ICloudIdentityQualificationHarnessError.expectationMismatch
        }
        if configuration.phase == "baseline" {
            try privateState.storeOrVerifyReference(
                reads[0].privateIdentityReference
            )
        }
        try EvidenceWriter.writeFresh(
            evidence,
            to: configuration.evidenceOutput,
            outside: fixture.root
        )
    }

    static func validateEnvironmentForTesting(
        _ environment: [String: String]
    ) throws {
        _ = try Configuration(environment: environment)
    }

    static func currentPlatformEnvironmentForTesting() throws -> [String: String] {
        let platform = try PlatformFacts.current()
        return [
            "DUX_ICLOUD_EXPECTED_OS_VERSION": platform.productVersion,
            "DUX_ICLOUD_EXPECTED_OS_BUILD": platform.build,
            "DUX_ICLOUD_EXPECTED_ARCHITECTURE": platform.architecture,
        ]
    }

    static func policyResultsForTesting(
        _ facts: FoundationICloudLocalCopyFacts
    ) -> (syncEligibility: String, identityReadiness: String) {
        (
            Evidence.syncEligibility(facts),
            Evidence.identityReadiness(facts)
        )
    }

    static func requireUsableBaseline(
        _ reference: FoundationICloudQualificationIdentityReference
    ) throws {
        guard reference.isComplete else {
            throw ICloudIdentityQualificationHarnessError.observationChanged
        }
    }
}

private extension ICloudIdentityQualificationHarness {
    struct Configuration {
        let sourceCommit: String
        let schemaSHA256: String
        let runStartedAtUnixNS: Int64
        let productVersion: String
        let build: String
        let architecture: String
        let accountLabel: String
        let fixtureLabel: String
        let phase: String
        let network: String
        let expectedSyncEligibility: String
        let expectedIdentityReadiness: String
        let fixtureRoot: String
        let fixturePath: String
        let privateStateDirectory: String
        let evidenceOutput: String

        init(environment: [String: String]) throws {
            guard
                environment["DUX_ICLOUD_REAL_DEVICE_TESTS"] == "1",
                environment["DUX_ICLOUD_DISPOSABLE_ACCOUNT_CONFIRMED"] == "YES",
                environment["DUX_ICLOUD_DISPOSABLE_FIXTURE_CONFIRMED"] == "YES",
                environment["DUX_ICLOUD_EXTERNAL_CONTENT_REFERENCE_CONFIRMED"] == "YES",
                environment["DUX_ICLOUD_EXCLUSIVE_SERIALIZATION"] == "1",
                environment["DUX_ICLOUD_DESTRUCTIVE_TESTS"] == nil,
                let sourceCommit = environment["DUX_ICLOUD_SOURCE_COMMIT"],
                sourceCommit.wholeMatch(of: /^[0-9a-f]{40}$/) != nil,
                let schemaSHA256 = environment["DUX_ICLOUD_EVIDENCE_SCHEMA_SHA256"],
                schemaSHA256.wholeMatch(of: /^[0-9a-f]{64}$/) != nil,
                let runString = environment["DUX_ICLOUD_RUN_STARTED_AT_UNIX_NS"],
                let runStartedAtUnixNS = Int64(runString),
                runStartedAtUnixNS > 0,
                let expectedVersion = environment["DUX_ICLOUD_EXPECTED_OS_VERSION"],
                expectedVersion.wholeMatch(of: /^[0-9]+(?:\.[0-9]+){1,3}$/) != nil,
                let expectedBuild = environment["DUX_ICLOUD_EXPECTED_OS_BUILD"],
                expectedBuild.wholeMatch(of: /^[A-Za-z0-9._-]{1,31}$/) != nil,
                let expectedArchitecture =
                    environment["DUX_ICLOUD_EXPECTED_ARCHITECTURE"],
                ["arm64", "x86_64"].contains(expectedArchitecture),
                let accountLabel = environment["DUX_ICLOUD_ACCOUNT_LABEL"],
                accountLabel.wholeMatch(of: /^acct-[A-Z2-7]{12}$/) != nil,
                let fixtureLabel = environment["DUX_ICLOUD_FIXTURE_LABEL"],
                fixtureLabel.wholeMatch(of: /^fixture-[A-Z2-7]{12}$/) != nil,
                let phase = environment["DUX_ICLOUD_PHASE"],
                Self.phases.contains(phase),
                let network = environment["DUX_ICLOUD_NETWORK"],
                ["online", "offline", "restored"].contains(network),
                let expectedSyncEligibility =
                    environment["DUX_ICLOUD_EXPECTED_SYNC_ELIGIBILITY"],
                Self.syncEligibilityResults.contains(expectedSyncEligibility),
                let expectedIdentityReadiness =
                    environment["DUX_ICLOUD_EXPECTED_IDENTITY_READINESS"],
                Self.identityReadinessResults.contains(expectedIdentityReadiness),
                let fixtureRoot = Self.absolutePath(
                    environment["DUX_ICLOUD_FIXTURE_ROOT"]
                ),
                let fixturePath = Self.absolutePath(
                    environment["DUX_ICLOUD_FIXTURE_PATH"]
                ),
                let privateStateDirectory = Self.absolutePath(
                    environment["DUX_ICLOUD_PRIVATE_STATE_DIRECTORY"]
                ),
                let evidenceOutput = Self.absolutePath(
                    environment["DUX_ICLOUD_EVIDENCE_OUTPUT"]
                ),
                (phase != "network_offline" || network == "offline"),
                (phase != "network_restored" || network == "restored"),
                (phase != "account_changed" || expectedIdentityReadiness == "blocked")
            else {
                throw ICloudIdentityQualificationHarnessError.gateRejected
            }

            let actual = try PlatformFacts.current()
            guard actual.productVersion == expectedVersion,
                  actual.build == expectedBuild,
                  actual.architecture == expectedArchitecture
            else {
                throw ICloudIdentityQualificationHarnessError.gateRejected
            }
            self.sourceCommit = sourceCommit
            self.schemaSHA256 = schemaSHA256
            self.runStartedAtUnixNS = runStartedAtUnixNS
            productVersion = actual.productVersion
            build = actual.build
            architecture = actual.architecture
            self.accountLabel = accountLabel
            self.fixtureLabel = fixtureLabel
            self.phase = phase
            self.network = network
            self.expectedSyncEligibility = expectedSyncEligibility
            self.expectedIdentityReadiness = expectedIdentityReadiness
            self.fixtureRoot = fixtureRoot
            self.fixturePath = fixturePath
            self.privateStateDirectory = privateStateDirectory
            self.evidenceOutput = evidenceOutput
        }

        private static func absolutePath(_ value: String?) -> String? {
            guard let value, value.hasPrefix("/"), !value.contains("\0") else {
                return nil
            }
            return value
        }

        private static let syncEligibilityResults = ["eligible", "blocked"]
        private static let identityReadinessResults = ["ready", "blocked"]
        private static let phases = [
            "baseline",
            "process_restart",
            "post_reboot",
            "post_account_session",
            "metadata_refresh",
            "content_edit",
            "rename",
            "move_within_tree",
            "move_between_containers",
            "remote_download",
            "network_offline",
            "network_restored",
            "sync_paused",
            "sync_resumed",
            "sharing_enabled",
            "sharing_disabled",
            "account_changed",
        ]
    }

    struct PlatformFacts {
        let productVersion: String
        let build: String
        let architecture: String

        static func current() throws -> Self {
            var system = utsname()
            guard uname(&system) == 0 else {
                throw ICloudIdentityQualificationHarnessError.gateRejected
            }
            let architecture = withUnsafeBytes(of: &system.machine) { bytes in
                String(decoding: bytes.prefix { $0 != 0 }, as: UTF8.self)
            }
            return Self(
                productVersion: try sysctlString("kern.osproductversion"),
                build: try sysctlString("kern.osversion"),
                architecture: architecture
            )
        }

        private static func sysctlString(_ name: String) throws -> String {
            var size = 0
            guard sysctlbyname(name, nil, &size, nil, 0) == 0,
                  size > 1,
                  size <= 128
            else {
                throw ICloudIdentityQualificationHarnessError.gateRejected
            }
            var buffer = [CChar](repeating: 0, count: size)
            guard sysctlbyname(name, &buffer, &size, nil, 0) == 0 else {
                throw ICloudIdentityQualificationHarnessError.gateRejected
            }
            return String(
                decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) },
                as: UTF8.self
            )
        }
    }

    struct FixtureBoundary {
        struct StatIdentity: Equatable {
            let device: dev_t
            let inode: ino_t
            let mode: mode_t
            let links: nlink_t
            let owner: uid_t
            let size: off_t
            let modifiedSeconds: Int
            let modifiedNanoseconds: Int
        }

        struct Snapshot {
            let statIdentity: StatIdentity
            let allocatedBytes: UInt64
        }

        let root: String
        let url: URL

        init(configuration: Configuration) throws {
            let root = try SecurePaths.canonicalExisting(
                configuration.fixtureRoot
            )
            let path = try SecurePaths.canonicalExisting(
                configuration.fixturePath
            )
            guard root == configuration.fixtureRoot,
                  path == configuration.fixturePath,
                  SecurePaths.isICloudStorage(root),
                  path.hasPrefix(root + "/"),
                  !SecurePaths.contains(path: configuration.privateStateDirectory, root: root),
                  !SecurePaths.contains(path: root, root: configuration.privateStateDirectory),
                  !SecurePaths.contains(path: configuration.evidenceOutput, root: root)
            else {
                throw ICloudIdentityQualificationHarnessError.unsafeFixture
            }
            var rootStat = stat()
            guard lstat(root, &rootStat) == 0,
                  rootStat.st_mode & S_IFMT == S_IFDIR
            else {
                throw ICloudIdentityQualificationHarnessError.unsafeFixture
            }
            self.root = root
            url = URL(fileURLWithPath: path, isDirectory: false)
            _ = try snapshot()
        }

        func snapshot() throws -> Snapshot {
            var value = stat()
            guard lstat(url.path, &value) == 0,
                  value.st_mode & S_IFMT == S_IFREG,
                  value.st_nlink == 1,
                  value.st_uid == getuid(),
                  value.st_size >= 0,
                  value.st_blocks >= 0
            else {
                throw ICloudIdentityQualificationHarnessError.unsafeFixture
            }
            return Snapshot(
                statIdentity: StatIdentity(
                    device: value.st_dev,
                    inode: value.st_ino,
                    mode: value.st_mode,
                    links: value.st_nlink,
                    owner: value.st_uid,
                    size: value.st_size,
                    modifiedSeconds: value.st_mtimespec.tv_sec,
                    modifiedNanoseconds: value.st_mtimespec.tv_nsec
                ),
                allocatedBytes: UInt64(value.st_blocks) * 512
            )
        }
    }

    enum SecurePaths {
        static func canonicalExisting(_ path: String) throws -> String {
            var buffer = [CChar](repeating: 0, count: Int(PATH_MAX))
            let resolved = buffer.withUnsafeMutableBufferPointer { pointer in
                realpath(path, pointer.baseAddress)
            }
            guard resolved != nil else {
                throw ICloudIdentityQualificationHarnessError.unsafeFixture
            }
            return String(
                decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) },
                as: UTF8.self
            )
        }

        static func contains(path: String, root: String) -> Bool {
            path == root || path.hasPrefix(root + "/")
        }

        static func isApplicationStorage(_ path: String) -> Bool {
            let manager = FileManager.default
            let roots = [
                manager.urls(for: .applicationSupportDirectory, in: .userDomainMask)
                    .first?.appending(path: "Dux", directoryHint: .isDirectory).path,
                manager.urls(for: .applicationSupportDirectory, in: .userDomainMask)
                    .first?.appending(path: "DUX", directoryHint: .isDirectory).path,
                manager.urls(for: .cachesDirectory, in: .userDomainMask)
                    .first?.appending(path: "Dux", directoryHint: .isDirectory).path,
                manager.urls(for: .cachesDirectory, in: .userDomainMask)
                    .first?.appending(path: "DUX", directoryHint: .isDirectory).path,
                manager.urls(for: .cachesDirectory, in: .userDomainMask)
                    .first?.appending(path: "com.dux", directoryHint: .isDirectory).path,
                manager.homeDirectoryForCurrentUser
                    .appending(path: "Library/Containers", directoryHint: .isDirectory).path,
            ].compactMap { $0 }
            return roots.contains { contains(path: path, root: $0) }
        }

        static func isICloudStorage(_ path: String) -> Bool {
            let components = path.split(separator: "/")
            return components.contains("Mobile Documents")
                || components.contains("CloudStorage")
        }

        static func isQualificationSourceTree(_ path: String) -> Bool {
            let repositoryRoot = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent()
                .deletingLastPathComponent()
                .deletingLastPathComponent()
                .path
            return contains(path: path, root: repositoryRoot)
        }

        static func canonicalOutputParts(
            _ path: String,
            outside fixtureRoot: String
        ) throws -> (directory: String, name: String) {
            let url = URL(fileURLWithPath: path)
            let name = url.lastPathComponent
            guard name.wholeMatch(
                of: /^[A-Za-z0-9][A-Za-z0-9._-]{0,54}\.json$/
            ) != nil else {
                throw ICloudIdentityQualificationHarnessError.evidenceUnavailable
            }
            let requestedParent = url.deletingLastPathComponent().path
            let parent = try canonicalExisting(requestedParent)
            guard parent == requestedParent,
                  !contains(path: parent, root: fixtureRoot),
                  !contains(path: fixtureRoot, root: parent),
                  !isICloudStorage(parent),
                  !isApplicationStorage(parent),
                  !isQualificationSourceTree(parent)
            else {
                throw ICloudIdentityQualificationHarnessError.evidenceUnavailable
            }
            return (parent, name)
        }
    }

    final class PrivateStateStore {
        private let directoryFD: Int32
        private let referenceName: String

        init(
            path: String,
            fixtureRoot: String,
            accountLabel: String,
            fixtureLabel: String
        ) throws {
            let canonical = try SecurePaths.canonicalExisting(path)
            guard canonical == path,
                  !SecurePaths.contains(path: canonical, root: fixtureRoot),
                  !SecurePaths.contains(path: fixtureRoot, root: canonical),
                  !SecurePaths.isICloudStorage(canonical),
                  !SecurePaths.isApplicationStorage(canonical),
                  !SecurePaths.isQualificationSourceTree(canonical)
            else {
                throw ICloudIdentityQualificationHarnessError.unsafePrivateState
            }
            let fd = Darwin.open(
                canonical,
                O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
            )
            guard fd >= 0 else {
                throw ICloudIdentityQualificationHarnessError.unsafePrivateState
            }
            var value = stat()
            guard fstat(fd, &value) == 0,
                  value.st_mode & S_IFMT == S_IFDIR,
                  value.st_mode & 0o777 == 0o700,
                  value.st_uid == getuid()
            else {
                Darwin.close(fd)
                throw ICloudIdentityQualificationHarnessError.unsafePrivateState
            }
            directoryFD = fd
            referenceName = "\(accountLabel).\(fixtureLabel).reference-v1.json"
        }

        deinit {
            Darwin.close(directoryFD)
        }

        func loadOrCreateKey() throws -> Data {
            if let existing = try readPrivateFile(
                named: "key-v1.bin",
                maximumBytes: 32,
                missingAllowed: true
            ) {
                guard existing.count == 32 else {
                    throw ICloudIdentityQualificationHarnessError.stateUnavailable
                }
                return existing
            }
            var key = Data(count: 32)
            let status = key.withUnsafeMutableBytes { bytes in
                SecRandomCopyBytes(kSecRandomDefault, 32, bytes.baseAddress!)
            }
            guard status == errSecSuccess else {
                throw ICloudIdentityQualificationHarnessError.stateUnavailable
            }
            try createPrivateFile(named: "key-v1.bin", data: key)
            return key
        }

        func loadReference(
            required: Bool
        ) throws -> FoundationICloudQualificationIdentityReference? {
            guard let data = try readPrivateFile(
                named: referenceName,
                maximumBytes: 16 * 1024,
                missingAllowed: !required
            ) else {
                return nil
            }
            do {
                return try JSONDecoder().decode(
                    FoundationICloudQualificationIdentityReference.self,
                    from: data
                )
            } catch {
                throw ICloudIdentityQualificationHarnessError.stateUnavailable
            }
        }

        func storeOrVerifyReference(
            _ reference: FoundationICloudQualificationIdentityReference
        ) throws {
            if let existing = try loadReference(required: false) {
                guard existing == reference else {
                    throw ICloudIdentityQualificationHarnessError.stateUnavailable
                }
                return
            }
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            let data = try encoder.encode(reference)
            try createPrivateFile(named: referenceName, data: data)
        }

        private func readPrivateFile(
            named name: String,
            maximumBytes: Int,
            missingAllowed: Bool
        ) throws -> Data? {
            let fd = openat(directoryFD, name, O_RDONLY | O_CLOEXEC | O_NOFOLLOW)
            guard fd >= 0 else {
                if missingAllowed, errno == ENOENT {
                    return nil
                }
                throw ICloudIdentityQualificationHarnessError.stateUnavailable
            }
            defer { Darwin.close(fd) }
            var value = stat()
            guard fstat(fd, &value) == 0,
                  value.st_mode & S_IFMT == S_IFREG,
                  value.st_mode & 0o777 == 0o600,
                  value.st_uid == getuid(),
                  value.st_nlink == 1,
                  value.st_size >= 0,
                  value.st_size <= maximumBytes
            else {
                throw ICloudIdentityQualificationHarnessError.stateUnavailable
            }
            return try POSIXFile.readExactly(fd: fd, count: Int(value.st_size))
        }

        private func createPrivateFile(named name: String, data: Data) throws {
            let fd = openat(
                directoryFD,
                name,
                O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
                0o600
            )
            guard fd >= 0 else {
                throw ICloudIdentityQualificationHarnessError.stateUnavailable
            }
            defer { Darwin.close(fd) }
            try POSIXFile.verifyPrivateRegular(fd: fd)
            try POSIXFile.writeAll(fd: fd, data: data)
            guard fsync(fd) == 0, fsync(directoryFD) == 0 else {
                throw ICloudIdentityQualificationHarnessError.stateUnavailable
            }
        }
    }

    enum POSIXFile {
        static func verifyPrivateRegular(fd: Int32) throws {
            var value = stat()
            guard fstat(fd, &value) == 0,
                  value.st_mode & S_IFMT == S_IFREG,
                  value.st_mode & 0o777 == 0o600,
                  value.st_uid == getuid(),
                  value.st_nlink == 1
            else {
                throw ICloudIdentityQualificationHarnessError.stateUnavailable
            }
        }

        static func readExactly(fd: Int32, count: Int) throws -> Data {
            var data = Data(count: count)
            var offset = 0
            while offset < count {
                let readCount = data.withUnsafeMutableBytes { bytes in
                    Darwin.read(
                        fd,
                        bytes.baseAddress!.advanced(by: offset),
                        count - offset
                    )
                }
                if readCount < 0, errno == EINTR {
                    continue
                }
                guard readCount > 0 else {
                    throw ICloudIdentityQualificationHarnessError.stateUnavailable
                }
                offset += readCount
            }
            return data
        }

        static func writeAll(fd: Int32, data: Data) throws {
            var offset = 0
            while offset < data.count {
                let written = data.withUnsafeBytes { bytes in
                    Darwin.write(
                        fd,
                        bytes.baseAddress!.advanced(by: offset),
                        data.count - offset
                    )
                }
                if written < 0, errno == EINTR {
                    continue
                }
                guard written > 0 else {
                    throw ICloudIdentityQualificationHarnessError.stateUnavailable
                }
                offset += written
            }
        }
    }

    struct Evidence: Encodable {
        struct Source: Encodable {
            let repository_commit: String
            let schema_sha256: String
            let workspace_clean = true
        }

        struct Platform: Encodable {
            let product = "macOS"
            let product_version: String
            let build: String
            let architecture: String
        }

        struct Fixture: Encodable {
            let account_label: String
            let fixture_label: String
            let phase: String
            let network: String
        }

        struct Execution: Encodable {
            let run_started_at_unix_ns: Int64
            let observation_count = 3
            let consecutive_exact_observations = true
            let exclusive_serialization = true
            let read_only = true
            let production_eligible = false
        }

        struct Isolation: Encodable {
            let application_database_accessed = false
            let candidate_created = false
            let plan_created = false
            let journal_or_history_written = false
            let provider_command_issued = false
            let effect_attempted = false
            let fixture_metadata_mutated = false
            let fixture_content_mutated = false
            let content_read = false
            let account_mutated = false
            let network_mutated = false
        }

        struct Expectation: Encodable {
            let sync_eligibility: String
            let identity_readiness: String
        }

        struct Stability: Encodable, Equatable {
            let account: String
            let provider_domain: String
            let provider_item: String
            let generation: String
            let file_version: String
        }

        struct Continuity: Encodable, Equatable {
            let account: String
            let provider_domain: String
            let provider_item: String
            let generation: String
            let file_version: String
        }

        struct Observation: Encodable, Equatable {
            let sequence: Int
            let stability: Stability
            let continuity: Continuity
            let shared: String
            let sync_paused: String
            let sync_eligibility: String
            let identity_readiness: String
        }

        struct Invariants: Encodable {
            let stat_unchanged = true
            let allocation_unchanged = true
            let external_content_reference_confirmed = true
            let raw_identity_emitted = false
            let path_emitted = false
            let filename_emitted = false
            let content_or_hash_emitted = false
            let error_detail_emitted = false
            let persistent_identity_state_private = true
        }

        let schema_version = 1
        let protocol_contract = 58
        let source: Source
        let platform: Platform
        let fixture: Fixture
        let execution: Execution
        let isolation = Isolation()
        let expectation: Expectation
        let observations: [Observation]
        let invariants = Invariants()

        func matches(
            expectedSyncEligibility: String,
            expectedIdentityReadiness: String
        ) -> Bool {
            observations.allSatisfy { observation in
                observation.sync_eligibility == expectedSyncEligibility
                    && observation.identity_readiness == expectedIdentityReadiness
            }
        }

        init(
            configuration: Configuration,
            reads: [FoundationICloudQualificationRead]
        ) {
            source = Source(
                repository_commit: configuration.sourceCommit,
                schema_sha256: configuration.schemaSHA256
            )
            platform = Platform(
                product_version: configuration.productVersion,
                build: configuration.build,
                architecture: configuration.architecture
            )
            fixture = Fixture(
                account_label: configuration.accountLabel,
                fixture_label: configuration.fixtureLabel,
                phase: configuration.phase,
                network: configuration.network
            )
            execution = Execution(
                run_started_at_unix_ns: configuration.runStartedAtUnixNS
            )
            expectation = Expectation(
                sync_eligibility: configuration.expectedSyncEligibility,
                identity_readiness: configuration.expectedIdentityReadiness
            )
            observations = reads.enumerated().map { index, read in
                Self.observation(
                    sequence: index + 1,
                    read: read,
                    baselinePhase: configuration.phase == "baseline"
                )
            }
        }

        private static func observation(
            sequence: Int,
            read: FoundationICloudQualificationRead,
            baselinePhase: Bool
        ) -> Observation {
            let identity = read.facts.identityCapability
            return Observation(
                sequence: sequence,
                stability: Stability(
                    account: stability(identity.accountTokenStability),
                    provider_domain: stability(identity.domainIdentifierStability),
                    provider_item: stability(identity.providerItemIdentifierStability),
                    generation: stability(identity.itemGenerationStability),
                    file_version: stability(identity.fileVersionPersistentIDStability)
                ),
                continuity: Continuity(
                    account: continuity(
                        read.continuity.accountToken,
                        stability: identity.accountTokenStability,
                        baselinePhase: baselinePhase
                    ),
                    provider_domain: continuity(
                        read.continuity.fileProviderDomain,
                        stability: identity.domainIdentifierStability,
                        baselinePhase: baselinePhase
                    ),
                    provider_item: continuity(
                        read.continuity.fileProviderItem,
                        stability: identity.providerItemIdentifierStability,
                        baselinePhase: baselinePhase
                    ),
                    generation: continuity(
                        read.continuity.itemGeneration,
                        stability: identity.itemGenerationStability,
                        baselinePhase: baselinePhase
                    ),
                    file_version: continuity(
                        read.continuity.fileVersion,
                        stability: identity.fileVersionPersistentIDStability,
                        baselinePhase: baselinePhase
                    )
                ),
                shared: triState(read.facts.isShared),
                sync_paused: triState(read.facts.isSyncPaused),
                sync_eligibility: syncEligibility(read.facts),
                identity_readiness: identityReadiness(read.facts)
            )
        }

        private static func stability(
            _ value: FoundationICloudIdentityStability
        ) -> String {
            switch value {
            case .stable: "stable"
            case .unavailable: "unavailable"
            case .changed: "changed_during_read"
            case .unsupported: "unsupported"
            }
        }

        private static func continuity(
            _ value: FoundationICloudQualificationContinuity,
            stability: FoundationICloudIdentityStability,
            baselinePhase: Bool
        ) -> String {
            if baselinePhase, stability == .stable {
                return "baseline_recorded"
            }
            return switch value {
            case .sameAsBaseline: "same_as_baseline"
            case .changedSinceBaseline: "changed_from_baseline"
            case .changedDuringRead: "changed_during_read"
            case .unavailable: "unavailable"
            case .unsupported: "unsupported"
            case .noBaseline: "no_baseline"
            }
        }

        private static func triState(_ value: Bool?) -> String {
            switch value {
            case true: "yes"
            case false: "no"
            case nil: "unknown"
            }
        }

        fileprivate static func syncEligibility(
            _ facts: FoundationICloudLocalCopyFacts
        ) -> String {
            let eligible = facts.itemKind == .regularFile
                && facts.isUbiquitous == true
                && facts.isUploaded == true
                && facts.isUploading == false
                && facts.uploadingErrorPresence == .absent
                && facts.hasUnresolvedConflicts == false
                && facts.downloadStatus == .current
                && facts.downloadRequested == false
                && facts.isDownloading == false
                && facts.downloadingErrorPresence == .absent
                && facts.isExcludedFromSync == false
                && (facts.allocatedBytes ?? 0) > 0
            return eligible ? "eligible" : "blocked"
        }

        fileprivate static func identityReadiness(
            _ facts: FoundationICloudLocalCopyFacts
        ) -> String {
            let identity = facts.identityCapability
            let ready = identity.accountTokenStability == .stable
                && identity.domainIdentifierStability == .stable
                && identity.providerItemIdentifierStability == .stable
                && identity.itemGenerationStability == .stable
                && identity.fileVersionPersistentIDStability == .stable
                && facts.isShared == false
                && facts.isSyncPaused == false
            return ready ? "ready" : "blocked"
        }
    }

    enum EvidenceWriter {
        static func writeFresh(
            _ evidence: Evidence,
            to path: String,
            outside fixtureRoot: String
        ) throws {
            let parts = try SecurePaths.canonicalOutputParts(
                path,
                outside: fixtureRoot
            )
            let directoryFD = Darwin.open(
                parts.directory,
                O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW
            )
            guard directoryFD >= 0 else {
                throw ICloudIdentityQualificationHarnessError.evidenceUnavailable
            }
            defer { Darwin.close(directoryFD) }
            var directoryStat = stat()
            guard fstat(directoryFD, &directoryStat) == 0,
                  directoryStat.st_mode & S_IFMT == S_IFDIR,
                  directoryStat.st_mode & 0o777 == 0o700,
                  directoryStat.st_uid == getuid()
            else {
                throw ICloudIdentityQualificationHarnessError.evidenceUnavailable
            }

            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            var data = try encoder.encode(evidence)
            data.append(0x0A)
            let fd = openat(
                directoryFD,
                parts.name,
                O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
                0o600
            )
            guard fd >= 0 else {
                throw ICloudIdentityQualificationHarnessError.evidenceUnavailable
            }
            defer { Darwin.close(fd) }
            do {
                try POSIXFile.verifyPrivateRegular(fd: fd)
                try POSIXFile.writeAll(fd: fd, data: data)
                guard fsync(fd) == 0, fsync(directoryFD) == 0 else {
                    throw ICloudIdentityQualificationHarnessError.evidenceUnavailable
                }
            } catch {
                throw ICloudIdentityQualificationHarnessError.evidenceUnavailable
            }
        }
    }
}
