use std::collections::BTreeSet;
use std::fs;

use serde_json::Value;
use sha2::{Digest, Sha256};

const CATALOG_PATH: &str = "catalogs/candidate-rules-v1.json";
const EXPECTED_SHA256: &str = "8ebb1d34c9362e13411b29bba2bcfae6f1225d1e2afec2e2223491c9cb28a7ef";
const SAFE_RUST_RULE_ID: &str = "developer.rust.target";
const SAFE_PYTHON_PYCACHE_RULE_ID: &str = "developer.python.pycache";
const SAFE_HOMEBREW_CACHE_RULE_ID: &str = "developer.homebrew.cache";
const SAFE_PIP_CACHE_RULE_ID: &str = "developer.python.pip_cache";

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
    assert_eq!(rules.len(), 13, "candidate catalog rule count changed");
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
        let expected_scope = match id {
            SAFE_HOMEBREW_CACHE_RULE_ID | SAFE_PIP_CACHE_RULE_ID => "user_cache_directory",
            _ => "selected_scan_root",
        };
        assert_eq!(
            rule.get("scope").and_then(Value::as_str),
            Some(expected_scope),
            "candidate rules must bind their exact reviewed discovery scope"
        );
        let (expected_revision, expected_safety, expected_action) = match id {
            SAFE_RUST_RULE_ID => (3, "safe_regenerable", "remove_known_regenerable_contents"),
            SAFE_PYTHON_PYCACHE_RULE_ID => {
                (2, "safe_regenerable", "remove_known_regenerable_contents")
            }
            SAFE_HOMEBREW_CACHE_RULE_ID | SAFE_PIP_CACHE_RULE_ID => {
                (1, "safe_regenerable", "remove_known_regenerable_contents")
            }
            _ => (1, "informational", "reveal_only"),
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
                rule.get("minimum_age_days").and_then(Value::as_u64),
                Some(7),
                "the Rust rule must retain its reviewed seven-day minimum age"
            );
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
        } else if id == SAFE_HOMEBREW_CACHE_RULE_ID {
            assert_eq!(
                rule.get("minimum_age_days").and_then(Value::as_u64),
                Some(7),
                "the Homebrew cache rule must retain its reviewed seven-day observation threshold"
            );
            assert_eq!(
                rule.get("path_component").and_then(Value::as_str),
                Some("Homebrew"),
                "the Homebrew cache rule must retain the exact conventional macOS component"
            );
            assert_eq!(
                rule.get("provenance"),
                Some(&Value::Array(vec![Value::String(
                    "https://docs.brew.sh/Manpage#cache-options".to_owned()
                )])),
                "the Homebrew cache rule must retain its reviewed source"
            );
        } else if id == SAFE_PIP_CACHE_RULE_ID {
            assert_eq!(
                rule.get("minimum_age_days").and_then(Value::as_u64),
                Some(7),
                "the pip cache rule must retain its reviewed seven-day observation threshold"
            );
            assert_eq!(
                rule.get("path_component").and_then(Value::as_str),
                Some("pip"),
                "the pip cache rule must retain the exact conventional macOS component"
            );
            assert_eq!(
                rule.get("provenance"),
                Some(&Value::Array(vec![Value::String(
                    "https://pip.pypa.io/en/stable/topics/caching/".to_owned()
                )])),
                "the pip cache rule must retain its reviewed source"
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
