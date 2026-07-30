//! Path-free diagnostic census for legacy unclaimed running scan rows.
//!
//! The census is read-only evidence. It never exposes row identity and cannot
//! recover, interrupt, remove, or otherwise mutate a scan.

pub const MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS: u16 = 64;
pub const MAX_CLAIMED_RUNNING_SCAN_PROVENANCE_CENSUS_ROWS: u16 = MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunningScanDebtCensus {
    inspected_unclaimed_count: u16,
    pristine_unclaimed_count: u16,
    unexplained_unclaimed_count: u16,
    has_more: bool,
}

impl RunningScanDebtCensus {
    pub(super) const fn new(
        inspected_unclaimed_count: u16,
        pristine_unclaimed_count: u16,
        unexplained_unclaimed_count: u16,
        has_more: bool,
    ) -> Self {
        Self {
            inspected_unclaimed_count,
            pristine_unclaimed_count,
            unexplained_unclaimed_count,
            has_more,
        }
    }

    pub const fn inspected_unclaimed_count(&self) -> u16 {
        self.inspected_unclaimed_count
    }

    pub const fn pristine_unclaimed_count(&self) -> u16 {
        self.pristine_unclaimed_count
    }

    pub const fn unexplained_unclaimed_count(&self) -> u16 {
        self.unexplained_unclaimed_count
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// One bounded, path-free partition of claimed running scans by the
/// relationship between their stored provenance and this process's context.
///
/// These categories are bookkeeping observations, not liveness, recovery, or
/// cleanup decisions. `has_more` means only that rows beyond this fixed page
/// were not inspected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClaimedRunningScanProvenanceCensus {
    inspected_claimed_count: u16,
    same_host_current_boot_count: u16,
    same_host_prior_boot_count: u16,
    foreign_host_count: u16,
    stored_unproven_count: u16,
    current_context_unavailable_count: u16,
    has_more: bool,
}

impl ClaimedRunningScanProvenanceCensus {
    pub(super) const fn new(
        inspected_claimed_count: u16,
        same_host_current_boot_count: u16,
        same_host_prior_boot_count: u16,
        foreign_host_count: u16,
        stored_unproven_count: u16,
        current_context_unavailable_count: u16,
        has_more: bool,
    ) -> Self {
        Self {
            inspected_claimed_count,
            same_host_current_boot_count,
            same_host_prior_boot_count,
            foreign_host_count,
            stored_unproven_count,
            current_context_unavailable_count,
            has_more,
        }
    }

    pub const fn inspected_claimed_count(&self) -> u16 {
        self.inspected_claimed_count
    }

    pub const fn same_host_current_boot_count(&self) -> u16 {
        self.same_host_current_boot_count
    }

    pub const fn same_host_prior_boot_count(&self) -> u16 {
        self.same_host_prior_boot_count
    }

    pub const fn foreign_host_count(&self) -> u16 {
        self.foreign_host_count
    }

    pub const fn stored_unproven_count(&self) -> u16 {
        self.stored_unproven_count
    }

    pub const fn current_context_unavailable_count(&self) -> u16 {
        self.current_context_unavailable_count
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RunningScanDebtCensusError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the running-scan debt query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable running-scan debt evidence is corrupt")]
    CorruptData,
    #[error("durable running-scan debt evidence is unavailable")]
    Unavailable,
    #[error("engine running-scan debt state is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClaimedRunningScanProvenanceCensusError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the claimed running-scan provenance query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable claimed running-scan provenance evidence is corrupt")]
    CorruptData,
    #[error("durable claimed running-scan provenance evidence is unavailable")]
    Unavailable,
    #[error("engine claimed running-scan provenance state is unavailable")]
    InternalState,
}
