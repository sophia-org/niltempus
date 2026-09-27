//! Loud shell-source slicing for tests that run a fragment of a launcher.
//!
//! A fragment that silently loses an opener (keeping a stray `fi`, say) would
//! run something other than what the test names. Every slice must therefore
//! have unique start and end markers, in that order, be non-empty, and parse
//! as a balanced bash program on its own (`bash -n`).

use std::process::Command;

pub fn slice<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    assert_eq!(
        source.matches(start).count(),
        1,
        "slice start marker is not unique: {start:?}"
    );
    assert_eq!(
        source.matches(end).count(),
        1,
        "slice end marker is not unique: {end:?}"
    );
    let a = source.find(start).unwrap();
    let b = source[a..]
        .find(end)
        .unwrap_or_else(|| panic!("slice end {end:?} does not follow start {start:?}"))
        + a;
    let fragment = &source[a..b];
    assert!(
        !fragment.trim().is_empty(),
        "empty slice between {start:?} and {end:?}"
    );
    let parsed = Command::new("bash")
        .args(["-n", "-c", fragment])
        .output()
        .expect("bash -n");
    assert!(
        parsed.status.success(),
        "unbalanced slice between {start:?} and {end:?}: {}\n{fragment}",
        String::from_utf8_lossy(&parsed.stderr)
    );
    fragment
}
