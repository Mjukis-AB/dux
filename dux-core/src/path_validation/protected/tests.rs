use super::*;
use crate::path_validation::{
    capture_path_snapshot, capture_scan_root, validate_cleanup_path, validate_scan_root,
};
use tempfile::tempdir_in;

fn policy(platform: PolicyPlatform, home: PolicyPath) -> PlatformPolicy {
    let home_paths = vec![home];
    PlatformPolicy {
        platform,
        profile_containers: profile_parents(&home_paths),
        home_paths,
    }
}

fn target(policy: &PlatformPolicy, path: &PolicyPath) -> Option<ProtectionMatch> {
    policy.classify_target(path)
}

fn denied(kind: ProtectedPathKind) -> Option<ProtectionMatch> {
    Some(ProtectionMatch {
        level: ProtectionLevel::Denied,
        kind,
    })
}

fn guarded(kind: ProtectedPathKind) -> Option<ProtectionMatch> {
    Some(ProtectionMatch {
        level: ProtectionLevel::SpecificRuleRequired,
        kind,
    })
}

#[test]
fn policy_tables_are_versioned_unique_and_have_expected_sizes() {
    assert_eq!(PROTECTED_ROOT_POLICY_REVISION, 2);
    for (platform, expected) in [
        (PolicyPlatform::MacOs, 18),
        (PolicyPlatform::Linux, 24),
        (PolicyPlatform::Windows, 10),
    ] {
        let rules = rules(platform);
        assert_eq!(rules.len(), expected);
        for (index, rule) in rules.iter().enumerate() {
            assert!(!rule.components.is_empty());
            assert!(!rules[..index].iter().any(|prior| {
                prior.components.len() == rule.components.len()
                    && prior
                        .components
                        .iter()
                        .zip(rule.components)
                        .all(|(left, right)| platform.components_equal(left, right))
            }));
        }
    }
}

#[test]
fn every_static_rule_has_exhaustive_target_and_scan_coverage() {
    for platform in [
        PolicyPlatform::MacOs,
        PolicyPlatform::Linux,
        PolicyPlatform::Windows,
    ] {
        for rule in rules(platform) {
            let exact = match platform {
                PolicyPlatform::Windows => PolicyPath::drive('Q', rule.components),
                _ => PolicyPath::unix(rule.components),
            };
            let mut descendant = exact.clone();
            descendant.components.push("child".to_owned());
            let matched = |level| {
                Some(ProtectionMatch {
                    level,
                    kind: rule.kind,
                })
            };
            let expected = match rule.coverage {
                ProtectedCoverage::Exact => (matched(ProtectionLevel::Denied), None, None, None),
                ProtectedCoverage::HardTree => (
                    matched(ProtectionLevel::Denied),
                    matched(ProtectionLevel::Denied),
                    matched(ProtectionLevel::Denied),
                    matched(ProtectionLevel::Denied),
                ),
                ProtectedCoverage::GuardedTree => (
                    matched(ProtectionLevel::Denied),
                    matched(ProtectionLevel::SpecificRuleRequired),
                    matched(ProtectionLevel::SpecificRuleRequired),
                    matched(ProtectionLevel::SpecificRuleRequired),
                ),
            };
            assert_eq!(
                (
                    rule.classify(platform, MatchContext::Target, &exact),
                    rule.classify(platform, MatchContext::Target, &descendant),
                    rule.classify(platform, MatchContext::ScanScope, &exact),
                    rule.classify(platform, MatchContext::ScanScope, &descendant),
                ),
                expected,
                "wrong coverage for {platform:?} {:?}",
                rule.components
            );
        }
    }
}

