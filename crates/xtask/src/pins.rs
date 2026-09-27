//! The pins that tie this repository to exactly one Sophia revision.
//!
//! `check` is local and offline: pins/sophia.toml, every Cargo manifest and
//! Cargo.lock must name the same canonical Sophia URL and revision, no manifest
//! may redirect a Sophia crate (path, branch, tag, patch, replace, source), and
//! every file copied from Sophia must still hash to its pinned digest.
//! Provisioning runs it before a vendored tree is accepted. `audit` re-derives
//! the pinned digests from an explicit Sophia clone at the pinned revision.
use crate::{hex, read, sha256};
use std::path::{Component, Path};
use std::process::Command;
use std::time::Duration;

pub const SOPHIA_URL: &str = "https://github.com/sophia-org/sophia.git";
pub const SOPHIA_REV: &str = "9fcaec782ce4fe9978568c0466ee17a78b3d4571";
/// The Sophia crates this repository names directly.
pub const SOPHIA_CRATES: [&str; 2] = ["sophia-protocol", "sophia-runtime"];

pub const FONT: &str = "assets/fonts/JetBrainsMonoNL-Regular.ttf";
pub const FONT_SHA256: &str = "fb3b2575d7b0657359707993288f12a7360344d39387bb26050e276d61f6bd2a";
pub const SDK_MANIFEST: &str = "pins/c-desktop-sdk/manifest.json";
pub const SDK_MANIFEST_SHA256: &str =
    "9da6ca11f381cb18d1772f5b6656a6e6a32dc003502daf53ce6d220e67e3b28e";
const SOPHIA_PIN: &str = "pins/sophia.toml";
const CONTRACTS: &str = "pins/contracts.sha256";
/// Sophia files the attended gates read read-only from the explicit Sophia
/// checkout under test. They stay in Sophia; only their digests are pinned.
pub const SHARED: &str = "pins/sophia-shared.sha256";
const SHARED_PATHS: [&str; 2] = [
    "tools/run_sophia_session.sh",
    "tools/fixtures/native_launcher_core.kdl",
];
const MANIFESTS: [&str; 4] = [
    "Cargo.toml",
    "crates/xtask/Cargo.toml",
    "crates/live-tests/Cargo.toml",
    ".cargo/config.toml",
];

/// Files copied byte for byte from Sophia at SOPHIA_REV:
/// (path here, path in Sophia, sha256).
const COPIES: [(&str, &str, &str); 3] = [
    (
        FONT,
        "assets/fonts/JetBrainsMonoNL-Regular.ttf",
        FONT_SHA256,
    ),
    (
        "assets/fonts/JetBrainsMono-OFL.txt",
        "assets/fonts/JetBrainsMono-OFL.txt",
        "30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad",
    ),
    (
        SDK_MANIFEST,
        "vendor/c-desktop-sdk/manifest.json",
        SDK_MANIFEST_SHA256,
    ),
];

/// Every contract binding the C SDK snapshot must carry unchanged:
/// (path in the SDK snapshot source, authoritative path in Sophia). The
/// digests live in pins/contracts.sha256; this list keeps any of the
/// eighteen from being dropped silently.
const BINDINGS: [(&str, &str); 18] = [
    (
        "spec/sophia-shell-files-v1.kdl",
        "protocol/sophia-shell-files-v1.kdl",
    ),
    ("spec/sophia-shell-v1.kdl", "protocol/sophia-shell-v1.kdl"),
    ("spec/sophia-9p-profile.md", "docs/sophia-9p-profile.md"),
    ("spec/sophia-shell-files.md", "docs/sophia-shell-files.md"),
    ("spec/sophia-wm-files.md", "docs/sophia-wm-files.md"),
    (
        "spec/references/diod-9p2000L-protocol.md",
        "docs/references/diod-9p2000L-protocol.md",
    ),
    ("src/sophia_wm_v1.c", "bindings/c/sophia_wm_v1.c"),
    ("src/sophia_wm_v1.h", "bindings/c/sophia_wm_v1.h"),
    (
        "spec/golden/sophia-shell-catalog-actions.frames",
        "protocol/golden/sophia-shell-catalog-actions.frames",
    ),
    (
        "spec/golden/sophia-shell-content-malformed.frames",
        "protocol/golden/sophia-shell-content-malformed.frames",
    ),
    (
        "spec/golden/sophia-shell-content.frames",
        "protocol/golden/sophia-shell-content.frames",
    ),
    (
        "spec/golden/sophia-shell-indicators.frames",
        "protocol/golden/sophia-shell-indicators.frames",
    ),
    (
        "spec/golden/sophia-shell-launcher.frames",
        "protocol/golden/sophia-shell-launcher.frames",
    ),
    (
        "spec/golden/sophia-shell-native-launcher.frames",
        "protocol/golden/sophia-shell-native-launcher.frames",
    ),
    (
        "spec/golden/sophia-shell-reference.frames",
        "protocol/golden/sophia-shell-reference.frames",
    ),
    (
        "spec/golden/sophia-shell-tabs.frames",
        "protocol/golden/sophia-shell-tabs.frames",
    ),
    (
        "spec/golden/sophia-shell-v1-malformed.frames",
        "protocol/golden/sophia-shell-v1-malformed.frames",
    ),
    (
        "spec/golden/sophia-shell-v1.frames",
        "protocol/golden/sophia-shell-v1.frames",
    ),
];

