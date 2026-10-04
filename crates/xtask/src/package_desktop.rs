// Provenance: ported from Sophia tools/package_live_session.sh at
// a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at
// the pin de776c68) (Sophia rule 13). The release layout, the schema-6
// manifest fields and SHA256SUMS are kept; every input is now explicit.
// Schema 7 (the director's release ruling) adds Hagia's vendored C SDK
// revision and manifest digest, with the manifest sealed in the release.
//! Package an immutable desktop release (`cargo xtask package-desktop`), the
//! E4 entry point:
//!
//! ```text
//! cargo xtask package-desktop --sophia-root=/ABS --sophia-rev=<pin> \
//!     --wm-pair=/ABS --wm-pair-commits=<hagia>,<narthex> \
//!     --wm-pair-sha256=<hagia>,<narthex> --wm-pair-profile-sha256=<sha> \
//!     --wm-pair-c-sdk-rev=<rev> --build-dir=/ABS --out=/ABS/NEW
//! ```
//!
//! Custody:
//! - Sophia is the explicit clean checkout whose HEAD is the pinned revision
//!   (pins/sophia.toml), signed (status G). Its exact tree is staged with
//!   `git archive` (tree hash proven) and built there into the private
//!   build directory; the release's retained generic session files come from
//!   that staged tree, never from a checkout.
//! - The Hagia/Narthex pair is a `prepare-wm-pair` directory bound to the
//!   operator's expected commits, binary digests and default-profile digest,
//!   and its recorded Hagia C SDK revision must equal the operator's
//!   `--wm-pair-c-sdk-rev` (no default).
//! - The provisioning marker is validated in full (canonical URL, pinned
//!   revision, Cargo.lock digest, and CARGO_HOME equal to the provisioned
//!   home) before anything is staged or built.
//! - This repository must be clean with a signed HEAD, and it is used only
//!   through that commit's exact staged tree (tracked files, tree-hash
//!   proven): the recipe tool and host checker are built there (offline,
//!   from the provisioned CARGO_HOME) and every packaged file is copied from
//!   there, never from the mutable checkout. Both staged trees are
//!   re-verified after the builds and before assembly.
//! - Every build writes only below the private build directory; the output
//!   is new and created last.
//!
//! The release never switches or overwrites the user's default window
//! manager. The installed session may still prefer the user's own policy
//! client at $XDG_STATE_HOME/sophia/bin/hagia (the reload workflow): the
//! packaged pair is the fallback and the promotion profile, not a
//! replacement, and nothing here writes the user's state.
use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::bemenu_artifact::{
    SignedTree, archive_tree, authorize, bounded, set_mode, tail, text, wait_logged,
};
use crate::wm_pair::{VerifiedPair, verify};
use crate::{hex, pins, read, sha256};

const USAGE: &str = "usage: cargo xtask package-desktop --sophia-root=/ABS --sophia-rev=SHA \
                     --wm-pair=/ABS --wm-pair-commits=HAGIA,NARTHEX \
                     --wm-pair-sha256=HAGIA,NARTHEX --wm-pair-profile-sha256=SHA \
                     --wm-pair-c-sdk-rev=REV --build-dir=/ABS --out=/ABS/NEW";
const OPTIONS: [&str; 9] = [
    "sophia-root",
    "sophia-rev",
    "wm-pair",
    "wm-pair-commits",
    "wm-pair-sha256",
    "wm-pair-profile-sha256",
    "wm-pair-c-sdk-rev",
    "build-dir",
    "out",
];
/// The release manifest schema. 7 requires Hagia's vendored C SDK revision
/// and manifest digest; a schema-6 release is not a candidate here.
pub const RELEASE_SCHEMA: &str = "7";
/// Hagia's vendored C SDK manifest, sealed in the release (SHA256SUMS).
pub const RELEASE_C_SDK_MANIFEST: &str = "share/sophia-policy/hagia/c-sdk.manifest.json";
const BUILD_TIMEOUT: Duration = Duration::from_secs(5400);
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Sophia's retained generic session files, from the staged pinned tree.
pub const SOPHIA_RETAINED: [(&str, u32); 5] = [
    ("tools/run_sophia_session.sh", 0o755),
    ("tools/stop_sophia_session.sh", 0o755),
    ("tools/sophia_tty_mode.py", 0o755),
    ("tools/lib/session_lifecycle.sh", 0o644),
    ("tools/lib/session_preparation.sh", 0o644),
];

