use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::*;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

fn config(temp: &TempDir) -> EngineConfig {
    EngineConfig::new(
        temp.path().join("data/dux.sqlite3"),
        temp.path().join("data/snapshots"),
        temp.path().join("cache"),
    )
    .unwrap()
}

fn engine_with_limits(limits: RegistryLimits) -> (TempDir, EngineHandle) {
    let temp = TempDir::new().unwrap();
    let engine = EngineHandle::open_with_limits(config(&temp), limits).unwrap();
    (temp, engine)
}

fn wait_terminal(engine: &EngineHandle, id: TaskId) -> TaskSnapshot {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let snapshot = engine.task_snapshot(id).unwrap();
        if snapshot.phase.is_terminal() {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "task did not quiesce");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn handle_is_send_sync_and_config_is_explicit() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<EngineHandle>();

    let temp = TempDir::new().unwrap();
    let expected = config(&temp);
    let engine = EngineHandle::open(expected.clone()).unwrap();
    assert_eq!(engine.config(), &expected);
    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
}

#[test]
fn real_format_batch_runs_through_registry_and_publishes_immutable_result() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(2, 4, 4, 8));
    let id = engine.start_format_size_batch(vec![0, 1_536]).unwrap();

    let snapshot = wait_terminal(&engine, id);
    assert_eq!(snapshot.phase, TaskPhase::Succeeded);
    assert!(snapshot.result_available);
    let result = engine.format_size_batch_result(id).unwrap().unwrap();
    assert_eq!(result.entries()[0].display, "0 B");
    assert_eq!(result.entries()[1].display, "1.5 KB");
    assert!(
        engine
            .task_events(id, 0, 8)
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(event.kind, TaskEventKind::Terminal { .. }))
    );
}

#[test]
fn format_batch_input_is_bounded() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    assert_eq!(
        engine.start_format_size_batch(vec![0; FORMAT_BATCH_LIMIT + 1]),
        Err(StartTaskError::InputTooLarge { limit: 256 })
    );
}

#[test]
fn identifiers_are_nonzero_monotonic_and_allocator_never_wraps() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 4, 4, 4));
    let first = engine.start_format_size_batch(Vec::new()).unwrap();
    let second = engine.start_format_size_batch(Vec::new()).unwrap();
    let (_other_temp, other_engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let across_engines = other_engine.start_format_size_batch(Vec::new()).unwrap();
    assert!(first.get() > 0);
    assert!(second > first);
    assert!(across_engines > second);

    let allocator = TaskIdAllocator::new(NonZeroU64::MAX);
    assert_eq!(allocator.allocate().unwrap().get(), u64::MAX);
    assert_eq!(allocator.allocate(), Err(StartTaskError::TaskIdExhausted));
}

#[test]
fn queue_capacity_is_fixed_and_queued_cancellation_never_executes() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 4, 8));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let running = engine
        .submit_test(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let executions = Arc::new(AtomicUsize::new(0));
    let executions_for_job = Arc::clone(&executions);
    let queued = engine
        .submit_test(Box::new(move |_| {
            executions_for_job.fetch_add(1, Ordering::SeqCst);
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    assert_eq!(
        engine.start_format_size_batch(Vec::new()),
        Err(StartTaskError::QueueFull)
    );
    assert_eq!(
        engine.cancel_task(queued).unwrap(),
        CancelOutcome::CancelledBeforeStart
    );
    assert_eq!(wait_terminal(&engine, queued).phase, TaskPhase::Cancelled);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    let replacement = engine.start_format_size_batch(vec![1]).unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, running).phase, TaskPhase::Succeeded);
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );
}

