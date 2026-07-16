use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::domain::{MAX_SCAN_ISSUE_OCCURRENCES, MAX_SCAN_ISSUES};
use crate::{ScanCoverage, ScanIssue, ScanIssueKind};

#[derive(Clone, Debug, PartialEq, Eq)]
struct IssueKey {
    kind: ScanIssueKind,
    path: Option<PathBuf>,
}

impl Ord for IssueKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.kind
            .canonical_rank()
            .cmp(&other.kind.canonical_rank())
            .then_with(|| self.path.cmp(&other.path))
    }
}

impl PartialOrd for IssueKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parallel walker callbacks record only bounded facts here. No filesystem
/// call is ever made while the accumulator mutex is held.
#[derive(Clone, Debug, Default)]
pub(super) struct IssueAccumulator {
    entries: BTreeMap<IssueKey, u32>,
    overflow_count: u32,
}

impl IssueAccumulator {
    fn is_terminal_cancellation(key: &IssueKey) -> bool {
        key.kind == ScanIssueKind::Cancelled && key.path.is_none()
    }

    fn largest_evictable_key(&self) -> Option<IssueKey> {
        self.entries
            .keys()
            .rev()
            .find(|key| !Self::is_terminal_cancellation(key))
            .cloned()
    }

    pub(super) fn record(&mut self, kind: ScanIssueKind, path: Option<PathBuf>) {
        let key = IssueKey { kind, path };
        if let Some(count) = self.entries.get_mut(&key) {
            *count = count.saturating_add(1).min(MAX_SCAN_ISSUE_OCCURRENCES);
            return;
        }

        // Reserve one final record for the fact that additional distinct
        // issues could not be retained.
        if ScanIssue::try_new(key.kind, key.path.clone(), 1).is_err() {
            self.add_overflow(1);
            return;
        }

        if self.entries.len() >= MAX_SCAN_ISSUES.saturating_sub(1) {
            let largest = self
                .largest_evictable_key()
                .expect("a full issue accumulator has an evictable key");
            if Self::is_terminal_cancellation(&key) || key < largest {
                let displaced = self
                    .entries
                    .remove(&largest)
                    .expect("the selected largest issue exists");
                self.add_overflow(displaced);
                self.entries.insert(key, 1);
            } else {
                self.add_overflow(1);
            }
            return;
        }
        self.entries.insert(key, 1);
    }

    fn add_overflow(&mut self, count: u32) {
        self.overflow_count = self
            .overflow_count
            .saturating_add(count)
            .min(MAX_SCAN_ISSUE_OCCURRENCES);
    }

    pub(super) fn record_once(&mut self, kind: ScanIssueKind, path: Option<PathBuf>) {
        let key = IssueKey { kind, path };
        if self.entries.contains_key(&key) {
            return;
        }
        self.record(key.kind, key.path);
    }

    pub(super) fn record_io_error(&mut self, path: PathBuf, error: &io::Error) {
        let kind = match error.kind() {
            io::ErrorKind::PermissionDenied => ScanIssueKind::PermissionDenied,
            io::ErrorKind::NotFound => ScanIssueKind::FileChangedDuringScan,
            io::ErrorKind::TimedOut => ScanIssueKind::TimedOut,
            _ => ScanIssueKind::MetadataError,
        };
        self.record(kind, Some(path));
    }

    pub(super) fn into_coverage(self) -> ScanCoverage {
        let mut issues = self
            .entries
            .into_iter()
            .map(|(key, count)| {
                ScanIssue::try_new(key.kind, key.path, count)
                    .expect("issue accumulator retains only validated facts")
            })
            .collect::<Vec<_>>();
        if self.overflow_count > 0
            && let Ok(overflow) =
                ScanIssue::try_new(ScanIssueKind::IssueLimitReached, None, self.overflow_count)
        {
            issues.push(overflow);
        }
        ScanCoverage::from_validated_terminal_issues(issues)
    }
}

pub(super) fn record_issue(
    issues: &std::sync::Mutex<IssueAccumulator>,
    kind: ScanIssueKind,
    path: Option<PathBuf>,
) {
    issues
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .record(kind, path);
}