#[test]
fn policy_revision_two_has_a_checked_full_table_fingerprint() {
    let mut signature = String::new();
    for platform in [
        PolicyPlatform::MacOs,
        PolicyPlatform::Linux,
        PolicyPlatform::Windows,
    ] {
        for rule in rules(platform) {
            use std::fmt::Write;
            writeln!(
                signature,
                "{platform:?}|{}|{:?}|{:?}",
                rule.components.join("/"),
                rule.coverage,
                rule.kind
            )
            .unwrap();
        }
    }
    let fingerprint = signature
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    const POLICY_REVISION_2_STATIC_TABLE_FINGERPRINT: u64 = 10_830_234_716_710_889_208;
    assert_eq!(fingerprint, POLICY_REVISION_2_STATIC_TABLE_FINGERPRINT);
}

#[test]
fn obvious_non_home_boundaries_are_rejected_as_home_evidence() {
    for (platform, paths) in [
        (
            PolicyPlatform::MacOs,
            vec![
                PolicyPath::unix(&[]),
                PolicyPath::unix(&["Users"]),
                PolicyPath::unix(&["System"]),
                PolicyPath::unix(&["Applications"]),
            ],
        ),
        (
            PolicyPlatform::Linux,
            vec![
                PolicyPath::unix(&[]),
                PolicyPath::unix(&["home"]),
                PolicyPath::unix(&["usr"]),
                PolicyPath::unix(&["opt"]),
            ],
        ),
        (
            PolicyPlatform::Windows,
            vec![
                PolicyPath::drive('C', &[]),
                PolicyPath::drive('C', &["Users"]),
                PolicyPath::drive('C', &["Windows"]),
                PolicyPath::drive('C', &["Program Files"]),
            ],
        ),
    ] {
        for path in paths {
            assert!(!valid_home_path(platform, &path), "accepted {path:?}");
        }
    }
}

#[test]
fn macos_hard_guarded_and_exact_roots_have_distinct_coverage() {
    let policy = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["System", "Library"])),
        denied(ProtectedPathKind::OperatingSystem)
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["Applications"])),
        denied(ProtectedPathKind::ApplicationInstallations)
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["Applications", "Tool.app"])),
        guarded(ProtectedPathKind::ApplicationInstallations)
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["Volumes"])),
        denied(ProtectedPathKind::VolumeMountContainer)
    );
    assert_eq!(
        target(
            &policy,
            &PolicyPath::unix(&["Volumes", "External", "cache"])
        ),
        None
    );
    assert_eq!(target(&policy, &PolicyPath::unix(&["var", "tmp"])), None);
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["private", "var", "tmp"])),
        denied(ProtectedPathKind::OperatingSystem)
    );
}

#[test]
fn macos_current_home_and_library_are_gated_while_foreign_homes_are_denied() {
    let policy = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["Users", "alice"])),
        denied(ProtectedPathKind::UserHome)
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["Users", "alice", "Library"])),
        denied(ProtectedPathKind::UserLibrary)
    );
    assert_eq!(
        target(
            &policy,
            &PolicyPath::unix(&["Users", "alice", "Library", "Caches"])
        ),
        guarded(ProtectedPathKind::UserLibrary)
    );
    assert_eq!(
        target(
            &policy,
            &PolicyPath::unix(&["Users", "alice", "Downloads", "file"])
        ),
        None
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["Users", "bob", "Downloads"])),
        denied(ProtectedPathKind::UserHome)
    );
}

#[test]
fn linux_covers_system_package_service_and_mount_anchors() {
    let policy = policy(PolicyPlatform::Linux, PolicyPath::unix(&["home", "alice"]));
    for (components, kind) in [
        (&["usr", "bin"][..], ProtectedPathKind::OperatingSystem),
        (&["var", "cache"][..], ProtectedPathKind::OperatingSystem),
        (&["opt", "tool"][..], ProtectedPathKind::ManagedSoftware),
        (&["srv", "data"][..], ProtectedPathKind::ServiceData),
        (&["nix", "store"][..], ProtectedPathKind::PackageStore),
        (&["snap", "core"][..], ProtectedPathKind::PackageStore),
    ] {
        assert_eq!(target(&policy, &PolicyPath::unix(components)), denied(kind));
    }
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["mnt"])),
        denied(ProtectedPathKind::VolumeMountContainer)
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["mnt", "disk", "cache"])),
        None
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["home", "bob", ".cache"])),
        denied(ProtectedPathKind::UserHome)
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["home", "alice", ".cache"])),
        None
    );
}

