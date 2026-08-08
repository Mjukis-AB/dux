use super::*;
use std::sync::{Arc, Barrier};
use std::thread;

use crate::DATABASE_SCHEMA_VERSION;
use crate::persistence::StoreCoordinator;
use tempfile::TempDir;

const BASE_HOUR_MS: u64 = 1_800_000_000_000;

fn sample(pressure: DiskPressure) -> RawCapacitySample {
    sample_at("volume:test", BASE_HOUR_MS + 123, 200, None, pressure)
}

fn sample_at(
    volume_id: &str,
    sampled_at_unix_ms: u64,
    available_bytes: u64,
    important_available_bytes: Option<u64>,
    pressure: DiskPressure,
) -> RawCapacitySample {
    RawCapacitySample::try_new(
        VolumeId::new(volume_id).unwrap(),
        PathBuf::from("/"),
        "Startup".to_owned(),
        "apfs".to_owned(),
        true,
        false,
        UNIX_EPOCH + Duration::from_millis(sampled_at_unix_ms),
        1_000,
        available_bytes,
        important_available_bytes,
        pressure,
    )
    .unwrap()
}

fn observation_at(sampled_at_unix_ms: u64, available_bytes: u64) -> RawCapacityObservation {
    RawCapacityObservation::try_new(
        VolumeId::new("volume:observed").unwrap(),
        PathBuf::from("/"),
        "Startup".to_owned(),
        "apfs".to_owned(),
        true,
        false,
        UNIX_EPOCH + Duration::from_millis(sampled_at_unix_ms),
        VolumeCapacity::new(1_024 * 1_024 * 1_024 * 1_024, Some(available_bytes), None).unwrap(),
    )
    .unwrap()
}

#[test]
fn pressure_episodes_are_atomic_idempotent_and_hysteresis_friendly() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let volume = "volume:observed";
    let warning_at = BASE_HOUR_MS;
    let critical_at = BASE_HOUR_MS + 60_000;
    let healthy_at = BASE_HOUR_MS + 120_000;

    let warning = sample_at(volume, warning_at, 100, None, DiskPressure::Warning);
    assert_eq!(
        store
            .record_raw_capacity_sample(&warning, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Inserted
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&warning, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::ExistingExact
    );

    let critical = sample_at(volume, critical_at, 50, None, DiskPressure::Critical);
    store
        .record_raw_capacity_sample(&critical, CapacityWriteReason::PressureTransition)
        .unwrap();
    let healthy = sample_at(volume, healthy_at, 900, None, DiskPressure::Healthy);
    store
        .record_raw_capacity_sample(&healthy, CapacityWriteReason::PressureTransition)
        .unwrap();

    let episodes = store
        .load_pressure_episode_page(&VolumeId::new(volume).unwrap(), 10)
        .unwrap();
    assert_eq!(episodes.len(), 2);
    assert_eq!(episodes[0].pressure, DiskPressure::Critical);
    assert_eq!(
        episodes[0].entered_at,
        UNIX_EPOCH + Duration::from_millis(critical_at)
    );
    assert_eq!(
        episodes[0].exited_at,
        Some(UNIX_EPOCH + Duration::from_millis(healthy_at))
    );
    assert_eq!(episodes[1].pressure, DiskPressure::Warning);
    assert_eq!(
        episodes[1].exited_at,
        Some(UNIX_EPOCH + Duration::from_millis(critical_at))
    );
}

