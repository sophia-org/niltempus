//! Frozen arm ordering for the 120 Hz sealed/bytes campaign.
//!
//! This validates the declared matrix only. Artifact verification, attended
//! execution, load evidence and environment capture are separate requirements;
//! a valid manifest is never a measurement verdict.
use crate::{hex, sha256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Condition {
    Idle,
    Load,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Workload {
    Panel,
    Bulk,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Measurement {
    Latency,
    CpuOnly,
    RustGlobal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Variant {
    A,
    B,
    C,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    pub id: String,
    pub condition: Condition,
    pub pair: u8,
    pub workload: Workload,
    pub measurement: Measurement,
    pub variant: Variant,
    /// Digest of the complete prepared native plan, not just its request.
    pub plan_sha256: String,
    pub peer_config_sha256: String,
    pub frames: usize,
    pub warmup: usize,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentTarget {
    pub card: String,
    pub output: u64,
    pub scale_milli: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u8,
    pub id: String,
    pub connector: String,
    pub width: u32,
    pub height: u32,
    pub mode_millihz: u64,
    pub vrr: bool,
    /// Frozen before the first run; never selected from a successful capture.
    pub environment: EnvironmentTarget,
    pub load_workers: u8,
    /// Human declaration of the quiet window; capture records actual interference.
    pub quiet_window: String,
    pub arms: Vec<Arm>,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 2
            || !identifier(&self.id)
            || !identifier(&self.connector)
            || self.width != 1920
            || self.height != 1080
            || self.mode_millihz != 120_000
            || self.vrr
            || self.load_workers != 16
            || self.quiet_window.trim().is_empty()
            || self.quiet_window.len() > 4096
        {
            return Err("campaign requires an identified 1920x1080@120 Hz output, VRR off, 16 workers and a quiet window".into());
        }
        let card = self.environment.card.strip_prefix("card");
        if !card.is_some_and(|digits| {
            digits
                .parse::<u32>()
                .is_ok_and(|id| id.to_string() == digits)
        }) || self.environment.output == 0
            || self.environment.scale_milli == 0
            || !self.environment.scale_milli.is_multiple_of(1000)
        {
            return Err(
                "campaign requires a declared card, output id and applied integer scale".into(),
            );
        }
        if self.arms.len() != 108 {
            return Err("campaign requires all 108 arms; failed arms cannot be replaced".into());
        }
        let mut ids = BTreeSet::new();
        let mut index = 0;
        for condition in [Condition::Idle, Condition::Load] {
            for pair in 1..=3 {
                for workload in [Workload::Panel, Workload::Bulk] {
                    for measurement in [
                        Measurement::Latency,
                        Measurement::CpuOnly,
                        Measurement::RustGlobal,
                    ] {
                        let order = match pair {
                            1 => [Variant::C, Variant::B, Variant::A],
                            2 => [Variant::A, Variant::B, Variant::C],
                            _ => [Variant::B, Variant::C, Variant::A],
                        };
                        for variant in order {
                            let arm = &self.arms[index];
                            index += 1;
                            if (
                                arm.condition,
                                arm.pair,
                                arm.workload,
                                arm.measurement,
                                arm.variant,
                            ) != (condition, pair, workload, measurement, variant)
                            {
                                return Err(format!(
                                    "arm {} differs from the frozen condition/pair/workload/rotation order",
                                    arm.id
                                ));
                            }
                            if !identifier(&arm.id)
                                || arm.id == "READY"
                                || !ids.insert(&arm.id)
                                || !hex(&arm.plan_sha256, 64)
                                || !hex(&arm.peer_config_sha256, 64)
                            {
                                return Err(
                                    "arm ids must be unique and artifact digests complete".into()
                                );
                            }
                            // Plans may have identical workload bytes across restarts. Identity
                            // comes from the arm id, never from changing an otherwise equal plan.
                            let expected = if measurement == Measurement::Latency {
                                (1100, 100)
                            } else {
                                (1000, 0)
                            };
                            if (arm.frames, arm.warmup) != expected || arm.runtime_ms != 60_000 {
                                return Err("sampling requires 1100/100 latency frames or 1000/0 cost frames and 60-second arms".into());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Stable digest of the validated declaration. No plan files are read here.
    pub fn digest(&self) -> Result<String, String> {
        self.validate()?;
        serde_json::to_vec(self)
            .map(|bytes| sha256(&bytes))
            .map_err(|e| e.to_string())
    }
}

/// Bind a matrix entry to the actual prepared workload, not caller labels.
pub fn validate_arm_plan(
    manifest: &Manifest,
    arm: &Arm,
    request: &crate::shell_timing_native::Request,
    probe_bytes: u64,
) -> Result<(), String> {
    use crate::shell_timing::ImageSource;
    let (wire, source) = match arm.variant {
        Variant::A => ("ipc", ImageSource::Bytes),
        Variant::B => ("files", ImageSource::Bytes),
        Variant::C => ("files", ImageSource::Isb),
    };
    let instrumentation = match arm.measurement {
        Measurement::Latency => "none",
        Measurement::CpuOnly => "cpu-only",
        Measurement::RustGlobal => "rust-global",
    };
    let probe = match arm.workload {
        Workload::Panel => 0,
        Workload::Bulk => 4_194_304,
    };
    if request.wire != wire
        || request.source != source
        || request.instrumentation != instrumentation
        || probe_bytes != probe
        || request.frames != arm.frames
        || request.runtime_ms != arm.runtime_ms
        || request.mode_millihz != manifest.mode_millihz
        || request.output != manifest.environment.output
        || !request.environment_readback
        || request.artifacts.get("peer-config").map(|a| &a.sha256) != Some(&arm.peer_config_sha256)
    {
        return Err(format!("prepared workload does not match arm {}", arm.id));
    }
    Ok(())
}

#[path = "shell_timing_campaign_store.rs"]
mod store;
pub use store::{freeze, verify_arm, verify_frozen};

#[path = "shell_timing_campaign_attempt.rs"]
mod attempt;
pub use attempt::{Attempt, AttemptOutcome, claim_attempt, read_finished};

/// Device-free preparation input. Plans are existing prepare-native outputs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FreezeRequest {
    manifest: Manifest,
    plans: std::collections::BTreeMap<String, std::path::PathBuf>,
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    use std::io::Read;
    let [command, input, root] = args else {
        return Err("expected freeze-campaign REQUEST CAMPAIGN_ROOT".into());
    };
    if command != "freeze-campaign" {
        return Err("expected freeze-campaign REQUEST CAMPAIGN_ROOT".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(input)
        .map_err(|e| e.to_string())?
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("campaign request exceeds input bound".into());
    }
    let request: FreezeRequest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let path = freeze(
        &request.manifest,
        &request.plans,
        std::path::Path::new(root),
    )?;
    Ok(vec![format!(
        "frozen {} sha256={} (not a measurement verdict)",
        path.display(),
        request.manifest.digest()?
    )])
}