#[test]
fn windows_rules_apply_on_every_drive_with_component_boundaries() {
    let policy = policy(
        PolicyPlatform::Windows,
        PolicyPath::drive('C', &["Users", "Alice"]),
    );
    assert_eq!(
        target(&policy, &PolicyPath::drive('D', &["WINDOWS", "System32"])),
        denied(ProtectedPathKind::OperatingSystem)
    );
    assert_eq!(
        target(&policy, &PolicyPath::drive('D', &["Program Files", "Tool"])),
        guarded(ProtectedPathKind::ProgramFiles)
    );
    assert_eq!(
        target(
            &policy,
            &PolicyPath::drive('C', &["Users", "Bob", "Downloads"])
        ),
        denied(ProtectedPathKind::UserHome)
    );
    assert_eq!(
        target(
            &policy,
            &PolicyPath::drive('C', &["Users", "Alice", "AppData"])
        ),
        denied(ProtectedPathKind::UserLibrary)
    );
    assert_eq!(
        target(
            &policy,
            &PolicyPath::drive('C', &["Users", "Alice", "AppData", "Local"])
        ),
        guarded(ProtectedPathKind::UserLibrary)
    );
    assert_eq!(
        target(&policy, &PolicyPath::drive('C', &["Windows.old", "file"])),
        None
    );
    assert_eq!(
        target(&policy, &PolicyPath::drive('C', &[])),
        denied(ProtectedPathKind::FilesystemRoot)
    );
}

#[test]
fn relocated_profile_container_siblings_are_denied() {
    let windows = policy(
        PolicyPlatform::Windows,
        PolicyPath::drive('D', &["Profiles", "Ålice"]),
    );
    assert_eq!(
        target(
            &windows,
            &PolicyPath::drive('D', &["Profiles", "Bob", "Downloads"])
        ),
        denied(ProtectedPathKind::UserHome)
    );
    assert_eq!(
        target(
            &windows,
            &PolicyPath::drive('D', &["Profiles", "Ålice", "Downloads"])
        ),
        None
    );
    assert_eq!(
        target(
            &windows,
            &PolicyPath::drive('D', &["Profiles", "ålice", "Downloads"])
        ),
        denied(ProtectedPathKind::UserHome)
    );

    let macos = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["home", "alice"]));
    assert_eq!(
        target(&macos, &PolicyPath::unix(&["home", "bob", "file"])),
        denied(ProtectedPathKind::UserHome)
    );
}

#[test]
fn foreign_profile_deny_overrides_library_and_appdata_guards() {
    let macos = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
    assert_eq!(
        target(
            &macos,
            &PolicyPath::unix(&["Users", "bob", "Library", "Caches"])
        ),
        denied(ProtectedPathKind::UserHome)
    );
    let windows = policy(
        PolicyPlatform::Windows,
        PolicyPath::drive('C', &["Users", "Alice"]),
    );
    assert_eq!(
        target(
            &windows,
            &PolicyPath::drive('C', &["Users", "Bob", "AppData", "Local"])
        ),
        denied(ProtectedPathKind::UserHome)
    );
}

#[test]
fn unresolved_windows_legacy_alias_never_becomes_positive_authority() {
    let policy = policy(
        PolicyPlatform::Windows,
        PolicyPath::drive('C', &["Users", "Alice"]),
    );
    let alias = PolicyPath::drive('C', &["PROGRA~1", "Tool"]);
    assert_eq!(
        policy.assess(&[(
            ProtectedPathForm::TargetRequested,
            MatchContext::Target,
            &alias,
        )]),
        ProtectedRootDisposition::NoTextualMatch {
            policy_revision: PROTECTED_ROOT_POLICY_REVISION,
        }
    );
}

