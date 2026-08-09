//! Path- and identity-free observations of active cleanup recovery state.

pub const MAX_CLEANUP_RECOVERY_DIAGNOSTIC_CENSUS_ROWS: u16 = 64;

/// One bounded partition of active cleanup journals by lifecycle phase and
/// stored/current execution provenance.
///
/// These counts are diagnostic evidence only. They contain no selector,
/// liveness fact, recovery authority, or cleanup-effect authority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CleanupRecoveryDiagnosticCensus {
    inspected_active_count: u16,
    running_count: u16,
    recovering_count: u16,
    same_host_current_boot_count: u16,
    same_host_prior_boot_count: u16,
    foreign_host_count: u16,
    stored_unproven_count: u16,
    current_context_unavailable_count: u16,
    has_more: bool,
}

impl CleanupRecoveryDiagnosticCensus {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        inspected_active_count: u16,
        running_count: u16,
        recovering_count: u16,
        same_host_current_boot_count: u16,
        same_host_prior_boot_count: u16,
        foreign_host_count: u16,
        stored_unproven_count: u16,
        current_context_unavailable_count: u16,
        has_more: bool,
    ) -> Self {
        Self {
            inspected_active_count,
            running_count,
            recovering_count,
            same_host_current_boot_count,
            same_host_prior_boot_count,
            foreign_host_count,
            stored_unproven_count,
            current_context_unavailable_count,
            has_more,
        }
    }

    pub const fn inspected_active_count(&self) -> u16 {
        self.inspected_active_count
    }

    pub const fn running_count(&self) -> u16 {
        self.running_count
    }

    pub const fn recovering_count(&self) -> u16 {
        self.recovering_count
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
pub enum CleanupRecoveryDiagnosticCensusError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the cleanup recovery diagnostic query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable cleanup recovery diagnostic evidence is corrupt")]
    CorruptData,
    #[error("durable cleanup recovery diagnostic evidence is unavailable")]
    Unavailable,
    #[error("engine cleanup recovery diagnostic state is unavailable")]
    InternalState,
}
