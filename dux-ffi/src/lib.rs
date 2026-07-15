//! Minimal foreign-language boundary for DUX.
//!
//! This first surface exists to prove Rust packaging and Swift binding
//! generation. It is not the final engine API or a localization contract.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Version information used to reject an incompatible generated binding.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LibraryVersion {
    /// Version of the `dux-ffi` Rust library.
    pub library_version: String,
    /// Version of the exported FFI contract, independent of product schemas.
    pub ffi_contract_version: u32,
}

/// One formatted byte count used by the initial Swift integration smoke test.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FormattedSize {
    /// Original byte count, preserved so the display string is not authoritative.
    pub bytes: u64,
    /// Current CLI-style rendering used only to prove a typed Rust result.
    pub display: String,
}

/// Stable errors produced by the engine-session boundary.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum EngineError {
    /// The session was explicitly closed and can no longer perform work.
    #[error("engine session is closed")]
    Closed,
}

/// Opaque application-scoped entry point to the shared engine.
///
/// The current methods still exercise the Phase 0 smoke surface. Later engine
/// extraction will add task and snapshot APIs without exposing core internals.
#[derive(uniffi::Object)]
pub struct DuxEngine {
    closed: AtomicBool,
}

impl Default for DuxEngine {
    fn default() -> Self {
        Self::new()
    }
}

const FFI_CONTRACT_VERSION: u32 = 2;
static LIVE_ENGINE_INSTANCE_COUNT: AtomicU64 = AtomicU64::new(0);

impl DuxEngine {
    fn ensure_open(&self) -> Result<(), EngineError> {
        if self.closed.load(Ordering::Acquire) {
            Err(EngineError::Closed)
        } else {
            Ok(())
        }
    }
}

#[uniffi::export]
impl DuxEngine {
    /// Create one application-scoped engine session.
    #[uniffi::constructor]
    pub fn new() -> Self {
        LIVE_ENGINE_INSTANCE_COUNT.fetch_add(1, Ordering::Relaxed);
        Self {
            closed: AtomicBool::new(false),
        }
    }

    /// Return library and FFI-contract versions for the integration handshake.
    pub fn library_version(&self) -> Result<LibraryVersion, EngineError> {
        self.ensure_open()?;
        Ok(LibraryVersion {
            library_version: env!("CARGO_PKG_VERSION").to_owned(),
            ffi_contract_version: FFI_CONTRACT_VERSION,
        })
    }

    /// Return a typed formatted-size value from `dux-core`.
    pub fn format_size(&self, bytes: u64) -> Result<FormattedSize, EngineError> {
        self.ensure_open()?;
        Ok(FormattedSize {
            bytes,
            display: dux_core::format_size(bytes),
        })
    }

    /// Close the session, returning whether this call performed the transition.
    pub fn close(&self) -> bool {
        !self.closed.swap(true, Ordering::AcqRel)
    }
}

impl Drop for DuxEngine {
    fn drop(&mut self) {
        LIVE_ENGINE_INSTANCE_COUNT.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Return the live Rust engine-object count for binding lifetime diagnostics.
///
/// This is an integration-test observation point, not application session state.
#[uniffi::export]
pub fn live_engine_instance_count() -> u64 {
    LIVE_ENGINE_INSTANCE_COUNT.load(Ordering::Relaxed)
}

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_library_and_contract_versions_through_engine() {
        let engine = DuxEngine::new();

        assert_eq!(
            engine.library_version(),
            Ok(LibraryVersion {
                library_version: env!("CARGO_PKG_VERSION").to_owned(),
                ffi_contract_version: 2,
            })
        );
    }

    #[test]
    fn delegates_size_formatting_to_core() {
        let engine = DuxEngine::new();
        let result = engine.format_size(1536).expect("open engine");

        assert_eq!(result.bytes, 1536);
        assert_eq!(result.display, dux_core::format_size(result.bytes));
        assert_eq!(result.display, "1.5 KB");
    }

    #[test]
    fn close_is_idempotent_and_rejects_later_calls() {
        let baseline = live_engine_instance_count();

        {
            let engine = DuxEngine::new();
            assert_eq!(live_engine_instance_count(), baseline + 1);
            assert!(engine.close());
            assert!(!engine.close());
            assert_eq!(engine.library_version(), Err(EngineError::Closed));
            assert_eq!(engine.format_size(1_536), Err(EngineError::Closed));
        }

        assert_eq!(live_engine_instance_count(), baseline);
    }
}
