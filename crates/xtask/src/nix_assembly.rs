//! Assemble a desktop release from prebuilt inputs (`cargo xtask assemble-nix`),
//! the release step of the Nix build. The flake builds every product from its
//! locked input, so this step builds nothing, reads no git repository and
//! applies no pins of its own. The commits it records are the locked inputs'
//! revisions; the digests are computed from the files given.
//!
//! cargo xtask assemble-nix --out=/ABS --built-at-utc=YYYY-MM-DDTHH:MM:SSZ \
//!   --release-id=ID --repo=/ABS --integration-commit=SHA --sophia-tree=/ABS \
//!   --sophia-rev=SHA --sophia=/ABS --factotum=/ABS --pam-helper=/ABS --xtask=/ABS \
//!   --preflight=/ABS --hagia=/ABS --hagia-commit=SHA --narthex=/ABS \
//!   --narthex-commit=SHA --profile=/ABS --c-sdk=/ABS --c-sdk-rev=SHA \
//!   [--verifier-interpreter=/ABS] [--file=DEST=/ABS ...]
//!
//! --release-id names the release directory under the install prefix; the
//! flake derives it from every locked input, and the rendered profile names
//! the release by it.
//!
//! --c-sdk is Hagia's vendored C SDK snapshot. It must be exactly
//! --c-sdk-rev, file for file, before its manifest is sealed in the release.
//!
//! --verifier-interpreter runs the release's packaged policy verifier through
//! that shell, for a build sandbox without /usr/bin/env; the shipped script
//! keeps its own interpreter line.
//!
//! --file=DEST=SRC adds one release file before the release is sealed: the
//! components and the rendered profile.

use std::path::{Path, PathBuf};

use crate::c_sdk_pin::{SNAPSHOT_MANIFEST, verify_vendored};
use crate::product_artifact::options;
use crate::release::{Assembly, Binaries, WmPair, assemble, valid_release_id, workspace_version};
use crate::{hex, read, sha256};

const USAGE: &str = "usage: cargo xtask assemble-nix --out=/ABS --built-at-utc=YYYY-MM-DDTHH:MM:SSZ \
--release-id=ID --repo=/ABS --integration-commit=SHA --sophia-tree=/ABS --sophia-rev=SHA \
--sophia=/ABS --factotum=/ABS --pam-helper=/ABS --xtask=/ABS --preflight=/ABS --hagia=/ABS \
--hagia-commit=SHA --narthex=/ABS --narthex-commit=SHA --profile=/ABS --c-sdk=/ABS \
--c-sdk-rev=SHA [--verifier-interpreter=/ABS] [--file=DEST=/ABS ...]";

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
    "c-sdk",
    "c-sdk-rev",
    "verifier-interpreter",
    "release-id",
];

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
    let release_id = get("release-id")?;
    if !valid_release_id(release_id) {
        return Err(format!(
            "--release-id must be 1-64 of [a-z0-9-], starting alphanumeric: {release_id:?}"
        ));
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
    let (hagia, narthex, profile) = (file("hagia")?, file("narthex")?, file("profile")?);
    let sdk = verify_vendored(&directory("c-sdk")?, get("c-sdk-rev")?)?;
    let pair = WmPair {
        hagia_sha256: sha256(&read(&hagia)?),
        narthex_sha256: sha256(&read(&narthex)?),
        profile_sha256: sha256(&read(&profile)?),
        hagia_commit: get("hagia-commit")?.to_owned(),
        narthex_commit: get("narthex-commit")?.to_owned(),
        hagia_c_sdk_revision: sdk.revision,
        hagia_c_sdk_manifest: directory("c-sdk")?.join(SNAPSHOT_MANIFEST),
        hagia_c_sdk_manifest_sha256: sdk.manifest_sha256,
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
        release_id: release_id.to_owned(),
    };
    assemble(&assembly)
}

fn absolute(key: &str, value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("--{key} must be absolute: {value}"));
    }
    Ok(path)
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
