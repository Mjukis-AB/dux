use std::collections::BTreeSet;
use std::fs;

use serde_json::Value;
use sha2::{Digest, Sha256};

const CATALOG_PATH: &str = "catalogs/candidate-rules-v1.json";
const EXPECTED_SHA256: &str = "63669eace629b010d78e8d4cd9fc72bf76dbf05a3c8edb64ec146b6b37f87c60";

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
        assert_eq!(
            rule.get("safety").and_then(Value::as_str),
            Some("informational"),
            "the discovery catalog must not grant cleanup safety"
        );
        assert_eq!(
            rule.get("action").and_then(Value::as_str),
            Some("reveal_only"),
            "the discovery catalog must remain reveal-only"
        );
        assert_eq!(
            rule.get("schedule_eligible").and_then(Value::as_bool),
            Some(false),
            "the discovery catalog must never be schedulable"
        );
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