#[test]
fn pressure_episode_reader_accepts_only_provable_observation_anchors() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let volume = "volume:pressure-anchor";
    let first_at = BASE_HOUR_MS;
    let current_at = BASE_HOUR_MS + 100;
    let id = VolumeId::new(volume).unwrap();

    let first = sample_at(volume, first_at, 900, None, DiskPressure::Healthy);
    assert_eq!(
        store
            .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Inserted
    );
    let current = sample_at(volume, current_at, 890, None, DiskPressure::Healthy);
    assert_eq!(
        store
            .record_raw_capacity_sample(&current, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Suppressed
    );

    assert!(
        store
            .load_pressure_episode_page_at_anchor(
                &id,
                UNIX_EPOCH + Duration::from_millis(first_at),
                10,
            )
            .unwrap()
            .is_empty(),
        "an exact durable raw observation is a valid historical anchor",
    );
    assert!(
        store
            .load_pressure_episode_page_at_anchor(
                &id,
                UNIX_EPOCH + Duration::from_millis(current_at),
                10,
            )
            .unwrap()
            .is_empty(),
        "current last_seen proves an accepted cadence-suppressed observation",
    );
    assert_eq!(
        store
            .load_pressure_episode_page_at_anchor(
                &id,
                UNIX_EPOCH + Duration::from_millis(first_at + 50),
                10,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidInput,
        "an arbitrary instant between accepted observations must fail closed",
    );

    store.with_connection(|connection| {
        connection
            .execute("DELETE FROM disk_samples WHERE volume_id = ?1", [volume])
            .unwrap();
    });
    assert_eq!(
        store
            .load_pressure_episode_page_at_anchor(
                &id,
                UNIX_EPOCH + Duration::from_millis(current_at),
                10,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData,
        "a current last_seen without the raw history it extends is corrupt",
    );
}

#[test]
fn pressure_episode_reader_rejects_overlapping_history() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let sample = sample_at(
        "volume:overlap",
        BASE_HOUR_MS,
        100,
        None,
        DiskPressure::Healthy,
    );
    store
        .record_raw_capacity_sample(&sample, CapacityWriteReason::Routine)
        .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO disk_pressure_episodes (
                     volume_id, pressure, entered_at_unix_ms, exited_at_unix_ms, policy_revision
                 ) VALUES ('volume:overlap', 'warning', 100, 300, 0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO disk_pressure_episodes (
                     volume_id, pressure, entered_at_unix_ms, exited_at_unix_ms, policy_revision
                 ) VALUES ('volume:overlap', 'critical', 200, 400, 0)",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        store
            .load_pressure_episode_page(&VolumeId::new("volume:overlap").unwrap(), 10)
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn pressure_episode_reader_validates_lookahead_boundary_and_open_order() {
    for (case, older_exit) in [("overlap", Some(70_i64)), ("older-open", None)] {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let volume = format!("volume:pressure-{case}");
        for offset in [0_u64, 100] {
            let sample = sample_at(
                &volume,
                BASE_HOUR_MS + offset,
                900,
                None,
                DiskPressure::Healthy,
            );
            store
                .record_raw_capacity_sample(&sample, CapacityWriteReason::Routine)
                .unwrap();
        }
        let base = i64::try_from(BASE_HOUR_MS).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO disk_pressure_episodes (
                         volume_id, pressure, entered_at_unix_ms,
                         exited_at_unix_ms, policy_revision
                     ) VALUES (?1, 'warning', ?2, ?3, 0)",
                    params![
                        volume.as_str(),
                        base + 20,
                        older_exit.map(|value| base + value)
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO disk_pressure_episodes (
                         volume_id, pressure, entered_at_unix_ms,
                         exited_at_unix_ms, policy_revision
                     ) VALUES (?1, 'critical', ?2, ?3, 0)",
                    params![volume.as_str(), base + 60, base + 80],
                )
                .unwrap();
        });

        assert_eq!(
            store
                .load_pressure_episode_page(&VolumeId::new(&volume).unwrap(), 1)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData,
            "{case} hidden in the lookahead row must fail closed",
        );
    }
}

#[test]
fn atomic_observation_reconciles_ambiguous_insert_exact_suppression_and_transition() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let gib = 1_024 * 1_024 * 1_024;
    let first = observation_at(BASE_HOUR_MS, 100 * gib);
    let inserted = store
        .observe_capacity_after_commit_failure_for_test(&first)
        .unwrap();
    assert_eq!(inserted.write, Some(CapacityWriteOutcome::Inserted));

    let exact = store
        .observe_capacity_after_commit_failure_for_test(&first)
        .unwrap();
    assert_eq!(exact.write, Some(CapacityWriteOutcome::ExistingExact));

    let routine = observation_at(BASE_HOUR_MS + 60_000, 90 * gib);
    let suppressed = store
        .observe_capacity_after_commit_failure_for_test(&routine)
        .unwrap();
    assert_eq!(suppressed.write, Some(CapacityWriteOutcome::Suppressed));

    let transition = observation_at(BASE_HOUR_MS + 120_000, 5 * gib);
    let transitioned = store
        .observe_capacity_after_commit_failure_for_test(&transition)
        .unwrap();
    assert_eq!(transitioned.write, Some(CapacityWriteOutcome::Inserted));
    assert_eq!(transitioned.evaluation.pressure(), DiskPressure::Critical);
    store.with_connection(|connection| {
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM disk_samples", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            2
        );
    });
}

