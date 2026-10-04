// Provenance: the layout is ported from Sophia tools/package_live_session.sh
// at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged
// at the pin de776c68) (Sophia rule 13). The release layout, the schema-6
// manifest fields and SHA256SUMS are kept. Schema 7 (the director's release
// ruling) adds Hagia's vendored C SDK revision and manifest digest, with the
// manifest sealed in the release.
//! The desktop release layout. `assemble` lays out a release from prebuilt
//! files, checks it with the packaged policy verifier and seals it with
//! SHA256SUMS; `cargo xtask assemble-nix` (nix_assembly.rs) is its caller in
//! the Nix build, which builds every input from its locked source.
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::bemenu_artifact::{bounded, set_mode};
use crate::{hex, read, sha256};

/// The release manifest schema. 7 requires Hagia's vendored C SDK revision
/// and manifest digest; a schema-6 release is not a candidate here.
pub const RELEASE_SCHEMA: &str = "7";
/// Hagia's vendored C SDK manifest, sealed in the release (SHA256SUMS).
pub const RELEASE_C_SDK_MANIFEST: &str = "share/sophia-policy/hagia/c-sdk.manifest.json";

/// Sophia's retained generic session files, from the staged pinned tree.
pub const SOPHIA_RETAINED: [(&str, u32); 5] = [
    ("tools/run_sophia_session.sh", 0o755),
    ("tools/stop_sophia_session.sh", 0o755),
    ("tools/sophia_tty_mode.py", 0o755),
    ("tools/lib/session_lifecycle.sh", 0o644),
    ("tools/lib/session_preparation.sh", 0o644),
];

/// Operator commands: (bin name, this repository's source).
pub const COMMANDS: [(&str, &str); 44] = [
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
    ("sophia-stop", "tools/installed/sophia-stop"),
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
pub const TOOLS: [(&str, u32); 16] = [
    ("tools/session/run_desktop_session.sh", 0o755),
    ("tools/start_sophia_native_hot_reload_tty3.sh", 0o755),
    ("tools/verify_packaged_policy.sh", 0o755),
    ("tools/verify_sophia_firefox_rendering_physical.sh", 0o755),
    ("tools/probes/uinput_text_injector.py", 0o755),
    ("tools/config/proof_helpers.sh", 0o644),
    ("tools/config/99-sophia-uinput.rules", 0o644),
    ("tools/config/sophia-uinput.conf", 0o644),
    ("tools/lib/installed_attempt_ledger.sh", 0o644),
    ("tools/lib/installed_hagia_evidence.sh", 0o644),
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

/// The window manager and shell the release packages: Hagia and narthex,
/// with Hagia's default profile and its vendored C SDK manifest. The
/// digests are of these exact files.
#[derive(Debug, Clone)]
pub struct WmPair {
    pub hagia: PathBuf,
    pub narthex: PathBuf,
    pub profile: PathBuf,
    pub hagia_commit: String,
    pub narthex_commit: String,
    pub hagia_sha256: String,
    pub narthex_sha256: String,
    pub profile_sha256: String,
    pub hagia_c_sdk_revision: String,
    pub hagia_c_sdk_manifest: PathBuf,
    pub hagia_c_sdk_manifest_sha256: String,
}

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
    pub pair: WmPair,
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
    /// The release ID: one path component under the install prefix, which
    /// names the release directory. The Nix build derives it from every
    /// locked input.
    pub release_id: String,
}

pub fn workspace_version(tree: &Path) -> Result<String, String> {
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

/// Lay out the schema-7 release in the new `out` directory, check it with
/// the packaged policy verifier and seal it with SHA256SUMS. On any failure
/// the partial output is removed.
pub fn assemble(assembly: &Assembly) -> Result<Vec<String>, String> {
    let out = &assembly.out;
    if !hex(&assembly.sophia_rev, 40) || !hex(&assembly.integration_commit, 40) {
        return Err("release commits must be 40 lowercase hex".into());
    }
    if !valid_release_id(&assembly.release_id) {
        return Err(format!(
            "release ID must be 1-64 of [a-z0-9-], starting alphanumeric: {:?}",
            assembly.release_id
        ));
    }
    std::fs::DirBuilder::new()
        .mode(0o755)
        .create(out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    match lay_out(assembly) {
        Ok(()) => Ok(vec![format!(
            "desktop_release status=packaged release_id={} sophia_commit={} integration_commit={} \
             hagia_commit={} narthex_commit={} dir={}",
            assembly.release_id,
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
    check_release_sdk(out, &a.pair)?;

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
        format!("release_id={}", a.release_id),
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

/// A release ID is one path component under the install prefix.
pub fn valid_release_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Cross-check a laid-out release's Hagia C SDK binding against the pair,
/// with no defaults. The manifest is schema 7 with `hagia_included=true` and
/// carries each SDK field exactly once and well formed, equal to the pair's
/// value. The sealed SDK manifest is a regular file that hashes to the
/// recorded digest and names the recorded revision.
fn check_release_sdk(release: &Path, pair: &WmPair) -> Result<(), String> {
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
    if revision != pair.hagia_c_sdk_revision {
        return Err(format!(
            "release hagia_c_sdk_revision {revision} is not the pair's {}",
            pair.hagia_c_sdk_revision
        ));
    }
    if digest != pair.hagia_c_sdk_manifest_sha256 {
        return Err(format!(
            "release hagia_c_sdk_manifest_sha256 {digest} is not the pair's {}",
            pair.hagia_c_sdk_manifest_sha256
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
    Ok(())
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
