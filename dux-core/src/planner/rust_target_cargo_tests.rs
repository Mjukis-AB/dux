#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::Arc;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde_json::json;
use tempfile::TempDir;

#[cfg(target_os = "macos")]
use super::cargo_code_signature_macos::inspect_cargo_code_signature;
use super::rust_target::validate_live_rust_target_for_test;
use super::rust_target_cargo::{
    CargoMetadataValidationError, CargoOutputStream, observe_cargo_executable,
    observe_cargo_executable_for_test, revalidate_retained_cargo_directory_after_hook_for_test,
    validate_cargo_metadata, validate_cargo_metadata_for_test,
    validate_cargo_metadata_with_input_fences_for_test,
};
#[cfg(target_os = "macos")]
use super::rust_target_cargo::{
    commit_direct_cargo_enrollment, inspect_direct_cargo_enrollment,
    observe_enrolled_cargo_executable_for_test, signed_cargo_output_limit_for_test,
    validate_cargo_metadata_with_enrollment_hook_for_test,
};
use super::rust_target_tests::{CARGO_CACHE_TAG_SIGNATURE, Fixture};
use crate::domain::BlockReason;

const VALID_VERSION: &str = "cargo 1.96.0 (30a34c682 2026-05-25)\nrelease: 1.96.0\ncommit-hash: 30a34c6821b57de0aaec83a901aca39f88f6778c\ncommit-date: 2026-05-25\nhost: aarch64-apple-darwin\n";

struct FakeCargo {
    _temp: TempDir,
    executable: PathBuf,
}

impl FakeCargo {
    fn new(metadata_action: &str) -> Self {
        Self::with_version(metadata_action, VALID_VERSION)
    }

    fn with_version(metadata_action: &str, version: &str) -> Self {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("cargo");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ] && [ \"$2\" = \"--verbose\" ] && [ \"$#\" -eq 2 ]; then\n  printf %s {}\n  exit 0\nfi\nif [ \"$1\" = \"metadata\" ]; then\n{}\nfi\nexit 64\n",
            shell_quote(version),
            metadata_action,
        );
        fs::write(&executable, script).unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        Self {
            _temp: temp,
            executable,
        }
    }

    fn with_metadata_sequence(first_action: &str, second_action: &str) -> Self {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("cargo");
        let marker = temp.path().join("metadata-discovered");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ] && [ \"$2\" = \"--verbose\" ] && [ \"$#\" -eq 2 ]; then\n  printf %s {}\n  exit 0\nfi\nif [ \"$1\" = \"metadata\" ]; then\n  if [ -e {} ]; then\n{}\n  else\n    : > {}\n{}\n  fi\nfi\nexit 64\n",
            shell_quote(VALID_VERSION),
            shell_quote(marker.to_str().unwrap()),
            second_action,
            shell_quote(marker.to_str().unwrap()),
            first_action,
        );
        fs::write(&executable, script).unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        Self {
            _temp: temp,
            executable,
        }
    }

    fn observe(&self) -> super::rust_target_cargo::CargoExecutableObservation {
        observe_cargo_executable_for_test(&self.executable).unwrap()
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn cargo_config_trace_lines(paths: &[&Path]) -> String {
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let line = format!(
                "   0.{index:09}s DEBUG cargo::util::context: load config from file path={path:?} why_load=FileDiscovery includes=true"
            );
            format!("    printf '%s\\n' {} >&2\n", shell_quote(&line))
        })
        .collect()
}

fn metadata_json(workspace_root: &Path, target_directory: &Path) -> String {
    let package_id = "fixture 0.1.0 (path+file:///fixture)";
    serde_json::to_string(&json!({
        "packages": [{
            "id": package_id,
            "manifest_path": workspace_root.join("Cargo.toml"),
            "source": null,
            "dependencies": []
        }],
        "workspace_members": [package_id],
        "workspace_default_members": [package_id],
        "resolve": null,
        "target_directory": target_directory,
        "version": 1,
        "workspace_root": workspace_root,
        "metadata": {},
        "future_additive_field": {"accepted": true}
    }))
    .unwrap()
}

fn package_metadata_json(
    fixture: &Fixture,
    packages: Vec<serde_json::Value>,
    workspace_members: Vec<&str>,
) -> String {
    serde_json::to_string(&json!({
        "packages": packages,
        "workspace_default_members": workspace_members,
        "workspace_members": workspace_members,
        "resolve": null,
        "target_directory": fixture.target,
        "version": 1,
        "workspace_root": fixture.manifest.parent().unwrap()
    }))
    .unwrap()
}

fn valid_metadata_action(fixture: &Fixture) -> String {
    let document = metadata_json(fixture.manifest.parent().unwrap(), &fixture.target);
    format!(
        "  [ \"$#\" -eq 10 ] || exit 70\n  [ \"$2\" = \"--format-version\" ] || exit 71\n  [ \"$3\" = \"1\" ] || exit 72\n  [ \"$4\" = \"--no-deps\" ] || exit 73\n  [ \"$5\" = \"--locked\" ] || exit 74\n  [ \"$6\" = \"--offline\" ] || exit 75\n  [ \"$7\" = \"--quiet\" ] || exit 76\n  [ \"$8\" = \"--color=never\" ] || exit 77\n  [ \"$9\" = \"--manifest-path\" ] || exit 78\n  [ \"${{10}}\" = {} ] || exit 79\n  [ \"$PWD\" = {} ] || exit 80\n  [ -n \"$HOME\" ] && [ -n \"$CARGO_HOME\" ] && [ -n \"$TMPDIR\" ] || exit 81\n  [ \"$PATH\" = \"/dev/null\" ] || exit 82\n  [ -z \"${{RUSTUP_TOOLCHAIN+x}}\" ] && [ -z \"${{CARGO_TARGET_DIR+x}}\" ] || exit 83\n  [ \"$CARGO_LOG\" = \"cargo::util::context=debug\" ] || exit 84\n  project-helper >/dev/null 2>&1 && exit 85\n  printf %s {}\n  exit 0",
        shell_quote(fixture.manifest.to_str().unwrap()),
        shell_quote(fixture.manifest.parent().unwrap().to_str().unwrap()),
        shell_quote(&document),
    )
}

