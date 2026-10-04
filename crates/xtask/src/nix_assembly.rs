//! Assemble a desktop release from prebuilt inputs (`cargo xtask assemble-nix`),
//! the release step of a Nix build. The Nix flake builds every product from
//! its locked input, and flake.lock pins those inputs, so this step builds
//! nothing, reads no git repository and applies no pins of its own. It lays
//! out exactly the release `package-desktop` lays out, through the same
//! `assemble`. The commits it records are the locked inputs' revisions; the
//! digests are computed from the files given.
//!
//! cargo xtask assemble-nix --out=/ABS --built-at-utc=YYYY-MM-DDTHH:MM:SSZ \
//!   --repo=/ABS --integration-commit=SHA --sophia-tree=/ABS --sophia-rev=SHA \
//!   --sophia=/ABS --factotum=/ABS --pam-helper=/ABS --xtask=/ABS --preflight=/ABS \
//!   --hagia=/ABS --hagia-commit=SHA --narthex=/ABS --narthex-commit=SHA \
//!   --profile=/ABS --c-sdk-manifest=/ABS --c-sdk-rev=SHA [--verifier-interpreter=/ABS] \
//!   [--release-id=ID] [--file=DEST=/ABS ...]
//!
//! --verifier-interpreter runs the release's packaged policy verifier through
//! that shell, for a build sandbox without /usr/bin/env; the shipped script
//! keeps its own interpreter line.
//!
//! --release-id names the release in place of the Sophia and integration
//! commits: the flake derives it from every locked input, and the rendered
//! profile names the release directory by it.

use std::path::{Path, PathBuf};

use crate::package_desktop::{Assembly, Binaries, assemble, workspace_version};
use crate::product_artifact::options;
use crate::wm_pair::VerifiedPair;
use crate::{hex, read, sha256};

const USAGE: &str = "usage: cargo xtask assemble-nix --out=/ABS --built-at-utc=YYYY-MM-DDTHH:MM:SSZ \
--repo=/ABS --integration-commit=SHA --sophia-tree=/ABS --sophia-rev=SHA --sophia=/ABS \
--factotum=/ABS --pam-helper=/ABS --xtask=/ABS --preflight=/ABS --hagia=/ABS --hagia-commit=SHA \
--narthex=/ABS --narthex-commit=SHA --profile=/ABS --c-sdk-manifest=/ABS --c-sdk-rev=SHA \
[--verifier-interpreter=/ABS] [--release-id=ID] [--file=DEST=/ABS ...]";

const KEYS: [&str; 20] = [
    "out",
    "built-at-utc",
    "repo",
    "integration-commit",
    "sophia-tree",
    "sophia-rev",
    "sophia",
    "factotum",
    "pam-helper",
    "xtask",
    "preflight",
    "hagia",
    "hagia-commit",
    "narthex",
    "narthex-commit",
    "profile",
    "c-sdk-manifest",
    "c-sdk-rev",
    "verifier-interpreter",
    "release-id",
];