#[test]
fn configured_workers_are_an_exact_concurrency_bound() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(2, 6, 8, 4));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let mut ids = Vec::new();

    for _ in 0..6 {
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        let started_tx = started_tx.clone();
        let release_rx = Arc::clone(&release_rx);
        ids.push(
            engine
                .submit_test(Box::new(move |_| {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    started_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    WorkOutcome::Succeeded(TaskResult::TestOnly)
                }))
                .unwrap(),
        );
    }

    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(active.load(Ordering::SeqCst), 2);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    for _ in 0..ids.len() {
        release_tx.send(()).unwrap();
    }
    for id in ids {
        assert_eq!(wait_terminal(&engine, id).phase, TaskPhase::Succeeded);
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[test]
fn one_worker_executes_queued_tasks_in_fifo_order() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 4, 4, 4));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let blocker = engine
        .submit_test(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let order = Arc::new(Mutex::new(Vec::new()));
    let mut queued = Vec::new();
    for index in 0..3 {
        let order = Arc::clone(&order);
        queued.push(
            engine
                .submit_test(Box::new(move |_| {
                    order.lock().unwrap().push(index);
                    WorkOutcome::Succeeded(TaskResult::TestOnly)
                }))
                .unwrap(),
        );
    }
    release_tx.send(()).unwrap();
    wait_terminal(&engine, blocker);
    for id in queued {
        wait_terminal(&engine, id);
    }
    assert_eq!(*order.lock().unwrap(), vec![0, 1, 2]);
}

#[test]
fn running_cancellation_is_intent_until_worker_returns() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let (started_tx, started_rx) = mpsc::channel();
    let (observed_tx, observed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let id = engine
        .submit_test(Box::new(move |context| {
            started_tx.send(()).unwrap();
            while !context.is_cancellation_requested() {
                std::thread::yield_now();
            }
            observed_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Cancelled
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(engine.cancel_task(id).unwrap(), CancelOutcome::Requested);
    assert_eq!(
        engine.cancel_task(id).unwrap(),
        CancelOutcome::AlreadyRequested
    );
    observed_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let pending = engine.task_snapshot(id).unwrap();
    assert_eq!(pending.phase, TaskPhase::Running);
    assert!(pending.cancellation_requested);
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, id).phase, TaskPhase::Cancelled);
    assert_eq!(
        engine.cancel_task(id).unwrap(),
        CancelOutcome::AlreadyTerminal
    );
}

#[test]
fn cancellation_intent_does_not_rewrite_completed_or_failed_work() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let completed = engine
        .submit_test(Box::new(move |_| {
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    ready_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(completed).unwrap(),
        CancelOutcome::Requested
    );
    release_tx.send(()).unwrap();
    let completed_snapshot = wait_terminal(&engine, completed);
    assert_eq!(completed_snapshot.phase, TaskPhase::Succeeded);
    assert!(completed_snapshot.cancellation_requested);
    assert!(completed_snapshot.result_available);

    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let failed = engine
        .submit_test(Box::new(move |_| {
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            panic!("sanitized task failure")
        }))
        .unwrap();
    ready_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(failed).unwrap(),
        CancelOutcome::Requested
    );
    release_tx.send(()).unwrap();
    let failed_snapshot = wait_terminal(&engine, failed);
    assert_eq!(failed_snapshot.phase, TaskPhase::Failed);
    assert!(failed_snapshot.cancellation_requested);
    assert_eq!(
        failed_snapshot.failure,
        Some(TaskFailureKind::InternalFailure)
    );
}

#[test]
fn worker_panic_is_contained_and_pool_survives() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let failed = engine
        .submit_test(Box::new(|_| panic!("payload must not escape")))
        .unwrap();
    let failed_snapshot = wait_terminal(&engine, failed);
    assert_eq!(failed_snapshot.phase, TaskPhase::Failed);
    assert_eq!(
        failed_snapshot.failure,
        Some(TaskFailureKind::InternalFailure)
    );

    let next = engine.start_format_size_batch(vec![1]).unwrap();
    assert_eq!(wait_terminal(&engine, next).phase, TaskPhase::Succeeded);
}