#[test]
fn input_is_validated_and_millisecond_canonical() {
    let mut value = sample(DiskPressure::Healthy);
    value.sampled_at += Duration::from_nanos(999_999);
    let normalized = RawCapacitySample::try_new(
        value.volume_id.clone(),
        value.mount_path.clone(),
        value.display_name.clone(),
        value.filesystem.clone(),
        value.is_internal,
        value.is_removable,
        value.sampled_at,
        value.total_bytes,
        value.available_bytes,
        value.important_available_bytes,
        value.pressure,
    )
    .unwrap();
    assert_eq!(
        normalized.sampled_at,
        sample(DiskPressure::Healthy).sampled_at
    );

    for (total, available, important) in [
        (0, 0, None),
        (10, 11, None),
        (10, 1, Some(11)),
        (i64::MAX as u64 + 1, 1, None),
    ] {
        assert_eq!(
            RawCapacitySample::try_new(
                VolumeId::new("volume:test").unwrap(),
                PathBuf::from("/"),
                "Startup".to_owned(),
                "apfs".to_owned(),
                true,
                false,
                UNIX_EPOCH,
                total,
                available,
                important,
                DiskPressure::Unknown,
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
    }
}

#[test]
fn paths_text_and_time_are_rejected_before_preparation() {
    let build = |path: PathBuf, name: String, filesystem: String, time: SystemTime| {
        RawCapacitySample::try_new(
            VolumeId::new("volume:test").unwrap(),
            path,
            name,
            filesystem,
            true,
            false,
            time,
            10,
            1,
            None,
            DiskPressure::Unknown,
        )
    };
    assert!(
        build(
            PathBuf::from("relative"),
            "x".into(),
            "apfs".into(),
            UNIX_EPOCH
        )
        .is_err()
    );
    assert!(build(PathBuf::from("/"), "".into(), "apfs".into(), UNIX_EPOCH).is_err());
    assert!(build(PathBuf::from("/"), " \t".into(), "apfs".into(), UNIX_EPOCH).is_err());
    assert!(build(PathBuf::from("/"), "x\n".into(), "apfs".into(), UNIX_EPOCH).is_err());
    assert!(build(PathBuf::from("/"), "x".into(), "".into(), UNIX_EPOCH).is_err());
    assert!(build(PathBuf::from("/"), "x".into(), "   ".into(), UNIX_EPOCH).is_err());
    assert!(
        build(
            PathBuf::from("/"),
            "x".into(),
            "apfs".into(),
            UNIX_EPOCH - Duration::from_millis(1),
        )
        .is_err()
    );
}

#[test]
fn all_pressure_values_have_stable_storage_names() {
    for (pressure, stored) in [
        (DiskPressure::Healthy, "healthy"),
        (DiskPressure::Warning, "warning"),
        (DiskPressure::Critical, "critical"),
        (DiskPressure::Unknown, "unknown"),
    ] {
        assert_eq!(pressure_as_stored(pressure), stored);
        assert_eq!(pressure_from_stored(stored).unwrap(), pressure);
    }
    assert!(pressure_from_stored("urgent").is_err());
}

#[test]
fn preparation_preserves_missing_important_capacity() {
    let prepared = PreparedCapacitySample::prepare(&sample(DiskPressure::Unknown)).unwrap();
    assert_eq!(prepared.important_available_bytes, None);
}

#[test]
fn page_limit_is_bounded_before_querying() {
    // The guard executes before the connection is used.
    let connection = Connection::open_in_memory().unwrap();
    let id = VolumeId::new("volume:test").unwrap();
    for limit in [0, MAX_PAGE_SIZE + 1] {
        assert_eq!(
            load_raw_capacity_page(&connection, &id, None, limit)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
    }
}

#[test]
fn raw_samples_round_trip_all_pressures_and_survive_reopen() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let pressures = [
        DiskPressure::Healthy,
        DiskPressure::Warning,
        DiskPressure::Critical,
        DiskPressure::Unknown,
    ];
    {
        let store = StoreCoordinator::open(&database).unwrap();
        for (index, pressure) in pressures.into_iter().enumerate() {
            let sample = sample_at(
                "volume:roundtrip",
                BASE_HOUR_MS + u64::try_from(index).unwrap() * UTC_HOUR_MS as u64,
                400 - u64::try_from(index).unwrap(),
                (index == 1).then_some(450),
                pressure,
            );
            assert_eq!(sample.mount_path(), Path::new("/"));
            assert_eq!(
                store
                    .record_raw_capacity_sample(&sample, CapacityWriteReason::Routine)
                    .unwrap(),
                CapacityWriteOutcome::Inserted
            );
        }
    }

    let reopened = StoreCoordinator::open(&database).unwrap();
    let volume_id = VolumeId::new("volume:roundtrip").unwrap();
    let latest = reopened
        .load_latest_raw_capacity_sample(&volume_id)
        .unwrap()
        .unwrap();
    assert_eq!(latest.pressure, DiskPressure::Unknown);
    assert_eq!(latest.important_available_bytes, None);
    assert_eq!(latest.total_bytes, 1_000);
    let page = reopened
        .load_raw_capacity_page(&volume_id, None, pressures.len())
        .unwrap();
    assert_eq!(page.samples.len(), pressures.len());
    assert_eq!(
        page.samples
            .iter()
            .rev()
            .map(|sample| sample.pressure)
            .collect::<Vec<_>>(),
        pressures
    );
    assert!(page.next_cursor.is_none());
}

#[test]
fn cadence_suppresses_routine_rows_but_keeps_real_transitions() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let first = sample_at(
        "volume:cadence",
        BASE_HOUR_MS + 10,
        300,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Inserted
    );
    let routine = sample_at(
        "volume:cadence",
        BASE_HOUR_MS + 1_000,
        290,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&routine, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Suppressed
    );
    let false_transition = sample_at(
        "volume:cadence",
        BASE_HOUR_MS + 2_000,
        280,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&false_transition, CapacityWriteReason::PressureTransition,)
            .unwrap(),
        CapacityWriteOutcome::Suppressed
    );
    let transition = sample_at(
        "volume:cadence",
        BASE_HOUR_MS + 3_000,
        270,
        Some(300),
        DiskPressure::Warning,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&transition, CapacityWriteReason::PressureTransition,)
            .unwrap(),
        CapacityWriteOutcome::Inserted
    );
    let next_hour = sample_at(
        "volume:cadence",
        BASE_HOUR_MS + UTC_HOUR_MS as u64,
        260,
        None,
        DiskPressure::Warning,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&next_hour, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Inserted
    );

    let page = store
        .load_raw_capacity_page(first.volume_id(), None, 10)
        .unwrap();
    assert_eq!(page.samples.len(), 3);
    assert_eq!(page.samples[0].sampled_at, next_hour.sampled_at());
    assert_eq!(page.samples[1].sampled_at, transition.sampled_at());
    assert_eq!(page.samples[2].sampled_at, first.sampled_at());
}

