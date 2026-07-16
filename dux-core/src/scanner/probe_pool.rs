use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, SendTimeoutError, Sender};

use super::walker::CancellationToken;

const PROBE_WORKER_COUNT: usize = 4;
const PROBE_QUEUE_CAPACITY: usize = PROBE_WORKER_COUNT * 2;
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);

struct ProbeJob {
    deadline: Instant,
    abandoned: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
    unavailable_workers: Arc<AtomicUsize>,
    task: Box<dyn FnOnce() + Send + 'static>,
}

const JOB_QUEUED: u8 = 0;
const JOB_RUNNING: u8 = 1;
const JOB_TIMED_OUT: u8 = 2;
const JOB_FINISHED: u8 = 3;
const JOB_EXPIRED: u8 = 4;
const JOB_ABANDONED: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbePoolError {
    QueueSaturated,
    DeadlineExceeded,
    Cancelled,
    Unavailable,
}

/// Fixed-size executor for filesystem calls that the OS cannot cancel.
///
/// Worker handles are intentionally detached: joining a worker blocked in a
/// kernel filesystem call could hang scanner shutdown. Keeping this pool
/// process-wide bounds that damage across repeated scans.
#[derive(Clone)]
pub(super) struct ProbePool {
    requests: Sender<ProbeJob>,
    worker_count: usize,
    unavailable_workers: Arc<AtomicUsize>,
}

impl ProbePool {
    pub(super) fn new(worker_count: usize, queue_capacity: usize) -> Self {
        assert!(worker_count > 0, "probe pool needs at least one worker");
        assert!(
            queue_capacity > 0,
            "probe queue must be bounded and non-empty"
        );

        let (requests, receiver) = crossbeam_channel::bounded::<ProbeJob>(queue_capacity);
        for index in 0..worker_count {
            spawn_worker(index, receiver.clone());
        }

        Self {
            requests,
            worker_count,
            unavailable_workers: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Run one job within a single deadline that includes queue admission.
    pub(super) fn run<T, F>(
        &self,
        timeout: Duration,
        cancellation: &CancellationToken,
        task: F,
    ) -> Result<T, ProbePoolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        if self.unavailable_workers.load(Ordering::Acquire) >= self.worker_count {
            return Err(ProbePoolError::QueueSaturated);
        }

        let deadline = Instant::now() + timeout;
        let abandoned = Arc::new(AtomicBool::new(false));
        let state = Arc::new(AtomicU8::new(JOB_QUEUED));
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        let mut pending_job = ProbeJob {
            deadline,
            abandoned: Arc::clone(&abandoned),
            state: Arc::clone(&state),
            unavailable_workers: Arc::clone(&self.unavailable_workers),
            task: Box::new(move || {
                let _ = result_tx.try_send(task());
            }),
        };

        loop {
            if cancellation.is_cancelled() {
                abandoned.store(true, Ordering::Release);
                return Err(ProbePoolError::Cancelled);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                abandoned.store(true, Ordering::Release);
                return Err(ProbePoolError::QueueSaturated);
            };

            match self
                .requests
                .send_timeout(pending_job, remaining.min(CANCELLATION_POLL_INTERVAL))
            {
                Ok(()) => break,
                Err(SendTimeoutError::Timeout(job)) => pending_job = job,
                Err(SendTimeoutError::Disconnected(_)) => {
                    abandoned.store(true, Ordering::Release);
                    return Err(ProbePoolError::Unavailable);
                }
            }
        }

        loop {
            if cancellation.is_cancelled() {
                abandoned.store(true, Ordering::Release);
                abandon_admitted_job(&state, &self.unavailable_workers);
                return Err(ProbePoolError::Cancelled);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                abandoned.store(true, Ordering::Release);
                return Err(classify_deadline(&state, &self.unavailable_workers));
            };

            match result_rx.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
                Ok(result) => {
                    // The caller can be descheduled after computing `remaining`.
                    // Do not accept a result that arrived while the caller was
                    // asleep after the end-to-end deadline had already passed.
                    if Instant::now() >= deadline {
                        abandoned.store(true, Ordering::Release);
                        return Err(classify_deadline(&state, &self.unavailable_workers));
                    }
                    return Ok(result);
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    abandoned.store(true, Ordering::Release);
                    if cancellation.is_cancelled() {
                        abandon_admitted_job(&state, &self.unavailable_workers);
                        return Err(ProbePoolError::Cancelled);
                    }
                    if state.load(Ordering::Acquire) == JOB_EXPIRED {
                        return Err(ProbePoolError::QueueSaturated);
                    }
                    if Instant::now() >= deadline {
                        return Err(classify_deadline(&state, &self.unavailable_workers));
                    }
                    return Err(ProbePoolError::Unavailable);
                }
            }
        }
    }

