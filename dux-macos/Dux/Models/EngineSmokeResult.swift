struct EngineSmokeResult: Equatable, Sendable {
    let libraryVersion: String
    let ffiContractVersion: UInt32
    let bytes: UInt64
    let displaySize: String
    let executedOffMainThread: Bool
}

enum EngineServiceError: Error, Equatable, Sendable {
    case closed
    case unexpected(String)
}

enum EngineSmokeState: Equatable {
    case idle
    case loading
    case loaded(EngineSmokeResult)
    case failed(EngineServiceError)
}
