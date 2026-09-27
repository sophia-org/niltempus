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
//! made read-only. Both halves go through the one corrected builder
//! (product_artifact::build): a private `--build-dir`, and each half's
//! REVIEWED Nim dependency manifest with its independently supplied sha256,
//! copied into the pair and bound by the pair manifest (schema 2).
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
use crate::nim_deps::{Manifest, NIM_SYSTEM_CFG};
use crate::product_artifact::{
    Built, TOOLCHAIN_NOTE, build, build_dir, nim_deps_option, options, product,
};
use crate::{hex, read, sha256};

const USAGE: &str = "usage: cargo xtask prepare-wm-pair --hagia <repo> <signed-commit> \
                     --narthex <repo> <signed-commit> <new-output-dir> --build-dir=/ABS \
                     --hagia-nim-deps=/ABS --hagia-nim-deps-sha256=<64 hex> \
                     --narthex-nim-deps=/ABS --narthex-nim-deps-sha256=<64 hex>";
const OPTIONS: [&str; 5] = [
    "build-dir",
    "hagia-nim-deps",
    "hagia-nim-deps-sha256",
    "narthex-nim-deps",
    "narthex-nim-deps-sha256",
];
pub const MANIFEST: &str = "wm-pair.manifest";
pub const PROFILE: &str = "default.kdl";
/// Hagia's canonical default profile, in its signed tree.
pub const PROFILE_SOURCE: &str = "examples/config/default.kdl";
pub const HAGIA: &str = "hagia";
pub const NARTHEX: &str = "narthex";
const HAGIA_COMMIT: &str = "hagia.commit";
const NARTHEX_COMMIT: &str = "narthex.commit";
const HAGIA_DEPS: &str = "hagia-nim-deps.manifest";
const NARTHEX_DEPS: &str = "narthex-nim-deps.manifest";
pub const SCHEMA: &str = "2";
const KEYS: [&str; 16] = [
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
    "hagia_nim_deps_sha256",
    "narthex_nim_deps_sha256",
    "nim_system_cfg",
    "toolchain_identity",
];

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    if args.len() < 7 {
        return Err(USAGE.into());
    }
    let (head, rest) = args.split_at(7);
    let [
        flag_h,
        hagia_repo,
        hagia_commit,
        flag_n,
        narthex_repo,
        narthex_commit,
        output,
    ] = head
    else {
        return Err(USAGE.into());
    };
    if flag_h != "--hagia" || flag_n != "--narthex" {
        return Err(USAGE.into());
    }
    let (hagia_source, output) = inputs(hagia_repo, hagia_commit, output)?;
    let (narthex_source, _) = inputs(narthex_repo, narthex_commit, &output.to_string_lossy())?;
    let options = options(rest, &OPTIONS, USAGE)?;
    let build_dir = build_dir(options.get("build-dir"))?;
    let required = |prefix: &str| {
        nim_deps_option(&options, prefix)?.ok_or_else(|| {
            format!("--{prefix}nim-deps and --{prefix}nim-deps-sha256 are required; {USAGE}")
        })
    };
    let (hagia_deps, narthex_deps) = (required("hagia-")?, required("narthex-")?);

    let hagia = build(
        product(HAGIA)?,
        &hagia_source,
        hagia_commit,
        &build_dir,
        Some(hagia_deps),
    )?;
    let profile_path = hagia.tree.tree_dir.join(PROFILE_SOURCE);
    if !std::fs::symlink_metadata(&profile_path).is_ok_and(|m| m.is_file()) {
        return Err(format!(
            "hagia {hagia_commit} has no regular {PROFILE_SOURCE}"
        ));
    }
    let profile = read(&profile_path)?;
    let narthex = build(
        product(NARTHEX)?,
        &narthex_source,
        narthex_commit,
        &build_dir,
        Some(narthex_deps),
    )?;

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
    let mut deps = Vec::new();
    for (name, built, commit_file, deps_file) in [
        (HAGIA, hagia, HAGIA_COMMIT, HAGIA_DEPS),
        (NARTHEX, narthex, NARTHEX_COMMIT, NARTHEX_DEPS),
    ] {
        let copy = output.join(name);
        std::fs::copy(&built.binary, &copy).map_err(|e| format!("copy {name}: {e}"))?;
        digests.push(sha256(&read(&copy)?));
        std::fs::write(output.join(commit_file), &built.tree.raw).map_err(|e| e.to_string())?;
        set_mode(&copy, 0o555)?;
        set_mode(&output.join(commit_file), 0o444)?;
        let reviewed = built
            .nim_deps
            .as_ref()
            .ok_or_else(|| format!("{name} was built without its reviewed dependencies"))?;
        std::fs::write(output.join(deps_file), &reviewed.text).map_err(|e| e.to_string())?;
        set_mode(&output.join(deps_file), 0o444)?;
        deps.push(reviewed.sha256.clone());
    }
    std::fs::write(output.join(PROFILE), profile).map_err(|e| e.to_string())?;
    set_mode(&output.join(PROFILE), 0o444)?;
    let manifest = [
        format!("schema={SCHEMA}"),
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
        format!("hagia_nim_deps_sha256={}", deps[0]),
        format!("narthex_nim_deps_sha256={}", deps[1]),
        format!("nim_system_cfg={}", NIM_SYSTEM_CFG.as_str()),
        format!("toolchain_identity={TOOLCHAIN_NOTE}"),
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
    /// The reviewed dependency manifests the halves were built from.
    pub hagia_nim_deps_sha256: String,
    pub narthex_nim_deps_sha256: String,
}

