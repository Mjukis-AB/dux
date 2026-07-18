#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde_json::json;
use tempfile::TempDir;

use super::rust_target::validate_live_rust_target;
use super::rust_target_cargo::{
    CargoMetadataValidationError, CargoOutputStream, observe_cargo_executable,
    validate_cargo_metadata, validate_cargo_metadata_for_test,
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

    fn observe(&self) -> super::rust_target_cargo::CargoExecutableObservation {
        observe_cargo_executable(&self.executable).unwrap()
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn metadata_json(workspace_root: &Path, target_directory: &Path) -> String {
    serde_json::to_string(&json!({
        "packages": [],
        "workspace_members": [],
        "workspace_default_members": [],
        "resolve": null,
        "target_directory": target_directory,
        "version": 1,
        "workspace_root": workspace_root,
        "metadata": {},
        "future_additive_field": {"accepted": true}
    }))
    .unwrap()
}

fn valid_metadata_action(fixture: &Fixture) -> String {
    let document = metadata_json(fixture.manifest.parent().unwrap(), &fixture.target);
    format!(
        "  [ \"$#\" -eq 10 ] || exit 70\n  [ \"$2\" = \"--format-version\" ] || exit 71\n  [ \"$3\" = \"1\" ] || exit 72\n  [ \"$4\" = \"--no-deps\" ] || exit 73\n  [ \"$5\" = \"--locked\" ] || exit 74\n  [ \"$6\" = \"--offline\" ] || exit 75\n  [ \"$7\" = \"--quiet\" ] || exit 76\n  [ \"$8\" = \"--color=never\" ] || exit 77\n  [ \"$9\" = \"--manifest-path\" ] || exit 78\n  [ \"${{10}}\" = {} ] || exit 79\n  [ \"$PWD\" = {} ] || exit 80\n  [ -n \"$HOME\" ] && [ -n \"$CARGO_HOME\" ] && [ -n \"$TMPDIR\" ] || exit 81\n  [ \"$PATH\" = \"/dev/null\" ] || exit 82\n  [ -z \"${{RUSTUP_TOOLCHAIN+x}}\" ] && [ -z \"${{CARGO_TARGET_DIR+x}}\" ] || exit 83\n  project-helper >/dev/null 2>&1 && exit 84\n  printf %s {}\n  exit 0",
        shell_quote(fixture.manifest.to_str().unwrap()),
        shell_quote(fixture.manifest.parent().unwrap().to_str().unwrap()),
        shell_quote(&document),
    )
}

fn live(fixture: &Fixture) -> super::rust_target::RustTargetLiveWitness {
    validate_live_rust_target(fixture.source(), &fixture.candidate()).unwrap()
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
        validate_live_rust_target(fixture.source(), &candidate).unwrap(),
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
    assert_eq!(witness.resolution_policy_revision(), 1);
    assert!(witness.live().protected_path_is_still_unresolved());
    assert_eq!(candidate.blockers(), [BlockReason::ProtectedPath]);
    assert!(!candidate.rule_marks_schedule_eligible());
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
        "cargo 2.0.0\nrelease: 2.0.0\n",
    ] {
        let invalid_version = FakeCargo::with_version("  exit 9", version);
        assert!(matches!(
            observe_cargo_executable(&invalid_version.executable),
            Err(CargoMetadataValidationError::InvalidCargoVersion)
        ));
    }

    let real = FakeCargo::new(&valid_metadata_action(&fixture));
    let link_root = TempDir::new().unwrap();
    let link = link_root.path().join("cargo");
    symlink(&real.executable, &link).unwrap();
    assert!(matches!(
        observe_cargo_executable(&link),
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
fn same_inode_manifest_rewrite_during_metadata_is_detected_by_full_digest() {
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
        Err(CargoMetadataValidationError::LiveEvidenceChanged)
    ));
}

#[test]
fn real_cargo_honors_project_config_and_no_deps_does_not_create_lockfile() {
    let Some(cargo) = direct_test_cargo() else {
        return;
    };
    let fixture = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let build_sentinel = prepare_real_package(&fixture, true);
    let observation = observe_cargo_executable(&cargo).unwrap();
    let _witness = validate_cargo_metadata(live(&fixture), &observation).unwrap();
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

    let unlocked = Fixture::new(CARGO_CACHE_TAG_SIGNATURE);
    let unlocked_sentinel = prepare_real_package(&unlocked, false);
    let lockfile = unlocked.manifest.parent().unwrap().join("Cargo.lock");
    let _witness = validate_cargo_metadata(live(&unlocked), &observation).unwrap();
    assert!(!lockfile.exists());
    assert!(!unlocked_sentinel.exists());
}

fn direct_test_cargo() -> Option<PathBuf> {
    let configured = std::env::var_os("CARGO")?;
    let canonical = fs::canonicalize(configured).ok()?;
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