/// Operator commands: (bin name, this repository's source).
pub const COMMANDS: [(&str, &str); 46] = [
    ("sophia-session", "tools/installed/sophia-session"),
    (
        "sophia-niltempus-desktop-session",
        "tools/installed/sophia-niltempus-desktop-session",
    ),
    (
        "sophia-hagia-session",
        "tools/installed/sophia-hagia-session",
    ),
    (
        "sophia-hagia-xtest-session",
        "tools/installed/sophia-hagia-xtest-session",
    ),
    (
        "sophia-hagia-promotion-session",
        "tools/installed/sophia-hagia-promotion-session",
    ),
    (
        "sophia-kitty-session",
        "tools/installed/sophia-kitty-session",
    ),
    (
        "sophia-firefox-proof",
        "tools/installed/sophia-firefox-proof",
    ),
    ("sophia-xterm-proof", "tools/installed/sophia-xterm-proof"),
    (
        "sophia-truecolor-proof",
        "tools/installed/sophia-truecolor-proof",
    ),
    (
        "sophia-recovery-proof",
        "tools/installed/sophia-recovery-proof",
    ),
    (
        "sophia-native-chrome-proof",
        "tools/installed/sophia-native-chrome-proof",
    ),
    (
        "capture-runtime-identity",
        "tools/installed/capture-runtime-identity.sh",
    ),
    ("sophia-setup-uinput", "tools/setup_sophia_uinput.sh"),
    ("sophia-status", "tools/status_live_session.sh"),
    ("sophia-stop", "tools/installed/sophia-stop"),
    ("sophia-rollback", "tools/rollback_live_session.sh"),
    (
        "sophia-record-firefox-attempt",
        "tools/record_installed_firefox_attempt.sh",
    ),
    (
        "sophia-record-xterm-run",
        "tools/record_installed_xterm_run.sh",
    ),
    (
        "sophia-record-truecolor-run",
        "tools/record_installed_truecolor_run.sh",
    ),
    (
        "sophia-record-fallback-run",
        "tools/record_installed_fallback_run.sh",
    ),
    (
        "sophia-record-emergency-run",
        "tools/record_installed_emergency_run.sh",
    ),
    (
        "sophia-record-watchdog-run",
        "tools/record_installed_watchdog_run.sh",
    ),
    (
        "sophia-record-native-chrome-run",
        "tools/record_installed_native_chrome_run.sh",
    ),
    (
        "sophia-record-hagia-run",
        "tools/record_installed_hagia_run.sh",
    ),
    (
        "sophia-verify-login-cycle",
        "tools/verify_installed_login_cycle.sh",
    ),
    (
        "sophia-verify-xterm-run",
        "tools/verify_installed_xterm_session.sh",
    ),
    (
        "sophia-verify-xterm-runs",
        "tools/verify_installed_xterm_runs.sh",
    ),
    (
        "sophia-verify-truecolor-run",
        "tools/verify_installed_truecolor_session.sh",
    ),
    (
        "sophia-verify-truecolor-runs",
        "tools/verify_installed_truecolor_runs.sh",
    ),
    (
        "sophia-verify-fallback-session",
        "tools/verify_installed_fallback_session.sh",
    ),
    (
        "sophia-verify-fallback",
        "tools/verify_installed_fallback_run.sh",
    ),
    (
        "sophia-verify-emergency",
        "tools/verify_installed_emergency_archive.sh",
    ),
    (
        "sophia-verify-runtime-identity",
        "tools/verify_installed_runtime_identity.sh",
    ),
    (
        "sophia-verify-hagia-session",
        "tools/verify_installed_hagia_session.sh",
    ),
    (
        "sophia-verify-hagia-recovery",
        "tools/verify_installed_hagia_recovery.sh",
    ),
    (
        "sophia-verify-hagia",
        "tools/verify_installed_hagia_archive.sh",
    ),
    (
        "sophia-verify-hagia-promotion",
        "tools/verify_installed_hagia_archive.sh",
    ),
    (
        "sophia-verify-lifecycle",
        "tools/verify_installed_session_lifecycle.sh",
    ),
    (
        "sophia-verify-watchdog-run",
        "tools/verify_installed_watchdog_recovery.sh",
    ),
    (
        "sophia-verify-watchdog",
        "tools/verify_installed_watchdog_archive.sh",
    ),
    (
        "sophia-verify-native-chrome-core",
        "tools/verify_sophia_native_chrome.sh",
    ),
    (
        "sophia-verify-native-chrome-session",
        "tools/verify_installed_native_chrome_session.sh",
    ),
    (
        "sophia-verify-native-chrome",
        "tools/verify_installed_native_chrome_archive.sh",
    ),
    (
        "sophia-verify-firefox-run",
        "tools/verify_sophia_firefox_physical.sh",
    ),
    (
        "sophia-record-firefox-run",
        "tools/record_sophia_firefox_physical_run.sh",
    ),
    (
        "sophia-verify-firefox-runs",
        "tools/verify_sophia_firefox_physical_runs.sh",
    ),
];

