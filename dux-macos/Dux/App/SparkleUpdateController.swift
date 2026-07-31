import Foundation
import Sparkle

enum SparkleUpdateAvailability: Equatable {
    case ready
    case unavailable(String)
}

struct SparkleUpdateConfiguration: Equatable {
    let feedURL: URL
    let publicEdKey: String

    static func load(
        info: [String: Any],
        bundleIdentifier: String?
    ) -> Result<Self, SparkleUpdateConfigurationError> {
        guard let bundleIdentifier,
              !bundleIdentifier.isEmpty,
              bundleIdentifier != "se.mjukis.dux.spike"
        else {
            return .failure(.placeholderBundleIdentity)
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
        guard let decodedKey = Data(base64Encoded: publicEdKey),
              decodedKey.count == 32
        else {
            return .failure(.invalidPublicKey)
        }
        return .success(Self(feedURL: feedURL, publicEdKey: publicEdKey))
    }
}

enum SparkleUpdateConfigurationError: Error, Equatable {
    case placeholderBundleIdentity
    case missingSecureFeed
    case missingPublicKey
    case invalidPublicKey

    var userMessage: String {
        switch self {
        case .placeholderBundleIdentity:
            "Waiting for DUX's production app identity."
        case .missingSecureFeed:
            "Waiting for the signed HTTPS update feed."
        case .missingPublicKey:
            "Waiting for the Sparkle verification key."
        case .invalidPublicKey:
            "The Sparkle verification key is invalid."
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