#[test]
fn transition_reason_requires_a_stored_pressure_baseline() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let value = sample_at(
        "volume:no-baseline",
        BASE_HOUR_MS,
        300,
        None,
        DiskPressure::Warning,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&value, CapacityWriteReason::PressureTransition)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    store.with_connection(|connection| {
        for table in ["volumes", "disk_samples"] {
            let count: i64 = connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        }
    });
}

#[test]
fn exact_retry_conflict_and_out_of_order_writes_are_distinct() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let first = sample_at(
        "volume:retry",
        BASE_HOUR_MS + 10,
        300,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Inserted
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::ExistingExact
    );

    let conflicting = sample_at(
        "volume:retry",
        BASE_HOUR_MS + 10,
        299,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&conflicting, CapacityWriteReason::Routine)
            .unwrap_err()
            .kind,
        HistoryErrorKind::AlreadyExists
    );
    let newer = sample_at(
        "volume:retry",
        BASE_HOUR_MS + UTC_HOUR_MS as u64,
        280,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&newer, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Inserted
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::ExistingExact
    );
    let stale = sample_at(
        "volume:retry",
        BASE_HOUR_MS + 20,
        295,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&stale, CapacityWriteReason::Routine)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        store
            .load_latest_raw_capacity_sample(first.volume_id())
            .unwrap()
            .unwrap()
            .available_bytes,
        280
    );
}

