import ServiceManagement
@testable import DUX
import XCTest

final class LoginItemTests: XCTestCase {
    private let en = Locale(identifier: "en_US")

    func testSystemStatusMapperCoversEveryKnownServiceManagementStatus() {
        XCTAssertEqual(LoginItemService.map(.notRegistered), .notRegistered)
        XCTAssertEqual(LoginItemService.map(.enabled), .enabled)
        XCTAssertEqual(LoginItemService.map(.requiresApproval), .requiresApproval)
        XCTAssertEqual(LoginItemService.map(.notFound), .notFound)
    }

    func testSystemErrorMapperIsTypedAndNeverRetainsRawText() {
        let domain = "SMAppServiceErrorDomain"
        XCTAssertEqual(
            LoginItemService.map(
                NSError(domain: domain, code: Int(kSMErrorInvalidSignature))
            ),
            .invalidSignature
        )
        XCTAssertEqual(
            LoginItemService.map(
                NSError(domain: domain, code: Int(kSMErrorLaunchDeniedByUser))
            ),
            .denied
        )
        XCTAssertEqual(
            LoginItemService.map(
                NSError(domain: domain, code: Int(kSMErrorAuthorizationFailure))
            ),
            .denied
        )
        XCTAssertEqual(
            LoginItemService.map(
                NSError(domain: domain, code: Int(kSMErrorServiceUnavailable))
            ),
            .serviceUnavailable
        )
        XCTAssertEqual(
            LoginItemService.map(NSError(domain: domain, code: -999)),
            .unexpected
        )
        XCTAssertEqual(
            LoginItemService.map(
                NSError(
                    domain: "unrelated.error-domain",
                    code: Int(kSMErrorInvalidSignature)
                )
            ),
            .unexpected
        )
    }

    func testProductionServiceReadsRealMainAppStatusWithoutMutation() async {
        let status = await LoginItemService().status()

        XCTAssertTrue(
            [
                LoginItemSystemStatus.notRegistered,
                .enabled,
                .requiresApproval,
                .notFound,
                .unknown,
            ].contains(status)
        )
    }

    func testEveryStatusHasHonestPresentation() {
        let cases: [
            (
                LoginItemSystemStatus,
                toggleOn: Bool,
                toggleEnabled: Bool,
                approval: Bool,
                refresh: Bool,
                title: String
            )
        ] = [
            (.notRegistered, false, true, false, false, "Off"),
            (.enabled, true, true, false, false, "Enabled by macOS"),
            (.requiresApproval, true, true, true, false, "Approval required"),
            (.notFound, false, false, false, true, "Login item unavailable"),
            (.unknown, false, false, false, true, "Login item status unknown"),
        ]

        for item in cases {
            let presentation = LoginItemPresentation.make(
                state: LoginItemState(
                    status: item.0,
                    activity: nil,
                    failure: nil
                ),
                locale: en
            )
            XCTAssertEqual(presentation.toggleOn, item.toggleOn)
            XCTAssertEqual(presentation.toggleEnabled, item.toggleEnabled)
            XCTAssertEqual(presentation.showsApprovalAction, item.approval)
            XCTAssertEqual(presentation.showsRefreshAction, item.refresh)
            XCTAssertEqual(presentation.statusTitle, item.title)
            XCTAssertFalse(presentation.statusDetail.isEmpty)
            XCTAssertNil(presentation.progressLabel)
        }
    }

    func testLoadingAndMutationsRetainStatusAndHaveDistinctProgressLabels() {
        let cases: [(LoginItemActivity, String)] = [
            (.loading, "Checking Launch at Login"),
            (.registering, "Enabling Launch at Login"),
            (.unregistering, "Disabling Launch at Login"),
        ]
        for (activity, label) in cases {
            let presentation = LoginItemPresentation.make(
                state: LoginItemState(
                    status: .enabled,
                    activity: activity,
                    failure: nil
                ),
                locale: en
            )
            XCTAssertTrue(presentation.toggleOn)
            XCTAssertFalse(presentation.toggleEnabled)
            XCTAssertEqual(presentation.progressLabel, label)
        }

        let initial = LoginItemPresentation.make(
            state: LoginItemState(status: nil, activity: .loading, failure: nil),
            locale: en
        )
        XCTAssertEqual(initial.statusTitle, "Checking…")
        XCTAssertFalse(initial.toggleOn)
        XCTAssertFalse(initial.toggleEnabled)
    }