/// This repository's files shipped under tools/ at the same relative path.
pub const TOOLS: [(&str, u32); 18] = [
    ("tools/session/run_desktop_session.sh", 0o755),
    ("tools/start_sophia_native_hot_reload_tty3.sh", 0o755),
    ("tools/verify_packaged_policy.sh", 0o755),
    ("tools/verify_sophia_firefox_rendering_physical.sh", 0o755),
    ("tools/probes/uinput_text_injector.py", 0o755),
    ("tools/config/proof_helpers.sh", 0o644),
    ("tools/config/99-sophia-uinput.rules", 0o644),
    ("tools/config/sophia-uinput.conf", 0o644),
    ("tools/lib/activation_ledger.sh", 0o644),
    ("tools/lib/installed_attempt_ledger.sh", 0o644),
    ("tools/lib/installed_hagia_evidence.sh", 0o644),
    ("tools/lib/live_session_surface.sh", 0o644),
    ("tools/lib/verify_firefox_rendering.awk", 0o644),
    ("tools/fixtures/firefox_m8_local_page.html", 0o644),
    ("tools/fixtures/firefox_m10_kitty_probe.sh", 0o755),
    ("tools/fixtures/firefox_m10_primary_kitty_probe.sh", 0o755),
    ("tools/fixtures/firefox_m10_selection_kitty_probe.sh", 0o755),
    ("tools/fixtures/truecolor_kitty_probe.sh", 0o755),
];

/// Login-menu entries: (file stem, Name, Comment, command).
pub const SESSIONS: [(&str, &str, &str, &str); 8] = [
    (
        "sophia-niltempus-desktop",
        "Sophia niltempus Desktop",
        "Hagia, Lom and Bemenu over 9P2000.L",
        "sophia-niltempus-desktop-session",
    ),
    (
        "sophia-hagia",
        "Sophia Hagia (Native Policy)",
        "Bounded Sophia native public-policy profile",
        "sophia-hagia-session",
    ),
    (
        "sophia-hagia-promotion",
        "Sophia Hagia Promotion (Packaged Default)",
        "Immutable Hagia packaged-default promotion profile",
        "sophia-hagia-promotion-session",
    ),
    (
        "sophia-hagia-xtest",
        "Sophia Hagia (XTEST automation)",
        "Hagia with synthetic input admitted; drives scenarios, accepts none",
        "sophia-hagia-xtest-session",
    ),
    (
        "sophia-kitty",
        "Sophia Kitty (Baseline)",
        "Sophia proven Kitty-only physical input baseline",
        "sophia-kitty-session",
    ),
    (
        "sophia-firefox-proof",
        "Sophia Firefox Proof",
        "Sophia installed physical Firefox promotion workflow",
        "sophia-firefox-proof",
    ),
    (
        "sophia-recovery-proof",
        "Sophia Recovery Proof",
        "Bounded installed session and automatic display-manager recovery",
        "sophia-recovery-proof",
    ),
    (
        "sophia-native-chrome-proof",
        "Sophia Native Chrome Proof",
        "Installed ring, frame, and combined chrome proof",
        "sophia-native-chrome-proof",
    ),
];

pub const OPERATIONS_DOC: &str = "docs/operations.md";

/// Prebuilt executables that the release seals under target/release/.
#[derive(Debug, Clone)]
pub struct Binaries {
    pub sophia: PathBuf,
    pub xtask: PathBuf,
    pub preflight: PathBuf,
    /// The session lock's authentication agent and its PAM helper, from the
    /// pinned Sophia tree. The release prefix is root's alone, which the
    /// agent requires of the helper before it starts.
    pub factotum: PathBuf,
    pub pam_helper: PathBuf,
}

/// Everything `assemble` needs, already verified.
#[derive(Debug, Clone)]
pub struct Assembly {
    /// This repository's staged signed tree (wrappers, adapter, fixtures,
    /// docs); never the mutable checkout.
    pub repo: PathBuf,
    /// The staged pinned Sophia tree (retained generic session files).
    pub sophia_tree: PathBuf,
    pub sophia_rev: String,
    pub sophia_version: String,
    pub integration_commit: String,
    pub binaries: Binaries,
    pub pair: VerifiedPair,
    pub out: PathBuf,
    pub built_at_utc: String,
    /// Runs the packaged policy verifier through this interpreter instead
    /// of its `#!/usr/bin/env bash` line, for builds (Nix) whose sandbox has
    /// no /usr/bin/env. The shipped script is never rewritten.
    pub verifier_interpreter: Option<PathBuf>,
    /// Further release files (source, release-relative destination), laid
    /// out before the release is checked and sealed: the components and the
    /// rendered profile a Nix build adds. Executable sources are installed
    /// 0755, others 0644.
    pub extra_files: Vec<(PathBuf, PathBuf)>,
    /// The release ID, when the caller binds more inputs than Sophia and
    /// this repository (a Nix build binds every locked input).
    pub release_id: Option<String>,
}

