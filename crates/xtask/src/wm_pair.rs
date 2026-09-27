//! Prepare and verify the immutable Hagia/Narthex window-manager pair that a
//! desktop release packages (`cargo xtask prepare-wm-pair`).
//!
//! Each half follows the product-artifact custody rules: SOURCE AUTHORIZATION
//! (`git verify-commit`, status G) happens only here, the build input is the
//! signed commit's exact tree (`git archive`, tree hash proven), and the build
//! runs low-priority, two jobs, bounded, in a private scratch tree. Nothing is
//! read from a checkout's working tree and no sibling checkout is consulted:
//! both repositories and commits are explicit. The canonical default profile
//! comes from Hagia's signed tree. The output directory is created last and
//! made read-only.
//!
//! ARTIFACT BINDING is `verify`'s separate job: the manifest is not signed, so
//! the packager requires the operator's expected commits and binary digests
//! and re-derives every identity from the files themselves.
//!
//! The pair is for explicit packaging and proofs only. Preparing or packaging
//! it never touches the user's own default window manager
//! ($XDG_STATE_HOME/sophia/bin/hagia and its reload workflow).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::bemenu_artifact::{commit_tree, inputs, set_mode};
use crate::product_artifact::{Built, build, product};
use crate::{hex, read, sha256};

const USAGE: &str = "usage: cargo xtask prepare-wm-pair --hagia <repo> <signed-commit> \
                     --narthex <repo> <signed-commit> <new-output-dir>";
pub const MANIFEST: &str = "wm-pair.manifest";
pub const PROFILE: &str = "default.kdl";
/// Hagia's canonical default profile, in its signed tree.
pub const PROFILE_SOURCE: &str = "examples/config/default.kdl";
pub const HAGIA: &str = "hagia";
pub const NARTHEX: &str = "narthex";
const HAGIA_COMMIT: &str = "hagia.commit";
const NARTHEX_COMMIT: &str = "narthex.commit";
const KEYS: [&str; 12] = [
    "schema",
    "hagia_source_commit",
    "hagia_source_tree",
    "hagia_signer_fingerprint",
    "hagia_binary_sha256",
    "narthex_source_commit",
    "narthex_source_tree",
    "narthex_signer_fingerprint",
    "narthex_binary_sha256",
    "default_profile",
    "default_profile_source",
    "default_profile_sha256",
];

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    let [
        flag_h,
        hagia_repo,
        hagia_commit,
        flag_n,
        narthex_repo,
        narthex_commit,
        output,
    ] = args
    else {
        return Err(USAGE.into());
    };
    if flag_h != "--hagia" || flag_n != "--narthex" {
        return Err(USAGE.into());
    }
    let (hagia_source, output) = inputs(hagia_repo, hagia_commit, output)?;
    let (narthex_source, _) = inputs(narthex_repo, narthex_commit, &output.to_string_lossy())?;

    let hagia = build(product(HAGIA)?, &hagia_source, hagia_commit)?;
    let profile_path = hagia.tree.tree_dir.join(PROFILE_SOURCE);
    if !std::fs::symlink_metadata(&profile_path).is_ok_and(|m| m.is_file()) {
        return Err(format!(
            "hagia {hagia_commit} has no regular {PROFILE_SOURCE}"
        ));
    }
    let profile = read(&profile_path)?;
    let narthex = build(product(NARTHEX)?, &narthex_source, narthex_commit)?;

    // Immutable output: created last, removed again if any step fails.
    std::fs::create_dir(&output).map_err(|e| format!("{}: {e}", output.display()))?;
    let written = write_pair(
        &output,
        &hagia,
        &narthex,
        &profile,
        hagia_commit,
        narthex_commit,
    );
    match written {
        Ok(line) => Ok(vec![line]),
        Err(error) => {
            let _ = set_mode(&output, 0o700);
            let _ = std::fs::remove_dir_all(&output);
            Err(error)
        }
    }
}

/// Fill the new output directory; returns the prepared-pair summary line.
fn write_pair(
    output: &Path,
    hagia: &Built,
    narthex: &Built,
    profile: &[u8],
    hagia_commit: &str,
    narthex_commit: &str,
) -> Result<String, String> {
    let mut digests = Vec::new();
    for (name, binary, raw, commit_file) in [
        (HAGIA, &hagia.binary, &hagia.tree.raw, HAGIA_COMMIT),
        (NARTHEX, &narthex.binary, &narthex.tree.raw, NARTHEX_COMMIT),
    ] {
        let copy = output.join(name);
        std::fs::copy(binary, &copy).map_err(|e| format!("copy {name}: {e}"))?;
        digests.push(sha256(&read(&copy)?));
        std::fs::write(output.join(commit_file), raw).map_err(|e| e.to_string())?;
        set_mode(&copy, 0o555)?;
        set_mode(&output.join(commit_file), 0o444)?;
    }
    std::fs::write(output.join(PROFILE), profile).map_err(|e| e.to_string())?;
    set_mode(&output.join(PROFILE), 0o444)?;
    let manifest = [
        "schema=1".to_owned(),
        format!("hagia_source_commit={hagia_commit}"),
        format!("hagia_source_tree={}", hagia.tree.tree),
        format!("hagia_signer_fingerprint={}", hagia.tree.signer),
        format!("hagia_binary_sha256={}", digests[0]),
        format!("narthex_source_commit={narthex_commit}"),
        format!("narthex_source_tree={}", narthex.tree.tree),
        format!("narthex_signer_fingerprint={}", narthex.tree.signer),
        format!("narthex_binary_sha256={}", digests[1]),
        format!("default_profile={PROFILE}"),
        format!("default_profile_source={PROFILE_SOURCE}"),
        format!("default_profile_sha256={}", sha256(profile)),
    ]
    .join("\n")
        + "\n";
    std::fs::write(output.join(MANIFEST), &manifest).map_err(|e| e.to_string())?;
    set_mode(&output.join(MANIFEST), 0o444)?;
    set_mode(output, 0o555)?;
    Ok(format!(
        "wm_pair status=prepared hagia_commit={hagia_commit} hagia_sha256={} \
             narthex_commit={narthex_commit} narthex_sha256={} default_profile_sha256={} dir={}",
        digests[0],
        digests[1],
        sha256(profile),
        output.display()
    ))
}

