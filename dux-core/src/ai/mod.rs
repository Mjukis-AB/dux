//! Provider-neutral AI explanation contracts.
//!
//! This module is deliberately crate-private. Its provider-neutral contract
//! imports no DUX type; the sealed privacy child imports only complete scan
//! coverage and a validated immutable snapshot observation. Neither surface
//! reaches engine, planner, live filesystem, FFI, provider, or cleanup types.
//! Parsing input proves only wire shape and cannot mint the privacy proof.

mod contract;

#[cfg(test)]
mod contract_tests;
