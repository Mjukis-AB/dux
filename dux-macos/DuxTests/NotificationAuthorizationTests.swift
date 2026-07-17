import UserNotifications
import XCTest

@testable import DUX

final class NotificationAuthorizationTests: XCTestCase {
    private let en = Locale(identifier: "en_US")

    func testSystemStatusMapperCoversEveryKnownMacOSStatus() {
        XCTAssertEqual(NotificationService.map(.notDetermined), .notDetermined)
        XCTAssertEqual(NotificationService.map(.denied), .denied)
        XCTAssertEqual(NotificationService.map(.authorized), .authorized)
        XCTAssertEqual(NotificationService.map(.provisional), .provisional)
    }

    func testSystemErrorMapperIsDomainGatedAndTyped() {
        XCTAssertEqual(
            NotificationService.map(
                NSError(
                    domain: UNErrorDomain,
                    code: UNError.notificationsNotAllowed.rawValue
                )
            ),
            .denied
        )
        XCTAssertEqual(
            NotificationService.map(
                NSError(domain: UNErrorDomain, code: UNError.attachmentInvalidURL.rawValue)
            ),
            .unexpected
        )
        XCTAssertEqual(
            NotificationService.map(
                NSError(
                    domain: "unrelated.error-domain",
                    code: UNError.notificationsNotAllowed.rawValue
                )
            ),
            .unexpected
        )
    }

    func testAuthorizationRequestUsesOnlyAlertAndSound() {
        let options = NotificationService.authorizationOptions
        XCTAssertTrue(options.contains(.alert))
        XCTAssertTrue(options.contains(.sound))
        XCTAssertFalse(options.contains(.badge))
        XCTAssertEqual(options, [.alert, .sound])
    }

    func testAdapterRechecksDeterminedStatusBeforeRequesting() async throws {
        let center = UserNotificationCenterClientSpy(status: .authorized)
        let service = NotificationService(center: center)

        try await service.requestAuthorization()

        let calls = await center.calls()
        XCTAssertEqual(calls.status, 1)
        XCTAssertEqual(calls.request, 0)
        XCTAssertNil(calls.optionsRawValue)
    }

    func testAdapterUsesExactOptionsAfterNotDeterminedRecheck() async throws {
        let center = UserNotificationCenterClientSpy(status: .notDetermined)
        let service = NotificationService(center: center)

        try await service.requestAuthorization()

        let calls = await center.calls()
        XCTAssertEqual(calls.status, 1)
        XCTAssertEqual(calls.request, 1)
        XCTAssertEqual(
            calls.optionsRawValue,
            NotificationService.authorizationOptions.rawValue
        )
    }

    func testProductionServiceReadsCurrentStatusWithoutRequestingPermission() async {
        let status = await NotificationService().authorizationStatus()

        XCTAssertTrue(
            [
                NotificationAuthorizationStatus.notDetermined,
                .denied,
                .authorized,
                .provisional,
                .unknown,
            ].contains(status)
        )
    }

    func testEveryStatusHasHonestPresentationAndActions() {
        let cases:
            [(
                NotificationAuthorizationStatus,
                request: Bool,
                settings: Bool,
                refresh: Bool,
                title: String
            )] = [
                (.notDetermined, true, false, false, "Not requested"),
                (.denied, false, true, true, "Blocked by macOS"),
                (.authorized, false, true, false, "Allowed by macOS"),
                (.provisional, false, true, false, "Delivered quietly"),
                (.unknown, false, false, true, "Notification permission unknown"),
            ]

        for item in cases {
            let presentation = NotificationAuthorizationPresentation.make(
                state: NotificationAuthorizationState(
                    status: item.0,
                    activity: nil,
                    failure: nil
                ),
                locale: en
            )
            XCTAssertEqual(presentation.showsRequestAction, item.request)
            XCTAssertEqual(presentation.showsSystemSettingsAction, item.settings)
            XCTAssertEqual(presentation.showsRefreshAction, item.refresh)
            XCTAssertEqual(presentation.statusTitle, item.title)
            XCTAssertFalse(presentation.statusDetail.isEmpty)
            XCTAssertNil(presentation.progressLabel)
        }
    }

    func testInitialLoadingAndRequestingPresentDistinctProgress() {
        let initial = NotificationAuthorizationPresentation.make(
            state: .idle,
            locale: en
        )
        XCTAssertEqual(initial.statusTitle, "Checking…")
        XCTAssertFalse(initial.showsRequestAction)

        let loading = NotificationAuthorizationPresentation.make(
            state: NotificationAuthorizationState(
                status: .authorized,
                activity: .loading,
                failure: nil
            ),
            locale: en
        )
        XCTAssertEqual(loading.progressLabel, "Checking notification permission")
        XCTAssertFalse(loading.showsSystemSettingsAction)

        let requesting = NotificationAuthorizationPresentation.make(
            state: NotificationAuthorizationState(
                status: .notDetermined,
                activity: .requesting,
                failure: nil
            ),
            locale: en
        )
        XCTAssertEqual(requesting.progressLabel, "Waiting for your notification choice")
        XCTAssertFalse(requesting.showsRequestAction)
    }

