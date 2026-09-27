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
//! A family whose directory does not exist (NotFound on the family root), or
//! that holds no runs, is reported absent and never fails. Every other error
//! fails with its path: an unreadable family root, a family path that is not a
//! directory, an unreadable entry, and any entry that is not a real directory.
//! SYMLINK POLICY: a run is a real directory; a symlinked (or dangling) entry
//! is refused, never followed, so the corpus cannot be redirected elsewhere.
//!
//! Sophia's commits are checked against the explicit SOPHIA_SOURCE repository
//! (and Hagia's and Narthex's against SOPHIA_HAGIA_ROOT and
//! SOPHIA_NARTHEX_ROOT), and this repository's against
//! SOPHIA_INTEGRATION_SOURCE, all of which the verifiers require; nothing is
//! inferred from this checkout. Archives written before the integration
//! binding verify only with `--legacy`, which is passed to every verifier and
//! reported per family as "integration identity unavailable".
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

/// The run directories of one family: `Ok(None)` only when the family root
/// does not exist; every other listing or metadata error, and every entry
/// that is not a real directory (symlinks included), is an error.
pub fn runs(directory: &Path) -> Result<Option<Vec<PathBuf>>, String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", directory.display())),
    };
    let mut runs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", directory.display()))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "{} is a symlink; promoted runs must be real directories",
                path.display()
            ));
        }
        if !meta.is_dir() {
            return Err(format!(
                "{} is not a directory; a promoted family holds only run directories",
                path.display()
            ));
        }
        runs.push(path);
    }
    runs.sort();
    Ok(Some(runs))
}

/// Re-verify every run of every family under `root` with this repository's
/// verifiers (in legacy mode when `legacy`); returns the summary line.
pub fn verify(repo: &Path, root: &Path, legacy: bool) -> Result<String, String> {
    let mut summary = Vec::new();
    let mut absent = Vec::new();
    for (family, tool) in FAMILIES {
        let runs = match runs(&root.join(family))? {
            Some(runs) if !runs.is_empty() => runs,
            _ => {
                absent.push(family);
                continue;
            }
        };
        let total = runs.len();
        let mut unavailable = 0;
        for run in runs {
            let mut command = Command::new(repo.join(tool));
            if legacy {
                command.arg("--legacy");
            }
            let output = bounded(
                command.arg(&run).current_dir(repo).stdin(Stdio::null()),
                VERIFY_TIMEOUT,
                tool,
            )
            .map_err(|error| {
                format!(
                    "promoted archive {} no longer verifies: {error}\nEither this change broke a verifier, or the archive was altered. Both are worth stopping for.",
                    run.display()
                )
            })?;
            if String::from_utf8_lossy(&output).contains("integration identity unavailable") {
                unavailable += 1;
            }
        }
        if unavailable == 0 {
            summary.push(format!("{family} {total}/{total}"));
        } else {
            summary.push(format!(
                "{family} {total}/{total} (legacy {unavailable}: integration identity unavailable)"
            ));
        }
    }
    if !absent.is_empty() {
        summary.push(format!("(absent: {})", absent.join(" ")));
    }
    Ok(format!("archives: {}", summary.join("  ")))
}

pub fn run(repo: &Path, arguments: &[String]) -> Result<Vec<String>, String> {
    let legacy = match arguments {
        [] => false,
        [flag] if flag == "--legacy" => true,
        _ => return Err("usage: cargo xtask verify-archives [--legacy]".into()),
    };
    let Some(root) = promotion_root() else {
        return Ok(vec!["archives: no state home, corpus skipped".to_owned()]);
    };
    Ok(vec![verify(repo, &root, legacy)?])
}
