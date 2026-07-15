use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;

const VALID_PATH_FIXTURE: &str = "tests/fixtures/rules/v1/schema-valid/path-selector.json";

fn manifest_path(relative: impl AsRef<Path>) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn fixture_bytes(relative: impl AsRef<Path>) -> Vec<u8> {
    fs::read(manifest_path(relative)).unwrap()
}

fn valid_document() -> Value {
    serde_json::from_slice(&fixture_bytes(VALID_PATH_FIXTURE)).unwrap()
}

fn fixture_paths(bucket: &str) -> Vec<PathBuf> {
    let mut pending = vec![manifest_path(format!("tests/fixtures/rules/v1/{bucket}"))];
    let mut fixtures = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                fixtures.push(path);
            }
        }
    }
    fixtures.sort();
    assert!(!fixtures.is_empty(), "fixture bucket {bucket:?} is empty");
    fixtures
}

fn schema() -> Value {
    serde_json::from_slice(&fixture_bytes("schema/rule-catalog-v1.schema.json")).unwrap()
}

fn assert_schema_valid(validator: &jsonschema::Validator, value: &Value, context: &Path) {
    let errors = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{}: {errors:#?}", context.display());
}

#[test]
fn checked_schema_and_fixture_buckets_enforce_their_contracts() {
    let schema = schema();
    assert!(jsonschema::draft202012::meta::is_valid(&schema));
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();

    for path in fixture_paths("schema-valid") {
        let bytes = fs::read(&path).unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_schema_valid(&validator, &value, &path);
        load_rule_registry_json(&bytes)
            .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
    }

    for path in fixture_paths("schema-invalid") {
        let bytes = fs::read(&path).unwrap();
        let schema_rejects = serde_json::from_slice::<Value>(&bytes)
            .map(|value| !validator.is_valid(&value))
            .unwrap_or(true);
        assert!(schema_rejects, "schema accepted {}", path.display());
        assert!(
            load_rule_registry_json(&bytes).is_err(),
            "loader accepted {}",
            path.display()
        );
    }

    for path in fixture_paths("domain-invalid") {
        let bytes = fs::read(&path).unwrap();
        assert!(
            load_rule_registry_json(&bytes).is_err(),
            "loader accepted {}",
            path.display()
        );
    }
}

#[test]
fn malformed_trailing_and_duplicate_json_fields_are_rejected() {
    let valid = fs::read_to_string(manifest_path(VALID_PATH_FIXTURE)).unwrap();
    let invalid_documents = [
        "{".to_owned(),
        format!("{valid}\n{{}}"),
        valid.replacen(
            "\"schema_version\": 1,",
            "\"schema_version\": 1, \"schema_version\": 1,",
            1,
        ),
        valid.replacen(
            "\"id\": \"fixture.developer.artifact\",",
            "\"id\": \"fixture.developer.artifact\", \"id\": \"fixture.duplicate\",",
            1,
        ),
    ];

    for document in invalid_documents {
        assert!(matches!(
            load_rule_registry_json(document.as_bytes()).unwrap_err(),
            RuleRegistryLoadError::MalformedJson { .. }
        ));
    }
    assert!(matches!(
        load_rule_registry_json(&[0xff]).unwrap_err(),
        RuleRegistryLoadError::MalformedJson { .. }
    ));
}

#[test]
fn every_document_field_is_explicit_and_unknown_fields_fail_closed() {
    let schema = schema();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    let base = valid_document();

    for field in ["schema_version", "rules"] {
        let mut changed = base.clone();
        changed.as_object_mut().unwrap().remove(field);
        assert!(
            !validator.is_valid(&changed),
            "schema allowed missing {field}"
        );
        assert!(load_rule_registry_json(&serde_json::to_vec(&changed).unwrap()).is_err());
    }

    let required_rule_fields = base["rules"][0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for field in required_rule_fields {
        let mut changed = base.clone();
        changed["rules"][0].as_object_mut().unwrap().remove(&field);
        assert!(
            !validator.is_valid(&changed),
            "schema allowed missing {field}"
        );
        assert!(
            load_rule_registry_json(&serde_json::to_vec(&changed).unwrap()).is_err(),
            "loader allowed missing {field}"
        );
    }

    for location in ["catalog", "rule", "activity_guard"] {
        let mut changed = base.clone();
        match location {
            "catalog" => {
                changed["unexpected"] = json!(true);
            }
            "rule" => {
                changed["rules"][0]["unexpected"] = json!(true);
            }
            "activity_guard" => {
                changed["rules"][0]["inactive_processes"][0]["unexpected"] = json!(true);
            }
            _ => unreachable!(),
        }
        assert!(!validator.is_valid(&changed));
        assert!(load_rule_registry_json(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
}

#[test]
fn schema_and_loader_cover_the_exhaustive_safety_action_matrix() {
    let schema = schema();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    let safety_tiers = [
        "safe_regenerable",
        "safe_evictable",
        "review_required",
        "informational",
        "protected",
    ];
    let actions = [
        "remove_known_regenerable_contents",
        "evict_local_copy",
        "move_to_trash",
        "reveal_only",
        "no_action",
    ];

    for safety in safety_tiers {
        for action in actions {
            let mut document = valid_document();
            let rule = &mut document["rules"][0];
            rule["safety"] = json!(safety);
            rule["action"] = json!(action);
            rule["schedule_eligible"] = json!(false);
            rule["requires_cloud_upload_complete"] = json!(safety == "safe_evictable");
            let compatible = matches!(
                (safety, action),
                ("safe_regenerable", "remove_known_regenerable_contents")
                    | ("safe_evictable", "evict_local_copy")
                    | ("review_required", "move_to_trash")
                    | ("informational" | "protected", "reveal_only" | "no_action")
            );
            let bytes = serde_json::to_vec(&document).unwrap();

            assert_eq!(
                validator.is_valid(&document),
                compatible,
                "schema mismatch for {safety}/{action}"
            );
            assert_eq!(
                load_rule_registry_json(&bytes).is_ok(),
                compatible,
                "loader mismatch for {safety}/{action}"
            );
        }
    }
}

#[test]
fn scheduling_and_cloud_guards_are_schema_and_domain_invariants() {
    let schema = schema();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();

    let mut invalid_schedule = valid_document();
    invalid_schedule["rules"][0]["safety"] = json!("review_required");
    invalid_schedule["rules"][0]["action"] = json!("move_to_trash");
    invalid_schedule["rules"][0]["schedule_eligible"] = json!(true);
    assert!(!validator.is_valid(&invalid_schedule));
    assert!(load_rule_registry_json(&serde_json::to_vec(&invalid_schedule).unwrap()).is_err());

    let mut invalid_eviction = valid_document();
    invalid_eviction["rules"][0]["safety"] = json!("safe_evictable");
    invalid_eviction["rules"][0]["action"] = json!("evict_local_copy");
    invalid_eviction["rules"][0]["schedule_eligible"] = json!(false);
    invalid_eviction["rules"][0]["requires_cloud_upload_complete"] = json!(false);
    assert!(!validator.is_valid(&invalid_eviction));
    assert!(load_rule_registry_json(&serde_json::to_vec(&invalid_eviction).unwrap()).is_err());
}

#[test]
fn registry_rejects_duplicate_ids_and_orders_distinct_rules_by_id() {
    let duplicate =
        fixture_bytes("tests/fixtures/rules/v1/domain-invalid/duplicate-rule-reference.json");
    assert!(matches!(
        load_rule_registry_json(&duplicate).unwrap_err(),
        RuleRegistryLoadError::DuplicateRuleReference {
            first_index: 0,
            duplicate_index: 1,
            ..
        }
    ));

    let mut conflicting: Value = serde_json::from_slice(&duplicate).unwrap();
    conflicting["rules"][1]["revision"] = json!(2);
    assert!(matches!(
        load_rule_registry_json(&serde_json::to_vec(&conflicting).unwrap()).unwrap_err(),
        RuleRegistryLoadError::ConflictingRuleRevisions {
            first_index: 0,
            duplicate_index: 1,
            ..
        }
    ));

    let mut distinct = valid_document();
    let mut second = distinct["rules"][0].clone();
    distinct["rules"][0]["id"] = json!("fixture.zeta.rule");
    distinct["rules"][0]["title_key"] = json!("rule.fixture.zeta.rule.title");
    distinct["rules"][0]["explanation_key"] = json!("rule.fixture.zeta.rule.explanation");
    second["id"] = json!("fixture.alpha.rule");
    second["title_key"] = json!("rule.fixture.alpha.rule.title");
    second["explanation_key"] = json!("rule.fixture.alpha.rule.explanation");
    distinct["rules"].as_array_mut().unwrap().push(second);

    let registry = load_rule_registry_json(&serde_json::to_vec(&distinct).unwrap()).unwrap();
    assert_eq!(registry.len(), 2);
    assert!(!registry.is_empty());
    assert!(
        registry
            .get(&RuleId::new("fixture.alpha.rule").unwrap())
            .is_some()
    );
    let ids = registry
        .iter()
        .map(|rule| rule.reference().id().as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["fixture.alpha.rule", "fixture.zeta.rule"]);
}

#[test]
fn loader_bounds_documents_rules_lists_strings_and_ages() {
    let oversized = vec![b' '; MAX_CATALOG_BYTES + 1];
    assert!(matches!(
        load_rule_registry_json(&oversized).unwrap_err(),
        RuleRegistryLoadError::DocumentTooLarge { .. }
    ));

    let mut too_many = valid_document();
    let rule = too_many["rules"][0].clone();
    too_many["rules"] = Value::Array(vec![rule; MAX_RULES + 1]);
    assert!(matches!(
        load_rule_registry_json(&serde_json::to_vec(&too_many).unwrap()).unwrap_err(),
        RuleRegistryLoadError::TooManyRules { .. }
    ));

    let mut too_many_markers = valid_document();
    too_many_markers["rules"][0]["required_markers_all"] = Value::Array(
        (0..=MAX_MATCHER_VALUES)
            .map(|index| json!(format!("marker-{index}")))
            .collect(),
    );
    assert!(matches!(
        load_rule_registry_json(&serde_json::to_vec(&too_many_markers).unwrap()).unwrap_err(),
        RuleRegistryLoadError::InvalidRule {
            source: RuleDocumentError::CollectionLimitExceeded {
                field: RuleDocumentField::RequiredMarkersAll,
                ..
            },
            ..
        }
    ));

    let mut long_provenance = valid_document();
    long_provenance["rules"][0]["provenance"] = json!([format!(
        "https://example.com/{}",
        "x".repeat(MAX_PROVENANCE_URL_BYTES)
    )]);
    assert!(matches!(
        load_rule_registry_json(&serde_json::to_vec(&long_provenance).unwrap()).unwrap_err(),
        RuleRegistryLoadError::InvalidRule {
            source: RuleDocumentError::StringLimitExceeded {
                field: RuleDocumentField::Provenance,
                ..
            },
            ..
        }
    ));

    let mut long_activity_guard = valid_document();
    long_activity_guard["rules"][0]["inactive_processes"] = json!([{
        "kind": "bundle_identifier",
        "value": format!("com.example.{}", "x".repeat(255))
    }]);
    assert!(matches!(
        load_rule_registry_json(&serde_json::to_vec(&long_activity_guard).unwrap()).unwrap_err(),
        RuleRegistryLoadError::InvalidRule {
            source: RuleDocumentError::StringLimitExceeded {
                field: RuleDocumentField::InactiveProcesses,
                ..
            },
            ..
        }
    ));

    let mut old = valid_document();
    old["schema_version"] = json!(0);
    assert!(matches!(
        load_rule_registry_json(&serde_json::to_vec(&old).unwrap()).unwrap_err(),
        RuleRegistryLoadError::UnsupportedSchemaVersion { found: 0, .. }
    ));

    let mut excessive_age = valid_document();
    excessive_age["rules"][0]["minimum_age_days"] = json!(MAX_MINIMUM_AGE_DAYS + 1);
    assert!(matches!(
        load_rule_registry_json(&serde_json::to_vec(&excessive_age).unwrap()).unwrap_err(),
        RuleRegistryLoadError::InvalidRule {
            source: RuleDocumentError::MinimumAgeOutOfRange { .. },
            ..
        }
    ));
}

#[test]
fn all_category_and_scope_spellings_map_without_hidden_defaults() {
    let categories = [
        "developer_artifact",
        "application_cache",
        "browser_cache",
        "log_and_diagnostic",
        "installer_and_download",
        "device_and_simulator_data",
        "cloud_file",
        "large_review_item",
        "protected_system_data",
        "unknown_storage",
    ];
    let scopes = [
        "home",
        "user_cache_directory",
        "configured_project_roots",
        "selected_scan_root",
    ];

    for category in categories {
        for scope in scopes {
            let mut document = valid_document();
            document["rules"][0]["category"] = json!(category);
            document["rules"][0]["scope"] = json!(scope);
            load_rule_registry_json(&serde_json::to_vec(&document).unwrap())
                .unwrap_or_else(|error| panic!("{category}/{scope}: {error:#}"));
        }
    }
}