    func testEveryFailureReasonHasBoundedUserCopy() {
        for reason in [
            NotificationAuthorizationFailureReason.denied,
            .outcomeUnknown,
            .unexpected,
        ] {
            let presentation = NotificationAuthorizationPresentation.make(
                state: NotificationAuthorizationState(
                    status: .notDetermined,
                    activity: nil,
                    failure: reason
                ),
                locale: en
            )
            XCTAssertNotNil(presentation.errorMessage)
            XCTAssertFalse(presentation.errorMessage?.isEmpty == true)
            XCTAssertFalse(presentation.errorMessage?.contains("test.error") == true)
        }
    }

    @MainActor
    func testAccessibilityIdentifiersAndSystemSettingsActionAreStable() {
        XCTAssertEqual(
            NotificationAuthorizationAccessibility.request,
            "notification-permission-request"
        )
        XCTAssertEqual(
            Set(NotificationAuthorizationAccessibility.allIdentifiers).count,
            NotificationAuthorizationAccessibility.allIdentifiers.count
        )
        var calls = 0
        AppActivation.openNotificationSettings { calls += 1 }
        XCTAssertEqual(calls, 1)
    }
}

@MainActor
final class NotificationAuthorizationAppModelTests: XCTestCase {
    func testRefreshPublishesEveryStatusWithoutRequestingPermission() async {
        for status in [
            NotificationAuthorizationStatus.notDetermined,
            .denied,
            .authorized,
            .provisional,
            .unknown,
        ] {
            let service = NotificationServiceSpy(status: status)
            let model = AppModel(notificationService: service)

            await model.refreshNotificationAuthorizationState()

            XCTAssertEqual(model.notificationAuthorizationState.status, status)
            XCTAssertNil(model.notificationAuthorizationState.activity)
            XCTAssertNil(model.notificationAuthorizationState.failure)
            let calls = await service.calls()
            XCTAssertEqual(calls.status, 1)
            XCTAssertEqual(calls.request, 0)
        }
    }

    func testExplicitRequestPublishesAuthoritativeAllowedStatus() async {
        let service = NotificationServiceSpy(status: .notDetermined)
        await service.configureRequest(status: .authorized)
        let model = AppModel(notificationService: service)
        await model.refreshNotificationAuthorizationState()

        await model.requestNotificationAuthorization()

        XCTAssertEqual(model.notificationAuthorizationState.status, .authorized)
        XCTAssertNil(model.notificationAuthorizationState.failure)
        let calls = await service.calls()
        XCTAssertEqual(calls.status, 2)
        XCTAssertEqual(calls.request, 1)
    }

    func testDeniedChoiceIsConfirmedStateRatherThanAnError() async {
        let service = NotificationServiceSpy(status: .notDetermined)
        await service.configureRequest(status: .denied, error: .denied)
        let model = AppModel(notificationService: service)
        await model.refreshNotificationAuthorizationState()

        await model.requestNotificationAuthorization()

        XCTAssertEqual(model.notificationAuthorizationState.status, .denied)
        XCTAssertNil(model.notificationAuthorizationState.failure)
    }

    func testRequestFailureRetainsTypedConfirmedState() async {
        let service = NotificationServiceSpy(status: .notDetermined)
        await service.configureRequest(status: .notDetermined, error: .unexpected)
        let model = AppModel(notificationService: service)
        await model.refreshNotificationAuthorizationState()

        await model.requestNotificationAuthorization()

        XCTAssertEqual(model.notificationAuthorizationState.status, .notDetermined)
        XCTAssertEqual(model.notificationAuthorizationState.failure, .unexpected)
    }

    func testSettledPostReadNormalizesRequestError() async {
        let service = NotificationServiceSpy(status: .notDetermined)
        await service.configureRequest(status: .provisional, error: .unexpected)
        let model = AppModel(notificationService: service)
        await model.refreshNotificationAuthorizationState()

        await model.requestNotificationAuthorization()

        XCTAssertEqual(model.notificationAuthorizationState.status, .provisional)
        XCTAssertNil(model.notificationAuthorizationState.failure)
    }