    #[cfg(test)]
    fn queued_requests(&self) -> usize {
        self.requests.len()
    }

    #[cfg(test)]
    pub(super) fn unavailable_worker_count(&self) -> usize {
        self.unavailable_workers.load(Ordering::Acquire)
    }
}

fn abandon_admitted_job(state: &AtomicU8, unavailable_workers: &AtomicUsize) {
    loop {
        match state.load(Ordering::Acquire) {
            JOB_QUEUED => {
                if state
                    .compare_exchange(JOB_QUEUED, JOB_EXPIRED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return;
                }
            }
            JOB_RUNNING => {
                if state
                    .compare_exchange(
                        JOB_RUNNING,
                        JOB_ABANDONED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    unavailable_workers.fetch_add(1, Ordering::AcqRel);
                    return;
                }
            }
            JOB_TIMED_OUT | JOB_FINISHED | JOB_EXPIRED | JOB_ABANDONED => return,
            _ => return,
        }
    }
}

fn classify_deadline(state: &AtomicU8, unavailable_workers: &AtomicUsize) -> ProbePoolError {
    loop {
        match state.load(Ordering::Acquire) {
            JOB_QUEUED => {
                if state
                    .compare_exchange(JOB_QUEUED, JOB_EXPIRED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return ProbePoolError::QueueSaturated;
                }
            }
            JOB_RUNNING => {
                if state
                    .compare_exchange(
                        JOB_RUNNING,
                        JOB_TIMED_OUT,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    unavailable_workers.fetch_add(1, Ordering::AcqRel);
                    return ProbePoolError::DeadlineExceeded;
                }
            }
            JOB_EXPIRED => return ProbePoolError::QueueSaturated,
            JOB_TIMED_OUT | JOB_FINISHED | JOB_ABANDONED => {
                return ProbePoolError::DeadlineExceeded;
            }
            _ => return ProbePoolError::Unavailable,
        }
    }
}