#[test]
fn samples_outside_volume_observation_interval_fail_closed() {
    for (case, first_offset, last_offset) in
        [("before-first", 1_i64, 2_i64), ("after-last", -2, -1)]
    {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let value = sample_at(
            &format!("volume:{case}"),
            BASE_HOUR_MS,
            300,
            None,
            DiskPressure::Healthy,
        );
        store
            .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
            .unwrap();

        let sampled_at = i64::try_from(BASE_HOUR_MS).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE volumes
                     SET first_seen_unix_ms = ?2, last_seen_unix_ms = ?3
                     WHERE volume_id = ?1",
                    params![
                        value.volume_id.as_str(),
                        sampled_at + first_offset,
                        sampled_at + last_offset,
                    ],
                )
                .unwrap();
        });

        assert_eq!(
            store
                .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData,
            "exact retry must reject {case} ordering",
        );
        assert_eq!(
            store
                .load_latest_raw_capacity_sample(value.volume_id())
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData,
            "latest read must reject {case} ordering",
        );
        assert_eq!(
            store
                .load_raw_capacity_page(value.volume_id(), None, 10)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData,
            "paged read must reject {case} ordering",
        );
    }
}

#[test]
fn volume_metadata_advances_monotonically_and_collision_rolls_back() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let first = sample_at(
        "volume:metadata",
        BASE_HOUR_MS + 1,
        400,
        None,
        DiskPressure::Healthy,
    );
    store
        .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
        .unwrap();
    let mut refreshed = sample_at(
        "volume:metadata",
        BASE_HOUR_MS + 2,
        390,
        None,
        DiskPressure::Healthy,
    );
    refreshed.mount_path = PathBuf::from("/Volumes/Renamed");
    refreshed.display_name = "Renamed".to_owned();
    assert_eq!(
        store
            .record_raw_capacity_sample(&refreshed, CapacityWriteReason::Routine)
            .unwrap(),
        CapacityWriteOutcome::Suppressed
    );

    let conflicting = RawCapacitySample::try_new(
        first.volume_id.clone(),
        PathBuf::from("/Volumes/Collision"),
        "Collision".to_owned(),
        "apfs".to_owned(),
        true,
        false,
        first.sampled_at,
        first.total_bytes,
        first.available_bytes + 1,
        first.important_available_bytes,
        first.pressure,
    )
    .unwrap();
    assert_eq!(
        store
            .record_raw_capacity_sample(&conflicting, CapacityWriteReason::Routine)
            .unwrap_err()
            .kind,
        HistoryErrorKind::AlreadyExists
    );

    store.with_connection(|connection| {
        let row: (String, i64, i64) = connection
            .query_row(
                "SELECT display_name, first_seen_unix_ms, last_seen_unix_ms
                 FROM volumes WHERE volume_id = ?1",
                [first.volume_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, "Renamed");
        assert_eq!(row.1, i64::try_from(BASE_HOUR_MS + 1).unwrap());
        assert_eq!(row.2, i64::try_from(BASE_HOUR_MS + 2).unwrap());
    });
}

#[test]
fn pages_are_bounded_stable_and_volume_isolated() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let pressures = [
        DiskPressure::Healthy,
        DiskPressure::Warning,
        DiskPressure::Healthy,
        DiskPressure::Warning,
        DiskPressure::Healthy,
    ];
    for (index, pressure) in pressures.into_iter().enumerate() {
        let value = sample_at(
            "volume:page-a",
            BASE_HOUR_MS + u64::try_from(index).unwrap(),
            500 - u64::try_from(index).unwrap(),
            None,
            pressure,
        );
        let reason = if index == 0 {
            CapacityWriteReason::Routine
        } else {
            CapacityWriteReason::PressureTransition
        };
        assert_eq!(
            store.record_raw_capacity_sample(&value, reason).unwrap(),
            CapacityWriteOutcome::Inserted
        );
    }
    let other = sample_at(
        "volume:page-b",
        BASE_HOUR_MS + 100,
        100,
        None,
        DiskPressure::Critical,
    );
    store
        .record_raw_capacity_sample(&other, CapacityWriteReason::Routine)
        .unwrap();

    let id = VolumeId::new("volume:page-a").unwrap();
    let first = store.load_raw_capacity_page(&id, None, 2).unwrap();
    assert_eq!(first.samples.len(), 2);
    let second = store
        .load_raw_capacity_page(&id, first.next_cursor, 2)
        .unwrap();
    assert_eq!(second.samples.len(), 2);
    let third = store
        .load_raw_capacity_page(&id, second.next_cursor, 2)
        .unwrap();
    assert_eq!(third.samples.len(), 1);
    assert!(third.next_cursor.is_none());
    let observed = first
        .samples
        .into_iter()
        .chain(second.samples)
        .chain(third.samples)
        .map(|sample| {
            system_time_to_unix_ms(sample.sampled_at, HistoryErrorKind::CorruptData).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        observed,
        (0..5)
            .rev()
            .map(|offset| i64::try_from(BASE_HOUR_MS).unwrap() + offset)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        store
            .load_raw_capacity_page(other.volume_id(), None, 10)
            .unwrap()
            .samples
            .len(),
        1
    );
}

