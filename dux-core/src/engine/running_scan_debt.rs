//! Path-free diagnostic census for legacy unclaimed running scan rows.
//!
//! The census is read-only evidence. It never exposes row identity and cannot
//! recover, interrupt, remove, or otherwise mutate a scan.

pub const MAX_RUNNING_SCAN_DEBT_CENSUS_ROWS: u16 = 64;

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
