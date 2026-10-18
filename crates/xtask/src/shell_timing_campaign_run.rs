//! One attended attempt, or a device-free import of a completed condition.
//! Runs never loop through the campaign automatically. Each invocation claims
//! exactly one frozen arm, and a failed attempt remains in the chain.
use crate::shell_timing_campaign::{
    AttemptOutcome, Condition, claim_attempt, verify_arm, verify_frozen,
};
use crate::shell_timing_campaign_import::{condition_verdict, import_condition, required_evidence};
use crate::shell_timing_campaign_observer::LiveCampaignObserver;
use crate::shell_timing_campaign_provenance::TrustedRunnerProvenance;
use crate::shell_timing_native;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

const USAGE: &str = "usage: xtask shell-timing run-campaign-arm CAMPAIGN SHA256 ARM | report-campaign-condition CAMPAIGN SHA256 idle|load";

fn read_file(path: &Path, cap: u64) -> Result<Vec<u8>, String> {
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let file = std::fs::File::from(fd);
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    let mut bytes = Vec::new();
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > cap {
        return Err(format!("{} exceeds evidence bound", path.display()));
    }
    Ok(bytes)
}

fn attempt_error(error: &str) -> String {
    // Preserve a bounded, valid UTF-8 failure even for a large nested diagnostic.
    let error = error.trim();
    let mut end = error.len().min(4000);
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    if error.is_empty() {
        "campaign attempt failed".into()
    } else {
        error[..end].into()
    }
}

pub fn run(arguments: &[String]) -> Result<Vec<String>, String> {
    let [command, campaign, digest, selector] = arguments else {
        return Err(USAGE.into());
    };
    if !matches!(
        command.as_str(),
        "run-campaign-arm" | "report-campaign-condition"
    ) {
        return Err(USAGE.into());
    }
    let directory = Path::new(campaign);
    if !directory.is_absolute()
        || directory
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("campaign path must be absolute without parent traversal".into());
    }
    let manifest = verify_frozen(directory, digest)?;
    if command == "report-campaign-condition" {
        let condition = match selector.as_str() {
            "idle" => Condition::Idle,
            "load" => Condition::Load,
            _ => return Err("condition must be idle or load".into()),
        };
        let provenance = TrustedRunnerProvenance::new(manifest)?;
        let imported = import_condition(directory, digest, condition, &provenance)?;
        let verdict = condition_verdict(&imported)?;
        let arms = imported
            .arms
            .iter()
            .map(|arm| {
                serde_json::json!({
                    "id": arm.id, "attempt_result_sha256": arm.attempt_result_sha256,
                    "evidence": arm.evidence, "environment": arm.environment,
                    "load": arm.load.as_ref().map(|load| serde_json::json!({
                        "workers": load.workers, "interval": load.interval,
                        "measurement": load.measurement, "worker_cpu_ticks": load.worker_cpu_ticks,
                    })),
                })
            })
            .collect::<Vec<_>>();
        return Ok(vec![
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": 1, "campaign_sha256": digest, "condition": condition,
                "scope": "trusted runner evidence; one declared-rate condition, not a B9 verdict",
                "comparison": verdict, "arms": arms,
            }))
            .map_err(|e| e.to_string())?,
        ]);
    }
    let arm = manifest
        .arms
        .iter()
        .find(|arm| arm.id == *selector)
        .ok_or("unknown arm")?;
    let plan_path = directory.join(&arm.id).join("plan.json");
    let plan = read_file(&plan_path, 1024 * 1024)?;
    let (request, _) = shell_timing_native::verify_prepared(&plan)?;
    // Routine preflight refusals do not consume a single-use attempt. Native
    // execution repeats these checks after the claim to catch later changes.
    shell_timing_native::launch_prerequisites(&request)?;
    verify_arm(directory, digest, &arm.id)?;
    let stage = directory.join(&arm.id).join("run");
    shell_timing_native::stage_available(&stage)?;
    let plan_name = plan_path.to_str().ok_or("plan path is not UTF-8")?;
    let stage_name = stage.to_str().ok_or("run path is not UTF-8")?;
    let attempt = claim_attempt(directory, digest, &arm.id)?;
    let mut observer = LiveCampaignObserver::new(arm.condition, manifest.environment.card.clone());
    let result = shell_timing_native::run_observed(
        &[
            "run-native".into(),
            plan_name.into(),
            arm.plan_sha256.clone(),
            stage_name.into(),
        ],
        &mut observer,
    )
    .and_then(|lines| {
        let mut evidence = BTreeMap::new();
        for name in required_evidence(arm.condition) {
            let cap = if name == "session.log" {
                64 * 1024 * 1024
            } else {
                1024 * 1024
            };
            evidence.insert(
                name.to_owned(),
                crate::sha256(&read_file(&stage.join(name), cap)?),
            );
        }
        Ok((lines, evidence))
    });
    match result {
        Ok((mut lines, evidence)) => {
            attempt.finish(AttemptOutcome::Captured { evidence })?;
            lines.push(format!(
                "arm {} captured; qualification requires condition import",
                arm.id
            ));
            Ok(lines)
        }
        Err(error) => {
            attempt
                .finish(AttemptOutcome::Failed {
                    error: attempt_error(&error),
                })
                .map_err(|finish| format!("{error}; attempt finalization failed: {finish}"))?;
            Err(error)
        }
    }
}