#[test]
fn terminal_records_and_events_are_bounded() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 1, 3));
    let first = engine.start_format_size_batch(vec![1, 2, 3, 4]).unwrap();
    wait_terminal(&engine, first);
    let page = engine.task_events(first, 0, 3).unwrap();
    assert_eq!(page.events.len(), 3);
    assert!(page.truncated);
    assert!(page.terminal);

    let second = engine.start_format_size_batch(Vec::new()).unwrap();
    wait_terminal(&engine, second);
    assert_eq!(
        engine.task_snapshot(first),
        Err(TaskAccessError::UnknownTask)
    );
    assert!(engine.task_snapshot(second).is_ok());
    assert_eq!(
        engine.task_events(second, 0, 0),
        Err(TaskAccessError::InvalidEventLimit { max: 3 })
    );
    assert_eq!(
        engine.task_events(second, 0, 4),
        Err(TaskAccessError::InvalidEventLimit { max: 3 })
    );
}

#[test]
fn aggregate_registry_state_is_bounded_and_workers_are_reaped() {
    let limits = RegistryLimits::testing(1, 2, 2, 4);
    let (_temp, engine) = engine_with_limits(limits);
    let shared = Arc::clone(&engine.inner.shared);
    let first_terminal = engine.start_format_size_batch(Vec::new()).unwrap();
    wait_terminal(&engine, first_terminal);
    let second_terminal = engine.start_format_size_batch(Vec::new()).unwrap();
    wait_terminal(&engine, second_terminal);

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let running = engine
        .submit_test(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Cancelled
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let executions = Arc::new(AtomicUsize::new(0));
    let mut queued = Vec::new();
    for _ in 0..limits.queued_tasks {
        let executions = Arc::clone(&executions);
        queued.push(
            engine
                .submit_test(Box::new(move |_| {
                    executions.fetch_add(1, Ordering::SeqCst);
                    WorkOutcome::Succeeded(TaskResult::TestOnly)
                }))
                .unwrap(),
        );
    }

    {
        let registry = shared.lock_registry_recover();
        assert_eq!(registry.live_workers, limits.workers);
        assert_eq!(registry.running_tasks, limits.workers);
        assert_eq!(registry.queue.len(), limits.queued_tasks);
        assert_eq!(
            registry.terminal_order.len(),
            limits.retained_terminal_tasks
        );
        assert_eq!(
            registry.records.len(),
            limits.workers + limits.queued_tasks + limits.retained_terminal_tasks
        );
        for id in [first_terminal, second_terminal, running]
            .into_iter()
            .chain(queued.iter().copied())
        {
            assert!(registry.records.contains_key(&id));
        }
    }

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    {
        let registry = shared.lock_registry_recover();
        assert_eq!(registry.live_workers, 0);
        assert_eq!(registry.running_tasks, 0);
        assert!(registry.queue.is_empty());
        assert!(registry.records.len() <= limits.retained_terminal_tasks);
    }
    assert!(engine.inner.workers.lock().unwrap().is_none());
}

#[test]
fn event_pages_are_contiguous_and_reject_future_cursors() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let id = engine.start_format_size_batch(vec![1, 2]).unwrap();
    let snapshot = wait_terminal(&engine, id);

    let first = engine.task_events(id, 0, 2).unwrap();
    let second = engine.task_events(id, first.next_sequence, 8).unwrap();
    let sequences: Vec<_> = first
        .events
        .iter()
        .chain(second.events.iter())
        .map(|event| event.sequence)
        .collect();
    assert_eq!(sequences, vec![1, 2, 3, 4, 5]);
    assert!(matches!(
        second.events.last().map(|event| &event.kind),
        Some(TaskEventKind::Terminal { .. })
    ));
    assert_eq!(snapshot.revision, 5);
    let current = engine.task_events(id, second.next_sequence, 8).unwrap();
    assert!(current.events.is_empty());
    assert_eq!(current.next_sequence, second.next_sequence);
    assert_eq!(
        engine.task_events(id, second.next_sequence + 1, 8),
        Err(TaskAccessError::InvalidEventCursor)
    );
    assert_eq!(
        engine.task_events(id, u64::MAX, 8),
        Err(TaskAccessError::InvalidEventCursor)
    );
}