fn live(fixture: &Fixture) -> super::rust_target::RustTargetLiveWitness {
    validate_live_rust_target_for_test(fixture.source(), &fixture.candidate()).unwrap()
}

#[test]
fn fixed_command_environment_and_exact_metadata_create_only_observational_witness() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project_helper = fixture.manifest.parent().unwrap().join("project-helper");
    fs::write(&project_helper, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&project_helper).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&project_helper, permissions).unwrap();
    let fake = FakeCargo::new(&valid_metadata_action(&fixture));
    let observation = fake.observe();
    let candidate = fixture.candidate();
    let witness = validate_cargo_metadata(
        validate_live_rust_target_for_test(fixture.source(), &candidate).unwrap(),
        &observation,
    )
    .unwrap();

    assert_eq!(witness.cargo_release(), (1, 96, 0));
    assert_ne!(witness.cargo_version_sha256(), [0; 32]);
    assert_eq!(witness.cargo_executable_path(), fake.executable);
    assert_eq!(
        witness.cargo_executable_sha256(),
        observation.executable_sha256()
    );
    assert_eq!(
        witness.cargo_executable_identity(),
        observation.executable_identity()
    );
    assert_eq!(
        witness.cargo_environment_identities(),
        observation.environment_identities()
    );
    assert_ne!(witness.cargo_executable_parent_identity().object(), 0);
    assert_ne!(witness.metadata_sha256(), [0; 32]);
    assert_eq!(witness.configuration_policy_revision(), 3);
    assert!(witness.configuration_lookup_count() >= 2);
    assert_eq!(witness.configuration_root_count(), 0);
    assert_eq!(witness.configuration_file_count(), 0);
    assert_eq!(witness.configuration_include_edge_count(), 0);
    assert_eq!(witness.configuration_byte_count(), 0);
    assert_ne!(witness.configuration_closure_sha256(), [0; 32]);
    assert_ne!(witness.configuration_read_intent_sha256(), [0; 32]);
    assert_eq!(witness.workspace_manifest_policy_revision(), 1);
    assert_eq!(witness.workspace_member_count(), 1);
    assert_eq!(witness.workspace_manifest_count(), 1);
    assert_ne!(witness.workspace_manifest_closure_sha256(), [0; 32]);
    assert_eq!(witness.path_dependency_policy_revision(), 1);
    assert_eq!(witness.dependency_declaration_count(), 0);
    assert_eq!(witness.local_path_dependency_count(), 0);
    assert_eq!(witness.unique_local_dependency_manifest_count(), 0);
    assert_ne!(witness.path_dependency_closure_sha256(), [0; 32]);
    assert_eq!(witness.manifest_probe_policy_revision(), 1);
    assert!(witness.manifest_probe_count() >= 1);
    assert_eq!(witness.present_ancestor_manifest_count(), 0);
    assert_eq!(witness.ancestor_manifest_byte_count(), 0);
    assert_ne!(witness.manifest_probe_closure_sha256(), [0; 32]);
    assert_eq!(witness.launch_policy_revision(), 0);
    assert_eq!(witness.running_code_directory_hash_sha256(), [0; 32]);
    assert_eq!(witness.resolution_policy_revision(), 7);
    assert!(witness.live().protected_path_is_still_unresolved());
    assert_eq!(candidate.blockers(), [BlockReason::ProtectedPath]);
    assert!(!candidate.rule_marks_schedule_eligible());
    witness.release().unwrap();
}

#[test]
fn workspace_target_resolve_and_document_shape_fail_closed() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project_root = fixture.manifest.parent().unwrap();
    let cases = [
        (
            metadata_json(&fixture.root, &fixture.target),
            "workspace mismatch",
        ),
        (
            metadata_json(project_root, &project_root.join("other-target")),
            "target mismatch",
        ),
        (
            json!({
                "version": 1,
                "workspace_root": project_root,
                "target_directory": fixture.target,
                "resolve": {"nodes": []}
            })
            .to_string(),
            "non-null resolve",
        ),
        (
            format!("{} trailing", metadata_json(project_root, &fixture.target)),
            "trailing bytes",
        ),
        ("{\"version\":1".to_owned(), "malformed document"),
    ];

    for (document, label) in cases {
        let action = format!("  printf %s {}\n  exit 0", shell_quote(&document));
        let fake = FakeCargo::new(&action);
        let error = match validate_cargo_metadata(live(&fixture), &fake.observe()) {
            Ok(_) => panic!("{label} unexpectedly validated"),
            Err(error) => error,
        };
        match label {
            "workspace mismatch" => {
                assert!(matches!(
                    error,
                    CargoMetadataValidationError::WorkspaceMismatch
                ))
            }
            "target mismatch" => assert!(
                matches!(error, CargoMetadataValidationError::TargetDirectoryMismatch),
                "unexpected target mismatch error: {error:?}"
            ),
            _ => assert!(matches!(
                error,
                CargoMetadataValidationError::InvalidMetadata
            )),
        }
    }
}

