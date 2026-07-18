//! Generated performance fixtures for the immutable snapshot review path.
//!
//! The small fixture is a normal correctness regression. Exact million-node
//! runs are ignored so ordinary parallel test jobs do not hide memory pressure
//! or become host-speed tests. Run the ignored lane alone in Release and use
//! its versioned JSON observation for comparison; elapsed times are evidence,
//! never pass/fail thresholds.

use std::ffi::OsStr;
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use tempfile::TempDir;

use super::*;
use crate::engine::{
    MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS, SnapshotReviewError, SnapshotReviewNodeKind,
    SnapshotReviewNodeSort,
};
use crate::persistence::snapshot::{
    HostValue, SnapshotDocument, SnapshotMetadata, SnapshotNode, SnapshotNodeKind,
    SnapshotScanFlags, SnapshotTimestamp, SnapshotTotals,
};

const PERFORMANCE_REPORT_VERSION: u32 = 1;
const SMOKE_NODE_COUNT: usize = 10_000;
const MILLION_NODE_COUNT: usize = 1_000_000;
const FIVE_MILLION_NODE_COUNT: usize = 5_000_000;
const BALANCED_DIRECTORY_COUNT: usize = 999;
const PAGE_LIMIT: u16 = 100;
const TREEMAP_LIMIT: u16 = 48;
const FIXTURE_CAPTURE_SECONDS: u64 = 1_750_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FixtureShape {
    Balanced,
    Wide,
}

impl FixtureShape {
    fn from_environment() -> Self {
        match std::env::var("DUX_SNAPSHOT_PERF_SHAPE").as_deref() {
            Ok("wide") => Self::Wide,
            Ok("balanced") | Err(_) => Self::Balanced,
            Ok(value) => panic!("unsupported DUX_SNAPSHOT_PERF_SHAPE {value:?}"),
        }
    }
}

#[derive(Debug, Serialize)]
struct PerformanceReport {
    report_version: u32,
    shape: FixtureShape,
    node_count: u64,
    directory_count: u64,
    file_count: u64,
    wire_bytes: u64,
    build_ms: u64,
    publish_ms: u64,
    acquire_ms: u64,
    first_review_ms: u64,
    root_page_ms: Option<u64>,
    cached_page_ms: Option<u64>,
    treemap_ms: Option<u64>,
    large_files_ms: Option<u64>,
    review_outcome: &'static str,
    root_page_nodes: Option<usize>,
    treemap_cells: Option<usize>,
    large_file_results: Option<usize>,
}

struct GeneratedFixture {
    document: SnapshotDocument,
    counts: ScanCounts,
}

#[test]
fn generated_snapshot_performance_fixture_smoke_uses_real_review_path() {
    let report = run_fixture(SMOKE_NODE_COUNT, FixtureShape::Balanced);
    assert_eq!(report.review_outcome, "reviewed");
    assert_eq!(report.node_count, SMOKE_NODE_COUNT as u64);
    assert_eq!(report.root_page_nodes, Some(PAGE_LIMIT as usize));
    assert_eq!(report.treemap_cells, Some(TREEMAP_LIMIT as usize));
    assert_eq!(
        report.large_file_results,
        Some(MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS as usize)
    );
}

#[test]
fn generated_wide_snapshot_performance_fixture_smoke_uses_real_review_path() {
    let report = run_fixture(SMOKE_NODE_COUNT, FixtureShape::Wide);
    assert_eq!(report.review_outcome, "wide_reviewed");
    assert_eq!(report.node_count, SMOKE_NODE_COUNT as u64);
    assert_eq!(report.root_page_nodes, Some(PAGE_LIMIT as usize));
    assert_eq!(report.treemap_cells, Some(TREEMAP_LIMIT as usize));
    assert_eq!(
        report.large_file_results,
        Some(MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS as usize)
    );
}