#[test]
fn scan_scope_propagates_only_hard_and_guarded_tree_restrictions() {
    let macos = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
    assert_eq!(
        macos.classify_scan_scope(&PolicyPath::unix(&["System", "Library"])),
        denied(ProtectedPathKind::OperatingSystem)
    );
    assert_eq!(
        macos.classify_scan_scope(&PolicyPath::unix(&["Applications"])),
        guarded(ProtectedPathKind::ApplicationInstallations)
    );
    assert_eq!(
        macos.classify_scan_scope(&PolicyPath::unix(&["Users"])),
        None
    );
    assert_eq!(
        macos.classify_scan_scope(&PolicyPath::unix(&["Users", "alice"])),
        None
    );
    assert_eq!(
        macos.classify_scan_scope(&PolicyPath::unix(&["Users", "bob"])),
        denied(ProtectedPathKind::UserHome)
    );
}

#[test]
fn hard_deny_overrides_guard_across_requested_and_canonical_forms() {
    let policy = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
    let requested = PolicyPath::unix(&["Users", "alice", "Library", "Caches"]);
    let canonical = PolicyPath::unix(&["System", "Volumes", "Data", "cache"]);
    assert_eq!(
        policy.assess(&[
            (
                ProtectedPathForm::TargetRequested,
                MatchContext::Target,
                &requested
            ),
            (
                ProtectedPathForm::TargetCanonical,
                MatchContext::Target,
                &canonical
            ),
        ]),
        ProtectedRootDisposition::Denied {
            form: ProtectedPathForm::TargetCanonical,
            kind: ProtectedPathKind::OperatingSystem,
            policy_revision: PROTECTED_ROOT_POLICY_REVISION,
        }
    );
}

#[test]
fn each_scan_and_target_path_form_can_independently_deny() {
    let policy = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "alice"]));
    let hard = PolicyPath::unix(&["System", "Library"]);
    for (form, context) in [
        (
            ProtectedPathForm::ScanRootRequested,
            MatchContext::ScanScope,
        ),
        (
            ProtectedPathForm::ScanRootCanonical,
            MatchContext::ScanScope,
        ),
        (ProtectedPathForm::TargetRequested, MatchContext::Target),
        (ProtectedPathForm::TargetCanonical, MatchContext::Target),
    ] {
        assert_eq!(
            policy.assess(&[(form, context, &hard)]),
            ProtectedRootDisposition::Denied {
                form,
                kind: ProtectedPathKind::OperatingSystem,
                policy_revision: PROTECTED_ROOT_POLICY_REVISION,
            }
        );
    }
}

#[test]
fn custom_home_is_exactly_protected_without_ambient_environment_reads() {
    let policy = policy(
        PolicyPlatform::Linux,
        PolicyPath::unix(&["custom", "alice"]),
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["custom", "alice"])),
        denied(ProtectedPathKind::UserHome)
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["custom", "alice", ".cache"])),
        None
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["home", "alice", ".cache"])),
        denied(ProtectedPathKind::UserHome)
    );
}

#[test]
fn matching_is_component_aware_and_platform_case_policy_is_conservative() {
    let linux = policy(PolicyPlatform::Linux, PolicyPath::unix(&["home", "alice"]));
    let macos = policy(PolicyPlatform::MacOs, PolicyPath::unix(&["Users", "Ålice"]));
    assert_eq!(
        target(&linux, &PolicyPath::unix(&["usr-local", "cache"])),
        None
    );
    assert_eq!(target(&linux, &PolicyPath::unix(&["USR", "bin"])), None);
    assert_eq!(
        target(&macos, &PolicyPath::unix(&["system", "library"])),
        denied(ProtectedPathKind::OperatingSystem)
    );
    assert_eq!(
        target(&macos, &PolicyPath::unix(&["Users", "ålice", "Downloads"])),
        denied(ProtectedPathKind::UserHome)
    );
}

