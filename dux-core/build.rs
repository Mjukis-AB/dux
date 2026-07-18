use std::collections::BTreeSet;
use std::fs;

use serde_json::Value;
use sha2::{Digest, Sha256};

const CATALOG_PATH: &str = "catalogs/candidate-rules-v1.json";
const EXPECTED_SHA256: &str = "dd9155d39998592244c94c55fbc817b716d0ebfd40c38213e14466244a0a0c91";
const SAFE_RUST_RULE_ID: &str = "developer.rust.target";
const SAFE_PYTHON_PYCACHE_RULE_ID: &str = "developer.python.pycache";

fn main() {
    println!("cargo:rerun-if-changed={CATALOG_PATH}");
    let bytes = fs::read(CATALOG_PATH).expect("candidate catalog must be readable at build time");
    assert!(
        bytes.len() <= 1024 * 1024,
        "candidate catalog exceeds its one-megabyte build limit"
    );
    let actual = lower_hex(&Sha256::digest(&bytes));
    assert_eq!(
        actual, EXPECTED_SHA256,
        "candidate catalog bytes changed without a reviewed digest update"
    );

    let document: Value =
        serde_json::from_slice(&bytes).expect("candidate catalog must be valid JSON at build time");
    assert_eq!(
        document.get("schema_version").and_then(Value::as_u64),
        Some(1),
        "candidate catalog must use schema version 1"
    );
    let rules = document
        .get("rules")
        .and_then(Value::as_array)
        .expect("candidate catalog must contain a rules array");
    assert_eq!(rules.len(), 11, "candidate catalog rule count changed");
    let mut ids = BTreeSet::new();
    for rule in rules {
        let rule = rule
            .as_object()
            .expect("every candidate catalog rule must be an object");
        let id = rule
            .get("id")
            .and_then(Value::as_str)
            .expect("every candidate catalog rule must have a string ID");
        assert!(ids.insert(id), "candidate catalog rule IDs must be unique");
        assert_eq!(
            rule.get("scope").and_then(Value::as_str),
            Some("selected_scan_root"),
            "discovery catalog rules must bind the selected scan root"
        );
        let (expected_revision, expected_safety, expected_action) =
            if matches!(id, SAFE_RUST_RULE_ID | SAFE_PYTHON_PYCACHE_RULE_ID) {
                (2, "safe_regenerable", "remove_known_regenerable_contents")
            } else {
                (1, "informational", "reveal_only")
            };
        assert_eq!(
            rule.get("revision").and_then(Value::as_u64),
            Some(expected_revision),
            "candidate rule revision changed outside the exact allowlist"
        );
        assert_eq!(
            rule.get("safety").and_then(Value::as_str),
            Some(expected_safety),
            "candidate rule safety changed outside the exact allowlist"
        );
        assert_eq!(
            rule.get("action").and_then(Value::as_str),
            Some(expected_action),
            "candidate rule action changed outside the exact allowlist"
        );
        assert_eq!(
            rule.get("schedule_eligible").and_then(Value::as_bool),
            Some(false),
            "the discovery catalog must never be schedulable"
        );
        if id == SAFE_RUST_RULE_ID {
            assert_eq!(
                rule.get("required_ancestor_markers_any"),
                Some(&Value::Array(vec![Value::String("Cargo.toml".to_owned())])),
                "the Rust rule must retain its direct manifest evidence"
            );
            assert_eq!(
                rule.get("required_markers_all"),
                Some(&Value::Array(vec![Value::String(
                    "CACHEDIR.TAG".to_owned()
                )])),
                "the Rust rule must retain its Cargo cache-tag evidence"
            );
            assert_eq!(
                rule.get("provenance"),
                Some(&Value::Array(vec![
                    Value::String(
                        "https://doc.rust-lang.org/cargo/reference/build-cache.html".to_owned()
                    ),
                    Value::String(
                        "https://doc.rust-lang.org/cargo/commands/cargo-clean.html".to_owned()
                    ),
                ])),
                "the Rust rule must retain both reviewed Cargo sources"
            );
        } else if id == SAFE_PYTHON_PYCACHE_RULE_ID {
            assert_eq!(
                rule.get("required_ancestor_markers_any"),
                Some(&Value::Array(Vec::new())),
                "the Python rule must leave extension evidence to the classifier"
            );
            assert_eq!(
                rule.get("required_markers_all"),
                Some(&Value::Array(Vec::new())),
                "the Python rule must leave extension evidence to the classifier"
            );
            assert_eq!(
                rule.get("provenance"),
                Some(&Value::Array(vec![
                    Value::String(
                        "https://docs.python.org/3/reference/import.html#cached-bytecode-invalidation"
                            .to_owned(),
                    ),
                    Value::String(
                        "https://docs.python.org/3/faq/programming.html#how-do-i-create-a-pyc-file"
                            .to_owned(),
                    ),
                    Value::String("https://peps.python.org/pep-3147/".to_owned()),
                ])),
                "the Python rule must retain all reviewed sources"
            );
        }
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