    func testEveryFailureReasonHasBoundedUserCopy() {
        let reasons: [LoginItemFailureReason] = [
            .invalidSignature,
            .denied,
            .serviceUnavailable,
            .outcomeUnknown,
            .unexpected,
        ]
        for reason in reasons {
            for failure in [
                LoginItemFailure.registration(reason),
                .unregistration(reason),
            ] {
                let presentation = LoginItemPresentation.make(
                    state: LoginItemState(
                        status: .notRegistered,
                        activity: nil,
                        failure: failure
                    ),
                    locale: en
                )
                let message = presentation.errorMessage
                XCTAssertNotNil(message)
                XCTAssertFalse(message?.isEmpty == true)
                XCTAssertFalse(message?.contains("test.service-management") == true)
            }
        }
    }

    @MainActor
    func testAccessibilityIdentifiersAndSystemSettingsActionAreStable() {
        XCTAssertEqual(LoginItemAccessibility.toggle, "launch-at-login-toggle")
        XCTAssertEqual(
            Set(LoginItemAccessibility.allIdentifiers).count,
            LoginItemAccessibility.allIdentifiers.count
        )
        var calls = 0
        AppActivation.openLoginItemsSettings { calls += 1 }
        XCTAssertEqual(calls, 1)
    }
}

@MainActor
final class LoginItemAppModelTests: XCTestCase {
    func testRefreshPublishesEveryStatusWithoutMutatingRegistration() async {
        for status in [
            LoginItemSystemStatus.notRegistered,
            .enabled,
            .requiresApproval,
            .notFound,
            .unknown,
        ] {
            let service = LoginItemServiceSpy(status: status)
            let model = AppModel(loginItemService: service)

            await model.refreshLoginItemState()

            XCTAssertEqual(model.loginItemState.status, status)
            XCTAssertNil(model.loginItemState.activity)
            XCTAssertNil(model.loginItemState.failure)
            let calls = await service.calls()
            XCTAssertEqual(calls.status, 1)
            XCTAssertEqual(calls.register, 0)
            XCTAssertEqual(calls.unregister, 0)
        }
    }

    func testEnableRegistersOnceThenPublishesAuthoritativeEnabledStatus() async {
        let service = LoginItemServiceSpy(status: .notRegistered)
        await service.configureRegister(status: .enabled)
        let model = AppModel(loginItemService: service)
        await model.refreshLoginItemState()

        await model.setLaunchAtLogin(true)

        XCTAssertEqual(model.loginItemState.status, .enabled)
        XCTAssertNil(model.loginItemState.failure)
        let calls = await service.calls()
        XCTAssertEqual(calls.status, 2)
        XCTAssertEqual(calls.register, 1)
        XCTAssertEqual(calls.unregister, 0)
    }

    func testDeniedRegistrationThatSettlesRequiresApprovalIsTruthfulSuccess() async {
        let service = LoginItemServiceSpy(status: .notRegistered)
        await service.configureRegister(
            status: .requiresApproval,
            error: .denied
        )
        let model = AppModel(loginItemService: service)
        await model.refreshLoginItemState()

        await model.setLaunchAtLogin(true)

        XCTAssertEqual(model.loginItemState.status, .requiresApproval)
        XCTAssertNil(model.loginItemState.failure)
        XCTAssertTrue(
            LoginItemPresentation.make(state: model.loginItemState).toggleOn
        )
    }

    func testDisableUnregistersEnabledAndApprovalRequiredStates() async {
        for initial in [
            LoginItemSystemStatus.enabled,
            .requiresApproval,
        ] {
            let service = LoginItemServiceSpy(status: initial)
            await service.configureUnregister(status: .notRegistered)
            let model = AppModel(loginItemService: service)
            await model.refreshLoginItemState()

            await model.setLaunchAtLogin(false)

            XCTAssertEqual(model.loginItemState.status, .notRegistered)
            XCTAssertNil(model.loginItemState.failure)
            let calls = await service.calls()
            XCTAssertEqual(calls.register, 0)
            XCTAssertEqual(calls.unregister, 1)
        }
    }

    func testIdempotentAndUnavailableRequestsNeverMutateTheSystem() async {
        let cases: [(LoginItemSystemStatus, Bool)] = [
            (.notRegistered, false),
            (.enabled, true),
            (.requiresApproval, true),
            (.notFound, true),
            (.unknown, true),
        ]
        for (status, requested) in cases {
            let service = LoginItemServiceSpy(status: status)
            let model = AppModel(loginItemService: service)
            await model.refreshLoginItemState()

            await model.setLaunchAtLogin(requested)

            let calls = await service.calls()
            XCTAssertEqual(calls.register, 0)
            XCTAssertEqual(calls.unregister, 0)
            XCTAssertEqual(model.loginItemState.status, status)
        }
    }