pub fn run(repo: &Path, args: &[String]) -> Result<Vec<String>, String> {
    run_with(repo, args, std::env::var("CARGO_HOME").ok().as_deref())
}

/// `run` with the caller's CARGO_HOME passed explicitly (tests).
pub fn run_with(
    repo: &Path,
    args: &[String],
    cargo_home: Option<&str>,
) -> Result<Vec<String>, String> {
    let options = parse(args)?;
    let get = |key: &str| options[key].as_str();
    for key in ["sophia-root", "wm-pair", "build-dir", "out"] {
        if !Path::new(get(key)).is_absolute() {
            return Err(format!("--{key} must be absolute: {}", get(key)));
        }
    }
    let rev = get("sophia-rev");
    if rev != pins::SOPHIA_REV {
        return Err(format!(
            "--sophia-rev {rev} is not the pinned revision {}",
            pins::SOPHIA_REV
        ));
    }
    let sdk_rev = get("wm-pair-c-sdk-rev");
    if !hex(sdk_rev, 40) {
        return Err(format!(
            "--wm-pair-c-sdk-rev must be 40 lowercase hex: {sdk_rev:?}"
        ));
    }
    let sophia_root = canonical(get("sophia-root"))?;
    let repo = canonical(&repo.to_string_lossy())?;
    let build_dir = PathBuf::from(get("build-dir"));
    let out = PathBuf::from(get("out"));
    for (key, path) in [("build-dir", &build_dir), ("out", &out)] {
        let resolved = resolve_lexically(path);
        for tree in [&repo, &sophia_root] {
            if resolved.starts_with(tree) || tree.starts_with(&resolved) {
                return Err(format!(
                    "--{key} must be outside every source tree: {}",
                    path.display()
                ));
            }
        }
    }
    if sophia_root.starts_with(&repo) || repo.starts_with(&sophia_root) {
        return Err("--sophia-root must not contain or lie inside this repository".into());
    }
    private_dir(&build_dir)?;
    if out.symlink_metadata().is_ok() {
        return Err(format!("--out already exists: {}", out.display()));
    }
    if !out.parent().is_some_and(Path::is_dir) {
        return Err(format!(
            "--out parent is not a directory: {}",
            out.display()
        ));
    }
    let pair = verify(
        Path::new(get("wm-pair")),
        pair_values(get("wm-pair-commits"), "--wm-pair-commits")?,
        pair_values(get("wm-pair-sha256"), "--wm-pair-sha256")?,
        get("wm-pair-profile-sha256"),
    )?;
    if pair.hagia_c_sdk_revision != sdk_rev {
        return Err(format!(
            "WM pair Hagia C SDK revision {} is not --wm-pair-c-sdk-rev {sdk_rev}",
            pair.hagia_c_sdk_revision
        ));
    }
    // The full provisioning marker (URL, pin, lock digest, home) before any
    // staging or build.
    let (cargo_home, lock_sha256) = provisioned(&repo, cargo_home)?;
    clean_checkout(&sophia_root, "Sophia checkout")?;
    let head = git_text(&sophia_root, &["rev-parse", "--verify", "HEAD"])?;
    if head.trim() != rev {
        return Err(format!(
            "Sophia HEAD {} is not the pinned revision {rev}",
            head.trim()
        ));
    }
    clean_checkout(&repo, "Integration repository")?;
    let integration_commit = git_text(&repo, &["rev-parse", "--verify", "HEAD"])?
        .trim()
        .to_owned();

    // Both inputs are the exact signed trees (authorization plus tree-hash
    // proof); the mutable checkouts are never read again.
    let integration = StagedTree::signed(&repo, &integration_commit, "integration")?;
    if sha256(&read(&integration.dir().join("Cargo.lock"))?) != lock_sha256 {
        return Err("the signed integration tree's Cargo.lock is not the provisioned lock".into());
    }
    let sophia = StagedTree::signed(&sophia_root, rev, "sophia")?;
    let sophia_version = workspace_version(sophia.dir())?;
    let sophia_target = build_dir.join("sophia-target");
    cargo(
        sophia.dir(),
        &sophia_target,
        None,
        &[
            "-p",
            "sophia-cli",
            "--features",
            "sophia-cli/native-session",
            "-p",
            "sophia-factotum",
            "-p",
            "sophia-factotum-pam",
        ],
        &build_dir.join("sophia-build.log"),
        "build Sophia",
    )?;
    let integration_target = build_dir.join("integration-target");
    cargo(
        integration.dir(),
        &integration_target,
        Some(cargo_home.as_path()),
        &["-p", "xtask", "--bins"],
        &build_dir.join("integration-build.log"),
        "build the recipe tool and host checker",
    )?;
    // The builds must not have changed either staged input.
    sophia.reverify()?;
    integration.reverify()?;
    let assembly = Assembly {
        repo: integration.dir().to_path_buf(),
        sophia_tree: sophia.dir().to_path_buf(),
        sophia_rev: rev.to_owned(),
        sophia_version,
        integration_commit,
        binaries: Binaries {
            sophia: sophia_target.join("release/sophia"),
            xtask: integration_target.join("release/xtask"),
            preflight: integration_target.join("release/active-session-preflight"),
            factotum: sophia_target.join("release/sophia-factotum"),
            pam_helper: sophia_target.join("release/sophia-factotum-pam"),
        },
        pair,
        out,
        built_at_utc: utc_now()?,
        verifier_interpreter: None,
        release_id: None,
        extra_files: Vec::new(),
    };
    assemble(&assembly)
}