#[test]
#[ignore = "isolated Release-only 1M/5M snapshot performance lane"]
fn million_node_snapshot_review_performance_fixture() {
    if cfg!(debug_assertions) {
        panic!("run the million-node fixture with cargo test --release");
    }
    let node_count = match std::env::var("DUX_SNAPSHOT_PERF_NODES") {
        Ok(value) => value
            .parse::<usize>()
            .expect("DUX_SNAPSHOT_PERF_NODES must be an integer"),
        Err(_) => MILLION_NODE_COUNT,
    };
    assert!(
        matches!(node_count, MILLION_NODE_COUNT | FIVE_MILLION_NODE_COUNT),
        "the isolated lane accepts exactly 1,000,000 or 5,000,000 nodes"
    );
    let shape = FixtureShape::from_environment();
    assert!(
        shape == FixtureShape::Balanced || node_count == MILLION_NODE_COUNT,
        "the wide fixture is fixed at exactly one million nodes"
    );

    let report = run_fixture(node_count, shape);
    println!(
        "DUX_SNAPSHOT_PERFORMANCE_JSON={}",
        serde_json::to_string(&report).unwrap()
    );

    match (node_count, shape) {
        (MILLION_NODE_COUNT, FixtureShape::Balanced) => {
            assert_eq!(report.review_outcome, "reviewed");
            assert_eq!(report.root_page_nodes, Some(PAGE_LIMIT as usize));
            assert_eq!(report.treemap_cells, Some(TREEMAP_LIMIT as usize));
        }
        (MILLION_NODE_COUNT, FixtureShape::Wide) => {
            assert_eq!(report.review_outcome, "wide_reviewed");
            assert_eq!(report.root_page_nodes, Some(PAGE_LIMIT as usize));
            assert_eq!(report.treemap_cells, Some(TREEMAP_LIMIT as usize));
        }
        (FIVE_MILLION_NODE_COUNT, FixtureShape::Balanced) => {
            assert_eq!(report.review_outcome, "decoded_review_budget_exceeded");
        }
        _ => unreachable!(),
    }
}