#[test]
fn malformed_or_ambiguous_workspace_members_fail_closed() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let root = fixture.manifest.parent().unwrap();
    let package = json!({
        "id": "member-a",
        "manifest_path": fixture.manifest,
        "source": null,
        "dependencies": []
    });
    let cases = [
        json!({
            "packages": [package.clone(), package.clone()],
            "workspace_members": ["member-a", "member-a"],
            "workspace_default_members": ["member-a"],
            "resolve": null,
            "target_directory": fixture.target,
            "version": 1,
            "workspace_root": root
        }),
        json!({
            "packages": [package.clone()],
            "workspace_members": ["missing"],
            "workspace_default_members": [],
            "resolve": null,
            "target_directory": fixture.target,
            "version": 1,
            "workspace_root": root
        }),
        json!({
            "packages": [
                package.clone(),
                {
                    "id": "member-b",
                    "manifest_path": fixture.manifest,
                    "source": null,
                    "dependencies": []
                }
            ],
            "workspace_members": ["member-a", "member-b"],
            "workspace_default_members": ["member-a", "member-b"],
            "resolve": null,
            "target_directory": fixture.target,
            "version": 1,
            "workspace_root": root
        }),
        json!({
            "packages": [package.clone()],
            "workspace_members": ["member-a"],
            "workspace_default_members": ["unknown"],
            "resolve": null,
            "target_directory": fixture.target,
            "version": 1,
            "workspace_root": root
        }),
        json!({
            "packages": [{
                "id": "member-a",
                "manifest_path": fixture.manifest,
                "source": "registry+https://example.invalid/index",
                "dependencies": []
            }],
            "workspace_members": ["member-a"],
            "workspace_default_members": ["member-a"],
            "resolve": null,
            "target_directory": fixture.target,
            "version": 1,
            "workspace_root": root
        }),
        json!({
            "packages": [],
            "workspace_members": [],
            "workspace_default_members": [],
            "resolve": null,
            "target_directory": fixture.target,
            "version": 1,
            "workspace_root": root
        }),
    ];

    for document in cases {
        let action = format!(
            "  printf %s {}\n  exit 0",
            shell_quote(&document.to_string())
        );
        let fake = FakeCargo::new(&action);
        assert!(matches!(
            validate_cargo_metadata(live(&fixture), &fake.observe()),
            Err(CargoMetadataValidationError::InvalidWorkspaceMembers)
        ));
    }
}

#[test]
fn reported_path_dependency_graph_is_bounded_and_observational() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    let member = project.join("member");
    fs::create_dir(&member).unwrap();
    fs::write(
        member.join("Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let document = package_metadata_json(
        &fixture,
        vec![
            json!({
                "id": "root",
                "manifest_path": fixture.manifest,
                "source": null,
                "dependencies": [
                    {"source": null, "path": member},
                    {"source": null, "path": member},
                    {"source": "registry+https://example.invalid/index", "path": null}
                ]
            }),
            json!({
                "id": "member",
                "manifest_path": member.join("Cargo.toml"),
                "source": null,
                "dependencies": []
            }),
        ],
        vec!["root", "member"],
    );
    let action = format!("  printf %s {}\n  exit 0", shell_quote(&document));
    let fake = FakeCargo::new(&action);
    let witness = validate_cargo_metadata(live(&fixture), &fake.observe()).unwrap();

    assert_eq!(witness.path_dependency_policy_revision(), 1);
    assert_eq!(witness.dependency_declaration_count(), 3);
    assert_eq!(witness.local_path_dependency_count(), 2);
    assert_eq!(witness.unique_local_dependency_manifest_count(), 1);
    assert_ne!(witness.path_dependency_closure_sha256(), [0; 32]);
    assert_eq!(witness.resolution_policy_revision(), 7);
    witness.release().unwrap();
}

#[test]
fn malformed_or_unreported_path_dependencies_fail_closed() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    let root_package = |dependencies: serde_json::Value| {
        json!({
            "id": "root",
            "manifest_path": fixture.manifest,
            "source": null,
            "dependencies": dependencies
        })
    };
    let invalid_cases = [
        root_package(json!([{"path": null}])),
        root_package(json!([{"source": null, "path": null}])),
        root_package(json!([{"source": {}, "path": null}])),
        root_package(json!([{"source": 7, "path": null}])),
        root_package(json!([{"source": "", "path": null}])),
        root_package(json!([{"source": "git\u{85}source", "path": null}])),
        root_package(json!([{
            "source": "registry+https://example.invalid/index",
            "path": project
        }])),
        root_package(json!([{"source": null, "path": "relative"}])),
        root_package(json!([{"source": null, "path": "/bad\u{85}path"}])),
        root_package(json!([{
            "source": null,
            "path": format!("{}/", project.display())
        }])),
    ];
    for package in invalid_cases {
        let document = package_metadata_json(&fixture, vec![package], vec!["root"]);
        let action = format!("  printf %s {}\n  exit 0", shell_quote(&document));
        let fake = FakeCargo::new(&action);
        assert!(matches!(
            validate_cargo_metadata(live(&fixture), &fake.observe()),
            Err(CargoMetadataValidationError::InvalidPathDependencies)
                | Err(CargoMetadataValidationError::InvalidMetadata)
        ));
    }

    let missing_dependencies = package_metadata_json(
        &fixture,
        vec![json!({
            "id": "root",
            "manifest_path": fixture.manifest,
            "source": null
        })],
        vec!["root"],
    );
    let fake = FakeCargo::new(&format!(
        "  printf %s {}\n  exit 0",
        shell_quote(&missing_dependencies)
    ));
    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &fake.observe()),
        Err(CargoMetadataValidationError::InvalidMetadata)
    ));

    let unreported = project.join("unreported");
    let document = package_metadata_json(
        &fixture,
        vec![root_package(json!([{"source": null, "path": unreported}]))],
        vec!["root"],
    );
    let fake = FakeCargo::new(&format!("  printf %s {}\n  exit 0", shell_quote(&document)));
    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &fake.observe()),
        Err(CargoMetadataValidationError::CargoPathDependenciesUnsupported)
    ));
}