#[test]
fn trend_reports_signed_changes_and_prefers_daily_points() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let day = UTC_DAY_MS as u64;
    let samples = [
        sample_at(
            "volume:trend",
            BASE_HOUR_MS + 100,
            900,
            Some(800),
            DiskPressure::Healthy,
        ),
        sample_at(
            "volume:trend",
            BASE_HOUR_MS + 2 * day + 100,
            800,
            Some(700),
            DiskPressure::Healthy,
        ),
        sample_at(
            "volume:trend",
            BASE_HOUR_MS + 7 * day + 100,
            700,
            Some(600),
            DiskPressure::Warning,
        ),
        sample_at(
            "volume:trend",
            BASE_HOUR_MS + 8 * day + 100,
            650,
            Some(550),
            DiskPressure::Warning,
        ),
    ];
    for (index, sample) in samples.iter().enumerate() {
        let reason = if index == 2 {
            CapacityWriteReason::PressureTransition
        } else {
            CapacityWriteReason::Routine
        };
        assert_eq!(
            store.record_raw_capacity_sample(sample, reason).unwrap(),
            CapacityWriteOutcome::Inserted
        );
    }
    let day_two_start = i64::try_from(BASE_HOUR_MS + 2 * day).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO disk_samples (
                    volume_id, sample_kind, sampled_at_unix_ms, total_bytes,
                    available_bytes, important_available_bytes, pressure, policy_revision
                 ) VALUES ('volume:trend', 'daily_rollup', ?1, 1000, 810, 710, 'healthy', 0)",
                [day_two_start],
            )
            .unwrap();
    });

    let anchor_at = UNIX_EPOCH + Duration::from_millis(BASE_HOUR_MS + 8 * day + 500);
    let trend = store
        .load_capacity_trend(&VolumeId::new("volume:trend").unwrap(), anchor_at)
        .unwrap()
        .unwrap();
    assert_eq!(trend.anchor.available_bytes, 650);
    let change_24h = trend.change_24h.unwrap();
    assert_eq!(change_24h.available_bytes, -50);
    assert_eq!(change_24h.important_available_bytes, Some(-50));
    let change_7d = trend.change_7d.unwrap();
    assert_eq!(change_7d.available_bytes, -250);
    assert_eq!(change_7d.important_available_bytes, Some(-250));
    assert_eq!(trend.points.len(), 4);
    assert_eq!(trend.points[0].sample.available_bytes, 900);
    assert_eq!(trend.points[1].sample.available_bytes, 810);
    assert_eq!(
        trend.points[1].source,
        CapacityTrendPointSource::DailyRollup
    );
    assert_eq!(trend.points[2].sample.available_bytes, 700);
    assert_eq!(trend.points[3].sample.available_bytes, 650);
    assert_eq!(trend.points[3].source, CapacityTrendPointSource::Raw);
}

#[test]
fn trend_requires_a_baseline_and_returns_none_without_an_anchor() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let id = VolumeId::new("volume:trend-empty").unwrap();
    assert!(
        store
            .load_capacity_trend(&id, UNIX_EPOCH + Duration::from_millis(BASE_HOUR_MS))
            .unwrap()
            .is_none()
    );
    let first = sample_at(
        "volume:trend-empty",
        BASE_HOUR_MS + 2 * UTC_DAY_MS as u64,
        500,
        None,
        DiskPressure::Healthy,
    );
    store
        .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
        .unwrap();
    let trend = store
        .load_capacity_trend(&id, first.sampled_at())
        .unwrap()
        .unwrap();
    assert!(trend.change_24h.is_none());
    assert!(trend.change_7d.is_none());
    assert_eq!(trend.points.len(), 1);
}