#[test]
fn close_is_nonblocking_idempotent_cancels_work_and_rejects_use() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let shared = Arc::clone(&engine.inner.shared);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let running = engine
        .submit_test(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Cancelled
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let queued_executions = Arc::clone(&executions);
    let queued = engine
        .submit_test(Box::new(move |_| {
            queued_executions.fetch_add(1, Ordering::SeqCst);
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    assert_eq!(engine.lifecycle(), EngineLifecycle::Closing);
    assert!(!engine.wait_until_closed(Duration::from_millis(10)));
    assert!(matches!(
        engine.close(),
        CloseOutcome::AlreadyClosing | CloseOutcome::AlreadyClosed
    ));
    assert_eq!(
        engine.start_format_size_batch(Vec::new()),
        Err(StartTaskError::Closed)
    );
    assert_eq!(engine.task_snapshot(running), Err(TaskAccessError::Closed));
    assert_eq!(engine.task_snapshot(queued), Err(TaskAccessError::Closed));
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(engine.lifecycle(), EngineLifecycle::Closed);
    assert_eq!(engine.close(), CloseOutcome::AlreadyClosed);
    let registry = shared.lock_registry_recover();
    assert_eq!(registry.records[&running].phase, TaskPhase::Cancelled);
    assert_eq!(registry.records[&queued].phase, TaskPhase::Cancelled);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    for id in [running, queued] {
        let record = &registry.records[&id];
        assert!(matches!(
            record.events.back().map(|event| &event.kind),
            Some(TaskEventKind::Terminal {
                phase: TaskPhase::Cancelled
            })
        ));
        assert_eq!(
            record
                .events
                .iter()
                .filter(|event| matches!(event.kind, TaskEventKind::Terminal { .. }))
                .count(),
            1
        );
    }
}

#[test]
fn only_last_handle_drop_cancels_running_and_queued_work() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 2, 8));
    let shared = Arc::clone(&engine.inner.shared);
    let (started_tx, started_rx) = mpsc::channel();
    let (cancelled_tx, cancelled_rx) = mpsc::channel();
    let running = engine
        .submit_test(Box::new(move |context| {
            started_tx.send(()).unwrap();
            while !context.is_cancellation_requested() {
                std::thread::yield_now();
            }
            cancelled_tx.send(()).unwrap();
            WorkOutcome::Cancelled
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let queued_executions = Arc::clone(&executions);
    let queued = engine
        .submit_test(Box::new(move |_| {
            queued_executions.fetch_add(1, Ordering::SeqCst);
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    let clone = engine.clone();
    drop(engine);
    assert_eq!(clone.lifecycle(), EngineLifecycle::Open);
    drop(clone);
    cancelled_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        if shared
            .registry
            .lock()
            .is_ok_and(|registry| registry.lifecycle == EngineLifecycle::Closed)
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let registry = shared.lock_registry_recover();
    assert_eq!(registry.records[&running].phase, TaskPhase::Cancelled);
    assert_eq!(registry.records[&queued].phase, TaskPhase::Cancelled);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
}

#[test]
fn poisoned_registry_never_masquerades_as_closed() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let shared = Arc::clone(&engine.inner.shared);
    let poison_shared = Arc::clone(&shared);
    let _ = std::thread::spawn(move || {
        let _guard = poison_shared.registry.lock().unwrap();
        panic!("poison registry for recovery test");
    })
    .join();

    assert_eq!(engine.lifecycle(), EngineLifecycle::Open);
    assert_eq!(engine.close(), CloseOutcome::Initiated);
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(engine.lifecycle(), EngineLifecycle::Closed);
}

#[test]
fn config_paths_do_not_depend_on_home() {
    let temp = TempDir::new().unwrap();
    let explicit = config(&temp);
    assert!(explicit.database_path().is_absolute());
    assert_ne!(explicit.database_path(), PathBuf::from("~/.dux"));
}
