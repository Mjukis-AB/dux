protocol DuxCapacityResampleRequesting: Sendable {
    func requestCapacityResample() async
}

actor DuxCapacityResampleRouter: DuxCapacityResampleRequesting {
    private weak var scheduler: (any DuxCapacityScheduling)?
    private var hasPendingRequest = false
    private var isInvalidated = false

    func attach(_ scheduler: any DuxCapacityScheduling) async {
        guard !isInvalidated else {
            return
        }
        self.scheduler = scheduler
        let hadPendingRequest = hasPendingRequest
        hasPendingRequest = false
        if hadPendingRequest {
            await scheduler.signal(.manual)
        }
    }

    func requestCapacityResample() async {
        guard !isInvalidated else {
            return
        }
        guard let scheduler else {
            hasPendingRequest = true
            return
        }
        await scheduler.signal(.manual)
    }

    func invalidate() {
        isInvalidated = true
        hasPendingRequest = false
        scheduler = nil
    }
}

actor NoopDuxCapacityResampleRequester: DuxCapacityResampleRequesting {
    func requestCapacityResample() {}
}
