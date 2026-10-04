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
pub const SOPHIA_REV: &str = "e9757abfa3b4dc6bf714a21df0c63d29ca4d1a96";
/// The Sophia crates this repository names directly.
pub const SOPHIA_CRATES: [&str; 8] = [
    "sophia-backend-live",
    "sophia-config",
    "sophia-conformance",
    "sophia-engine",
    "sophia-protocol",
    "sophia-runtime",
    "sophia-shell-client",
    "sophia-x-authority",
];

pub const FONT: &str = "assets/fonts/JetBrainsMonoNL-Regular.ttf";
pub const FONT_SHA256: &str = "fb3b2575d7b0657359707993288f12a7360344d39387bb26050e276d61f6bd2a";
pub const SDK_MANIFEST: &str = "pins/c-desktop-sdk/manifest.json";
pub const SDK_MANIFEST_SHA256: &str =
    "3339afc578a324b3a6cf936fd52db259d44159b799e83b0615d239ec8e23e4b2";
const SOPHIA_PIN: &str = "pins/sophia.toml";
const CONTRACTS: &str = "pins/contracts.sha256";
const MANIFESTS: [&str; 6] = [
    "Cargo.toml",
    "crates/xtask/Cargo.toml",
    "crates/live-tests/Cargo.toml",
    "crates/desktop-comparison/Cargo.toml",
    "crates/quickshell-probe/Cargo.toml",
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
/// digests live in pins/contracts.sha256; this list keeps any of the fourteen
/// from being dropped silently. It is Sophia's own list at the pinned
/// revision (crates/xtask/src/c_desktop_sdk.rs), in the same order.
const BINDINGS: [(&str, &str); 14] = [
    (
        "spec/sophia-shell-descriptors.md",
        "docs/sophia-shell-descriptors.md",
    ),
    (
        "spec/sophia-shell-files-v1.kdl",
        "protocol/sophia-shell-files-v1.kdl",
    ),
    ("spec/sophia-9p-profile.md", "docs/sophia-9p-profile.md"),
    ("spec/sophia-shell-files.md", "docs/sophia-shell-files.md"),
    ("spec/sophia-wm-files.md", "docs/sophia-wm-files.md"),
    (
        "spec/sophia-wm-files-v1.kdl",
        "protocol/sophia-wm-files-v1.kdl",
    ),
    ("spec/sophia-wm-api.md", "docs/sophia-wm-api.md"),
    ("spec/sophia-output-files.md", "docs/sophia-output-files.md"),
    (
        "spec/sophia-output-files-v1.kdl",
        "protocol/sophia-output-files-v1.kdl",
    ),
    (
        "spec/golden/sophia-wm-v1.records",
        "protocol/golden/sophia-wm-v1.records",
    ),
    (
        "spec/sophia-lock-files-v1.kdl",
        "protocol/sophia-lock-files-v1.kdl",
    ),
    ("spec/sophia-lock-files.md", "docs/sophia-lock-files.md"),
    (
        "spec/golden/sophia-lock-files-v1.records",
        "protocol/golden/sophia-lock-files-v1.records",
    ),
    (
        "spec/references/diod-9p2000L-protocol.md",
        "docs/references/diod-9p2000L-protocol.md",
    ),
];

const AUDIT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Contract {
    pub digest: String,
    pub local: String,
    pub authoritative: String,
}

/// The fourteen contract digests, in BINDINGS order, fail closed.
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
            "{CONTRACTS}: bindings differ from the fourteen required contract paths"
        ));
    }
    for contract in &contracts {
        relative(&contract.local)?;
        relative(&contract.authoritative)?;
    }
    Ok(contracts)
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
    // The pinned SDK manifest must itself carry the fourteen contract digests.
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
    // A present provisioning marker must match this pin and this lock file:
    // a stale provisioned CARGO_HOME fails here rather than building an old
    // dependency set. (Provisioning itself runs this before writing it.)
    let provision = match std::fs::read(repo.join(PROVISION_MARKER)) {
        Ok(marker) => {
            check_marker(
                &String::from_utf8(marker).map_err(|e| e.to_string())?,
                &lock_sha256(repo)?,
                None,
            )?;
            "matched"
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "unmarked",
        Err(error) => return Err(format!("{PROVISION_MARKER}: {error}")),
    };
    Ok(vec![format!(
        "pins status=pass sophia={SOPHIA_REV} contracts={} copies={} provision={provision}",
        contracts.len(),
        COPIES.len()
    )])
}