#[test]
fn path_dependency_count_and_byte_limits_fail_closed() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let registry_dependencies: Vec<_> = (0..4_096)
        .map(|_| json!({"source": "registry+https://example.invalid/index", "path": null}))
        .collect();
    let package = json!({
        "id": "root",
        "manifest_path": fixture.manifest,
        "source": null,
        "dependencies": registry_dependencies
    });
    let document = package_metadata_json(&fixture, vec![package], vec!["root"]);
    let fake = FakeCargo::new(&format!("  printf %s {}\n  exit 0", shell_quote(&document)));
    let witness = validate_cargo_metadata(live(&fixture), &fake.observe()).unwrap();
    assert_eq!(witness.dependency_declaration_count(), 4_096);
    assert_eq!(witness.local_path_dependency_count(), 0);
    witness.release().unwrap();

    let registry_dependencies: Vec<_> = (0..4_097)
        .map(|_| json!({"source": "registry+https://example.invalid/index", "path": null}))
        .collect();
    let package = json!({
        "id": "root",
        "manifest_path": fixture.manifest,
        "source": null,
        "dependencies": registry_dependencies
    });
    let document = package_metadata_json(&fixture, vec![package], vec!["root"]);
    let fake = FakeCargo::new(&format!("  printf %s {}\n  exit 0", shell_quote(&document)));
    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &fake.observe()),
        Err(CargoMetadataValidationError::InvalidPathDependencies)
    ));

    let oversized_path = format!("/{}", "a".repeat(256 * 1024));
    let package = json!({
        "id": "root",
        "manifest_path": fixture.manifest,
        "source": null,
        "dependencies": [{"source": null, "path": oversized_path}]
    });
    let document = package_metadata_json(&fixture, vec![package], vec!["root"]);
    let fake = FakeCargo::new(&format!("  printf %s {}\n  exit 0", shell_quote(&document)));
    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &fake.observe()),
        Err(CargoMetadataValidationError::InvalidPathDependencies)
    ));
}

#[test]
fn metadata_member_declaration_must_be_identical_across_both_passes() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let first = metadata_json(fixture.manifest.parent().unwrap(), &fixture.target);
    let mut second: serde_json::Value = serde_json::from_str(&first).unwrap();
    second["future_additive_field"] = json!({"accepted": false});
    let first_action = format!("    printf %s {}\n    exit 0", shell_quote(&first));
    let second_text = second.to_string();
    let second_action = format!("    printf %s {}\n    exit 0", shell_quote(&second_text));
    let fake = FakeCargo::with_metadata_sequence(&first_action, &second_action);

    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &fake.observe()),
        Err(CargoMetadataValidationError::WorkspaceManifestChanged)
    ));
}

#[test]
fn timeout_and_both_output_limits_kill_the_process_group_and_reap_the_child() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let timeout = FakeCargo::new("  /bin/sleep 30 &\n  wait");
    let started = Instant::now();
    let timeout_result = validate_cargo_metadata_for_test(
        live(&fixture),
        &timeout.observe(),
        Duration::from_millis(100),
        1024,
        1024,
    );
    assert!(
        matches!(timeout_result, Err(CargoMetadataValidationError::Timeout)),
        "unexpected timeout result: {:?}",
        timeout_result.err()
    );
    assert!(started.elapsed() < Duration::from_secs(2));

    for (action, expected_stream) in [
        (
            "  while :; do printf 0123456789abcdef; done",
            CargoOutputStream::Stdout,
        ),
        (
            "  while :; do printf 0123456789abcdef >&2; done",
            CargoOutputStream::Stderr,
        ),
    ] {
        let fake = FakeCargo::new(action);
        assert!(matches!(
            validate_cargo_metadata_for_test(
                live(&fixture),
                &fake.observe(),
                Duration::from_secs(1),
                128,
                128,
            ),
            Err(CargoMetadataValidationError::OutputLimit { stream }) if stream == expected_stream
        ));
    }
}

#[test]
fn successful_command_also_kills_residual_process_group_members() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let pid_root = TempDir::new().unwrap();
    let pid_file = pid_root.path().join("descendant.pid");
    let document = metadata_json(fixture.manifest.parent().unwrap(), &fixture.target);
    let action = format!(
        "  /bin/sleep 30 </dev/null >/dev/null 2>&1 &\n  printf %s \"$!\" > {}\n  printf %s {}\n  exit 0",
        shell_quote(pid_file.to_str().unwrap()),
        shell_quote(&document),
    );
    let fake = FakeCargo::new(&action);
    let _witness = validate_cargo_metadata(live(&fixture), &fake.observe()).unwrap();
    let pid: i32 = fs::read_to_string(pid_file).unwrap().parse().unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match kill(Pid::from_raw(pid), None) {
            Err(Errno::ESRCH) => break,
            Ok(()) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            result => panic!("residual Cargo descendant remained alive: {result:?}"),
        }
    }
}