fn parse(args: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut options = BTreeMap::new();
    for arg in args {
        let (key, value) = arg
            .strip_prefix("--")
            .and_then(|a| a.split_once('='))
            .ok_or_else(|| format!("unexpected argument {arg:?}; {USAGE}"))?;
        if !OPTIONS.contains(&key) {
            return Err(format!("unknown option --{key}; {USAGE}"));
        }
        if value.is_empty() {
            return Err(format!("--{key} is empty; {USAGE}"));
        }
        if options.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(format!("--{key} is repeated"));
        }
    }
    for key in OPTIONS {
        if !options.contains_key(key) {
            return Err(format!("--{key} is required (no default); {USAGE}"));
        }
    }
    Ok(options)
}

fn pair_values<'a>(value: &'a str, what: &str) -> Result<[&'a str; 2], String> {
    match value.split(',').collect::<Vec<_>>()[..] {
        [hagia, narthex] => Ok([hagia, narthex]),
        _ => Err(format!("{what} must be HAGIA,NARTHEX")),
    }
}

fn canonical(path: &str) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))
}

/// Absolute path with `.`/`..` removed, without following links.
pub(crate) fn resolve_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    // Resolve the deepest existing ancestor so symlinked parents compare
    // against the real source trees.
    let mut existing = out.clone();
    let mut rest = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => rest.push(name.to_owned()),
            None => break,
        }
        existing.pop();
    }
    let mut resolved = std::fs::canonicalize(&existing).unwrap_or(existing);
    for name in rest.into_iter().rev() {
        resolved.push(name);
    }
    resolved
}