#[test]
fn concurrent_exact_writers_have_one_insert_and_one_adoption() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let sample = sample_at(
        "volume:race",
        BASE_HOUR_MS,
        300,
        None,
        DiskPressure::Healthy,
    );
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let store = Arc::clone(&store);
        let sample = sample.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            store
                .record_raw_capacity_sample(&sample, CapacityWriteReason::Routine)
                .unwrap()
        }));
    }
    barrier.wait();
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == CapacityWriteOutcome::Inserted)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == CapacityWriteOutcome::ExistingExact)
            .count(),
        1
    );
}

#[test]
fn post_commit_failures_reconcile_inserted_and_suppressed_outcomes() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let first = sample_at(
        "volume:ambiguous",
        BASE_HOUR_MS + 1,
        300,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample_after_commit_failure_for_test(
                &first,
                CapacityWriteReason::Routine,
            )
            .unwrap(),
        CapacityWriteOutcome::Inserted
    );
    let suppressed = sample_at(
        "volume:ambiguous",
        BASE_HOUR_MS + 2,
        299,
        None,
        DiskPressure::Healthy,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample_after_commit_failure_for_test(
                &suppressed,
                CapacityWriteReason::Routine,
            )
            .unwrap(),
        CapacityWriteOutcome::Suppressed
    );
    assert_eq!(
        store
            .load_raw_capacity_page(first.volume_id(), None, 10)
            .unwrap()
            .samples
            .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn post_commit_fact_match_cannot_mask_unsafe_storage() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let marker = database.with_extension("sqlite3.writer.lock");
    let store = StoreCoordinator::open(&database).unwrap();
    let value = sample_at(
        "volume:unsafe-post-commit",
        BASE_HOUR_MS,
        300,
        None,
        DiskPressure::Healthy,
    );

    let error = store
        .record_raw_capacity_sample_with_after_commit_hook_for_test(
            &value,
            CapacityWriteReason::Routine,
            || {
                std::fs::write(&marker, b"NOT-A-DUX-MARKER")
                    .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, HistoryErrorKind::OutcomeUnknown);

    // The exact row committed, so a fact-only recovery would have returned
    // success despite the now-invalid retained storage marker.
    store.with_connection(|connection| {
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM disk_samples WHERE volume_id = ?1",
                [value.volume_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    });
}

#[test]
fn malformed_rows_fail_closed_and_leave_query_budget_reusable() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let value = sample_at(
        "volume:corrupt",
        BASE_HOUR_MS,
        300,
        None,
        DiskPressure::Healthy,
    );
    store
        .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
        .unwrap();
    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE disk_samples SET pressure = 'urgent' WHERE volume_id = ?1",
                [value.volume_id.as_str()],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store
            .load_latest_raw_capacity_sample(value.volume_id())
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
    store.with_connection(|connection| {
        assert_eq!(
            connection
                .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    });
}

#[test]
fn malformed_volume_metadata_is_rejected_by_history_reads() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let value = sample_at(
        "volume:bad-metadata",
        BASE_HOUR_MS,
        300,
        None,
        DiskPressure::Healthy,
    );
    store
        .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
        .unwrap();
    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE volumes SET mount_path_encoding = 99 WHERE volume_id = ?1",
                [value.volume_id.as_str()],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store
            .load_raw_capacity_page(value.volume_id(), None, 10)
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn external_newer_schema_fences_capacity_writes() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let store = StoreCoordinator::open(&database).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO schema_migrations
                 (version, name, checksum_sha256, applied_at_unix_ms)
                 VALUES (?1, 'future-capacity', zeroblob(32), 2)",
                [i64::from(DATABASE_SCHEMA_VERSION + 1)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION + 1)
            .unwrap();
    });
    let value = sample_at(
        "volume:fenced",
        BASE_HOUR_MS,
        300,
        None,
        DiskPressure::Unknown,
    );
    assert_eq!(
        store
            .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
            .unwrap_err()
            .kind,
        HistoryErrorKind::IncompatibleSchema
    );
    store.with_connection(|connection| {
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM disk_samples", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    });
}
