struct EngineStatus: Equatable, Sendable {
    let libraryVersion: String
    let ffiContractVersion: UInt32
    let databaseSchemaVersion: UInt32
    let snapshotFormatVersion: UInt32
    let executedOffMainThread: Bool

    init(
        libraryVersion: String,
        ffiContractVersion: UInt32,
        databaseSchemaVersion: UInt32 = 0,
        snapshotFormatVersion: UInt32 = 0,
        executedOffMainThread: Bool
    ) {
        self.libraryVersion = libraryVersion
        self.ffiContractVersion = ffiContractVersion
        self.databaseSchemaVersion = databaseSchemaVersion
        self.snapshotFormatVersion = snapshotFormatVersion
        self.executedOffMainThread = executedOffMainThread
    }
}

enum EngineServiceError: Error, Equatable, Sendable {
    case closed
    case invalidCapacityObservation
    case conflictingCapacityObservation
    case supersededCapacityObservation
    case retryable
    case unavailable
    case unexpected(String)
}

enum EngineConnectionState: Equatable {
    case idle
    case loading
    case loaded(EngineStatus)
    case failed(EngineServiceError)
}