pub(super) fn record_issue_once(
    issues: &std::sync::Mutex<IssueAccumulator>,
    kind: ScanIssueKind,
    path: Option<PathBuf>,
) {
    issues
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .record_once(kind, path);
}

pub(super) fn record_io_error(
    issues: &std::sync::Mutex<IssueAccumulator>,
    path: &Path,
    error: &io::Error,
) {
    issues
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .record_io_error(path.to_path_buf(), error);
}

pub(super) fn coverage_from_issues(issues: &std::sync::Mutex<IssueAccumulator>) -> ScanCoverage {
    issues
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .into_coverage()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(index: usize) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"C:\fixture\{index}"))
        } else {
            PathBuf::from(format!("/fixture/{index}"))
        }
    }

    #[test]
    fn duplicate_facts_aggregate_and_output_is_canonical() {
        let mut accumulator = IssueAccumulator::default();
        accumulator.record(ScanIssueKind::MetadataError, Some(path(2)));
        accumulator.record(ScanIssueKind::PermissionDenied, Some(path(1)));
        accumulator.record(ScanIssueKind::MetadataError, Some(path(2)));

        let coverage = accumulator.into_coverage();
        assert_eq!(coverage.issues().len(), 2);
        assert_eq!(coverage.issues()[0].kind(), ScanIssueKind::PermissionDenied);
        assert_eq!(coverage.issues()[1].occurrence_count(), 2);
    }

    #[test]
    fn distinct_issue_overflow_is_retained_as_a_fact() {
        let mut accumulator = IssueAccumulator::default();
        for index in 0..MAX_SCAN_ISSUES + 10 {
            accumulator.record(ScanIssueKind::MetadataError, Some(path(index)));
        }
        let coverage = accumulator.into_coverage();
        assert_eq!(coverage.issues().len(), MAX_SCAN_ISSUES);
        let overflow = coverage
            .issues()
            .iter()
            .find(|issue| issue.kind() == ScanIssueKind::IssueLimitReached)
            .unwrap();
        assert_eq!(overflow.occurrence_count(), 11);
    }

    #[test]
    fn retained_subset_is_deterministic_when_capacity_is_exceeded() {
        let mut forward = IssueAccumulator::default();
        let mut reverse = IssueAccumulator::default();
        for index in 0..MAX_SCAN_ISSUES + 10 {
            forward.record(ScanIssueKind::MetadataError, Some(path(index)));
        }
        for index in (0..MAX_SCAN_ISSUES + 10).rev() {
            reverse.record(ScanIssueKind::MetadataError, Some(path(index)));
        }

        assert_eq!(forward.into_coverage(), reverse.into_coverage());
    }

    #[test]
    fn terminal_cancellation_is_retained_even_after_issue_capacity_is_full() {
        let mut accumulator = IssueAccumulator::default();
        for index in 0..MAX_SCAN_ISSUES + 10 {
            accumulator.record(ScanIssueKind::MetadataError, Some(path(index)));
        }
        accumulator.record_once(ScanIssueKind::Cancelled, None);
        accumulator.record_once(ScanIssueKind::Cancelled, None);

        let coverage = accumulator.into_coverage();
        let cancellation = coverage
            .issues()
            .iter()
            .find(|issue| issue.kind() == ScanIssueKind::Cancelled)
            .unwrap();
        assert_eq!(cancellation.occurrence_count(), 1);
        assert_eq!(coverage.issues().len(), MAX_SCAN_ISSUES);
    }

    #[test]
    fn terminal_cancellation_stays_pinned_when_lower_ranked_facts_arrive_later() {
        let mut accumulator = IssueAccumulator::default();
        accumulator.record_once(ScanIssueKind::Cancelled, None);
        for index in 0..MAX_SCAN_ISSUES + 10 {
            accumulator.record(ScanIssueKind::PermissionDenied, Some(path(index)));
        }

        let coverage = accumulator.into_coverage();
        assert_eq!(coverage.issues().len(), MAX_SCAN_ISSUES);
        assert!(
            coverage
                .issues()
                .iter()
                .any(|issue| issue.kind() == ScanIssueKind::Cancelled)
        );
        assert!(
            coverage
                .issues()
                .iter()
                .any(|issue| issue.kind() == ScanIssueKind::IssueLimitReached)
        );
    }
}
