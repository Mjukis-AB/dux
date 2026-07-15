//! Filesystem-free invariant oracle for the isolated cargo-fuzz target.

use std::path::{Component, Path};

use super::{protected, validate_cleanup_path, validate_scan_root};

const MAX_FUZZ_INPUT_UNITS: usize = 64 * 1024;

pub(crate) fn exercise(input: &[u8]) {
    let input = &input[..input.len().min(MAX_FUZZ_INPUT_UNITS)];
    let (tag, payload) = input.split_first().unwrap_or((&b'L', &[]));
    match *tag {
        b'L' => exercise_host_lexical(payload),
        // The checked-in seed for this tag reaches traversal validation even
        // though text fixtures conventionally end with a newline.
        b'T' => exercise_host_lexical(b"../protected"),
        b'S' | b'G' | b'P' | b'A' => protected::assert_fuzz_invariants(input),
        _ => {
            exercise_host_lexical(input);
            protected::assert_fuzz_invariants(input);
        }
    }
}

#[cfg(unix)]
fn exercise_host_lexical(input: &[u8]) {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    const SCAN_ROOT: &[u8] = b"/dux-fuzz-root";
    let scan_path = Path::new(std::ffi::OsStr::from_bytes(SCAN_ROOT));
    let scan = validate_scan_root(scan_path).expect("fixed fuzz scan root is valid");
    let mut candidate = Vec::with_capacity(SCAN_ROOT.len() + 1 + input.len());
    candidate.extend_from_slice(SCAN_ROOT);
    candidate.push(b'/');
    candidate.extend_from_slice(input);
    let candidate = OsString::from_vec(candidate);
    assert_accepted_lexical_invariants(&scan, Path::new(&candidate));
}

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[cfg(windows)]
fn exercise_host_lexical(input: &[u8]) {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let scan_text = r"C:\dux-fuzz-root";
    let scan = validate_scan_root(Path::new(scan_text)).expect("fixed fuzz scan root is valid");
    let mut units = scan_text.encode_utf16().collect::<Vec<_>>();
    units.push(u16::from(b'\\'));
    units.extend(
        input
            .chunks(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk.get(1).copied().unwrap_or_default()])),
    );
    let candidate = OsString::from_wide(&units);
    assert_accepted_lexical_invariants(&scan, Path::new(&candidate));
}

#[cfg(not(any(unix, windows)))]
fn exercise_host_lexical(_input: &[u8]) {}

fn assert_accepted_lexical_invariants(scan: &super::LexicalScanRoot, candidate: &Path) {
    let Ok(evidence) = validate_cleanup_path(scan, candidate) else {
        return;
    };
    assert_eq!(evidence.as_path().as_os_str(), candidate.as_os_str());
    assert!(evidence.as_path().is_absolute());
    assert_ne!(evidence.as_path(), evidence.scan_root());
    assert!(!evidence.relative_to_scan_root().as_os_str().is_empty());
    assert!(
        evidence
            .relative_to_scan_root()
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    );
    #[cfg(unix)]
    assert!(std::str::from_utf8(evidence.as_path().as_os_str().as_bytes()).is_ok());
    #[cfg(windows)]
    assert!(evidence.as_path().to_str().is_some());
}
