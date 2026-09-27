// Provenance: adapted from Sophia crates/xtask/src/c_desktop_sdk.rs at
// 9fcaec782ce4fe9978568c0466ee17a78b3d4571. Temporary duplicate. Sophia read
// its own vendor/c-desktop-sdk and contract files; here the manifest is the
// pinned copy in pins/c-desktop-sdk and the contracts are the eighteen
// digests in pins/contracts.sha256, both taken from that revision.
//! Verify a client's vendored C SDK snapshot against the pinned SDK identity.
//!
//! This repository never compiles or reads the SDK's own repository: a client
//! artifact is accepted only when its vendored snapshot is byte-for-byte the
//! audited snapshot Sophia pinned, and carries every protocol contract
//! unchanged.
use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde::Deserialize;

use crate::{hex, pins, read, sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    repository: String,
    revision: String,
    files: BTreeMap<String, String>,
}

/// Returns the pinned SDK revision when `snapshot` (a directory holding
/// manifest.json, upstream.commit and source/) is exactly the pinned SDK.
pub fn verify(snapshot: &Path, repo: &Path) -> Result<String, String> {
    let pinned = read(&repo.join(pins::SDK_MANIFEST))?;
    if sha256(&pinned) != pins::SDK_MANIFEST_SHA256 {
        return Err(format!(
            "{} differs from its pinned digest",
            pins::SDK_MANIFEST
        ));
    }
    let bytes = read(&snapshot.join("manifest.json"))?;
    if bytes != pinned {
        return Err(format!(
            "C SDK manifest differs from the pinned {}",
            pins::SDK_MANIFEST
        ));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if manifest.schema != 1
        || manifest.repository != "https://github.com/sophia-org/sophia-desktop-sdk-c"
        || !hex(&manifest.revision, 40)
        || manifest.files.is_empty()
    {
        return Err("invalid C SDK snapshot identity".into());
    }
    for (name, digest) in &manifest.files {
        let path = Path::new(name);
        if name.is_empty()
            || name.split('/').any(|part| matches!(part, "" | "." | ".."))
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !hex(digest, 64)
        {
            return Err(format!("invalid C SDK manifest entry {name:?}"));
        }
    }
    let source = snapshot.join("source");
    let inventory = crate::git_tree::inventory(&source)?;
    let actual = inventory.files;
    if actual != manifest.files {
        let changed = actual
            .iter()
            .find(|(name, digest)| manifest.files.get(*name) != Some(*digest))
            .map(|(name, _)| name.as_str())
            .or_else(|| {
                manifest
                    .files
                    .keys()
                    .find(|name| !actual.contains_key(*name))
                    .map(String::as_str)
            })
            .unwrap_or("unknown");
        return Err(format!("C SDK snapshot differs from its pin: {changed}"));
    }
    let commit = read(&snapshot.join("upstream.commit"))?;
    crate::git_tree::verify_commit(&commit, &manifest.revision, &inventory.tree)?;
    for contract in pins::contracts(repo)? {
        if sha256(&read(&source.join(&contract.local))?) != contract.digest {
            return Err(format!(
                "C SDK contract drift: {} (Sophia {} at {})",
                contract.local,
                contract.authoritative,
                pins::SOPHIA_REV
            ));
        }
    }
    Ok(manifest.revision)
}
