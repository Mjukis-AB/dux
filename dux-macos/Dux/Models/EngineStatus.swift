struct EngineStatus: Equatable, Sendable {
    let libraryVersion: String
    let ffiContractVersion: UInt32
    let executedOffMainThread: Bool
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
