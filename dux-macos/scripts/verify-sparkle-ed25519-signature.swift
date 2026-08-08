#!/usr/bin/env swift

import CryptoKit
import Darwin
import Foundation

private enum VerificationError: Error, CustomStringConvertible {
    case usage
    case invalidBase64(String)
    case invalidLength(String, expected: Int, actual: Int)
    case invalidFile(String)
    case oversizedFile(Int)
    case invalidSignature

    var description: String {
        switch self {
        case .usage:
            return "usage: verify-sparkle-ed25519-signature.swift --public-key-base64 KEY --signature-base64 SIGNATURE --file PATH"
        case let .invalidBase64(name):
            return "\(name) must be canonical base64"
        case let .invalidLength(name, expected, actual):
            return "\(name) must decode to exactly \(expected) bytes (found \(actual))"
        case let .invalidFile(reason):
            return "canary file is invalid: \(reason)"
        case let .oversizedFile(size):
            return "canary file exceeds the 16 MiB recovery-drill limit (found \(size) bytes)"
        case .invalidSignature:
            return "signature does not verify against the supplied public key"
        }
    }
}

private func decodeCanonicalBase64(
    _ encoded: String,
    name: String,
    expectedLength: Int
) throws -> Data {
    guard let decoded = Data(base64Encoded: encoded),
          decoded.base64EncodedString() == encoded
    else {
        throw VerificationError.invalidBase64(name)
    }
    guard decoded.count == expectedLength else {
        throw VerificationError.invalidLength(
            name,
            expected: expectedLength,
            actual: decoded.count
        )
    }
    return decoded
}

private func arguments() throws -> (publicKey: String, signature: String, file: String) {
    let raw = Array(CommandLine.arguments.dropFirst())
    guard raw.count == 6 else { throw VerificationError.usage }
    var values: [String: String] = [:]
    var index = 0
    while index < raw.count {
        let key = raw[index]
        guard ["--public-key-base64", "--signature-base64", "--file"].contains(key),
              values[key] == nil
        else {
            throw VerificationError.usage
        }
        values[key] = raw[index + 1]
        index += 2
    }
    guard let publicKey = values["--public-key-base64"],
          let signature = values["--signature-base64"],
          let file = values["--file"],
          !file.isEmpty
    else {
        throw VerificationError.usage
    }
    return (publicKey, signature, file)
}

do {
    let input = try arguments()
    let descriptor = open(input.file, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
    guard descriptor >= 0 else {
        throw VerificationError.invalidFile("path must open read-only without following a symlink")
    }
    defer { close(descriptor) }

    var status = stat()
    guard fstat(descriptor, &status) == 0 else {
        throw VerificationError.invalidFile("file metadata is unavailable")
    }
    guard status.st_mode & S_IFMT == S_IFREG, status.st_nlink == 1 else {
        throw VerificationError.invalidFile("path must be a single-link regular file")
    }
    let maximumSize = 16 * 1_024 * 1_024
    guard status.st_size >= 0, status.st_size <= maximumSize else {
        throw VerificationError.oversizedFile(Int(status.st_size))
    }

    let publicKeyBytes = try decodeCanonicalBase64(
        input.publicKey,
        name: "public key",
        expectedLength: 32
    )
    let signatureBytes = try decodeCanonicalBase64(
        input.signature,
        name: "signature",
        expectedLength: 64
    )
    let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: false)
    let payload = try handle.read(upToCount: maximumSize + 1) ?? Data()
    guard payload.count <= maximumSize else {
        throw VerificationError.oversizedFile(payload.count)
    }
    guard payload.count == Int(status.st_size) else {
        throw VerificationError.invalidFile("file changed size while it was read")
    }
    let publicKey = try Curve25519.Signing.PublicKey(rawRepresentation: publicKeyBytes)
    guard publicKey.isValidSignature(signatureBytes, for: payload) else {
        throw VerificationError.invalidSignature
    }
    print("Sparkle Ed25519 signature is valid.")
} catch {
    FileHandle.standardError.write(Data("error: \(error)\n".utf8))
    exit(1)
}