/// Bind a prepared pair to the operator's expected commits and binary digests
/// (hagia first) and the expected default-profile digest. Every identity is
/// re-derived from the files: the raw commit objects must hash to the
/// commits, the binaries and the profile to the operator's digests, and the
/// unsigned manifest must agree with all of them. The manifest alone binds
/// nothing: replacing a file together with its manifest hash is refused.
pub fn verify(
    dir: &Path,
    commits: [&str; 2],
    digests: [&str; 2],
    profile_digest: &str,
) -> Result<VerifiedPair, String> {
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
    for value in digests.into_iter().chain([profile_digest]) {
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
        // The reviewed dependency manifest this half was built from: its
        // bytes hash to the bound value, and it was reviewed for this commit.
        let deps_file = if name == HAGIA {
            HAGIA_DEPS
        } else {
            NARTHEX_DEPS
        };
        let deps_bytes = read(&regular(deps_file)?)?;
        let deps_key = format!("{name}_nim_deps_sha256");
        let deps_sha256 = sha256(&deps_bytes);
        if manifest.get(deps_key.as_str()) != Some(&deps_sha256) {
            return Err(format!(
                "WM pair manifest {deps_key} is not the bound value"
            ));
        }
        let deps = Manifest::parse(
            &String::from_utf8(deps_bytes).map_err(|_| format!("{deps_file} is not UTF-8"))?,
        )?;
        if deps.status != "reviewed" || deps.product != name || deps.source_commit != commits[index]
        {
            return Err(format!(
                "{deps_file} is not {name}'s reviewed dependency manifest"
            ));
        }
        derived.push((binary, digest, deps_sha256));
    }
    let profile = regular(PROFILE)?;
    let profile_sha256 = sha256(&read(&profile)?);
    if profile_sha256 != profile_digest {
        return Err(format!("{PROFILE} SHA-256 is not the expected one"));
    }
    for (key, expected) in [
        ("schema", SCHEMA),
        ("toolchain_identity", TOOLCHAIN_NOTE),
        ("default_profile", PROFILE),
        ("default_profile_source", PROFILE_SOURCE),
        ("default_profile_sha256", profile_sha256.as_str()),
    ] {
        if manifest.get(key).map(String::as_str) != Some(expected) {
            return Err(format!("WM pair manifest {key} is not the bound value"));
        }
    }
    if !["kept-hashed", "skipped"].contains(&manifest["nim_system_cfg"].as_str()) {
        return Err("WM pair manifest nim_system_cfg is not a known mode".into());
    }
    let (narthex, narthex_sha256, narthex_nim_deps_sha256) = derived.pop().expect("two halves");
    let (hagia, hagia_sha256, hagia_nim_deps_sha256) = derived.pop().expect("two halves");
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
        hagia_nim_deps_sha256,
        narthex_nim_deps_sha256,
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