#[test]
fn nonzero_exit_invalid_version_symlink_and_executable_rewrite_are_rejected() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let failed = FakeCargo::new("  printf failure >&2\n  exit 9");
    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &failed.observe()),
        Err(CargoMetadataValidationError::ProcessFailed)
    ));

    for version in [
        "cargo 1.96.1 (30a34c682 2026-05-25)\nrelease: 1.96.1\ncommit-hash: 30a34c6821b57de0aaec83a901aca39f88f6778c\nhost: aarch64-apple-darwin\n",
        "cargo 1.97.0 (30a34c682 2026-05-25)\nrelease: 1.97.0\ncommit-hash: 30a34c6821b57de0aaec83a901aca39f88f6778c\nhost: aarch64-apple-darwin\n",
        "cargo 1.96.0-nightly (30a34c682 2026-05-25)\nrelease: 1.96.0-nightly\ncommit-hash: 30a34c6821b57de0aaec83a901aca39f88f6778c\nhost: aarch64-apple-darwin\n",
        "cargo 1.96.0 (000000000 2026-05-25)\nrelease: 1.96.0\ncommit-hash: 0000000000000000000000000000000000000000\nhost: aarch64-apple-darwin\n",
        "cargo 2.0.0\nrelease: 2.0.0\n",
    ] {
        let invalid_version = FakeCargo::with_version("  exit 9", version);
        assert!(matches!(
            observe_cargo_executable_for_test(&invalid_version.executable),
            Err(CargoMetadataValidationError::InvalidCargoVersion)
        ));
    }

    let real = FakeCargo::new(&valid_metadata_action(&fixture));
    let link_root = TempDir::new().unwrap();
    let link = link_root.path().join("cargo");
    symlink(&real.executable, &link).unwrap();
    assert!(matches!(
        observe_cargo_executable_for_test(&link),
        Err(CargoMetadataValidationError::InvalidExecutableLocator)
    ));

    let observation = real.observe();
    fs::write(&real.executable, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &observation),
        Err(CargoMetadataValidationError::ExecutableChanged)
    ));
}

#[test]
fn same_inode_manifest_rewrite_during_metadata_is_detected_by_workspace_guard() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let document = metadata_json(fixture.manifest.parent().unwrap(), &fixture.target);
    let action = format!(
        "  printf '\\n# changed during metadata\\n' >> {}\n  printf %s {}\n  exit 0",
        shell_quote(fixture.manifest.to_str().unwrap()),
        shell_quote(&document),
    );
    let fake = FakeCargo::new(&action);

    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &fake.observe()),
        Err(CargoMetadataValidationError::WorkspaceManifestChanged)
    ));
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test renames only a TempDir-owned cwd to prove retained-directory identity rejection"
)]
fn retained_working_directory_rejects_path_replacement() {
    let temp = TempDir::new().unwrap();
    let parent = fs::canonicalize(temp.path()).unwrap();
    let current = parent.join("current");
    let displaced = parent.join("displaced");
    fs::create_dir(&current).unwrap();

    let result = revalidate_retained_cargo_directory_after_hook_for_test(&current, || {
        // DUX-DESTRUCTIVE: allow=test-cargo-cwd-replace -- rename only the TempDir-owned retained cwd to prove pathname replacement is rejected
        fs::rename(&current, &displaced).unwrap();
        fs::create_dir(&current).unwrap();
    });
    assert!(matches!(
        result,
        Err(CargoMetadataValidationError::CargoWorkingDirectoryChanged)
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn create_then_remove_config_during_metadata_is_terminal_and_kills_cargo() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let fake = FakeCargo::new(
        "  /bin/mkdir .cargo\n  : > .cargo/config.toml\n  /bin/rm .cargo/config.toml\n  /bin/rmdir .cargo\n  /bin/sleep 30",
    );
    let started = Instant::now();
    let result =
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &fake.observe());
    assert!(matches!(
        result,
        Err(CargoMetadataValidationError::CargoConfigurationChanged)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(target_os = "macos")]
#[test]
fn member_manifest_write_and_restore_during_accepted_pass_is_terminal() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    let member_directory = project.join("member");
    let member_manifest = member_directory.join("Cargo.toml");
    fs::create_dir(&member_directory).unwrap();
    let member_contents = "[package]\nname = \"member\"\nversion = \"0.1.0\"\n";
    fs::write(&member_manifest, member_contents).unwrap();
    let document = json!({
        "packages": [
            {
                "id": "root",
                "manifest_path": fixture.manifest,
                "source": null,
                "dependencies": []
            },
            {
                "id": "member",
                "manifest_path": member_manifest,
                "source": null,
                "dependencies": []
            }
        ],
        "workspace_members": ["root", "member"],
        "workspace_default_members": ["root", "member"],
        "resolve": null,
        "target_directory": fixture.target,
        "version": 1,
        "workspace_root": project,
        "metadata": {}
    })
    .to_string();
    let first_action = format!("    printf %s {}\n    exit 0", shell_quote(&document));
    let second_action = format!(
        "    printf %s {} > {}\n    printf %s {}\n    exit 0",
        shell_quote(member_contents),
        shell_quote(member_manifest.to_str().unwrap()),
        shell_quote(&document),
    );
    let fake = FakeCargo::with_metadata_sequence(&first_action, &second_action);

    let result =
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &fake.observe());
    match result {
        Err(CargoMetadataValidationError::WorkspaceManifestChanged) => {}
        Err(error) => panic!("unexpected mutation error: {error:?}"),
        Ok(_) => panic!("member manifest write-and-restore unexpectedly validated"),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn ancestor_manifest_write_and_restore_during_discovery_is_terminal() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let ancestor_manifest = fixture.root.join("Cargo.toml");
    let original = "[workspace]\nexclude = [\"project\"]\n";
    fs::write(&ancestor_manifest, original).unwrap();
    let action = format!(
        "  printf %s {} > {}\n  printf %s {} > {}\n  /bin/sleep 30",
        shell_quote("[workspace]\n"),
        shell_quote(ancestor_manifest.to_str().unwrap()),
        shell_quote(original),
        shell_quote(ancestor_manifest.to_str().unwrap()),
    );
    let fake = FakeCargo::new(&action);
    let started = Instant::now();
    let result =
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &fake.observe());
    assert!(matches!(
        result,
        Err(CargoMetadataValidationError::CargoManifestProbesChanged)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fs::read_to_string(ancestor_manifest).unwrap(), original);
}

#[cfg(target_os = "macos")]
#[test]
fn included_config_write_and_restore_during_accepted_pass_is_terminal() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    let config_directory = project.join(".cargo");
    let root_config = config_directory.join("config.toml");
    let included_config = config_directory.join("included.toml");
    fs::create_dir(&config_directory).unwrap();
    fs::write(&root_config, "include = [\"included.toml\"]\n").unwrap();
    let original = "[term]\ncolor = \"never\"\n";
    fs::write(&included_config, original).unwrap();

    let document = metadata_json(project, &fixture.target);
    let trace = cargo_config_trace_lines(&[&root_config, &included_config]);
    let first_action = format!(
        "{trace}    printf %s {}\n    exit 0",
        shell_quote(&document)
    );
    let second_action = format!(
        "    printf %s {} > {}\n    printf %s {} > {}\n{trace}    printf %s {}\n    exit 0",
        shell_quote("[term]\ncolor = \"always\"\n"),
        shell_quote(included_config.to_str().unwrap()),
        shell_quote(original),
        shell_quote(included_config.to_str().unwrap()),
        shell_quote(&document),
    );
    let fake = FakeCargo::with_metadata_sequence(&first_action, &second_action);

    let result =
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &fake.observe());
    assert!(matches!(
        result,
        Err(CargoMetadataValidationError::CargoConfigurationChanged)
    ));
}