#[test]
fn every_no_match_disposition_is_explicitly_non_authoritative_and_versioned() {
    let policy = policy(PolicyPlatform::Linux, PolicyPath::unix(&["home", "alice"]));
    let path = PolicyPath::unix(&["home", "alice", ".cache"]);
    let disposition = policy.assess(&[(
        ProtectedPathForm::TargetRequested,
        MatchContext::Target,
        &path,
    )]);
    assert_eq!(
        disposition,
        ProtectedRootDisposition::NoTextualMatch {
            policy_revision: PROTECTED_ROOT_POLICY_REVISION,
        }
    );
    assert_eq!(
        disposition.policy_revision(),
        PROTECTED_ROOT_POLICY_REVISION
    );
}

#[test]
fn diagnostics_never_include_user_paths() {
    let error = ProtectedRootError::InvalidPath {
        form: ProtectedPathForm::TargetCanonical,
    };
    assert!(!error.to_string().contains("Users"));
    assert!(!std::mem::needs_drop::<ProtectedRootError>());
}

#[test]
fn live_preflight_and_canonical_assessment_do_not_mutate_the_target() {
    let current = std::env::current_dir().unwrap();
    let temp = tempdir_in(current).unwrap();
    let home_path = std::fs::canonicalize(temp.path()).unwrap();
    let target_path = home_path.join("ordinary-cache");
    std::fs::write(&target_path, b"unchanged").unwrap();
    let lexical_home = validate_scan_root(&home_path).unwrap();
    let canonical_home = capture_scan_root(lexical_home.clone()).unwrap();
    let lexical_target = validate_cleanup_path(&lexical_home, &target_path).unwrap();
    let registry = ProtectedRootRegistry::with_home_directory_evidence(&canonical_home).unwrap();
    assert!(matches!(
        registry.preflight(&lexical_target).unwrap(),
        ProtectedRootDisposition::NoTextualMatch { .. }
    ));
    let snapshot = capture_path_snapshot(&canonical_home, lexical_target).unwrap();
    assert!(matches!(
        registry.assess(&canonical_home, &snapshot).unwrap(),
        ProtectedRootDisposition::NoTextualMatch { .. }
    ));
    assert_eq!(std::fs::read(&target_path).unwrap(), b"unchanged");
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test replaces a TempDir-owned scan root to exercise identity rejection"
)]
fn replacement_scan_root_at_the_same_path_rejects_old_target_evidence() {
    let current = std::env::current_dir().unwrap();
    let container = tempdir_in(current).unwrap();
    let root_path = container.path().join("scan-root");
    let moved_root_path = container.path().join("moved-scan-root");
    std::fs::create_dir(&root_path).unwrap();
    let target_path = root_path.join("ordinary-cache");
    std::fs::write(&target_path, b"old-root").unwrap();

    let old_lexical_root = validate_scan_root(&root_path).unwrap();
    let old_canonical_root = capture_scan_root(old_lexical_root.clone()).unwrap();
    let old_lexical_target = validate_cleanup_path(&old_lexical_root, &target_path).unwrap();
    let old_snapshot = capture_path_snapshot(&old_canonical_root, old_lexical_target).unwrap();
    let registry =
        ProtectedRootRegistry::with_home_directory_evidence(&old_canonical_root).unwrap();

    // DUX-DESTRUCTIVE: allow=test-protected-replaced-root -- replace a TempDir-owned root to verify stale root evidence rejection
    std::fs::rename(&root_path, &moved_root_path).unwrap();
    std::fs::create_dir(&root_path).unwrap();
    let replacement_root = capture_scan_root(validate_scan_root(&root_path).unwrap()).unwrap();
    assert_ne!(old_canonical_root.identity(), replacement_root.identity());
    assert_eq!(
        registry.assess(&replacement_root, &old_snapshot),
        Err(ProtectedRootError::MismatchedScanRootEvidence)
    );
}