    func testOperationFailureRechecksAndRetainsTypedConfirmedState() async {
        let service = LoginItemServiceSpy(status: .notRegistered)
        await service.configureRegister(
            status: .notRegistered,
            error: .invalidSignature
        )
        let model = AppModel(loginItemService: service)
        await model.refreshLoginItemState()

        await model.setLaunchAtLogin(true)

        XCTAssertEqual(model.loginItemState.status, .notRegistered)
        XCTAssertEqual(
            model.loginItemState.failure,
            .registration(.invalidSignature)
        )
        XCTAssertTrue(
            LoginItemPresentation.make(state: model.loginItemState)
                .errorMessage?
                .contains("signed DUX app") == true
        )
    }

    func testSuccessfulCallWithoutMatchingStatusIsOutcomeUnknown() async {
        let service = LoginItemServiceSpy(status: .notRegistered)
        await service.configureRegister(status: .notRegistered)
        let model = AppModel(loginItemService: service)
        await model.refreshLoginItemState()

        await model.setLaunchAtLogin(true)

        XCTAssertEqual(model.loginItemState.status, .notRegistered)
        XCTAssertEqual(
            model.loginItemState.failure,
            .registration(.outcomeUnknown)
        )
    }

    func testErrorIsNormalizedWhenPostReadAlreadyMatchesRequestedState() async {
        let service = LoginItemServiceSpy(status: .enabled)
        await service.configureUnregister(
            status: .notRegistered,
            error: .unexpected
        )
        let model = AppModel(loginItemService: service)
        await model.refreshLoginItemState()

        await model.setLaunchAtLogin(false)

        XCTAssertEqual(model.loginItemState.status, .notRegistered)
        XCTAssertNil(model.loginItemState.failure)
    }

    func testRefreshObservesExternalSystemSettingsChange() async {
        let service = LoginItemServiceSpy(status: .enabled)
        let model = AppModel(loginItemService: service)
        await model.refreshLoginItemState()
        await service.setStatus(.requiresApproval)

        await model.refreshLoginItemState()

        XCTAssertEqual(model.loginItemState.status, .requiresApproval)
        XCTAssertNil(model.loginItemState.failure)
    }

    func testDuplicateMutationIsSingleFlightAndSurvivesCallerCancellation() async {
        let service = LoginItemServiceSpy(status: .notRegistered)
        await service.suspendNextRegister()
        let model = AppModel(loginItemService: service)
        await model.refreshLoginItemState()

        let first = Task { await model.setLaunchAtLogin(true) }
        while await service.calls().register == 0 {
            await Task.yield()
        }
        let duplicate = Task { await model.setLaunchAtLogin(true) }
        first.cancel()
        await service.resumeRegister(status: .enabled)
        await first.value
        await duplicate.value

        XCTAssertEqual(model.loginItemState.status, .enabled)
        XCTAssertNil(model.loginItemState.failure)
        let calls = await service.calls()
        XCTAssertEqual(calls.register, 1)
        XCTAssertEqual(calls.status, 2)
    }
}

private actor LoginItemServiceSpy: LoginItemServing {
    struct CallCounts: Equatable, Sendable {
        var status = 0
        var register = 0
        var unregister = 0
    }

    private var currentStatus: LoginItemSystemStatus
    private var callCounts = CallCounts()
    private var registerStatus: LoginItemSystemStatus?
    private var registerError: LoginItemServiceError?
    private var unregisterStatus: LoginItemSystemStatus?
    private var unregisterError: LoginItemServiceError?
    private var shouldSuspendRegister = false
    private var registerContinuation: CheckedContinuation<Void, Never>?

    init(status: LoginItemSystemStatus) {
        currentStatus = status
    }

    func status() -> LoginItemSystemStatus {
        callCounts.status += 1
        return currentStatus
    }

    func register() async throws {
        callCounts.register += 1
        if shouldSuspendRegister {
            shouldSuspendRegister = false
            await withCheckedContinuation { continuation in
                registerContinuation = continuation
            }
        }
        if let registerStatus {
            currentStatus = registerStatus
        }
        if let registerError {
            throw registerError
        }
    }

    func unregister() async throws {
        callCounts.unregister += 1
        if let unregisterStatus {
            currentStatus = unregisterStatus
        }
        if let unregisterError {
            throw unregisterError
        }
    }

    func configureRegister(
        status: LoginItemSystemStatus,
        error: LoginItemServiceError? = nil
    ) {
        registerStatus = status
        registerError = error
    }

    func configureUnregister(
        status: LoginItemSystemStatus,
        error: LoginItemServiceError? = nil
    ) {
        unregisterStatus = status
        unregisterError = error
    }

    func setStatus(_ status: LoginItemSystemStatus) {
        currentStatus = status
    }

    func suspendNextRegister() {
        shouldSuspendRegister = true
    }

    func resumeRegister(status: LoginItemSystemStatus) {
        currentStatus = status
        registerContinuation?.resume()
        registerContinuation = nil
    }

    func calls() -> CallCounts {
        callCounts
    }
}
