use std::collections::HashSet;
use std::path::{Component, Path};

use serde::Deserialize;

use super::*;
use crate::path_validation::{LexicalPathError, validate_cleanup_path, validate_scan_root};

const CORPUS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/dangerous-paths/v1/corpus.json"
));

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DangerousPathCorpus {
    schema_version: u32,
    lexical_cases: Vec<LexicalCase>,
    policy_roots: Vec<PolicyRootCase>,
    dynamic_policy_cases: Vec<DynamicPolicyCase>,
    known_gaps: Vec<KnownGap>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CorpusPlatform {
    Unix,
    Macos,
    Linux,
    Windows,
    All,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LexicalCase {
    id: String,
    platform: CorpusPlatform,
    operation: LexicalOperation,
    scan_root: Option<String>,
    input: String,
    expected: LexicalExpected,
    rationale: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LexicalOperation {
    Scan,
    Cleanup,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LexicalExpected {
    Reject,
    OkNonAuthoritative,
    Empty,
    NotAbsolute,
    RepeatedSeparator,
    TrailingSeparator,
    CurrentDirectory,
    ParentTraversal,
    ControlCharacter,
    FilesystemRoot,
    EqualToScanRoot,
    OutsideScanRoot,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyRootCase {
    id: String,
    platform: CorpusPlatform,
    path: String,
    coverage: CorpusCoverage,
    kind: CorpusKind,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CorpusCoverage {
    Exact,
    HardTree,
    GuardedTree,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CorpusKind {
    FilesystemRoot,
    OperatingSystem,
    ApplicationInstallations,
    SharedSystemLibrary,
    UserHomesContainer,
    VolumeMountContainer,
    UserHome,
    UserLibrary,
    ManagedSoftware,
    ServiceData,
    PackageStore,
    ProgramFiles,
    ProgramData,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DynamicPolicyCase {
    id: String,
    platform: CorpusPlatform,
    home: String,
    context: CorpusContext,
    path: String,
    expected: CorpusDisposition,
    kind: Option<CorpusKind>,
    rationale: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CorpusContext {
    Target,
    ScanScope,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CorpusDisposition {
    Denied,
    SpecificRuleRequired,
    NoTextualMatch,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownGap {
    id: String,
    platform: CorpusPlatform,
    expected: KnownGapExpected,
    rationale: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum KnownGapExpected {
    NonAuthoritative,
}

fn corpus() -> DangerousPathCorpus {
    serde_json::from_str(CORPUS).expect("dangerous-path corpus must remain valid JSON")
}

fn policy_platform(platform: CorpusPlatform) -> PolicyPlatform {
    match platform {
        CorpusPlatform::Macos => PolicyPlatform::MacOs,
        CorpusPlatform::Linux => PolicyPlatform::Linux,
        CorpusPlatform::Windows => PolicyPlatform::Windows,
        CorpusPlatform::Unix | CorpusPlatform::All => {
            panic!("corpus platform {platform:?} is not a protected-policy platform")
        }
    }
}

fn policy_path(platform: PolicyPlatform, value: &str) -> PolicyPath {
    match platform {
        PolicyPlatform::MacOs | PolicyPlatform::Linux | PolicyPlatform::Unsupported => {
            let relative = value
                .strip_prefix('/')
                .unwrap_or_else(|| panic!("Unix policy path is not absolute: {value:?}"));
            if relative.is_empty() {
                return PolicyPath::unix(&[]);
            }
            let components = relative.split('/').collect::<Vec<_>>();
            assert!(
                components
                    .iter()
                    .all(|component| !component.is_empty() && !matches!(*component, "." | "..")),
                "ambiguous Unix policy path: {value:?}"
            );
            PolicyPath::unix(&components)
        }
        PolicyPlatform::Windows => {
            let bytes = value.as_bytes();
            assert!(
                bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && bytes[2] == b'\\',
                "Windows policy path is not drive-absolute: {value:?}"
            );
            let relative = &value[3..];
            if relative.is_empty() {
                return PolicyPath::drive(char::from(bytes[0]), &[]);
            }
            let components = relative.split('\\').collect::<Vec<_>>();
            assert!(
                components
                    .iter()
                    .all(|component| !component.is_empty() && !matches!(*component, "." | "..")),
                "ambiguous Windows policy path: {value:?}"
            );
            PolicyPath::drive(char::from(bytes[0]), &components)
        }
    }
}

fn protected_kind(kind: CorpusKind) -> ProtectedPathKind {
    match kind {
        CorpusKind::FilesystemRoot => ProtectedPathKind::FilesystemRoot,
        CorpusKind::OperatingSystem => ProtectedPathKind::OperatingSystem,
        CorpusKind::ApplicationInstallations => ProtectedPathKind::ApplicationInstallations,
        CorpusKind::SharedSystemLibrary => ProtectedPathKind::SharedSystemLibrary,
        CorpusKind::UserHomesContainer => ProtectedPathKind::UserHomesContainer,
        CorpusKind::VolumeMountContainer => ProtectedPathKind::VolumeMountContainer,
        CorpusKind::UserHome => ProtectedPathKind::UserHome,
        CorpusKind::UserLibrary => ProtectedPathKind::UserLibrary,
        CorpusKind::ManagedSoftware => ProtectedPathKind::ManagedSoftware,
        CorpusKind::ServiceData => ProtectedPathKind::ServiceData,
        CorpusKind::PackageStore => ProtectedPathKind::PackageStore,
        CorpusKind::ProgramFiles => ProtectedPathKind::ProgramFiles,
        CorpusKind::ProgramData => ProtectedPathKind::ProgramData,
    }
}

fn protected_coverage(coverage: CorpusCoverage) -> ProtectedCoverage {
    match coverage {
        CorpusCoverage::Exact => ProtectedCoverage::Exact,
        CorpusCoverage::HardTree => ProtectedCoverage::HardTree,
        CorpusCoverage::GuardedTree => ProtectedCoverage::GuardedTree,
    }
}

fn matched(level: ProtectionLevel, kind: CorpusKind) -> Option<ProtectionMatch> {
    Some(ProtectionMatch {
        level,
        kind: protected_kind(kind),
    })
}

fn lexical_case_applies(platform: CorpusPlatform) -> bool {
    match platform {
        CorpusPlatform::Unix => cfg!(unix),
        CorpusPlatform::Windows => cfg!(windows),
        CorpusPlatform::Macos | CorpusPlatform::Linux | CorpusPlatform::All => false,
    }
}

fn lexical_matches<T>(result: &Result<T, LexicalPathError>, expected: LexicalExpected) -> bool {
    match expected {
        LexicalExpected::Reject => result.is_err(),
        LexicalExpected::OkNonAuthoritative => result.is_ok(),
        LexicalExpected::Empty => matches!(result, Err(LexicalPathError::Empty)),
        LexicalExpected::NotAbsolute => matches!(result, Err(LexicalPathError::NotAbsolute)),
        LexicalExpected::RepeatedSeparator => {
            matches!(result, Err(LexicalPathError::RepeatedSeparator { .. }))
        }
        LexicalExpected::TrailingSeparator => {
            matches!(result, Err(LexicalPathError::TrailingSeparator))
        }
        LexicalExpected::CurrentDirectory => {
            matches!(result, Err(LexicalPathError::CurrentDirectory { .. }))
        }
        LexicalExpected::ParentTraversal => {
            matches!(result, Err(LexicalPathError::ParentTraversal { .. }))
        }
        LexicalExpected::ControlCharacter => {
            matches!(result, Err(LexicalPathError::ControlCharacter { .. }))
        }
        LexicalExpected::FilesystemRoot => {
            matches!(result, Err(LexicalPathError::FilesystemRoot))
        }
        LexicalExpected::EqualToScanRoot => {
            matches!(result, Err(LexicalPathError::EqualToScanRoot))
        }
        LexicalExpected::OutsideScanRoot => {
            matches!(result, Err(LexicalPathError::OutsideScanRoot))
        }
    }
}

#[test]
fn versioned_corpus_is_explicit_unique_and_stage_scoped() {
    let corpus = corpus();
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.lexical_cases.len(), 37);
    assert_eq!(corpus.policy_roots.len(), 52);
    assert_eq!(corpus.dynamic_policy_cases.len(), 24);
    assert_eq!(corpus.known_gaps.len(), 6);

    assert!(corpus.lexical_cases.iter().all(|case| matches!(
        case.platform,
        CorpusPlatform::Unix | CorpusPlatform::Windows
    )));
    assert!(corpus.policy_roots.iter().all(|case| matches!(
        case.platform,
        CorpusPlatform::Macos | CorpusPlatform::Linux | CorpusPlatform::Windows
    )));
    assert!(corpus.dynamic_policy_cases.iter().all(|case| matches!(
        case.platform,
        CorpusPlatform::Macos | CorpusPlatform::Linux | CorpusPlatform::Windows
    )));

    let mut ids = HashSet::new();
    for (id, rationale) in corpus
        .lexical_cases
        .iter()
        .map(|case| (&case.id, Some(&case.rationale)))
        .chain(corpus.policy_roots.iter().map(|case| (&case.id, None)))
        .chain(
            corpus
                .dynamic_policy_cases
                .iter()
                .map(|case| (&case.id, Some(&case.rationale))),
        )
        .chain(
            corpus
                .known_gaps
                .iter()
                .map(|case| (&case.id, Some(&case.rationale))),
        )
    {
        assert!(!id.is_empty());
        assert!(ids.insert(id), "duplicate dangerous-path case ID {id:?}");
        if let Some(rationale) = rationale {
            assert!(!rationale.trim().is_empty(), "missing rationale for {id:?}");
        }
    }
    for gap in corpus.known_gaps {
        assert_eq!(gap.expected, KnownGapExpected::NonAuthoritative);
        assert!(matches!(
            gap.platform,
            CorpusPlatform::Macos
                | CorpusPlatform::Linux
                | CorpusPlatform::Windows
                | CorpusPlatform::All
        ));
    }
}

#[test]
fn native_lexical_corpus_has_only_stage_specific_non_authoritative_success() {
    let mut exercised = 0;
    for case in corpus()
        .lexical_cases
        .into_iter()
        .filter(|case| lexical_case_applies(case.platform))
    {
        exercised += 1;
        match case.operation {
            LexicalOperation::Scan => {
                assert!(
                    case.scan_root.is_none(),
                    "{} has an unused scan root",
                    case.id
                );
                let result = validate_scan_root(Path::new(&case.input));
                assert!(
                    lexical_matches(&result, case.expected),
                    "{}: expected {:?}, observed {result:?}",
                    case.id,
                    case.expected
                );
            }
            LexicalOperation::Cleanup => {
                let scan_root = case
                    .scan_root
                    .as_deref()
                    .unwrap_or_else(|| panic!("{} is missing scan_root", case.id));
                let scan_root = validate_scan_root(Path::new(scan_root))
                    .unwrap_or_else(|error| panic!("{} has invalid scan root: {error}", case.id));
                let result = validate_cleanup_path(&scan_root, Path::new(&case.input));
                assert!(
                    lexical_matches(&result, case.expected),
                    "{}: expected {:?}, observed {result:?}",
                    case.id,
                    case.expected
                );
            }
        }
    }
    let expected = if cfg!(windows) { 20 } else { 17 };
    assert_eq!(exercised, expected);
}

#[test]
fn independently_authored_policy_root_corpus_matches_all_coverage_semantics() {
    let corpus = corpus();
    let counts = [
        (CorpusPlatform::Macos, 18),
        (CorpusPlatform::Linux, 24),
        (CorpusPlatform::Windows, 10),
    ];
    for (platform, expected_count) in counts {
        assert_eq!(
            corpus
                .policy_roots
                .iter()
                .filter(|case| case.platform == platform)
                .count(),
            expected_count
        );

        let policy_platform = policy_platform(platform);
        for rule in rules(policy_platform) {
            let production_path = match policy_platform {
                PolicyPlatform::Windows => PolicyPath::drive('C', rule.components),
                _ => PolicyPath::unix(rule.components),
            };
            let matching_cases = corpus
                .policy_roots
                .iter()
                .filter(|case| case.platform == platform)
                .filter(|case| {
                    let fixture_path = policy_path(policy_platform, &case.path);
                    fixture_path.is_exact(policy_platform, &production_path)
                        && protected_coverage(case.coverage) == rule.coverage
                        && protected_kind(case.kind) == rule.kind
                })
                .count();
            assert_eq!(
                matching_cases, 1,
                "production rule {:?} has {matching_cases} corpus matches",
                rule.components
            );
        }
    }

    for case in corpus.policy_roots {
        let platform = policy_platform(case.platform);
        let exact = policy_path(platform, &case.path);
        let mut descendant = exact.clone();
        descendant.components.push("dangerous-child".to_owned());
        let mut sibling = exact.clone();
        sibling.components[0].push_str("-lookalike");
        let kind = case.kind;
        let expected = match case.coverage {
            CorpusCoverage::Exact => (matched(ProtectionLevel::Denied, kind), None, None, None),
            CorpusCoverage::HardTree => (
                matched(ProtectionLevel::Denied, kind),
                matched(ProtectionLevel::Denied, kind),
                matched(ProtectionLevel::Denied, kind),
                matched(ProtectionLevel::Denied, kind),
            ),
            CorpusCoverage::GuardedTree => (
                matched(ProtectionLevel::Denied, kind),
                matched(ProtectionLevel::SpecificRuleRequired, kind),
                matched(ProtectionLevel::SpecificRuleRequired, kind),
                matched(ProtectionLevel::SpecificRuleRequired, kind),
            ),
        };
        assert_eq!(
            (
                classify_static(platform, MatchContext::Target, &exact),
                classify_static(platform, MatchContext::Target, &descendant),
                classify_static(platform, MatchContext::ScanScope, &exact),
                classify_static(platform, MatchContext::ScanScope, &descendant),
            ),
            expected,
            "{} has wrong exact/descendant target/scan coverage",
            case.id
        );
        assert_eq!(
            classify_static(platform, MatchContext::Target, &sibling),
            None,
            "{} matched a component-prefix sibling",
            case.id
        );
    }
}

#[test]
fn dynamic_home_and_guard_corpus_never_produces_positive_authority() {
    for case in corpus().dynamic_policy_cases {
        let platform = policy_platform(case.platform);
        let home = policy_path(platform, &case.home);
        let home_paths = vec![home];
        let policy = PlatformPolicy {
            platform,
            profile_containers: profile_parents(&home_paths),
            home_paths,
        };
        let path = policy_path(platform, &case.path);
        let observed = match case.context {
            CorpusContext::Target => policy.classify_target(&path),
            CorpusContext::ScanScope => policy.classify_scan_scope(&path),
        };
        let expected = match case.expected {
            CorpusDisposition::Denied => matched(
                ProtectionLevel::Denied,
                case.kind
                    .unwrap_or_else(|| panic!("{} is missing expected kind", case.id)),
            ),
            CorpusDisposition::SpecificRuleRequired => matched(
                ProtectionLevel::SpecificRuleRequired,
                case.kind
                    .unwrap_or_else(|| panic!("{} is missing expected kind", case.id)),
            ),
            CorpusDisposition::NoTextualMatch => {
                assert!(case.kind.is_none(), "{} has a kind for a miss", case.id);
                None
            }
        };
        assert_eq!(observed, expected, "{} has wrong disposition", case.id);
    }
}

#[test]
fn property_all_control_characters_and_dot_segments_are_rejected() {
    let (scan_text, separator) = if cfg!(windows) {
        (r"C:\fixture\root", '\\')
    } else {
        ("/fixture/root", '/')
    };
    let scan = validate_scan_root(Path::new(scan_text)).unwrap();

    for character in ('\u{0}'..='\u{1f}').chain('\u{7f}'..='\u{9f}') {
        let path = format!("{scan_text}{separator}before{character}after");
        assert!(matches!(
            validate_cleanup_path(&scan, Path::new(&path)),
            Err(LexicalPathError::ControlCharacter { .. })
        ));
    }
    for segment in [".", ".."] {
        for suffix in ["", "tail"] {
            let path = if suffix.is_empty() {
                format!("{scan_text}{separator}{segment}")
            } else {
                format!("{scan_text}{separator}{segment}{separator}{suffix}")
            };
            assert!(validate_cleanup_path(&scan, Path::new(&path)).is_err());
        }
    }
}

#[test]
fn property_generated_successes_remain_lossless_strict_normal_descendants() {
    let (scan_text, separator) = if cfg!(windows) {
        (r"C:\fixture\root", '\\')
    } else {
        ("/fixture/root", '/')
    };
    let scan = validate_scan_root(Path::new(scan_text)).unwrap();
    let atoms = [
        "a",
        "cache-01",
        "日本語",
        "🙂",
        "with space",
        "é",
        "e\u{301}",
    ];

    for first in atoms {
        for second in atoms {
            let path_text = format!("{scan_text}{separator}{first}{separator}{second}");
            let path = Path::new(&path_text);
            let evidence = validate_cleanup_path(&scan, path).unwrap();
            assert_eq!(evidence.as_path().as_os_str(), path.as_os_str());
            assert!(evidence.as_path().is_absolute());
            assert_ne!(evidence.as_path(), evidence.scan_root());
            assert!(!evidence.relative_to_scan_root().as_os_str().is_empty());
            assert!(
                evidence
                    .relative_to_scan_root()
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
            );
        }
    }
}

#[test]
fn property_encoded_length_limit_is_exact_in_host_native_units() {
    const MAX_UNITS: usize = 32 * 1024;

    #[cfg(unix)]
    {
        let exact_ascii = format!("/{}", "a".repeat(MAX_UNITS - 1));
        assert!(validate_scan_root(Path::new(&exact_ascii)).is_ok());
        let exact_multibyte = format!("/{}aaa", "🙂".repeat(8_191));
        assert_eq!(exact_multibyte.len(), MAX_UNITS);
        assert!(validate_scan_root(Path::new(&exact_multibyte)).is_ok());
        let too_long = format!("{exact_multibyte}b");
        assert!(matches!(
            validate_scan_root(Path::new(&too_long)),
            Err(LexicalPathError::TooLong { .. })
        ));
    }

    #[cfg(windows)]
    {
        let exact_ascii = format!(r"C:\{}", "a".repeat(MAX_UNITS - 3));
        assert!(validate_scan_root(Path::new(&exact_ascii)).is_ok());
        let exact_multibyte = format!(r"C:\{}a", "🙂".repeat(16_382));
        assert_eq!(exact_multibyte.encode_utf16().count(), MAX_UNITS);
        assert!(validate_scan_root(Path::new(&exact_multibyte)).is_ok());
        let too_long = format!("{exact_multibyte}b");
        assert!(matches!(
            validate_scan_root(Path::new(&too_long)),
            Err(LexicalPathError::TooLong { .. })
        ));
    }
}

#[test]
fn property_more_evidence_forms_can_never_weaken_a_protected_result() {
    let policy = PlatformPolicy {
        platform: PolicyPlatform::MacOs,
        home_paths: vec![PolicyPath::unix(&["Users", "alice"])],
        profile_containers: vec![PolicyPath::unix(&["Users"])],
    };
    let miss = PolicyPath::unix(&["Users", "alice", "Downloads", "file"]);
    let guarded = PolicyPath::unix(&["Users", "alice", "Library", "Caches"]);
    let denied = PolicyPath::unix(&["System", "Library"]);
    let forms = [
        ProtectedPathForm::ScanRootRequested,
        ProtectedPathForm::ScanRootCanonical,
        ProtectedPathForm::TargetRequested,
        ProtectedPathForm::TargetCanonical,
    ];

    for guarded_form in forms {
        for denied_form in forms {
            for paths in [
                vec![
                    (guarded_form, MatchContext::Target, &guarded),
                    (denied_form, MatchContext::Target, &denied),
                ],
                vec![
                    (denied_form, MatchContext::Target, &denied),
                    (guarded_form, MatchContext::Target, &guarded),
                    (
                        ProtectedPathForm::TargetRequested,
                        MatchContext::Target,
                        &miss,
                    ),
                ],
            ] {
                assert!(matches!(
                    policy.assess(&paths),
                    ProtectedRootDisposition::Denied { .. }
                ));
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn raw_unix_invalid_encoding_classes_reject_every_insertion_position() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let base = b"/fixture/root";
    let invalid_sequences: &[&[u8]] = &[
        &[0xff],             // impossible leading byte
        &[0x80],             // stray continuation byte
        &[0xc0, 0x80],       // overlong NUL
        &[0xe2, 0x82],       // truncated three-byte sequence
        &[0xed, 0xa0, 0x80], // UTF-8 encoding of a surrogate
    ];
    for sequence in invalid_sequences {
        for index in 0..=base.len() {
            let mut invalid = base.to_vec();
            invalid.splice(index..index, sequence.iter().copied());
            assert_eq!(
                validate_scan_root(Path::new(&OsString::from_vec(invalid))),
                Err(LexicalPathError::InvalidEncoding)
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn raw_windows_unpaired_utf16_corpus_rejects_every_insertion_position() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let base = r"C:\fixture\root".encode_utf16().collect::<Vec<_>>();
    for surrogate in [0xd800, 0xdc00] {
        for index in 0..=base.len() {
            let mut units = base.clone();
            units.insert(index, surrogate);
            assert_eq!(
                validate_scan_root(Path::new(&OsString::from_wide(&units))),
                Err(LexicalPathError::InvalidEncoding)
            );
        }
    }

    let valid_pair = r"C:\fixture\root\🙂".encode_utf16().collect::<Vec<_>>();
    assert!(validate_scan_root(Path::new(&OsString::from_wide(&valid_pair))).is_ok());
}
