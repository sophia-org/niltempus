// Provenance: ported from Sophia crates/xtask/src/check.rs (`archives`) at
// de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). The
// hagia-native and mirror-group families moved here with their shell
// verifiers; Sophia keeps the direct-scanout family and its Rust verifier.
//! Promoted archives, re-verified as a regression corpus
//! (`cargo xtask verify-archives`).
//!
//! These are the only real-hardware evidence the physical gates produce, and
//! the verifiers that read them are the code most likely to rot silently: a
//! reader that stops matching still returns Ok on a synthetic fixture built
//! from the same assumption it just broke. Re-verifying the archives is how a
//! broken reader is caught by a machine rather than by a burned TTY.
//!
//! Absent families are reported and never fail. Sophia's commits are checked
//! against the explicit SOPHIA_SOURCE repository (and Hagia's and Narthex's
//! against SOPHIA_HAGIA_ROOT and SOPHIA_NARTHEX_ROOT), which the verifiers
//! require; nothing is inferred from this checkout.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::bemenu_artifact::bounded;

/// (state-directory family, this repository's verifier)
pub const FAMILIES: [(&str, &str); 2] = [
    (
        "hagia-native-runs",
        "tools/verify_hagia_native_session_archive.sh",
    ),
    (
        "mirror-group-runs",
        "tools/verify_mirror_group_physical_archive.sh",
    ),
];
const VERIFY_TIMEOUT: Duration = Duration::from_secs(300);

/// `$XDG_STATE_HOME/sophia/promotion`, or `$HOME/.local/state/...`.
pub fn promotion_root() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/state"))
        })?;
    Some(state.join("sophia/promotion"))
}

/// Re-verify every run of every family under `root` with this repository's
/// verifiers; returns the summary line.
pub fn verify(repo: &Path, root: &Path) -> Result<String, String> {
    let mut summary = Vec::new();
    let mut absent = Vec::new();
    for (family, tool) in FAMILIES {
        let directory = root.join(family);
        let Ok(entries) = std::fs::read_dir(&directory) else {
            absent.push(family);
            continue;
        };
        let mut runs = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        runs.sort();
        if runs.is_empty() {
            absent.push(family);
            continue;
        }
        let total = runs.len();
        for run in runs {
            bounded(
                Command::new(repo.join(tool))
                    .arg(&run)
                    .current_dir(repo)
                    .stdin(Stdio::null()),
                VERIFY_TIMEOUT,
                tool,
            )
            .map_err(|error| {
                format!(
                    "promoted archive {} no longer verifies: {error}\nEither this change broke a verifier, or the archive was altered. Both are worth stopping for.",
                    run.display()
                )
            })?;
        }
        summary.push(format!("{family} {total}/{total}"));
    }
    if !absent.is_empty() {
        summary.push(format!("(absent: {})", absent.join(" ")));
    }
    Ok(format!("archives: {}", summary.join("  ")))
}

pub fn run(repo: &Path, arguments: &[String]) -> Result<Vec<String>, String> {
    if !arguments.is_empty() {
        return Err("usage: cargo xtask verify-archives".into());
    }
    let Some(root) = promotion_root() else {
        return Ok(vec!["archives: no state home, corpus skipped".to_owned()]);
    };
    Ok(vec![verify(repo, &root)?])
}
