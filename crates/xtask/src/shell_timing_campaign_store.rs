//! Campaign persistence. No process launch or device access.
use super::{Manifest, validate_arm_plan};
use crate::{
    sha256,
    shell_timing_native::{
        campaign_image_bounds, campaign_profile, verify_prepared, verify_sources,
    },
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

fn read_plan(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("plan exceeds input bound".into());
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
}

/// Verify every arm before creating a campaign directory. The directory is
/// create-only, so a failed preparation is retained and cannot become a retry
/// under the same id. READY is written last; its absence means incomplete.
/// Artifact paths remain bound by plan hashes and must be rechecked at launch.
pub fn freeze(
    manifest: &Manifest,
    plans: &BTreeMap<String, PathBuf>,
    root: &Path,
) -> Result<PathBuf, String> {
    manifest.validate()?;
    if !root.is_absolute() || plans.len() != manifest.arms.len() {
        return Err("absolute campaign root and exactly one plan per arm required".into());
    }
    let mut verified = Vec::new();
    let mut shared = None;
    let mut binaries = BTreeMap::new();
    let mut image_bounds = None;
    for arm in &manifest.arms {
        let path = plans.get(&arm.id).ok_or("missing arm plan")?;
        let bytes = read_plan(path)?;
        if sha256(&bytes) != arm.plan_sha256 {
            return Err(format!("plan digest differs for {}", arm.id));
        }
        let (request, probe) = verify_prepared(&bytes)?;
        validate_arm_plan(manifest, arm, &request, probe)?;
        verify_sources(&request)?;
        if let Some(bounds) = campaign_image_bounds(&request)? {
            if image_bounds
                .as_ref()
                .is_some_and(|expected| expected != &bounds)
            {
                return Err("sealed adapter bounds differ across campaign arms".into());
            }
            image_bounds = Some(bounds);
        }
        let common = serde_json::to_vec(&(
            &request.sophia_source,
            &request.integration_source,
            &request.tty,
            request.display,
            &request.input_seat,
            request.output,
            campaign_profile(&request)?,
            [
                "wm",
                "factotum",
                "pam-helper",
                "preflight",
                "runner",
                "core",
            ]
            .map(|key| &request.artifacts[key].sha256),
        ))
        .map_err(|e| e.to_string())?;
        if let Some(expected) = &shared {
            if expected != &common {
                return Err("campaign source, environment or common artifacts differ".into());
            }
        } else {
            shared = Some(common);
        }
        // Instrumentation needs separate builds; within a method all variants
        // must use the exact same owner and peer executables.
        let pair = (
            request.artifacts["sophia"].sha256.clone(),
            request.artifacts["peer"].sha256.clone(),
        );
        if let Some(old) = binaries.insert(request.instrumentation, pair.clone())
            && old != pair
        {
            return Err("owner or peer binary differs within an instrumentation method".into());
        }
        verified.push((arm.id.clone(), bytes));
    }
    persist(
        root,
        &manifest.id,
        &serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?,
        &manifest.digest()?,
        verified,
    )
}

// Kept separate so crash/retry persistence can be tested without bypassing the
// public freeze API's signed-source and artifact checks.
fn persist(
    root: &Path,
    id: &str,
    manifest: &[u8],
    digest: &str,
    plans: Vec<(String, Vec<u8>)>,
) -> Result<PathBuf, String> {
    let directory = root.join(id);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|e| e.to_string())?;
    write_new(&directory.join("manifest.json"), manifest)?;
    for (id, bytes) in plans {
        let arm_dir = directory.join(id);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&arm_dir)
            .map_err(|e| e.to_string())?;
        write_new(&arm_dir.join("plan.json"), &bytes)?;
        fs::File::open(&arm_dir)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    write_new(&directory.join("READY"), digest.as_bytes())?;
    fs::File::open(&directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::File::open(root)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(directory)
}

#[cfg(test)]
#[path = "../tests/support/shell_timing_campaign_store.rs"]
mod tests;

/// Reopen frozen evidence by its caller-held digest. This checks preparation
/// integrity only; it neither certifies the environment nor authorizes a run.
pub fn verify_frozen(directory: &Path, digest: &str) -> Result<Manifest, String> {
    if !directory.is_absolute() || !crate::hex(digest, 64) {
        return Err("absolute campaign directory and full digest required".into());
    }
    let manifest: Manifest = serde_json::from_slice(&read_plan(&directory.join("manifest.json"))?)
        .map_err(|e| e.to_string())?;
    if manifest.digest()? != digest
        || read_plan(&directory.join("READY"))? != digest.as_bytes()
        || directory.file_name().and_then(|s| s.to_str()) != Some(&manifest.id)
    {
        return Err("campaign identity or readiness differs".into());
    }
    for arm in &manifest.arms {
        if sha256(&read_plan(&directory.join(&arm.id).join("plan.json"))?) != arm.plan_sha256 {
            return Err(format!("frozen plan changed for {}", arm.id));
        }
    }
    Ok(manifest)
}

/// Recheck a selected frozen arm and every artifact it uses immediately before
/// launch. Attended, environment, load and run-order checks remain the runner's
/// obligation; successful verification alone must never start a session.
pub fn verify_arm(
    directory: &Path,
    digest: &str,
    arm_id: &str,
) -> Result<crate::shell_timing_native::Request, String> {
    let manifest = verify_frozen(directory, digest)?;
    let arm = manifest
        .arms
        .iter()
        .find(|a| a.id == arm_id)
        .ok_or("unknown campaign arm")?;
    let bytes = read_plan(&directory.join(&arm.id).join("plan.json"))?;
    if sha256(&bytes) != arm.plan_sha256 {
        return Err("frozen plan changed during verification".into());
    }
    let (request, probe) = verify_prepared(&bytes)?;
    validate_arm_plan(&manifest, arm, &request, probe)?;
    verify_sources(&request)?;
    Ok(request)
}