fn run_fixture(node_count: usize, shape: FixtureShape) -> PerformanceReport {
    let temp = TempDir::new().unwrap();
    let config = EngineConfig::new(
        temp.path().join("data/dux.sqlite3"),
        temp.path().join("data/snapshots"),
        temp.path().join("cache"),
    )
    .unwrap();
    let root = temp.path().join("synthetic-snapshot-root");
    std::fs::create_dir(&root).unwrap();
    let engine = EngineHandle::open(config.clone()).unwrap();

    let build_started = Instant::now();
    let fixture = generate_fixture(node_count, shape, &root);
    let build_ms = elapsed_ms(build_started);
    let scan_id = fixture.document.metadata.scan_id.clone();
    let started_at = SystemTime::UNIX_EPOCH + Duration::from_secs(FIXTURE_CAPTURE_SECONDS);
    engine
        .inner
        .store
        .record_scan_started(&NewScanRecord::try_new(scan_id.clone(), root, started_at).unwrap())
        .unwrap();

    let publish_started = Instant::now();
    let reference = engine
        .inner
        .snapshots
        .complete_scan(
            started_at + Duration::from_secs(1),
            fixture.counts,
            &ScanCoverage::try_from_terminal(None, Vec::new()).unwrap(),
            &fixture.document,
        )
        .unwrap();
    let publish_ms = elapsed_ms(publish_started);
    let wire_bytes = std::fs::metadata(
        config
            .snapshots_directory()
            .join(reference.file_name().as_str()),
    )
    .unwrap()
    .len();
    let directory_count = fixture.document.metadata.totals.directory_count;
    let file_count = fixture.document.metadata.totals.file_count;
    drop(fixture);

    let acquire_started = Instant::now();
    let mut review = engine.acquire_explorer_snapshot_review(&scan_id).unwrap();
    let acquire_ms = elapsed_ms(acquire_started);
    let first_review_started = Instant::now();
    let root_node = match review.root_node() {
        Ok(root_node) => root_node,
        Err(SnapshotReviewError::BudgetExceeded) => {
            let first_review_ms = elapsed_ms(first_review_started);
            review.release().unwrap();
            engine.close();
            assert!(engine.wait_until_closed(Duration::from_secs(5)));
            return PerformanceReport {
                report_version: PERFORMANCE_REPORT_VERSION,
                shape,
                node_count: node_count as u64,
                directory_count,
                file_count,
                wire_bytes,
                build_ms,
                publish_ms,
                acquire_ms,
                first_review_ms,
                root_page_ms: None,
                cached_page_ms: None,
                treemap_ms: None,
                large_files_ms: None,
                review_outcome: "decoded_review_budget_exceeded",
                root_page_nodes: None,
                treemap_cells: None,
                large_file_results: None,
            };
        }
        Err(error) => panic!("unexpected first-review failure: {error:?}"),
    };
    let first_review_ms = elapsed_ms(first_review_started);
    assert_eq!(root_node.id, 0);
    assert_eq!(root_node.kind, SnapshotReviewNodeKind::Directory);
    assert_eq!(root_node.file_count, file_count);

    let root_page_started = Instant::now();
    let root_page = review.child_nodes(
        root_node.id,
        SnapshotReviewNodeSort::NameAscending,
        0,
        PAGE_LIMIT,
    );
    let root_page_ms = elapsed_ms(root_page_started);
    let root_page = match root_page {
        Ok(page) => page,
        Err(SnapshotReviewError::BudgetExceeded) if shape == FixtureShape::Wide => {
            let large_files_started = Instant::now();
            let large_files = review
                .large_files(1, None, MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS)
                .unwrap();
            let large_files_ms = elapsed_ms(large_files_started);
            assert_eq!(large_files.total_matching_files, file_count);
            review.release().unwrap();
            engine.close();
            assert!(engine.wait_until_closed(Duration::from_secs(5)));
            return PerformanceReport {
                report_version: PERFORMANCE_REPORT_VERSION,
                shape,
                node_count: node_count as u64,
                directory_count,
                file_count,
                wire_bytes,
                build_ms,
                publish_ms,
                acquire_ms,
                first_review_ms,
                root_page_ms: Some(root_page_ms),
                cached_page_ms: None,
                treemap_ms: None,
                large_files_ms: Some(large_files_ms),
                review_outcome: "wide_sort_budget_exceeded",
                root_page_nodes: None,
                treemap_cells: None,
                large_file_results: Some(large_files.files.len()),
            };
        }
        Err(error) => panic!("unexpected root-page failure: {error:?}"),
    };
    assert_eq!(root_page.total_children, root_node.child_count);
    let (cached_parent, cached_sort, cached_total) = if shape == FixtureShape::Balanced {
        let first_directory = root_page
            .nodes
            .iter()
            .find(|node| node.kind == SnapshotReviewNodeKind::Directory)
            .expect("balanced root page must contain a directory");
        let nested_page = review
            .child_nodes(
                first_directory.id,
                SnapshotReviewNodeSort::LogicalBytesDescending,
                0,
                PAGE_LIMIT,
            )
            .unwrap();
        (
            first_directory.id,
            SnapshotReviewNodeSort::LogicalBytesDescending,
            nested_page.total_children,
        )
    } else {
        (
            root_node.id,
            SnapshotReviewNodeSort::NameAscending,
            root_page.total_children,
        )
    };
    let cached_offset = cached_total.min(u64::from(PAGE_LIMIT));
    let cached_page_started = Instant::now();
    let cached_page = review
        .child_nodes(cached_parent, cached_sort, cached_offset, PAGE_LIMIT)
        .unwrap();
    let cached_page_ms = elapsed_ms(cached_page_started);
    assert_eq!(cached_page.offset, cached_offset);

    let treemap_started = Instant::now();
    let treemap = review.treemap(root_node.id, TREEMAP_LIMIT).unwrap();
    let treemap_ms = elapsed_ms(treemap_started);
    assert_eq!(treemap.total_children, root_node.child_count);
    assert_eq!(
        treemap.other_child_count + treemap.cells.len() as u64,
        treemap.total_children
    );

    let large_files_started = Instant::now();
    let large_files = review
        .large_files(1, None, MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS)
        .unwrap();
    let large_files_ms = elapsed_ms(large_files_started);
    assert_eq!(large_files.total_matching_files, file_count);
    assert_eq!(
        large_files.files.len(),
        MAX_SNAPSHOT_REVIEW_LARGE_FILE_RESULTS as usize
    );

    review.release().unwrap();
    engine.close();
    assert!(engine.wait_until_closed(Duration::from_secs(5)));
    PerformanceReport {
        report_version: PERFORMANCE_REPORT_VERSION,
        shape,
        node_count: node_count as u64,
        directory_count,
        file_count,
        wire_bytes,
        build_ms,
        publish_ms,
        acquire_ms,
        first_review_ms,
        root_page_ms: Some(root_page_ms),
        cached_page_ms: Some(cached_page_ms),
        treemap_ms: Some(treemap_ms),
        large_files_ms: Some(large_files_ms),
        review_outcome: if shape == FixtureShape::Balanced {
            "reviewed"
        } else {
            "wide_reviewed"
        },
        root_page_nodes: Some(root_page.nodes.len()),
        treemap_cells: Some(treemap.cells.len()),
        large_file_results: Some(large_files.files.len()),
    }
}