/// An existing directory owned by this user, not a link, private (0700).
pub(crate) fn private_dir(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("--build-dir {}: {e}", path.display()))?;
    if !meta.is_dir()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(format!(
            "--build-dir must be a private (0700) directory owned by this user: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn git_text(repo: &Path, args: &[&str]) -> Result<String, String> {
    text(bounded(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0"),
        GIT_TIMEOUT,
        &format!("git {}", args.first().copied().unwrap_or("")),
    )?)
}

pub(crate) fn clean_checkout(repo: &Path, what: &str) -> Result<(), String> {
    let status = git_text(repo, &["status", "--porcelain", "--untracked-files=normal"])
        .map_err(|e| format!("{what} {} is not a readable checkout: {e}", repo.display()))?;
    if !status.is_empty() {
        return Err(format!("{what} must be clean: {}", repo.display()));
    }
    Ok(())
}

/// The full provisioning marker check (pins::check_marker: canonical URL,
/// pinned revision, this checkout's Cargo.lock digest) plus the caller's
/// CARGO_HOME, which must be the provisioned one. Returns (home, lock digest).
pub(crate) fn provisioned(
    repo: &Path,
    cargo_home: Option<&str>,
) -> Result<(PathBuf, String), String> {
    let marker =
        String::from_utf8(read(&repo.join(pins::PROVISION_MARKER))?).map_err(|e| e.to_string())?;
    let lock = sha256(&read(&repo.join("Cargo.lock"))?);
    let home = cargo_home
        .filter(|home| home.starts_with('/'))
        .ok_or("CARGO_HOME must name the provisioned private CARGO_HOME")?;
    pins::check_marker(&marker, &lock, Some(home))?;
    Ok((PathBuf::from(home), lock))
}

/// An exact commit's tree staged in a private scratch directory: tracked
/// files only, hashing to the commit's tree. Removed when dropped.
pub struct StagedTree(SignedTree);

impl StagedTree {
    /// Signer authorization (verify-commit, status G), then the exact tree.
    pub fn signed(source: &Path, commit: &str, label: &str) -> Result<Self, String> {
        let signer = authorize(source, commit)?;
        Ok(Self(archive_tree(source, commit, label, signer)?))
    }

    /// The exact tree WITHOUT signer authorization: tests only, and any
    /// caller that authorized the commit itself.
    pub fn unauthorized(source: &Path, commit: &str, label: &str) -> Result<Self, String> {
        Ok(Self(archive_tree(source, commit, label, String::new())?))
    }

    pub fn dir(&self) -> &Path {
        &self.0.tree_dir
    }

    /// The staged tree still hashes to exactly the commit's tree.
    pub fn reverify(&self) -> Result<(), String> {
        let inventory = crate::git_tree::inventory(&self.0.tree_dir)?;
        if inventory.tree != self.0.tree {
            return Err(format!(
                "the staged tree {} changed after staging",
                self.0.tree_dir.display()
            ));
        }
        Ok(())
    }
}

pub(crate) fn workspace_version(tree: &Path) -> Result<String, String> {
    let manifest = String::from_utf8(read(&tree.join("Cargo.toml"))?).map_err(|e| e.to_string())?;
    manifest
        .lines()
        .find_map(|line| {
            line.strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))
        })
        .filter(|v| {
            !v.is_empty()
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        })
        .map(str::to_owned)
        .ok_or_else(|| "could not resolve Sophia's workspace version".into())
}

/// Offline, locked release build at the caller's priority with
/// [`crate::product_artifact::build_jobs`] jobs, in a private process group and bounded by time and log size.
pub(crate) fn cargo(
    dir: &Path,
    target: &Path,
    cargo_home: Option<&Path>,
    packages: &[&str],
    log: &Path,
    what: &str,
) -> Result<(), String> {
    let log_file = File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let jobs = crate::product_artifact::build_jobs()?;
    let mut command = Command::new("cargo");
    command
        .args(["build", "--offline", "--locked", "--release"])
        .args(["--jobs", &jobs])
        .args(packages)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_BUILD_JOBS", &jobs)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|e| e.to_string())?)
        .stderr(log_file);
    if let Some(home) = cargo_home {
        command.env("CARGO_HOME", home);
    }
    let child = command.spawn().map_err(|e| format!("{what}: {e}"))?;
    wait_logged(child, log, BUILD_TIMEOUT, what).map_err(|e| format!("{e}\n{}", tail(log)))
}

fn utc_now() -> Result<String, String> {
    Ok(text(bounded(
        Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
        GIT_TIMEOUT,
        "date",
    )?)?
    .trim()
    .to_owned())
}

/// The release id: Sophia version, then the Sophia and integration commits.
pub fn release_id(assembly: &Assembly) -> String {
    if let Some(id) = &assembly.release_id {
        return id.clone();
    }
    format!(
        "{}-{}-{}",
        assembly.sophia_version,
        &assembly.sophia_rev[..12],
        &assembly.integration_commit[..12]
    )
}

