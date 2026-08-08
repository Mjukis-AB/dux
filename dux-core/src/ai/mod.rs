//! Provider-neutral AI explanation contracts.
//!
//! This module is deliberately crate-private and imports no DUX domain,
//! engine, persistence, planner, filesystem, FFI, or cleanup type. A validated
//! input proves only wire shape and boundedness; it is not proof that privacy
//! redaction has run and cannot be sent to a provider by this module.

mod contract;

#[cfg(test)]
mod contract_tests;