/// A pair whose files were bound to the operator's expected identities.
#[derive(Debug, Clone)]
pub struct VerifiedPair {
    pub dir: PathBuf,
    pub hagia: PathBuf,
    pub narthex: PathBuf,
    pub profile: PathBuf,
    pub hagia_commit: String,
    pub narthex_commit: String,
    pub hagia_sha256: String,
    pub narthex_sha256: String,
    pub profile_sha256: String,
}

/// Bind a prepared pair to the operator's expected commits and binary digests
/// (hagia first). Every identity is re-derived from the files: the raw commit
/// objects must hash to the commits, the binaries to the digests, and the
/// unsigned manifest must agree with all of them.
pub fn verify(dir: &Path, commits: [&str; 2], digests: [&str; 2]) -> Result<VerifiedPair, String> {
    if !dir.is_absolute() {
        return Err(format!(
            "WM pair must be an absolute directory: {}",
            dir.display()
        ));
    }
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return Err(format!("WM pair is not a directory: {}", dir.display()));
    }
    for value in commits {
        if !hex(value, 40) {
            return Err(format!(
                "WM pair commit must be 40 lowercase hex: {value:?}"
            ));
        }
    }
    for value in digests {
        if !hex(value, 64) {
            return Err(format!(
                "WM pair SHA-256 must be 64 lowercase hex: {value:?}"
            ));
        }
    }
    let regular = |name: &str| -> Result<PathBuf, String> {
        let path = dir.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            Ok(path)
        } else {
            Err(format!("WM pair has no regular {name}"))
        }
    };
    let manifest_text = String::from_utf8(read(&regular(MANIFEST)?)?).map_err(|e| e.to_string())?;
    let manifest = parse_manifest(&manifest_text)?;
    let mut derived = Vec::new();
    for (index, (name, commit_file)) in [(HAGIA, HAGIA_COMMIT), (NARTHEX, NARTHEX_COMMIT)]
        .into_iter()
        .enumerate()
    {
        let raw = read(&regular(commit_file)?)?;
        let tree = commit_tree(&raw)?;
        crate::git_tree::verify_commit(&raw, commits[index], &tree)
            .map_err(|e| format!("{name} commit mismatch: {e}"))?;
        let binary = regular(name)?;
        let digest = sha256(&read(&binary)?);
        if digest != digests[index] {
            return Err(format!("{name} binary SHA-256 is not the expected one"));
        }
        for (key, expected) in [
            (format!("{name}_source_commit"), commits[index].to_owned()),
            (format!("{name}_source_tree"), tree),
            (format!("{name}_binary_sha256"), digest.clone()),
        ] {
            if manifest.get(key.as_str()) != Some(&expected) {
                return Err(format!("WM pair manifest {key} is not the bound value"));
            }
        }
        derived.push((binary, digest));
    }
    let profile = regular(PROFILE)?;
    let profile_sha256 = sha256(&read(&profile)?);
    for (key, expected) in [
        ("schema", "1"),
        ("default_profile", PROFILE),
        ("default_profile_source", PROFILE_SOURCE),
        ("default_profile_sha256", profile_sha256.as_str()),
    ] {
        if manifest.get(key).map(String::as_str) != Some(expected) {
            return Err(format!("WM pair manifest {key} is not the bound value"));
        }
    }
    let (narthex, narthex_sha256) = derived.pop().expect("two halves");
    let (hagia, hagia_sha256) = derived.pop().expect("two halves");
    Ok(VerifiedPair {
        dir: dir.to_path_buf(),
        hagia,
        narthex,
        profile,
        hagia_commit: commits[0].to_owned(),
        narthex_commit: commits[1].to_owned(),
        hagia_sha256,
        narthex_sha256,
        profile_sha256,
    })
}

/// The manifest's exact key set, each key once.
fn parse_manifest(text: &str) -> Result<BTreeMap<&str, String>, String> {
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("WM pair manifest line is not key=value: {line:?}"))?;
        if !KEYS.contains(&key) {
            return Err(format!("WM pair manifest has an unknown key: {key}"));
        }
        if fields.insert(key, value.to_owned()).is_some() {
            return Err(format!("WM pair manifest repeats {key}"));
        }
    }
    if fields.len() != KEYS.len() {
        return Err("WM pair manifest is missing fields".into());
    }
    Ok(fields)
}