fn spawn_worker(index: usize, receiver: Receiver<ProbeJob>) {
    std::thread::Builder::new()
        .name(format!("dux-fs-probe-{index}"))
        .spawn(move || {
            while let Ok(job) = receiver.recv() {
                // Do not start work whose caller has already timed out in the queue.
                if job.abandoned.load(Ordering::Acquire) {
                    let _ = job.state.compare_exchange(
                        JOB_QUEUED,
                        JOB_EXPIRED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    continue;
                }
                if Instant::now() >= job.deadline {
                    let _ = job.state.compare_exchange(
                        JOB_QUEUED,
                        JOB_EXPIRED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    continue;
                }
                if job
                    .state
                    .compare_exchange(JOB_QUEUED, JOB_RUNNING, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    continue;
                }

                // A platform wrapper should not panic, but keep one bad job from
                // permanently reducing the process-wide pool's capacity.
                let _ = catch_unwind(AssertUnwindSafe(job.task));
                if matches!(
                    job.state.swap(JOB_FINISHED, Ordering::AcqRel),
                    JOB_TIMED_OUT | JOB_ABANDONED
                ) {
                    job.unavailable_workers.fetch_sub(1, Ordering::AcqRel);
                }
            }
        })
        .expect("failed to start filesystem probe worker");
}

static DIRECTORY_PROBE_POOL: OnceLock<ProbePool> = OnceLock::new();

pub(super) fn directory_probe_pool() -> &'static ProbePool {
    DIRECTORY_PROBE_POOL.get_or_init(|| ProbePool::new(PROBE_WORKER_COUNT, PROBE_QUEUE_CAPACITY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Condvar, Mutex};

    #[test]
    fn returns_successful_job_results() {
        let pool = ProbePool::new(1, 1);
        assert_eq!(
            pool.run(Duration::from_secs(1), &CancellationToken::new(), || 42),
            Ok(42)
        );
    }

    #[test]
    fn limits_concurrent_jobs_to_worker_count() {
        const WORKERS: usize = 2;
        const JOBS: usize = 8;

        let pool = Arc::new(ProbePool::new(WORKERS, JOBS));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(AtomicUsize::new(0));
        let first_workers_ready = Arc::new(Barrier::new(WORKERS + 1));

        let mut handles = Vec::new();
        for _ in 0..JOBS {
            let pool = Arc::clone(&pool);
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let order = Arc::clone(&order);
            let first_workers_ready = Arc::clone(&first_workers_ready);
            handles.push(std::thread::spawn(move || {
                pool.run(
                    Duration::from_secs(2),
                    &CancellationToken::new(),
                    move || {
                        let position = order.fetch_add(1, Ordering::SeqCst);
                        let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now_active, Ordering::SeqCst);
                        if position < WORKERS {
                            first_workers_ready.wait();
                        }
                        std::thread::sleep(Duration::from_millis(5));
                        active.fetch_sub(1, Ordering::SeqCst);
                    },
                )
            }));
        }

        first_workers_ready.wait();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), Ok(()));
        }
        assert_eq!(peak.load(Ordering::SeqCst), WORKERS);
    }

    #[test]
    fn saturated_queue_times_out_and_discards_expired_work() {
        let pool = Arc::new(ProbePool::new(1, 1));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = crossbeam_channel::bounded(1);
        let executions = Arc::new(AtomicUsize::new(0));

        let first_pool = Arc::clone(&pool);
        let first_release = Arc::clone(&release);
        let first_executions = Arc::clone(&executions);
        let first = std::thread::spawn(move || {
            first_pool.run(
                Duration::from_secs(1),
                &CancellationToken::new(),
                move || {
                    first_executions.fetch_add(1, Ordering::SeqCst);
                    started_tx.send(()).unwrap();
                    let (lock, wake) = &*first_release;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = wake.wait(released).unwrap();
                    }
                    1
                },
            )
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let second_pool = Arc::clone(&pool);
        let second_executions = Arc::clone(&executions);
        let second = std::thread::spawn(move || {
            second_pool.run(
                Duration::from_millis(40),
                &CancellationToken::new(),
                move || {
                    second_executions.fetch_add(1, Ordering::SeqCst);
                    2
                },
            )
        });

        let queue_deadline = Instant::now() + Duration::from_secs(1);
        while pool.queued_requests() != 1 {
            assert!(Instant::now() < queue_deadline, "request was not queued");
            std::thread::yield_now();
        }

        assert_eq!(
            pool.run(Duration::from_millis(10), &CancellationToken::new(), || 3),
            Err(ProbePoolError::QueueSaturated)
        );
        assert_eq!(second.join().unwrap(), Err(ProbePoolError::QueueSaturated));
        let (lock, wake) = &*release;
        *lock.lock().unwrap() = true;
        wake.notify_all();
        assert_eq!(first.join().unwrap(), Ok(1));
        assert_eq!(
            pool.run(Duration::from_secs(1), &CancellationToken::new(), || 4),
            Ok(4)
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn queue_and_execution_share_one_deadline() {
        let pool = Arc::new(ProbePool::new(1, 1));
        let (started_tx, started_rx) = crossbeam_channel::bounded(1);

        let first_pool = Arc::clone(&pool);
        let first = std::thread::spawn(move || {
            first_pool.run(
                Duration::from_secs(1),
                &CancellationToken::new(),
                move || {
                    started_tx.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(60));
                },
            )
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let result = pool.run(
            Duration::from_millis(100),
            &CancellationToken::new(),
            || {
                std::thread::sleep(Duration::from_millis(60));
                2
            },
        );

        assert_eq!(result, Err(ProbePoolError::DeadlineExceeded));
        assert_eq!(first.join().unwrap(), Ok(()));
    }

    #[test]
    fn worker_survives_panicking_job() {
        let pool = ProbePool::new(1, 1);
        let failed = pool.run(
            Duration::from_secs(1),
            &CancellationToken::new(),
            || -> usize {
                panic!("intentional probe panic");
            },
        );

        assert_eq!(failed, Err(ProbePoolError::Unavailable));
        assert_eq!(
            pool.run(Duration::from_secs(1), &CancellationToken::new(), || 7),
            Ok(7)
        );
    }

    #[test]
    fn cancellation_abandons_queued_work() {
        let pool = Arc::new(ProbePool::new(1, 1));
        let cancellation = CancellationToken::new();
        let blocker = Arc::new(Barrier::new(2));

        let first_pool = Arc::clone(&pool);
        let first_blocker = Arc::clone(&blocker);
        let first = std::thread::spawn(move || {
            first_pool.run(
                Duration::from_secs(1),
                &CancellationToken::new(),
                move || {
                    first_blocker.wait();
                    std::thread::sleep(Duration::from_millis(300));
                },
            )
        });
        blocker.wait();

        let queued_pool = Arc::clone(&pool);
        let queued_cancellation = cancellation.clone();
        let executed = Arc::new(AtomicBool::new(false));
        let queued_executed = Arc::clone(&executed);
        let queued = std::thread::spawn(move || {
            queued_pool.run(Duration::from_secs(1), &queued_cancellation, move || {
                queued_executed.store(true, Ordering::SeqCst)
            })
        });

        let queue_deadline = Instant::now() + Duration::from_secs(1);
        while pool.queued_requests() != 1 {
            assert!(Instant::now() < queue_deadline, "request was not queued");
            std::thread::yield_now();
        }
        cancellation.cancel();

        assert_eq!(queued.join().unwrap(), Err(ProbePoolError::Cancelled));
        assert_eq!(first.join().unwrap(), Ok(()));
        assert_eq!(
            pool.run(Duration::from_secs(1), &CancellationToken::new(), || 3),
            Ok(3)
        );
        assert!(!executed.load(Ordering::SeqCst));
    }

    #[test]
    fn timed_out_worker_opens_a_fast_fail_circuit_until_it_returns() {
        let pool = Arc::new(ProbePool::new(1, 1));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = crossbeam_channel::bounded(1);
        let running_pool = Arc::clone(&pool);
        let running_release = Arc::clone(&release);
        let running = std::thread::spawn(move || {
            running_pool.run(
                Duration::from_millis(20),
                &CancellationToken::new(),
                move || {
                    started_tx.send(()).unwrap();
                    let (lock, wake) = &*running_release;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = wake.wait(released).unwrap();
                    }
                    1
                },
            )
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            running.join().unwrap(),
            Err(ProbePoolError::DeadlineExceeded)
        );
        assert_eq!(
            pool.run(Duration::from_secs(1), &CancellationToken::new(), || 2),
            Err(ProbePoolError::QueueSaturated)
        );
        let (lock, wake) = &*release;
        *lock.lock().unwrap() = true;
        wake.notify_all();
        let recovery_deadline = Instant::now() + Duration::from_secs(1);
        while pool.unavailable_worker_count() != 0 {
            assert!(
                Instant::now() < recovery_deadline,
                "probe pool did not recover"
            );
            std::thread::yield_now();
        }
        assert_eq!(
            pool.run(Duration::from_secs(1), &CancellationToken::new(), || 3),
            Ok(3)
        );
    }

    #[test]
    fn cancelled_running_worker_opens_circuit_until_it_returns() {
        let pool = Arc::new(ProbePool::new(1, 1));
        let cancellation = CancellationToken::new();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = crossbeam_channel::bounded(1);

        let running_pool = Arc::clone(&pool);
        let running_cancellation = cancellation.clone();
        let running_release = Arc::clone(&release);
        let running = std::thread::spawn(move || {
            running_pool.run(Duration::from_secs(2), &running_cancellation, move || {
                started_tx.send(()).unwrap();
                let (lock, wake) = &*running_release;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
                1
            })
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        cancellation.cancel();
        assert_eq!(running.join().unwrap(), Err(ProbePoolError::Cancelled));
        assert_eq!(pool.unavailable_worker_count(), 1);
        assert_eq!(
            pool.run(Duration::from_secs(1), &CancellationToken::new(), || 2),
            Err(ProbePoolError::QueueSaturated)
        );

        let (lock, wake) = &*release;
        *lock.lock().unwrap() = true;
        wake.notify_all();
        let recovery_deadline = Instant::now() + Duration::from_secs(1);
        while pool.unavailable_worker_count() != 0 {
            assert!(
                Instant::now() < recovery_deadline,
                "cancelled probe worker did not recover"
            );
            std::thread::yield_now();
        }
        assert_eq!(
            pool.run(Duration::from_secs(1), &CancellationToken::new(), || 3),
            Ok(3)
        );
    }
}