#[test]
fn ambiguous_dual_config_names_reject_before_metadata_subprocess_execution() {
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let sentinel = fixture.manifest.parent().unwrap().join("spawned");
    let fake = FakeCargo::new(&format!(
        "  : > {}\n  exit 0",
        shell_quote(sentinel.to_str().unwrap())
    ));
    let config = fixture.manifest.parent().unwrap().join(".cargo");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("config"), "[build]\n").unwrap();
    fs::write(config.join("config.toml"), "").unwrap();

    assert!(matches!(
        validate_cargo_metadata(live(&fixture), &fake.observe()),
        Err(CargoMetadataValidationError::CargoConfigurationUnsupported)
    ));
    assert!(!sentinel.exists());
}

#[test]
fn real_cargo_attests_project_config_intent_and_no_deps_does_not_create_lockfile() {
    let Some(cargo) = direct_test_cargo() else {
        return;
    };
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let build_sentinel = prepare_real_package(&fixture, true);
    let observation = observe_cargo_executable(&cargo).unwrap();
    #[cfg(target_os = "macos")]
    {
        let overflow =
            signed_cargo_output_limit_for_test(&observation, fixture.manifest.parent().unwrap());
        assert!(matches!(
            overflow,
            Err(CargoMetadataValidationError::OutputLimit {
                stream: CargoOutputStream::Stdout
            })
        ));
    }
    let witness =
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &observation).unwrap();
    #[cfg(target_os = "macos")]
    {
        assert_eq!(witness.launch_policy_revision(), 1);
        assert_ne!(witness.running_code_directory_hash_sha256(), [0; 32]);
    }
    witness.release().unwrap();
    assert!(!build_sentinel.exists());

    let configured = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let configured_sentinel = prepare_real_package(&configured, true);
    let project_helper = configured.manifest.parent().unwrap().join("project-helper");
    fs::write(
        &project_helper,
        format!(
            "#!/bin/sh\nprintf executed > {}\n",
            shell_quote(configured_sentinel.to_str().unwrap())
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&project_helper).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&project_helper, permissions).unwrap();
    let cargo_config = configured.manifest.parent().unwrap().join(".cargo");
    fs::create_dir(&cargo_config).unwrap();
    fs::write(
        cargo_config.join("config.toml"),
        format!(
            "[build]\ntarget-dir = \"relocated-target\"\nrustc-wrapper = {}\n\n[target.'cfg(all())']\nrunner = {}\n",
            toml_string(&project_helper),
            toml_string(&project_helper),
        ),
    )
    .unwrap();
    assert!(matches!(
        validate_cargo_metadata(live(&configured), &observation),
        Err(CargoMetadataValidationError::TargetDirectoryMismatch)
    ));
    assert!(!configured_sentinel.exists());

    let included = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let included_sentinel = prepare_real_package(&included, true);
    let included_config = included.manifest.parent().unwrap().join(".cargo");
    fs::create_dir(&included_config).unwrap();
    fs::write(
        included_config.join("config.toml"),
        "include = [\"shared.toml\"]\n\n[build]\ntarget-dir = \"target\"\n",
    )
    .unwrap();
    fs::write(
        included_config.join("shared.toml"),
        "[term]\ncolor = \"never\"\n",
    )
    .unwrap();
    let included_witness =
        validate_cargo_metadata_with_input_fences_for_test(live(&included), &observation).unwrap();
    assert_eq!(included_witness.configuration_policy_revision(), 3);
    assert_eq!(included_witness.configuration_root_count(), 1);
    assert_eq!(included_witness.configuration_file_count(), 2);
    assert_eq!(included_witness.configuration_include_edge_count(), 1);
    assert!(included_witness.configuration_byte_count() > 0);
    assert_ne!(included_witness.configuration_closure_sha256(), [0; 32]);
    assert_ne!(included_witness.configuration_read_intent_sha256(), [0; 32]);
    included_witness.release().unwrap();
    assert!(!included_sentinel.exists());

    let unlocked = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let unlocked_sentinel = prepare_real_package(&unlocked, false);
    let lockfile = unlocked.manifest.parent().unwrap().join("Cargo.lock");
    let _witness = validate_cargo_metadata(live(&unlocked), &observation).unwrap();
    assert!(!lockfile.exists());
    assert!(!unlocked_sentinel.exists());

    // The exact enrolled Cargo 1.96 metadata --no-deps path does not load the
    // lockfile. Keep that version-specific assumption executable: a malformed
    // lockfile is deliberately outside this slice's claimed input closure.
    let malformed_lock = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let malformed_sentinel = prepare_real_package(&malformed_lock, false);
    fs::write(
        malformed_lock.manifest.parent().unwrap().join("Cargo.lock"),
        "this is not a Cargo lockfile\n",
    )
    .unwrap();
    let malformed_witness =
        validate_cargo_metadata_with_input_fences_for_test(live(&malformed_lock), &observation)
            .unwrap();
    malformed_witness.release().unwrap();
    assert!(!malformed_sentinel.exists());
}

