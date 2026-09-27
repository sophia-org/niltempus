//! `xtask verify-release DIR --c-sdk-rev=<40 lowercase hex>`: read-only
//! verification of a sealed desktop release, for the installer to call
//! instead of duplicating the schema-7 rules.
//!
//! It checks:
//! - the sealed contents against `SHA256SUMS`: every regular file under
//!   `bin`, `share`, `target` and `tools` is listed with its digest, and
//!   every listed file exists; nothing else may sit at the top level except
//!   `manifest`, `SHA256SUMS` and the installer's `desktop-manifest.json`;
//! - one well-formed `release_id`;
//! - the C SDK binding, through the same schema-7 rule packaging applies
//!   (`package_desktop::check_release_sdk`) against the supplied revision.
//!
//! It fails closed, only reads, never follows a symlink inside the release,
//! and never executes anything from the candidate (not its verifier and not
//! its scripts). It runs before the workspace-root lookup, so the installed
//! binary works with its checkout gone.
//!
//! Passing grants nothing. It does not authorize activation and confers no
//! activation-ledger history. Operator input binding, the plan checks and
//! the activation/rollback ledger stay with the installer.
use std::collections::BTreeMap;
use std::path::{Component, Path};

use crate::{hex, read, sha256};

const USAGE: &str = "usage: xtask verify-release /ABS/RELEASE-DIR --c-sdk-rev=<40 lowercase hex>";
/// The sealed trees SHA256SUMS covers.
const SEALED: [&str; 4] = ["bin", "share", "target", "tools"];
/// Top-level files that may sit beside the sealed trees. The installer's
/// own reseal lists `manifest` too, so a listed `manifest` is accepted.
const UNSEALED: [&str; 3] = ["manifest", "SHA256SUMS", "desktop-manifest.json"];

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    let (dir, revision) = match args {
        [dir, option] => (
            dir.as_str(),
            option
                .strip_prefix("--c-sdk-rev=")
                .ok_or_else(|| USAGE.to_owned())?,
        ),
        _ => return Err(USAGE.into()),
    };
    if !hex(revision, 40) {
        return Err(format!(
            "--c-sdk-rev must be 40 lowercase hex: {revision:?}"
        ));
    }
    let release = Path::new(dir);
    if !release.is_absolute() {
        return Err(format!("release directory must be absolute: {dir}"));
    }
    if !std::fs::symlink_metadata(release).is_ok_and(|m| m.is_dir()) {
        return Err(format!(
            "release is not a directory (symlinks refused): {dir}"
        ));
    }
    let files = verify_sums(release)?;
    let release_id = release_id(release)?;
    let (revision, digest) =
        crate::package_desktop::check_release_sdk(release, "the supplied", revision, None)?;
    Ok(vec![format!(
        "release_verification schema=1 status=pass release_id={release_id} c_sdk_revision={revision} c_sdk_manifest_sha256={digest} sealed_files={files}"
    )])
}

/// The release contents against SHA256SUMS; returns the number of sealed
/// files.
fn verify_sums(release: &Path) -> Result<usize, String> {
    for entry in std::fs::read_dir(release).map_err(|e| format!("{}: {e}", release.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF-8 release entry")?;
        let meta = std::fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        let allowed = (SEALED.contains(&name.as_str()) && meta.is_dir())
            || (UNSEALED.contains(&name.as_str()) && meta.is_file());
        if !allowed {
            return Err(format!("unexpected release entry: {name}"));
        }
    }
    for name in ["manifest", "SHA256SUMS"] {
        if !std::fs::symlink_metadata(release.join(name)).is_ok_and(|m| m.is_file()) {
            return Err(format!("release has no regular {name}"));
        }
    }
    let sums = String::from_utf8(read(&release.join("SHA256SUMS"))?)
        .map_err(|_| "SHA256SUMS is not UTF-8".to_owned())?;
    let mut listed = BTreeMap::new();
    for line in sums.lines() {
        let (digest, path) = line
            .split_once("  ")
            .ok_or_else(|| format!("malformed SHA256SUMS line: {line:?}"))?;
        let safe = !path.is_empty()
            && !path.contains(['\\', '\r'])
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
        if !hex(digest, 64) || !safe {
            return Err(format!("malformed SHA256SUMS line: {line:?}"));
        }
        if listed.insert(path.to_owned(), digest.to_owned()).is_some() {
            return Err(format!("SHA256SUMS repeats {path}"));
        }
    }
    let mut actual = BTreeMap::new();
    for top in SEALED {
        let dir = release.join(top);
        if dir.exists() {
            collect(release, &dir, &mut actual)?;
        }
    }
    let sealed = actual.len();
    for (path, digest) in &listed {
        match actual.remove(path) {
            Some(real) if &real == digest => {}
            Some(_) => return Err(format!("{path} does not match SHA256SUMS")),
            None if path == "manifest" => {
                let real = sha256(&read(&release.join("manifest"))?);
                if &real != digest {
                    return Err("manifest does not match SHA256SUMS".into());
                }
            }
            None => return Err(format!("SHA256SUMS lists a missing file: {path}")),
        }
    }
    if let Some(path) = actual.keys().next() {
        return Err(format!("release file is not in SHA256SUMS: {path}"));
    }
    Ok(sealed)
}

/// Every regular file below `dir`, by its path relative to `root`, with its
/// digest. Symlinks and other non-regular entries are refused.
fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            collect(root, &path, out)?;
        } else if meta.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("non-UTF-8 release path")?
                .to_owned();
            out.insert(relative, sha256(&read(&path)?));
        } else {
            return Err(format!(
                "release contains a non-regular entry: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn release_id(release: &Path) -> Result<String, String> {
    let text = String::from_utf8(read(&release.join("manifest"))?)
        .map_err(|_| "release manifest is not UTF-8".to_owned())?;
    let ids = text
        .lines()
        .filter_map(|line| line.strip_prefix("release_id="))
        .collect::<Vec<_>>();
    match ids[..] {
        [id] if !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) =>
        {
            Ok(id.to_owned())
        }
        [] => Err("release manifest has no release_id".into()),
        _ => Err("release manifest has an invalid or repeated release_id".into()),
    }
}
