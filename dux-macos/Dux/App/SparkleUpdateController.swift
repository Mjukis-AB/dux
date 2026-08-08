import Foundation
import Sparkle

enum SparkleUpdateAvailability: Equatable {
    case ready
    case unavailable(String)
}

struct SparkleUpdateConfiguration: Equatable {
    static let productionBundleIdentifier = "se.mjukis.dux"
    static let productionPublicEdKey =
        "UmMI6TWBdBm2fKEmmk5xi2T+lu7K5KJl1abwIBRLSQo="

    let feedURL: URL
    let publicEdKey: String

    static func load(
        info: [String: Any],
        bundleIdentifier: String?
    ) -> Result<Self, SparkleUpdateConfigurationError> {
        guard bundleIdentifier == productionBundleIdentifier else {
            return .failure(.unexpectedBundleIdentity)
        }
        guard let feedValue = info["SUFeedURL"] as? String,
              let feedURL = URL(string: feedValue),
              feedURL.scheme?.lowercased() == "https",
              feedURL.host != nil,
              feedURL.user == nil,
              feedURL.password == nil
        else {
            return .failure(.missingSecureFeed)
        }
        guard let publicEdKey = info["SUPublicEDKey"] as? String,
              !publicEdKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            return .failure(.missingPublicKey)
        }
        guard publicEdKey == productionPublicEdKey,
              let decodedKey = Data(base64Encoded: publicEdKey),
              decodedKey.count == 32
        else {
            return .failure(.unexpectedPublicKey)
        }
        guard info["SURequireSignedFeed"] as? Bool == true,
              info["SUVerifyUpdateBeforeExtraction"] as? Bool == true,
              info["SUSignedFeedFailureExpirationInterval"] as? Int == 0
        else {
            return .failure(.unsafeVerificationPolicy)
        }
        return .success(Self(feedURL: feedURL, publicEdKey: publicEdKey))
    }
}

enum SparkleUpdateConfigurationError: Error, Equatable {
    case unexpectedBundleIdentity
    case missingSecureFeed
    case missingPublicKey
    case unexpectedPublicKey
    case unsafeVerificationPolicy

    var userMessage: String {
        switch self {
        case .unexpectedBundleIdentity:
            "Waiting for DUX's production app identity."
        case .missingSecureFeed:
            "Waiting for the signed HTTPS update feed."
        case .missingPublicKey:
            "Waiting for the Sparkle verification key."
        case .unexpectedPublicKey:
            "The Sparkle verification key does not match DUX's release identity."
        case .unsafeVerificationPolicy:
            "Waiting for DUX's strict signed-update policy."
        }
    }
}

@MainActor
final class SparkleUpdateController {
    static let shared = SparkleUpdateController()

    let availability: SparkleUpdateAvailability

    private let standardController: SPUStandardUpdaterController?

    private init(bundle: Bundle = .main) {
        switch SparkleUpdateConfiguration.load(
            info: bundle.infoDictionary ?? [:],
            bundleIdentifier: bundle.bundleIdentifier
        ) {
        case .success:
            availability = .ready
            standardController = SPUStandardUpdaterController(
                startingUpdater: true,
                updaterDelegate: nil,
                userDriverDelegate: nil
            )
        case let .failure(error):
            availability = .unavailable(error.userMessage)
            standardController = nil
        }
    }

    func checkForUpdates() {
        standardController?.checkForUpdates(nil)
    }
}