/// Records what the Nix closure replaces: Hagia and narthex were built
/// from their locked flake inputs, not from a reviewed Nim manifest. The
/// release manifest does not carry it.
const NIX_CLOSURE: &str = "nix-flake-closure";

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    // --file=DEST=SRC may repeat: each adds one release file before sealing.
    let mut extra_files = Vec::new();
    let mut rest = Vec::new();
    for arg in args {
        match arg.strip_prefix("--file=") {
            Some(value) => {
                let (dest, source) = value
                    .split_once('=')
                    .ok_or_else(|| format!("--file takes DEST=SRC: {value:?}"))?;
                let source = absolute("file", source)?;
                if !std::fs::metadata(&source).is_ok_and(|m| m.is_file()) {
                    return Err(format!(
                        "--file source is not a regular file: {}",
                        source.display()
                    ));
                }
                extra_files.push((source, PathBuf::from(dest)));
            }
            None => rest.push(arg.clone()),
        }
    }
    let args = &rest;
    let options = options(args, &KEYS, USAGE)?;
    let verifier_interpreter = match options.get("verifier-interpreter") {
        Some(value) => {
            let path = absolute("verifier-interpreter", value)?;
            if !std::fs::metadata(&path).is_ok_and(|m| m.is_file()) {
                return Err(format!(
                    "--verifier-interpreter is not a regular file: {}",
                    path.display()
                ));
            }
            Some(path)
        }
        None => None,
    };
    let release_id = match options.get("release-id") {
        Some(id) if valid_release_id(id) => Some((*id).to_owned()),
        Some(id) => {
            return Err(format!(
                "--release-id must be 1-64 of [a-z0-9-], starting alphanumeric: {id:?}"
            ));
        }
        None => None,
    };
    let get = |key: &str| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| format!("--{key} is required; {USAGE}"))
    };
    for key in [
        "integration-commit",
        "sophia-rev",
        "hagia-commit",
        "narthex-commit",
        "c-sdk-rev",
    ] {
        if !hex(get(key)?, 40) {
            return Err(format!("--{key} must be 40 lowercase hex: {:?}", get(key)?));
        }
    }
    let built_at_utc = get("built-at-utc")?;
    if !utc_timestamp(built_at_utc) {
        return Err(format!(
            "--built-at-utc must be YYYY-MM-DDTHH:MM:SSZ: {built_at_utc:?}"
        ));
    }
    let directory = |key: &str| -> Result<PathBuf, String> {
        let path = absolute(key, get(key)?)?;
        if !std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
            return Err(format!("--{key} is not a directory: {}", path.display()));
        }
        Ok(path)
    };
    let file = |key: &str| -> Result<PathBuf, String> {
        let path = absolute(key, get(key)?)?;
        if !std::fs::metadata(&path).is_ok_and(|m| m.is_file()) {
            return Err(format!("--{key} is not a regular file: {}", path.display()));
        }
        Ok(path)
    };
    let out = absolute("out", get("out")?)?;
    if out.symlink_metadata().is_ok() {
        return Err(format!("--out already exists: {}", out.display()));
    }
    if !out.parent().is_some_and(Path::is_dir) {
        return Err(format!(
            "--out parent is not a directory: {}",
            out.display()
        ));
    }
    let sophia_tree = directory("sophia-tree")?;
    let (hagia, narthex, profile, sdk_manifest) = (
        file("hagia")?,
        file("narthex")?,
        file("profile")?,
        file("c-sdk-manifest")?,
    );
    let pair = VerifiedPair {
        dir: hagia.parent().unwrap_or(Path::new("/")).to_path_buf(),
        hagia_sha256: sha256(&read(&hagia)?),
        narthex_sha256: sha256(&read(&narthex)?),
        profile_sha256: sha256(&read(&profile)?),
        hagia_commit: get("hagia-commit")?.to_owned(),
        narthex_commit: get("narthex-commit")?.to_owned(),
        hagia_nim_deps_sha256: NIX_CLOSURE.to_owned(),
        narthex_nim_deps_sha256: NIX_CLOSURE.to_owned(),
        hagia_c_sdk_manifest_sha256: sha256(&read(&sdk_manifest)?),
        hagia_c_sdk_revision: get("c-sdk-rev")?.to_owned(),
        hagia_c_sdk_manifest: sdk_manifest,
        hagia,
        narthex,
        profile,
    };
    let assembly = Assembly {
        repo: directory("repo")?,
        sophia_version: workspace_version(&sophia_tree)?,
        sophia_tree,
        sophia_rev: get("sophia-rev")?.to_owned(),
        integration_commit: get("integration-commit")?.to_owned(),
        binaries: Binaries {
            sophia: file("sophia")?,
            xtask: file("xtask")?,
            preflight: file("preflight")?,
            factotum: file("factotum")?,
            pam_helper: file("pam-helper")?,
        },
        pair,
        out,
        built_at_utc: built_at_utc.to_owned(),
        verifier_interpreter,
        extra_files,
        release_id,
    };
    // `assemble` checks the C SDK manifest names --c-sdk-rev and runs the
    // packaged policy verifier, exactly as for package-desktop.
    assemble(&assembly)
}

fn absolute(key: &str, value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("--{key} must be absolute: {value}"));
    }
    Ok(path)
}

/// A release ID is one path component under the install prefix.
fn valid_release_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `YYYY-MM-DDTHH:MM:SSZ`, digits where digits belong. A Nix build passes
/// the time its sources fix (SOURCE_DATE_EPOCH), so the release is
/// reproducible.
fn utc_timestamp(value: &str) -> bool {
    value.len() == 20
        && value.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            10 => b == b'T',
            13 | 16 => b == b':',
            19 => b == b'Z',
            _ => b.is_ascii_digit(),
        })
}
