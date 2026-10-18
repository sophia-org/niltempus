//! Persistent attempt ordering. Claims are never removed, even after failure.
//! This records runner transitions, not that a native run or its measurements
//! are valid. The attended runner must check artifacts, environment and load.
use super::{Arm, Manifest, verify_frozen};
use crate::sha256;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Claim {
    schema: u8,
    campaign_sha256: String,
    arm: String,
    index: usize,
    previous_result_sha256: Option<String>,
}

/// Captured is not a passing arm. Its evidence still needs qualification.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptOutcome {
    Captured { evidence: BTreeMap<String, String> },
    Failed { error: String },
}

impl AttemptOutcome {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Captured { evidence }
                if !evidence.is_empty()
                    && evidence.len() <= 32
                    && evidence.iter().all(|(name, hash)| {
                        !name.is_empty()
                            && name.len() <= 96
                            && name
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                            && crate::hex(hash, 64)
                    }) =>
            {
                Ok(())
            }
            Self::Failed { error } if !error.trim().is_empty() && error.len() <= 4096 => Ok(()),
            _ => Err("attempt needs bounded failure detail or named evidence digests".into()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultRecord {
    schema: u8,
    claim_sha256: String,
    outcome: AttemptOutcome,
}

/// Single-use completion handle. Dropping it leaves an unfinished claim which
/// prevents later arms and replacement attempts under this campaign identity.
pub struct Attempt {
    campaign: PathBuf,
    digest: String,
    arm: Arm,
    claim: Vec<u8>,
}

impl Attempt {
    pub fn arm(&self) -> &Arm {
        &self.arm
    }

    pub fn finish(self, outcome: AttemptOutcome) -> Result<(), String> {
        outcome.validate()?;
        let manifest = verify_frozen(&self.campaign, &self.digest)?;
        let index = manifest
            .arms
            .iter()
            .position(|arm| arm.id == self.arm.id)
            .ok_or("unknown arm")?;
        let previous =
            history(&self.campaign, &manifest, &self.digest, index)?.map(|(_, digest)| digest);
        let claim: Claim = serde_json::from_slice(&self.claim).map_err(|e| e.to_string())?;
        if claim != expected(&manifest, &self.digest, index, previous) {
            return Err("attempt predecessor changed during run".into());
        }
        let directory = self.campaign.join(&self.arm.id).join("attempt");
        if read(&directory.join("claim.json"))? != self.claim {
            return Err("attempt claim changed".into());
        }
        write_new(
            &directory.join("result.json"),
            &serde_json::to_vec(&ResultRecord {
                schema: 1,
                claim_sha256: sha256(&self.claim),
                outcome,
            })
            .map_err(|e| e.to_string())?,
        )?;
        sync(&directory)
    }
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65_536 {
        return Err("attempt record too large".into());
    }
    Ok(bytes)
}

fn sync(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
}

fn expected(manifest: &Manifest, digest: &str, index: usize, previous: Option<String>) -> Claim {
    Claim {
        schema: 1,
        campaign_sha256: digest.into(),
        arm: manifest.arms[index].id.clone(),
        index,
        previous_result_sha256: previous,
    }
}

/// Read the exact completed prefix, sharing the same chain checks between
/// admission of the next attempt and later evidence import.
fn history(
    campaign: &Path,
    manifest: &Manifest,
    digest: &str,
    count: usize,
) -> Result<Option<(AttemptOutcome, String)>, String> {
    let mut previous = None;
    let mut last = None;
    for (index, arm) in manifest.arms.iter().take(count).enumerate() {
        let directory = campaign.join(&arm.id).join("attempt");
        let bytes = read(&directory.join("claim.json"))?;
        let claim: Claim = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if claim != expected(manifest, digest, index, previous) {
            return Err("attempt order or campaign identity changed".into());
        }
        let result_bytes = read(&directory.join("result.json"))?;
        let result: ResultRecord =
            serde_json::from_slice(&result_bytes).map_err(|e| e.to_string())?;
        if result.schema != 1 || result.claim_sha256 != sha256(&bytes) {
            return Err("result does not belong to its attempt".into());
        }
        result.outcome.validate()?;
        let digest = sha256(&result_bytes);
        previous = Some(digest.clone());
        last = Some((result.outcome, digest));
    }
    Ok(last)
}

/// Return a completed arm only after validating its entire predecessor chain.
/// Failed attempts are returned as failures for the importer to reject; they
/// remain legitimate history and never authorize a replacement run.
pub fn read_finished(
    campaign: &Path,
    digest: &str,
    arm_id: &str,
) -> Result<(AttemptOutcome, String), String> {
    let manifest = verify_frozen(campaign, digest)?;
    let index = manifest
        .arms
        .iter()
        .position(|arm| arm.id == arm_id)
        .ok_or("unknown arm")?;
    history(campaign, &manifest, digest, index + 1)?.ok_or("missing attempt".into())
}

/// Claim exactly the next arm, after all preceding attempts have finished.
/// Failure is a retained outcome, not permission to retry the same arm. A crash
/// or partial write blocks progression; starting a new campaign is explicit.
/// The directory creation arbitrates concurrent claimants before any launch.
pub fn claim_attempt(campaign: &Path, digest: &str, arm_id: &str) -> Result<Attempt, String> {
    let manifest = verify_frozen(campaign, digest)?;
    let index = manifest
        .arms
        .iter()
        .position(|a| a.id == arm_id)
        .ok_or("unknown arm")?;
    let previous = history(campaign, &manifest, digest, index)?.map(|(_, hash)| hash);
    for arm in manifest.arms.iter().skip(index) {
        let directory = campaign.join(&arm.id).join("attempt");
        match fs::symlink_metadata(&directory) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
            Ok(_) => return Err("arm already claimed or later attempt exists".into()),
        }
    }
    let directory = campaign.join(arm_id).join("attempt");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|e| e.to_string())?;
    // Preserve even an incomplete claim: a failure here is never rolled back.
    sync(directory.parent().ok_or("attempt has no parent")?)?;
    let claim = serde_json::to_vec(&expected(&manifest, digest, index, previous))
        .map_err(|e| e.to_string())?;
    write_new(&directory.join("claim.json"), &claim)?;
    sync(&directory)?;
    Ok(Attempt {
        campaign: campaign.into(),
        digest: digest.into(),
        arm: manifest.arms[index].clone(),
        claim,
    })
}