#[test]
fn unsupported_policy_denies_every_target() {
    let policy = policy(
        PolicyPlatform::Unsupported,
        PolicyPath::unix(&["home", "alice"]),
    );
    assert_eq!(
        target(&policy, &PolicyPath::unix(&["tmp", "file"])),
        denied(ProtectedPathKind::FilesystemRoot)
    );
}

#[cfg(unix)]
#[test]
fn current_account_registry_uses_os_account_home_and_stays_text_only() {
    let registry = ProtectedRootRegistry::from_current_account().unwrap();
    let home = User::from_uid(geteuid()).unwrap().unwrap().dir;
    let lexical_home = validate_scan_root(&home).unwrap();
    let lexical_target = validate_cleanup_path(&lexical_home, &home.join("Library")).unwrap();
    assert!(matches!(
        registry.preflight(&lexical_target).unwrap(),
        ProtectedRootDisposition::Denied { .. }
            | ProtectedRootDisposition::SpecificRuleRequired { .. }
    ));
    assert!(!format!("{registry:?}").contains("HOME"));
}

#[cfg(target_os = "macos")]
#[test]
fn trusted_home_mount_witness_is_current_account_bound_and_revalidatable() {
    let home = CurrentAccountHomeEvidence::capture().unwrap();
    let witness = TrustedHomeMountWitness::capture(home.root()).unwrap();
    assert_eq!(witness.proof_revision(), TRUSTED_HOME_MOUNT_PROOF_REVISION);
    witness.revalidate().unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn trusted_home_mount_witness_rejects_a_foreign_scan_root() {
    let foreign = std::fs::canonicalize("/tmp").unwrap();
    let root = capture_scan_root(validate_scan_root(&foreign).unwrap()).unwrap();
    assert!(TrustedHomeMountWitness::capture(&root).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn known_user_library_caches_is_derived_from_the_os_account_home() {
    let expected_home = User::from_uid(geteuid()).unwrap().unwrap().dir;
    let known = KnownUserLibraryCachesPath::capture().unwrap();
    assert_eq!(known.path(), expected_home.join("Library").join("Caches"));
    assert_eq!(
        known.proof_revision,
        KNOWN_USER_LIBRARY_CACHES_PROOF_REVISION
    );
    known.revalidate().unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn known_user_library_caches_retains_a_revalidatable_home_mount_witness() {
    let known = KnownUserLibraryCachesPath::capture().unwrap();
    let Ok(root) = capture_scan_root(validate_scan_root(known.path()).unwrap()) else {
        eprintln!("skipping known-cache witness test because Library/Caches is unavailable");
        return;
    };
    let witness = known.capture_mount_witness(&root).unwrap();
    assert_eq!(witness.proof_revision(), TRUSTED_HOME_MOUNT_PROOF_REVISION);
    witness.revalidate().unwrap();

    let home = CurrentAccountHomeEvidence::capture().unwrap();
    assert!(matches!(
        known.capture_mount_witness(home.root()),
        Err(KnownUserLibraryCachesError::RootMismatch)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn trusted_home_mount_witness_fails_closed_outside_macos() {
    let root = capture_scan_root(validate_scan_root(Path::new("/tmp")).unwrap()).unwrap();
    assert!(matches!(
        TrustedHomeMountWitness::capture(&root),
        Err(TrustedHomeMountError::UnsupportedPlatform)
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn known_user_library_caches_fails_closed_outside_macos() {
    assert!(matches!(
        KnownUserLibraryCachesPath::capture(),
        Err(KnownUserLibraryCachesError::UnsupportedPlatform)
    ));
}

#[cfg(unix)]
#[test]
fn account_home_validation_rejects_ambiguous_or_lossy_paths() {
    for path in [Path::new("relative/home"), Path::new("")] {
        assert_eq!(
            validate_account_home_path(path),
            Err(ProtectedRootError::InvalidHomeDirectory)
        );
    }
    assert!(validate_account_home_path(Path::new("/tmp")).is_ok());
}
