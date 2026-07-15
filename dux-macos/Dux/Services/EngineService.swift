import Dispatch
import Foundation

protocol EngineServing: Sendable {
    func loadSmokeResult(bytes: UInt64) async throws -> EngineSmokeResult
}

struct EngineService: EngineServing, Sendable {
    private let queue = DispatchQueue(
        label: "se.mjukis.dux.spike.engine",
        qos: .userInitiated
    )
    private let engine: DuxEngine

    init(engine: DuxEngine = DuxEngine()) {
        self.engine = engine
    }

    func loadSmokeResult(bytes: UInt64) async throws -> EngineSmokeResult {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                let executedOffMainThread = !Thread.isMainThread
                precondition(executedOffMainThread, "Blocking FFI work reached the main thread")

                do {
                    let version = try engine.libraryVersion()
                    let size = try engine.formatSize(bytes: bytes)

                    continuation.resume(
                        returning: EngineSmokeResult(
                            libraryVersion: version.libraryVersion,
                            ffiContractVersion: version.ffiContractVersion,
                            bytes: size.bytes,
                            displaySize: size.display,
                            executedOffMainThread: executedOffMainThread
                        )
                    )
                } catch let error as EngineError {
                    switch error {
                    case .Closed:
                        continuation.resume(throwing: EngineServiceError.closed)
                    }
                } catch {
                    continuation.resume(
                        throwing: EngineServiceError.unexpected(String(describing: error))
                    )
                }
            }
        }
    }

    func close() async -> Bool {
        await withCheckedContinuation { continuation in
            queue.async {
                continuation.resume(returning: engine.close())
            }
        }
    }
}