const AUDIT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Contract {
    pub digest: String,
    pub local: String,
    pub authoritative: String,
}

/// The eighteen contract digests, in BINDINGS order, fail closed.
pub fn contracts(repo: &Path) -> Result<Vec<Contract>, String> {
    parse_contracts(&String::from_utf8(read(&repo.join(CONTRACTS))?).map_err(|e| e.to_string())?)
}

pub fn parse_contracts(text: &str) -> Result<Vec<Contract>, String> {
    let contracts = text
        .lines()
        .map(|line| match line.split(' ').collect::<Vec<_>>()[..] {
            [digest, local, authoritative] if hex(digest, 64) => Ok(Contract {
                digest: digest.to_owned(),
                local: local.to_owned(),
                authoritative: authoritative.to_owned(),
            }),
            _ => Err(format!("{CONTRACTS}: malformed line {line:?}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pairs = contracts
        .iter()
        .map(|c| (c.local.as_str(), c.authoritative.as_str()))
        .collect::<Vec<_>>();
    if pairs != BINDINGS {
        return Err(format!(
            "{CONTRACTS}: bindings differ from the eighteen required contract paths"
        ));
    }
    for contract in &contracts {
        relative(&contract.local)?;
        relative(&contract.authoritative)?;
    }
    Ok(contracts)
}

/// The pinned digests of the shared Sophia files, in SHARED_PATHS order.
pub fn shared(repo: &Path) -> Result<Vec<(String, String)>, String> {
    parse_shared(&String::from_utf8(read(&repo.join(SHARED))?).map_err(|e| e.to_string())?)
}

pub fn parse_shared(text: &str) -> Result<Vec<(String, String)>, String> {
    let entries = text
        .lines()
        .map(|line| match line.split(' ').collect::<Vec<_>>()[..] {
            [digest, path] if hex(digest, 64) => Ok((digest.to_owned(), path.to_owned())),
            _ => Err(format!("{SHARED}: malformed line {line:?}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if entries.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>() != SHARED_PATHS {
        return Err(format!("{SHARED}: paths differ from {SHARED_PATHS:?}"));
    }
    Ok(entries)
}

/// Local, offline consistency of every pin; see the module comment.
pub fn check(repo: &Path) -> Result<Vec<String>, String> {
    let text =
        |name: &str| String::from_utf8(read(&repo.join(name))?).map_err(|e| format!("{name}: {e}"));
    check_pin_file(&text(SOPHIA_PIN)?)?;
    for name in MANIFESTS {
        check_manifest(name, &text(name)?, name == "Cargo.toml")?;
    }
    check_lock(&text("Cargo.lock")?)?;
    for (path, _, digest) in COPIES {
        if sha256(&read(&repo.join(path))?) != digest {
            return Err(format!("{path} differs from its pinned digest"));
        }
    }
    // The pinned SDK manifest must itself carry the eighteen contract digests.
    let manifest: serde_json::Value =
        serde_json::from_slice(&read(&repo.join(SDK_MANIFEST))?).map_err(|e| e.to_string())?;
    let contracts = contracts(repo)?;
    for contract in &contracts {
        if manifest["files"][&contract.local].as_str() != Some(contract.digest.as_str()) {
            return Err(format!(
                "{SDK_MANIFEST} does not pin {} at the contract digest",
                contract.local
            ));
        }
    }
    let shared = shared(repo)?;
    Ok(vec![format!(
        "pins status=pass sophia={SOPHIA_REV} contracts={} copies={} shared={}",
        contracts.len(),
        COPIES.len(),
        shared.len()
    )])
}

/// pins/sophia.toml names exactly the canonical URL and revision.
pub fn check_pin_file(text: &str) -> Result<(), String> {
    let mut entries = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(" = ")
            .ok_or_else(|| format!("{SOPHIA_PIN}: malformed line {line:?}"))?;
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .ok_or_else(|| format!("{SOPHIA_PIN}: unquoted value {line:?}"))?;
        entries.push((key, value));
    }
    if entries != [("url", SOPHIA_URL), ("rev", SOPHIA_REV)] {
        return Err(format!(
            "{SOPHIA_PIN} does not pin url={SOPHIA_URL} rev={SOPHIA_REV}: {entries:?}"
        ));
    }
    Ok(())
}

/// A manifest or Cargo config may reach Sophia only through the pinned git
/// dependency in the workspace root; members inherit it with `workspace`.
pub fn check_manifest(name: &str, text: &str, root: bool) -> Result<(), String> {
    let pinned = format!("{{ git = \"{SOPHIA_URL}\", rev = \"{SOPHIA_REV}\" }}");
    let mut found = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[')
            && ["patch", "replace", "source"]
                .iter()
                .any(|section| line.trim_start_matches('[').starts_with(section))
        {
            return Err(format!("{name}: redirecting section {line:?}"));
        }
        let key = line.split(['=', '.', ' ']).next().unwrap_or("");
        let compact = line.replace(' ', "");
        if compact.starts_with("paths=") || compact.contains("branch=") || compact.contains("tag=")
        {
            return Err(format!("{name}: unpinned source {line:?}"));
        }
        if let Some(value) = compact.split("path=\"").nth(1) {
            let value = value.split('"').next().unwrap_or("");
            let mut parts = Path::new(value).components();
            if key.starts_with("sophia")
                || !(parts.next() == Some(Component::ParentDir)
                    && matches!(parts.next(), Some(Component::Normal(_)))
                    && parts.next().is_none())
            {
                return Err(format!("{name}: path outside this workspace {line:?}"));
            }
        }
        if key.starts_with("sophia") {
            let value = line
                .split_once('=')
                .map(|(_, v)| v.trim())
                .unwrap_or_default();
            let member = compact == format!("{key}.workspace=true")
                || compact == format!("{key}={{workspace=true}}");
            if root && SOPHIA_CRATES.contains(&key) && value == pinned {
                found.push(key);
            } else if root || !member {
                return Err(format!("{name}: Sophia crate not at the pin: {line:?}"));
            }
        } else if compact.contains("git=") {
            return Err(format!("{name}: git dependency outside the pin: {line:?}"));
        }
    }
    found.sort_unstable();
    if root && found != SOPHIA_CRATES {
        return Err(format!(
            "{name}: expected pinned {SOPHIA_CRATES:?}, found {found:?}"
        ));
    }
    Ok(())
}

/// Every git source in Cargo.lock is the pinned revision, and every Sophia
/// package came from it (none from a path or a registry).
pub fn check_lock(text: &str) -> Result<(), String> {
    let pinned = format!("git+{SOPHIA_URL}?rev={SOPHIA_REV}#{SOPHIA_REV}");
    let mut named = Vec::new();
    for block in text.split("[[package]]").skip(1) {
        let field = |key: &str| {
            block.lines().find_map(|line| {
                line.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix(" = \""))
                    .and_then(|rest| rest.strip_suffix('"'))
            })
        };
        let name = field("name").ok_or("Cargo.lock: package without a name")?;
        let source = field("source");
        let git = source.is_some_and(|s| s.starts_with("git+"));
        if (git || name.starts_with("sophia")) && source != Some(pinned.as_str()) {
            return Err(format!(
                "Cargo.lock: {name} source {source:?} is not {pinned}"
            ));
        }
        if SOPHIA_CRATES.contains(&name) {
            named.push(name);
        }
    }
    named.sort_unstable();
    if named != SOPHIA_CRATES {
        return Err(format!(
            "Cargo.lock: expected {SOPHIA_CRATES:?} at the pin, found {named:?}"
        ));
    }
    Ok(())
}

/// Re-derive every copied file and contract digest from an explicit Sophia
/// clone at SOPHIA_REV. Read-only; used when provisioning or moving the pin.
pub fn audit(repo: &Path, sophia: &Path) -> Result<Vec<String>, String> {
    if !sophia.is_absolute() {
        return Err(format!(
            "Sophia repository must be absolute: {}",
            sophia.display()
        ));
    }
    let git = |args: &[&str]| {
        crate::bemenu_artifact::bounded(
            Command::new("git")
                .arg("-C")
                .arg(sophia)
                .args(["-c", "core.fsmonitor=false"])
                .args(args)
                .env("GIT_OPTIONAL_LOCKS", "0"),
            AUDIT_TIMEOUT,
            &format!("git {}", args.first().copied().unwrap_or("")),
        )
    };
    let resolved = git(&[
        "rev-parse",
        "--verify",
        "--end-of-options",
        &format!("{SOPHIA_REV}^{{commit}}"),
    ])?;
    if String::from_utf8_lossy(&resolved).trim() != SOPHIA_REV {
        return Err(format!("{} does not hold {SOPHIA_REV}", sophia.display()));
    }
    let digest = |path: &str| git(&["show", &format!("{SOPHIA_REV}:{path}")]).map(|b| sha256(&b));
    for (_, path, pinned) in COPIES {
        if digest(path)? != pinned {
            return Err(format!("Sophia {SOPHIA_REV}:{path} differs from its pin"));
        }
    }
    let contracts = contracts(repo)?;
    for contract in &contracts {
        if digest(&contract.authoritative)? != contract.digest {
            return Err(format!(
                "Sophia {SOPHIA_REV}:{} differs from {CONTRACTS}",
                contract.authoritative
            ));
        }
    }
    let shared = shared(repo)?;
    for (pinned, path) in &shared {
        if digest(path)? != *pinned {
            return Err(format!("Sophia {SOPHIA_REV}:{path} differs from {SHARED}"));
        }
    }
    Ok(vec![format!(
        "pins status=audited sophia={SOPHIA_REV} contracts={} copies={} shared={}",
        contracts.len(),
        COPIES.len(),
        shared.len()
    )])
}

fn relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("{CONTRACTS}: invalid path {path:?}"));
    }
    Ok(())
}