#[test]
fn real_cargo_attests_virtual_root_and_every_workspace_member_manifest() {
    let Some(cargo) = direct_test_cargo() else {
        return;
    };
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    fs::write(
        &fixture.manifest,
        "[workspace]\nmembers = [\"alpha\", \"beta\"]\nresolver = \"3\"\n",
    )
    .unwrap();
    for member in ["alpha", "beta"] {
        let directory = project.join(member);
        fs::create_dir_all(directory.join("src")).unwrap();
        fs::write(
            directory.join("Cargo.toml"),
            format!("[package]\nname = \"{member}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        )
        .unwrap();
        fs::write(directory.join("src/lib.rs"), "pub fn member() {}\n").unwrap();
    }
    fs::write(
        project.join("Cargo.lock"),
        "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n[[package]]\nname = \"alpha\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"beta\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let observation = observe_cargo_executable(&cargo).unwrap();
    let witness =
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &observation).unwrap();

    assert_eq!(witness.workspace_member_count(), 2);
    assert_eq!(witness.workspace_manifest_count(), 3);
    assert_ne!(witness.workspace_manifest_closure_sha256(), [0; 32]);
    assert_eq!(witness.resolution_policy_revision(), 7);
    witness.release().unwrap();
}

#[test]
fn real_cargo_accepts_only_reported_internal_path_dependencies() {
    let Some(cargo) = direct_test_cargo() else {
        return;
    };
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    fs::write(
        &fixture.manifest,
        "[package]\nname = \"root\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ninternal = { path = \"internal\" }\n\n[workspace]\nresolver = \"3\"\n",
    )
    .unwrap();
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/lib.rs"), "pub fn root() {}\n").unwrap();
    let internal = project.join("internal");
    fs::create_dir_all(internal.join("src")).unwrap();
    fs::write(
        internal.join("Cargo.toml"),
        "[package]\nname = \"internal\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(internal.join("src/lib.rs"), "pub fn internal() {}\n").unwrap();

    let observation = observe_cargo_executable(&cargo).unwrap();
    let witness =
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &observation).unwrap();
    assert_eq!(witness.workspace_member_count(), 2);
    assert_eq!(witness.workspace_manifest_count(), 2);
    assert_eq!(witness.dependency_declaration_count(), 1);
    assert_eq!(witness.local_path_dependency_count(), 1);
    assert_eq!(witness.unique_local_dependency_manifest_count(), 1);
    assert_ne!(witness.path_dependency_closure_sha256(), [0; 32]);
    assert_eq!(witness.resolution_policy_revision(), 7);
    witness.release().unwrap();
}

#[test]
fn real_cargo_rejects_unreported_external_path_dependencies() {
    let Some(cargo) = direct_test_cargo() else {
        return;
    };
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    fs::write(
        &fixture.manifest,
        "[package]\nname = \"root\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[target.'cfg(any())'.build-dependencies]\nexternal = { path = \"../external\", optional = true }\n\n[workspace]\nresolver = \"3\"\n",
    )
    .unwrap();
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/lib.rs"), "pub fn root() {}\n").unwrap();
    let external = fixture.root.join("external");
    fs::create_dir_all(external.join("src")).unwrap();
    fs::write(
        external.join("Cargo.toml"),
        "[package]\nname = \"external\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(external.join("src/lib.rs"), "pub fn external() {}\n").unwrap();
    fs::write(
        fixture.root.join("Cargo.toml"),
        "[workspace]\nexclude = [\"project\", \"external\"]\nresolver = \"3\"\n",
    )
    .unwrap();

    let observation = observe_cargo_executable(&cargo).unwrap();
    assert!(matches!(
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &observation),
        Err(CargoMetadataValidationError::CargoPathDependenciesUnsupported)
    ));
}

#[test]
fn real_cargo_conservatively_rejects_excluded_path_dependencies() {
    let Some(cargo) = direct_test_cargo() else {
        return;
    };
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    fs::write(
        &fixture.manifest,
        "[package]\nname = \"root\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nexcluded = { path = \"excluded\" }\n\n[workspace]\nexclude = [\"excluded\"]\nresolver = \"3\"\n",
    )
    .unwrap();
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/lib.rs"), "pub fn root() {}\n").unwrap();
    let excluded = project.join("excluded");
    fs::create_dir_all(excluded.join("src")).unwrap();
    fs::write(
        excluded.join("Cargo.toml"),
        "[package]\nname = \"excluded\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(excluded.join("src/lib.rs"), "pub fn excluded() {}\n").unwrap();

    let observation = observe_cargo_executable(&cargo).unwrap();
    assert!(matches!(
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &observation),
        Err(CargoMetadataValidationError::CargoPathDependenciesUnsupported)
    ));
}

#[test]
fn real_cargo_conservatively_rejects_unread_standalone_path_dependencies() {
    let Some(cargo) = direct_test_cargo() else {
        return;
    };
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let project = fixture.manifest.parent().unwrap();
    fs::write(
        &fixture.manifest,
        "[package]\nname = \"root\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nstandalone = { path = \"../standalone\" }\n",
    )
    .unwrap();
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(project.join("src/lib.rs"), "pub fn root() {}\n").unwrap();
    let standalone = fixture.root.join("standalone");
    fs::create_dir(&standalone).unwrap();
    // Exact Cargo 1.96 does not parse this dependency manifest in the
    // standalone `metadata --no-deps` path. DUX still rejects its serialized
    // unreported path conservatively.
    fs::write(standalone.join("Cargo.toml"), "[\n").unwrap();

    let observation = observe_cargo_executable(&cargo).unwrap();
    assert!(matches!(
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &observation),
        Err(CargoMetadataValidationError::CargoPathDependenciesUnsupported)
    ));
}

#[test]
fn real_cargo_attests_excluding_ancestor_manifest_probe() {
    let Some(cargo) = direct_test_cargo() else {
        return;
    };
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    prepare_real_package(&fixture, true);
    let ancestor = fixture.root.join("Cargo.toml");
    fs::write(&ancestor, "[workspace]\nexclude = [\"project\"]\n").unwrap();
    let observation = observe_cargo_executable(&cargo).unwrap();
    let witness =
        validate_cargo_metadata_with_input_fences_for_test(live(&fixture), &observation).unwrap();
    assert_eq!(witness.manifest_probe_policy_revision(), 1);
    assert!(witness.manifest_probe_count() >= 1);
    assert_eq!(witness.present_ancestor_manifest_count(), 1);
    assert_eq!(
        witness.ancestor_manifest_byte_count(),
        fs::metadata(ancestor).unwrap().len()
    );
    assert_ne!(witness.manifest_probe_closure_sha256(), [0; 32]);
    assert_eq!(witness.resolution_policy_revision(), 7);
    witness.release().unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn exact_same_store_enrollment_is_retained_and_a_during_metadata_revoke_rejects() {
    let cargo = direct_test_cargo().expect("macOS tests require the direct stable Cargo");
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    prepare_real_package(&fixture, true);
    let store_temp = TempDir::new().unwrap();
    let store =
        crate::persistence::StoreCoordinator::open(&store_temp.path().join("store/dux.sqlite3"))
            .unwrap();
    let enrolled = commit_direct_cargo_enrollment(
        inspect_direct_cargo_enrollment(Arc::clone(&store), &cargo).unwrap(),
    )
    .unwrap();
    let observation = observe_cargo_executable(&cargo).unwrap();
    let witness = validate_cargo_metadata_with_enrollment_hook_for_test(
        live(&fixture),
        &observation,
        Arc::clone(&store),
        enrolled.setting.clone(),
        || {},
    )
    .unwrap();
    assert_eq!(witness.enrollment_revision(), 1);
    witness.release().unwrap();

    let changed_fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    prepare_real_package(&changed_fixture, true);
    assert!(matches!(
        validate_cargo_metadata_with_enrollment_hook_for_test(
            live(&changed_fixture),
            &observation,
            Arc::clone(&store),
            enrolled.setting,
            || {
                store.revoke_cargo_executable().unwrap();
            },
        ),
        Err(CargoMetadataValidationError::CargoEnrollmentChanged)
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn automatic_enrolled_observation_rejects_unmatched_signed_bytes_before_execution() {
    let executable_temp = TempDir::new().unwrap();
    let executable = executable_temp.path().join("cargo");
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o600)).unwrap();
    let executable = fs::canonicalize(executable).unwrap();
    let signature = inspect_cargo_code_signature(&executable).unwrap();

    let store_temp = TempDir::new().unwrap();
    let store =
        crate::persistence::StoreCoordinator::open(&store_temp.path().join("store/dux.sqlite3"))
            .unwrap();
    store
        .enroll_cargo_executable(
            crate::persistence::CargoExecutableEnrollmentIdentity::new(
                executable,
                [0; 32],
                [0; 32],
                crate::persistence::CARGO_ENROLLMENT_SUPPORTED_RELEASE,
                signature,
            )
            .unwrap(),
        )
        .unwrap();
    let enrollment = store.load_cargo_enrollment().unwrap();
    assert!(matches!(
        observe_enrolled_cargo_executable_for_test(&store, &enrollment),
        Err(CargoMetadataValidationError::CargoEnrollmentChanged)
    ));
}

fn direct_test_cargo() -> Option<PathBuf> {
    let configured = std::env::var_os("CARGO").and_then(|value| fs::canonicalize(value).ok());
    #[cfg(target_os = "macos")]
    let configured = configured.or_else(|| {
        let rustup_home = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")))?;
        let toolchain = std::env::var_os("RUSTUP_TOOLCHAIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(format!("stable-{}-apple-darwin", std::env::consts::ARCH))
            });
        fs::canonicalize(
            rustup_home
                .join("toolchains")
                .join(toolchain)
                .join("bin/cargo"),
        )
        .ok()
    });
    let canonical = configured?;
    (canonical.file_name()?.to_str()? == "cargo").then_some(canonical)
}

fn prepare_real_package(fixture: &Fixture, lockfile: bool) -> PathBuf {
    let project = fixture.manifest.parent().unwrap();
    let build_sentinel = project.join("build-script-executed");
    fs::write(
        &fixture.manifest,
        "[package]\nname = \"dux-cargo-observer-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\nbuild = \"build.rs\"\n",
    )
    .unwrap();
    fs::write(
        project.join("build.rs"),
        format!(
            "fn main() {{ std::fs::write({:?}, b\"executed\").unwrap(); }}\n",
            build_sentinel.to_str().unwrap()
        ),
    )
    .unwrap();
    let source = project.join("src");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("lib.rs"), "pub fn fixture() {}\n").unwrap();
    if lockfile {
        fs::write(
            project.join("Cargo.lock"),
            "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n[[package]]\nname = \"dux-cargo-observer-fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
    }
    build_sentinel
}

fn toml_string(path: &Path) -> String {
    serde_json::to_string(path.to_str().unwrap()).unwrap()
}