/// Lay out the schema-7 release in the new `out` directory, check it with
/// the packaged policy verifier and seal it with SHA256SUMS. On any failure
/// the partial output is removed.
pub fn assemble(assembly: &Assembly) -> Result<Vec<String>, String> {
    let out = &assembly.out;
    if !hex(&assembly.sophia_rev, 40) || !hex(&assembly.integration_commit, 40) {
        return Err("release commits must be 40 lowercase hex".into());
    }
    std::fs::DirBuilder::new()
        .mode(0o755)
        .create(out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    match lay_out(assembly) {
        Ok(()) => Ok(vec![format!(
            "desktop_release status=packaged release_id={} sophia_commit={} integration_commit={} \
             hagia_commit={} narthex_commit={} dir={}",
            release_id(assembly),
            assembly.sophia_rev,
            assembly.integration_commit,
            assembly.pair.hagia_commit,
            assembly.pair.narthex_commit,
            out.display()
        )]),
        Err(error) => {
            let _ = std::fs::remove_dir_all(out);
            Err(error)
        }
    }
}

fn install(source: &Path, dest: &Path, mode: u32) -> Result<(), String> {
    if !std::fs::symlink_metadata(source).is_ok_and(|m| m.is_file()) {
        return Err(format!(
            "release input is not a regular file: {}",
            source.display()
        ));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        set_mode(parent, 0o755)?;
    }
    std::fs::copy(source, dest).map_err(|e| format!("copy {}: {e}", source.display()))?;
    set_mode(dest, mode)
}

fn lay_out(a: &Assembly) -> Result<(), String> {
    let out = &a.out;
    for dir in [
        "bin",
        "target/release",
        "tools",
        "share/doc/sophia",
        "share/sophia-policy/hagia",
        "share/wayland-sessions",
    ] {
        std::fs::create_dir_all(out.join(dir)).map_err(|e| e.to_string())?;
    }
    let release = out.join("target/release");
    for (source, name) in [
        (&a.binaries.sophia, "sophia"),
        (&a.binaries.xtask, "sophia-integration-xtask"),
        (&a.binaries.preflight, "active-session-preflight"),
        (&a.binaries.factotum, "sophia-factotum"),
        (&a.binaries.pam_helper, "sophia-factotum-pam"),
        (&a.pair.hagia, "hagia"),
        (&a.pair.narthex, "narthex"),
    ] {
        install(source, &release.join(name), 0o755)?;
    }
    for (name, source) in COMMANDS {
        install(&a.repo.join(source), &out.join("bin").join(name), 0o755)?;
    }
    for (path, mode) in TOOLS {
        install(&a.repo.join(path), &out.join(path), mode)?;
    }
    for (path, mode) in SOPHIA_RETAINED {
        install(&a.sophia_tree.join(path), &out.join(path), mode)?;
    }
    install(
        &a.repo.join(OPERATIONS_DOC),
        &out.join("share/doc/sophia/operations.md"),
        0o644,
    )?;
    install(
        &a.pair.profile,
        &out.join("share/sophia-policy/hagia/default.kdl"),
        0o644,
    )?;
    install(
        &a.pair.hagia_c_sdk_manifest,
        &out.join(RELEASE_C_SDK_MANIFEST),
        0o644,
    )?;
    for (stem, name, comment, command) in SESSIONS {
        let entry = format!(
            "[Desktop Entry]\nName={name}\nComment={comment}\n\
             Exec=@SOPHIA_INSTALL_PREFIX@/current/bin/{command}\nType=Application\nDesktopNames=Sophia\n"
        );
        let path = out
            .join("share/wayland-sessions")
            .join(format!("{stem}.desktop"));
        std::fs::write(&path, entry).map_err(|e| e.to_string())?;
        set_mode(&path, 0o644)?;
    }
    for (source, dest) in &a.extra_files {
        if dest.is_absolute()
            || dest
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "release file path must be relative: {}",
                dest.display()
            ));
        }
        let target = out.join(dest);
        if target.symlink_metadata().is_ok() {
            return Err(format!("release file already laid out: {}", dest.display()));
        }
        let executable = std::fs::metadata(source)
            .map_err(|e| format!("{}: {e}", source.display()))?
            .mode()
            & 0o111
            != 0;
        install(source, &target, if executable { 0o755 } else { 0o644 })?;
    }
    let manifest = manifest(a);
    std::fs::write(out.join("manifest"), manifest).map_err(|e| e.to_string())?;
    set_mode(&out.join("manifest"), 0o644)?;
    verify_release_sdk(out, &a.pair)?;

    let script = out.join("tools/verify_packaged_policy.sh");
    let mut command = match &a.verifier_interpreter {
        Some(interpreter) => {
            let mut command = Command::new(interpreter);
            command.arg(&script);
            command
        }
        None => Command::new(&script),
    };
    let verifier = bounded(
        command.arg(out),
        Duration::from_secs(120),
        "packaged policy verification",
    );
    verifier.map_err(|e| format!("packaged policy verification failed: {e}"))?;

    let mut files = Vec::new();
    for top in ["bin", "share", "target", "tools"] {
        collect_files(out, &out.join(top), &mut files)?;
    }
    files.sort();
    let mut sums = String::new();
    for relative in files {
        sums.push_str(&format!(
            "{}  {relative}\n",
            sha256(&read(&out.join(&relative))?)
        ));
    }
    std::fs::write(out.join("SHA256SUMS"), sums).map_err(|e| e.to_string())?;
    set_mode(&out.join("SHA256SUMS"), 0o644)
}