    func testSuccessfulCallWithoutSettledStatusIsOutcomeUnknown() async {
        for status in [
            NotificationAuthorizationStatus.notDetermined,
            .unknown,
        ] {
            let service = NotificationServiceSpy(status: .notDetermined)
            await service.configureRequest(status: status)
            let model = AppModel(notificationService: service)
            await model.refreshNotificationAuthorizationState()

            await model.requestNotificationAuthorization()

            XCTAssertEqual(model.notificationAuthorizationState.status, status)
            XCTAssertEqual(
                model.notificationAuthorizationState.failure,
                .outcomeUnknown
            )
        }
    }

    func testOnlyConfirmedNotDeterminedStatusCanRequestPermission() async {
        for status in [
            NotificationAuthorizationStatus.denied,
            .authorized,
            .provisional,
            .unknown,
        ] {
            let service = NotificationServiceSpy(status: status)
            let model = AppModel(notificationService: service)
            await model.refreshNotificationAuthorizationState()

            await model.requestNotificationAuthorization()

            let calls = await service.calls()
            XCTAssertEqual(calls.request, 0)
            XCTAssertEqual(model.notificationAuthorizationState.status, status)
        }
    }

    func testRefreshObservesExternalSystemSettingsChange() async {
        let service = NotificationServiceSpy(status: .denied)
        let model = AppModel(notificationService: service)
        await model.refreshNotificationAuthorizationState()
        await service.setStatus(.authorized)

        await model.refreshNotificationAuthorizationState()

        XCTAssertEqual(model.notificationAuthorizationState.status, .authorized)
        XCTAssertNil(model.notificationAuthorizationState.failure)
    }

    func testDuplicateRequestIsSingleFlightAndSurvivesCallerCancellation() async {
        let service = NotificationServiceSpy(status: .notDetermined)
        await service.suspendNextRequest()
        let model = AppModel(notificationService: service)
        await model.refreshNotificationAuthorizationState()

        let first = Task { await model.requestNotificationAuthorization() }
        while await service.calls().request == 0 {
            await Task.yield()
        }
        let duplicate = Task { await model.requestNotificationAuthorization() }
        first.cancel()
        await service.resumeRequest(status: .authorized)
        await first.value
        await duplicate.value

        XCTAssertEqual(model.notificationAuthorizationState.status, .authorized)
        XCTAssertNil(model.notificationAuthorizationState.failure)
        let calls = await service.calls()
        XCTAssertEqual(calls.request, 1)
        XCTAssertEqual(calls.status, 2)
    }
}

private actor NotificationServiceSpy: NotificationServing {
    struct CallCounts: Equatable, Sendable {
        var status = 0
        var request = 0
    }

    private var currentStatus: NotificationAuthorizationStatus
    private var callCounts = CallCounts()
    private var requestedStatus: NotificationAuthorizationStatus?
    private var requestError: NotificationServiceError?
    private var shouldSuspendRequest = false
    private var requestContinuation: CheckedContinuation<Void, Never>?

    init(status: NotificationAuthorizationStatus) {
        currentStatus = status
    }

    func authorizationStatus() -> NotificationAuthorizationStatus {
        callCounts.status += 1
        return currentStatus
    }

    func requestAuthorization() async throws {
        callCounts.request += 1
        if shouldSuspendRequest {
            shouldSuspendRequest = false
            await withCheckedContinuation { continuation in
                requestContinuation = continuation
            }
        }
        if let requestedStatus {
            currentStatus = requestedStatus
        }
        if let requestError {
            throw requestError
        }
    }

    func configureRequest(
        status: NotificationAuthorizationStatus,
        error: NotificationServiceError? = nil
    ) {
        requestedStatus = status
        requestError = error
    }

    func setStatus(_ status: NotificationAuthorizationStatus) {
        currentStatus = status
    }

    func suspendNextRequest() {
        shouldSuspendRequest = true
    }

    func resumeRequest(status: NotificationAuthorizationStatus) {
        currentStatus = status
        requestContinuation?.resume()
        requestContinuation = nil
    }

    func calls() -> CallCounts {
        callCounts
    }
}

private actor UserNotificationCenterClientSpy: UserNotificationCenterClient {
    struct CallCounts: Equatable, Sendable {
        var status = 0
        var request = 0
        var optionsRawValue: UInt?
    }

    private let currentStatus: NotificationAuthorizationStatus
    private var callCounts = CallCounts()

    init(status: NotificationAuthorizationStatus) {
        currentStatus = status
    }

    func authorizationStatus() -> NotificationAuthorizationStatus {
        callCounts.status += 1
        return currentStatus
    }

    func requestAuthorization(options: UNAuthorizationOptions) {
        callCounts.request += 1
        callCounts.optionsRawValue = options.rawValue
    }

    func calls() -> CallCounts {
        callCounts
    }
}