fn generate_fixture(
    node_count: usize,
    shape: FixtureShape,
    root: &std::path::Path,
) -> GeneratedFixture {
    assert!(node_count >= 3);
    let file_count = match shape {
        FixtureShape::Balanced => {
            node_count - 1 - BALANCED_DIRECTORY_COUNT.min(node_count.saturating_sub(2))
        }
        FixtureShape::Wide => node_count - 1,
    };
    let directory_children = match shape {
        FixtureShape::Balanced => BALANCED_DIRECTORY_COUNT.min(node_count.saturating_sub(2)),
        FixtureShape::Wide => 0,
    };
    let directory_count = directory_children + 1;
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(node_count).unwrap();
    nodes.push(snapshot_node(
        0,
        None,
        0,
        SnapshotNodeKind::Directory,
        None,
        file_count as u64,
        file_count as u64,
        match shape {
            FixtureShape::Balanced => directory_children as u64,
            FixtureShape::Wide => file_count as u64,
        },
    ));

    let mut next_id = 1_u64;
    if shape == FixtureShape::Balanced {
        let files_per_directory = file_count / directory_children;
        let directories_with_extra_file = file_count % directory_children;
        for directory_ordinal in 0..directory_children {
            let children =
                files_per_directory + usize::from(directory_ordinal < directories_with_extra_file);
            let directory_name = format!("dir-{directory_ordinal:04}");
            let directory_id = next_id;
            nodes.push(snapshot_node(
                directory_id,
                Some(0),
                1,
                SnapshotNodeKind::Directory,
                Some(&directory_name),
                children as u64,
                children as u64,
                children as u64,
            ));
            next_id += 1;
            for file_ordinal in 0..children {
                let file_name = format!("file-{file_ordinal:07}");
                nodes.push(snapshot_node(
                    next_id,
                    Some(directory_id),
                    2,
                    SnapshotNodeKind::File,
                    Some(&file_name),
                    1,
                    1,
                    0,
                ));
                next_id += 1;
            }
        }
    } else {
        for file_ordinal in 0..file_count {
            let file_name = format!("file-{file_ordinal:07}");
            nodes.push(snapshot_node(
                next_id,
                Some(0),
                1,
                SnapshotNodeKind::File,
                Some(&file_name),
                1,
                1,
                0,
            ));
            next_id += 1;
        }
    }
    assert_eq!(nodes.len(), node_count);
    assert_eq!(next_id, node_count as u64);

    let scan_id = ScanId::new(format!(
        "scan:performance:{}:{node_count}",
        match shape {
            FixtureShape::Balanced => "balanced",
            FixtureShape::Wide => "wide",
        }
    ))
    .unwrap();
    GeneratedFixture {
        document: SnapshotDocument {
            metadata: SnapshotMetadata {
                scan_id,
                root: HostValue::from_root(root).unwrap(),
                captured_at: SnapshotTimestamp::new(FIXTURE_CAPTURE_SECONDS, 0).unwrap(),
                totals: SnapshotTotals {
                    directory_count: directory_count as u64,
                    file_count: file_count as u64,
                    logical_bytes: file_count as u64,
                    allocated_bytes: Some(file_count as u64),
                },
            },
            nodes,
        },
        counts: ScanCounts {
            directory_count: directory_count as u64,
            file_count: file_count as u64,
            logical_bytes: file_count as u64,
            allocated_bytes: Some(file_count as u64),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn snapshot_node(
    id: u64,
    parent: Option<u64>,
    depth: u32,
    kind: SnapshotNodeKind,
    name: Option<&str>,
    logical_bytes: u64,
    file_count: u64,
    child_count: u64,
) -> SnapshotNode {
    SnapshotNode {
        id,
        parent,
        depth,
        kind,
        name: name.map(|name| HostValue::from_component(OsStr::new(name)).unwrap()),
        logical_bytes,
        allocated_bytes: Some(logical_bytes),
        file_count,
        child_count,
        modified_at: None,
        accessed_at: None,
        scan_flags: SnapshotScanFlags::NONE,
        unix_identity: None,
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