/// The marker tools/provision.sh writes once check-pins has passed against
/// the provisioned private CARGO_HOME.
pub const PROVISION_MARKER: &str = ".provision/accepted";

fn lock_sha256(repo: &Path) -> Result<String, String> {
    Ok(sha256(&read(&repo.join("Cargo.lock"))?))
}

/// Gate entry: the CARGO_HOME this gate uses is the provisioned one, accepted
/// for exactly this pin and this Cargo.lock. Gates run it in addition to
/// check-pins.
pub fn check_provision(repo: &Path) -> Result<Vec<String>, String> {
    let marker = std::fs::read(repo.join(PROVISION_MARKER))
        .map_err(|e| format!("{PROVISION_MARKER}: {e}; run tools/provision.sh"))?;
    let lock = lock_sha256(repo)?;
    let home = std::env::var("CARGO_HOME")
        .map_err(|_| "CARGO_HOME must name the provisioned private CARGO_HOME".to_owned())?;
    check_marker(
        &String::from_utf8(marker).map_err(|e| e.to_string())?,
        &lock,
        Some(&home),
    )?;
    Ok(vec![format!(
        "provision status=pass sophia={SOPHIA_REV} cargo_lock_sha256={lock} cargo_home={home}"
    )])
}

/// The marker names exactly url, rev, the Cargo.lock digest and the absolute
/// private CARGO_HOME, in order. With `cargo_home`, that home must match.
pub fn check_marker(text: &str, lock_sha256: &str, cargo_home: Option<&str>) -> Result<(), String> {
    let lines = text.lines().collect::<Vec<_>>();
    let expected = [
        format!("url={SOPHIA_URL}"),
        format!("rev={SOPHIA_REV}"),
        format!("cargo_lock_sha256={lock_sha256}"),
    ];
    let recorded = match lines.as_slice() {
        [url, rev, lock, home]
            if [*url, *rev, *lock] == expected.each_ref().map(String::as_str) =>
        {
            home.strip_prefix("cargo_home=")
                .filter(|home| home.starts_with('/'))
        }
        _ => None,
    };
    let Some(recorded) = recorded else {
        return Err(format!(
            "{PROVISION_MARKER} is stale or malformed (pin or Cargo.lock changed since \
             provisioning); re-run tools/provision.sh"
        ));
    };
    if cargo_home.is_some_and(|home| home != recorded) {
        return Err(format!(
            "CARGO_HOME is not the provisioned {recorded}; re-run tools/provision.sh or use it"
        ));
    }
    Ok(())
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
                || compact == format!("{key}={{workspace=true}}")
                || compact
                    .strip_prefix(&format!("{key}={{workspace=true,features=["))
                    .and_then(|rest| rest.strip_suffix("]}"))
                    .is_some_and(|features| {
                        !features.is_empty()
                            && features.split(',').all(|f| {
                                f.len() > 2
                                    && f.starts_with('"')
                                    && f.ends_with('"')
                                    && f[1..f.len() - 1]
                                        .bytes()
                                        .all(|b| b.is_ascii_lowercase() || b == b'-')
                            })
                    });
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
    Ok(vec![format!(
        "pins status=audited sophia={SOPHIA_REV} contracts={} copies={}",
        contracts.len(),
        COPIES.len()
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
