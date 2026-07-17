import Darwin
import Foundation

protocol StorageAccessProbing: Sendable {
    func probe() async throws -> StorageAccessEvidence
}

enum StorageAccessLocationObservation: Sendable {
    case readable
    case unreadable
    case unobserved
}

enum StorageAccessProbeError: Error {
    case invalidEvidence
}

struct StorageAccessProbeService: StorageAccessProbing, Sendable {
    private static let relativeLocations = [
        "Library/Mail",
        "Library/Messages",
        "Library/Safari",
    ]

    private let homeDirectory: URL
    private let observe: @Sendable (URL) -> StorageAccessLocationObservation
    private let now: @Sendable () -> Date

    init(homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser) {
        self.init(
            homeDirectory: homeDirectory,
            observe: Self.observeLocation,
            now: Date.init
        )
    }

    init(
        homeDirectory: URL,
        observe: @escaping @Sendable (URL) -> StorageAccessLocationObservation,
        now: @escaping @Sendable () -> Date
    ) {
        self.homeDirectory = homeDirectory
        self.observe = observe
        self.now = now
    }

    func probe() async throws -> StorageAccessEvidence {
        let homeDirectory = homeDirectory
        let observe = observe
        let now = now
        return try await Task.detached(priority: .utility) {
            var readable: UInt8 = 0
            var unreadable: UInt8 = 0
            var unobserved: UInt8 = 0
            for relativeLocation in Self.relativeLocations {
                let location = homeDirectory.appending(
                    path: relativeLocation,
                    directoryHint: .isDirectory
                )
                switch observe(location) {
                case .readable: readable += 1
                case .unreadable: unreadable += 1
                case .unobserved: unobserved += 1
                }
            }
            guard let evidence = StorageAccessEvidence(
                readableLocationCount: readable,
                unreadableLocationCount: unreadable,
                unobservedLocationCount: unobserved,
                observedAt: now()
            ) else {
                throw StorageAccessProbeError.invalidEvidence
            }
            return evidence
        }.value
    }

    private static func observeLocation(_ location: URL) -> StorageAccessLocationObservation {
        let descriptor = location.path.withCString {
            Darwin.open($0, O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        }
        guard descriptor >= 0 else {
            return errno == ENOENT || errno == ENOTDIR ? .unobserved : .unreadable
        }
        Darwin.close(descriptor)
        return .readable
    }
}