/// The schema-7 manifest: the retained release fields, then the integration
/// and WM-pair identities, then Hagia's vendored C SDK.
pub fn manifest(a: &Assembly) -> String {
    [
        format!("schema={RELEASE_SCHEMA}"),
        format!("version={}", a.sophia_version),
        format!("commit={}", a.sophia_rev),
        format!("release_id={}", release_id(a)),
        format!("built_at_utc={}", a.built_at_utc),
        "hagia_included=true".to_owned(),
        format!("hagia_source_commit={}", a.pair.hagia_commit),
        format!("hagia_default_profile_sha256={}", a.pair.profile_sha256),
        format!("hagia_binary_sha256={}", a.pair.hagia_sha256),
        format!("hagia_shell_binary_sha256={}", a.pair.narthex_sha256),
        format!("narthex_source_commit={}", a.pair.narthex_commit),
        format!("integration_commit={}", a.integration_commit),
        format!("hagia_c_sdk_revision={}", a.pair.hagia_c_sdk_revision),
        format!(
            "hagia_c_sdk_manifest_sha256={}",
            a.pair.hagia_c_sdk_manifest_sha256
        ),
    ]
    .join("\n")
        + "\n"
}

/// Cross-check a laid-out release's Hagia C SDK binding against the verified
/// pair, with no defaults: the manifest is schema 7 and carries each field
/// exactly once and well formed, equal to the pair's value; the sealed SDK
/// manifest is a regular file hashing to the recorded digest and naming the
/// recorded revision.
pub fn verify_release_sdk(release: &Path, pair: &VerifiedPair) -> Result<(), String> {
    check_release_sdk(
        release,
        "the pair's",
        &pair.hagia_c_sdk_revision,
        Some(&pair.hagia_c_sdk_manifest_sha256),
    )
    .map(|_| ())
}

/// The schema-7 C SDK binding rule, shared by packaging (against the
/// verified pair) and `xtask verify-release` (against the supplied
/// revision). Reads only; returns the recorded revision and digest.
///
/// The manifest is schema 7 with `hagia_included=true`, and carries each SDK
/// field exactly once and well formed. The revision equals `expected_revision`
/// (and the digest `expected_digest`, when given). The sealed SDK manifest is
/// a regular file that hashes to the recorded digest and names the recorded
/// revision.
pub fn check_release_sdk(
    release: &Path,
    expected_from: &str,
    expected_revision: &str,
    expected_digest: Option<&str>,
) -> Result<(String, String), String> {
    let text = String::from_utf8(read(&release.join("manifest"))?)
        .map_err(|_| "release manifest is not UTF-8".to_owned())?;
    let values = |key: &str| {
        text.lines()
            .filter_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .collect::<Vec<_>>()
    };
    let one = |key: &str| match values(key)[..] {
        [value] => Ok(value),
        [] => Err(format!("release manifest has no {key}")),
        _ => Err(format!("release manifest repeats {key}")),
    };
    if one("schema")? != RELEASE_SCHEMA {
        return Err(format!("release manifest is not schema {RELEASE_SCHEMA}"));
    }
    if one("hagia_included")? != "true" {
        return Err("release does not include Hagia, so it has no C SDK binding".into());
    }
    let revision = one("hagia_c_sdk_revision")?;
    let digest = one("hagia_c_sdk_manifest_sha256")?;
    if !hex(revision, 40) {
        return Err(format!(
            "release hagia_c_sdk_revision is malformed: {revision:?}"
        ));
    }
    if !hex(digest, 64) {
        return Err(format!(
            "release hagia_c_sdk_manifest_sha256 is malformed: {digest:?}"
        ));
    }
    if revision != expected_revision {
        return Err(format!(
            "release hagia_c_sdk_revision {revision} is not {expected_from} {expected_revision}"
        ));
    }
    if let Some(expected) = expected_digest
        && digest != expected
    {
        return Err(format!(
            "release hagia_c_sdk_manifest_sha256 {digest} is not {expected_from} {expected}"
        ));
    }
    let sealed = release.join(RELEASE_C_SDK_MANIFEST);
    if !std::fs::symlink_metadata(&sealed).is_ok_and(|m| m.is_file()) {
        return Err(format!("release has no regular {RELEASE_C_SDK_MANIFEST}"));
    }
    let bytes = read(&sealed)?;
    if sha256(&bytes) != digest {
        return Err(format!(
            "sealed {RELEASE_C_SDK_MANIFEST} does not hash to hagia_c_sdk_manifest_sha256"
        ));
    }
    let named = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| value["revision"].as_str().map(str::to_owned));
    if named.as_deref() != Some(revision) {
        return Err(format!(
            "sealed {RELEASE_C_SDK_MANIFEST} does not name revision {revision}"
        ));
    }
    Ok((revision.to_owned(), digest.to_owned()))
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            collect_files(root, &path, files)?;
        } else if meta.is_file() {
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            files.push(relative.to_string_lossy().into_owned());
        } else {
            return Err(format!(
                "release contains a non-regular file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}
