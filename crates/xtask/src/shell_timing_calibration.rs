//! t338 calibration: one paired files panel CPU run, bytes then sealed,
//! against a frozen environment target, with the same authenticated
//! environment evidence as a campaign arm.
//!
//! This is not a campaign. Nothing here validates or relaxes the 108-arm
//! manifest, and the report is a source comparison only, never a condition
//! or gate verdict. Each run directory is single-use; every launch check of
//! run-native applies unchanged.
use crate::shell_timing::ImageSource;
use crate::shell_timing_campaign::{
    Arm, AttemptOutcome, Condition, Measurement, Variant, Workload as ArmWorkload,
};
use crate::shell_timing_campaign_import::{
    ArmFiles, DerivedArm, RUN_DIRECTORY, VerifiedEnvironment, check_measured_interval,
    check_run_record, derive_arm_for_output, read_run, required_evidence,
};
use crate::shell_timing_campaign_observer::LiveCampaignObserver;
use crate::shell_timing_campaign_provenance::{ProvenanceTarget, verify_environment};
use crate::shell_timing_environment::{Environment, Vrr, environment_record};
use crate::shell_timing_native::{
    Request, campaign_profile, launch_prerequisites, run_observed, stage_available,
    verify_prepared, verify_sources,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// The frozen descriptor inside the calibration directory.
pub const DESCRIPTOR: &str = "calibration.json";
/// The attempt record beside each run's stage.
pub const ATTEMPT: &str = "attempt.json";
/// Declared plan order.
pub const LABELS: [&str; 2] = ["bytes", "sealed"];
const MAX_RECORD_BYTES: u64 = 1024 * 1024;
const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
const FRAMES: usize = 1000;
const RUNTIME_MS: u64 = 60_000;
const PANEL_HEIGHT: u64 = 24;

const USAGE: &str = "usage: xtask shell-timing prepare-calibration REQUEST NEW_FROZEN | run-calibration-arm FROZEN SHA256 bytes|sealed NEW_RUN_DIR | report-calibration FROZEN SHA256 BYTES_RUN SEALED_RUN";

/// Preparation input: the target, a quiet-window declaration and the two
/// prepared plans.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationRequest {
    pub schema: u8,
    pub target: ProvenanceTarget,
    pub quiet_window: String,
    pub bytes: PathBuf,
    pub sealed: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRef {
    pub label: String,
    pub path: PathBuf,
    pub sha256: String,
}

/// The frozen calibration. Plans are bound by absolute path and digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: u8,
    pub target: ProvenanceTarget,
    pub quiet_window: String,
    pub plans: Vec<PlanRef>,
}

/// One run's outcome, written beside its stage after success or failure.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationAttempt {
    pub schema: u8,
    pub descriptor_sha256: String,
    pub label: String,
    pub plan_sha256: String,
    pub outcome: AttemptOutcome,
}

/// A verified plan: its exact bytes and request.
pub struct VerifiedPlan {
    pub reference: PlanRef,
    pub bytes: Vec<u8>,
    pub request: Request,
}

/// A regular file, not a link, read with a hard bound.
pub(crate) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use rustix::fs::OFlags;
    let not_regular = || format!("{} is not a regular file", path.display());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(|e| {
            if e.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) {
                not_regular()
            } else {
                format!("{}: {e}", path.display())
            }
        })?;
    if !file
        .metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?
        .is_file()
    {
        return Err(not_regular());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} is too large", path.display()));
    }
    Ok(bytes)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn create_private(directory: &Path) -> Result<(), String> {
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(directory)
        .map_err(|e| format!("{}: {e}", directory.display()))
}

pub(crate) fn absolute(path: &Path) -> Result<&str, String> {
    let text = path.to_str().ok_or("path is not UTF-8")?;
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(format!("{text} must be absolute without parent traversal"));
    }
    Ok(text)
}

/// The fields both plans must share: sources, console, display, seat,
/// output, every artifact except the per-variant profile and peer
/// configuration, and the parsed profile policy. The campaign profile
/// comparison masks only the component executable, config and transport
/// values; validate() binds the executable to the shared peer artifact and
/// the transport to the shared files wire, so only the config path differs.
fn shared_identity(request: &Request) -> Result<Vec<u8>, String> {
    let artifacts: Vec<_> = request
        .artifacts
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "profile" | "peer-config"))
        .collect();
    serde_json::to_vec(&(
        &request.sophia_source,
        &request.integration_source,
        &request.tty,
        request.display,
        &request.input_seat,
        request.output,
        artifacts,
        campaign_profile(request)?,
    ))
    .map_err(|e| e.to_string())
}

/// Check one plan against its declared variant and the target.
fn check_plan(target: &ProvenanceTarget, label: &str, bytes: &[u8]) -> Result<Request, String> {
    let (request, probe) = verify_prepared(bytes)?;
    let source = match label {
        "bytes" => ImageSource::Bytes,
        "sealed" => ImageSource::Sealed,
        _ => return Err("calibration plans are bytes then sealed".into()),
    };
    let peer = request
        .artifacts
        .get("peer-config")
        .ok_or("plan has no peer configuration")?;
    let config = read_bounded(&peer.path, MAX_RECORD_BYTES)?;
    if crate::sha256(&config) != peer.sha256 {
        return Err("peer configuration changed".into());
    }
    let height =
        serde_json::from_slice::<serde_json::Value>(&config).map_err(|e| e.to_string())?["height"]
            .as_u64();
    if request.wire != "files"
        || request.source != source
        || request.instrumentation != "cpu-only"
        || request.frames != FRAMES
        || request.runtime_ms != RUNTIME_MS
        || probe != 0
        || height != Some(PANEL_HEIGHT)
        || !request.environment_readback
        || request.mode_millihz != target.mode_millihz
        || request.output != target.output
    {
        return Err(format!(
            "{label} plan is not a files {label} panel cpu-only calibration for the target"
        ));
    }
    verify_sources(&request)?;
    Ok(request)
}

/// Check both plans and that they differ only in variant.
pub fn check_pair(
    target: &ProvenanceTarget,
    plans: Vec<(PlanRef, Vec<u8>)>,
) -> Result<Vec<VerifiedPlan>, String> {
    target.validate()?;
    let labels: Vec<&str> = plans.iter().map(|(r, _)| r.label.as_str()).collect();
    if labels != LABELS {
        return Err("calibration plans are bytes then sealed".into());
    }
    let mut verified = Vec::new();
    for (reference, bytes) in plans {
        absolute(&reference.path)?;
        if crate::sha256(&bytes) != reference.sha256 {
            return Err(format!("{} plan digest differs", reference.label));
        }
        let request = check_plan(target, &reference.label, &bytes)?;
        verified.push(VerifiedPlan {
            reference,
            bytes,
            request,
        });
    }
    if shared_identity(&verified[0].request)? != shared_identity(&verified[1].request)? {
        return Err("calibration plans differ beyond their variant".into());
    }
    Ok(verified)
}

/// Reopen a frozen calibration by its digest and re-verify both plans.
pub fn verify_descriptor(
    directory: &Path,
    digest: &str,
) -> Result<(Descriptor, Vec<VerifiedPlan>), String> {
    absolute(directory)?;
    if !crate::hex(digest, 64) {
        return Err("full calibration digest required".into());
    }
    let bytes = read_bounded(&directory.join(DESCRIPTOR), MAX_RECORD_BYTES)?;
    if crate::sha256(&bytes) != digest {
        return Err("calibration descriptor differs from its digest".into());
    }
    let descriptor: Descriptor = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if descriptor.schema != 1
        || descriptor.quiet_window.trim().is_empty()
        || descriptor.quiet_window.len() > 4096
    {
        return Err("calibration descriptor needs schema 1 and a quiet window".into());
    }
    let plans = descriptor
        .plans
        .iter()
        .map(|reference| {
            absolute(&reference.path)?;
            Ok((
                reference.clone(),
                read_bounded(&reference.path, MAX_RECORD_BYTES)?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let verified = check_pair(&descriptor.target, plans)?;
    Ok((descriptor, verified))
}

/// Freeze a calibration. The directory is create-only.
pub fn prepare(request: &Path, frozen: &Path) -> Result<String, String> {
    let request: CalibrationRequest =
        serde_json::from_slice(&read_bounded(request, MAX_RECORD_BYTES)?)
            .map_err(|e| e.to_string())?;
    absolute(frozen)?;
    if request.schema != 1
        || request.quiet_window.trim().is_empty()
        || request.quiet_window.len() > 4096
    {
        return Err("calibration request needs schema 1 and a quiet window".into());
    }
    let mut plans = Vec::new();
    for (label, path) in LABELS.into_iter().zip([&request.bytes, &request.sealed]) {
        absolute(path)?;
        let bytes = read_bounded(path, MAX_RECORD_BYTES)?;
        plans.push((
            PlanRef {
                label: label.into(),
                path: path.clone(),
                sha256: crate::sha256(&bytes),
            },
            bytes,
        ));
    }
    let verified = check_pair(&request.target, plans)?;
    let descriptor = Descriptor {
        schema: 1,
        target: request.target,
        quiet_window: request.quiet_window,
        plans: verified.into_iter().map(|plan| plan.reference).collect(),
    };
    let bytes = serde_json::to_vec_pretty(&descriptor).map_err(|e| e.to_string())?;
    create_private(frozen)?;
    write_new(&frozen.join(DESCRIPTOR), &bytes)?;
    Ok(format!(
        "frozen calibration {} sha256={} (not a campaign or gate verdict)",
        frozen.display(),
        crate::sha256(&bytes)
    ))
}

pub(crate) fn attempt_error(error: &str) -> String {
    let error = error.trim();
    let mut end = error.len().min(4000);
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    if error.is_empty() {
        "calibration attempt failed".into()
    } else {
        error[..end].into()
    }
}

/// The campaign arm shape a calibration plan is rebuilt and verified as.
/// Projection only: it never enters a campaign manifest.
fn projected_arm(plan: &VerifiedPlan) -> Arm {
    Arm {
        id: format!("calibration-{}", plan.reference.label),
        condition: Condition::Idle,
        pair: 1,
        workload: ArmWorkload::Panel,
        measurement: Measurement::CpuOnly,
        variant: if plan.reference.label == "bytes" {
            Variant::B
        } else {
            Variant::C
        },
        plan_sha256: plan.reference.sha256.clone(),
        peer_config_sha256: plan.request.artifacts["peer-config"].sha256.clone(),
        frames: FRAMES,
        warmup: 0,
        runtime_ms: RUNTIME_MS,
    }
}

/// Run one calibration plan in a new run directory. The stage is
/// `<run>/run`; the attempt record is written beside it on success and
/// on any failure after the directory exists.
pub fn run_arm(
    frozen: &Path,
    digest: &str,
    label: &str,
    run: &Path,
) -> Result<Vec<String>, String> {
    let (descriptor, plans) = verify_descriptor(frozen, digest)?;
    let plan = plans
        .iter()
        .find(|plan| plan.reference.label == label)
        .ok_or("calibration label must be bytes or sealed")?;
    let plan_path = absolute(&plan.reference.path)?.to_owned();
    let run_path = absolute(run)?;
    let stage = run.join(RUN_DIRECTORY);
    let stage_path = stage.to_str().ok_or("run path is not UTF-8")?.to_owned();
    // Routine refusals happen before any directory exists.
    launch_prerequisites(&plan.request)?;
    stage_available(run)?;
    create_private(run)?;
    let mut observer = LiveCampaignObserver::new(Condition::Idle, descriptor.target.card.clone());
    let result = run_observed(
        &[
            "run-native".into(),
            plan_path,
            plan.reference.sha256.clone(),
            stage_path,
        ],
        &mut observer,
    )
    .and_then(|lines| {
        let mut evidence = std::collections::BTreeMap::new();
        for name in required_evidence(Condition::Idle) {
            let cap = if name == "session.log" {
                MAX_SESSION_BYTES
            } else {
                MAX_RECORD_BYTES
            };
            evidence.insert(
                name.to_owned(),
                crate::sha256(&read_bounded(&stage.join(name), cap)?),
            );
        }
        Ok((lines, evidence))
    });
    let (outcome, reply) = match result {
        Ok((mut lines, evidence)) => {
            lines.push(format!(
                "calibration {label} captured in {run_path}; report-calibration qualifies it"
            ));
            (AttemptOutcome::Captured { evidence }, Ok(lines))
        }
        Err(error) => (
            AttemptOutcome::Failed {
                error: attempt_error(&error),
            },
            Err(error),
        ),
    };
    let record = CalibrationAttempt {
        schema: 1,
        descriptor_sha256: digest.into(),
        label: label.into(),
        plan_sha256: plan.reference.sha256.clone(),
        outcome,
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
    match (write_new(&run.join(ATTEMPT), &bytes), reply) {
        (Ok(()), reply) => reply,
        (Err(write), Ok(_)) => Err(format!("calibration attempt record failed: {write}")),
        (Err(write), Err(error)) => Err(format!("{error}; attempt record failed: {write}")),
    }
}

/// Static and scheduling fact values two arms must share. Pids, process
/// start times, capture timestamps and source origins are excluded; the
/// derived timing refresh is reported, never compared.
pub(crate) fn comparable(environment: &Environment) -> serde_json::Value {
    let output = &environment.output.value;
    let renderer = &environment.renderer.value;
    let processes: Vec<_> = environment
        .processes
        .iter()
        .map(|process| {
            let scheduling = &process.scheduling.value;
            serde_json::json!({
                "role": process.role,
                "comm": scheduling.comm,
                "nice": scheduling.nice,
                "policy": scheduling.policy,
                "rt_priority": scheduling.rt_priority,
                "affinity": process.affinity.value,
            })
        })
        .collect();
    let governors: Vec<_> = environment.governors.iter().map(|g| &g.value).collect();
    serde_json::json!({
        "kernel_release": environment.kernel.release.value,
        "kernel_version": environment.kernel.version.value,
        "cpus_online": environment.cpus_online.value,
        "governors": governors,
        "drm_driver": environment.drm_driver.value,
        "output": {
            "output": output.output,
            "card": output.card,
            "connector": output.connector,
            "width": output.width,
            "height": output.height,
            "refresh_millihz": output.refresh_millihz,
            "scale_milli": output.scale_milli,
            "vrr": output.vrr,
        },
        "renderer": {
            "output": renderer.output,
            "card": renderer.card,
            "driver": renderer.driver,
            "vendor": renderer.vendor,
            "renderer": renderer.renderer,
            "version": renderer.version,
        },
        "processes": processes,
    })
}

type EnvironmentCheck<'a> =
    &'a dyn Fn(&ProvenanceTarget, &ArmFiles<'_>) -> Result<VerifiedEnvironment, String>;

/// The provider's expectation must be the frozen target.
pub(crate) fn check_target(
    target: &ProvenanceTarget,
    verified: &VerifiedEnvironment,
) -> Result<(), String> {
    let expect = &verified.expect;
    let vrr = if target.vrr { Vrr::On } else { Vrr::Off };
    if expect.connector != target.connector
        || expect.width != target.width
        || expect.height != target.height
        || expect.refresh_millihz != target.mode_millihz
        || expect.vrr != vrr
        || expect.card != target.card
        || expect.output != target.output
        || expect.scale_milli != target.scale_milli
    {
        return Err("environment expectation differs from the calibration target".into());
    }
    Ok(())
}

/// Rebuild and compare both runs. `verify` is the environment provider:
/// [`verify_environment`] for the CLI, a supplied verdict in tests.
pub fn report(
    frozen: &Path,
    digest: &str,
    runs: [&Path; 2],
    verify: EnvironmentCheck<'_>,
) -> Result<serde_json::Value, String> {
    let (descriptor, plans) = verify_descriptor(frozen, digest)?;
    if runs[0] == runs[1] {
        return Err("bytes and sealed runs must be different directories".into());
    }
    let target = &descriptor.target;
    let mut costs = Vec::new();
    let mut arms = Vec::new();
    let mut environments = Vec::new();
    for (plan, run) in plans.iter().zip(runs) {
        let label = &plan.reference.label;
        let fail = |e: String| format!("{label} run: {e}");
        absolute(run).map_err(fail)?;
        let attempt_bytes = read_bounded(&run.join(ATTEMPT), MAX_RECORD_BYTES).map_err(fail)?;
        let attempt: CalibrationAttempt =
            serde_json::from_slice(&attempt_bytes).map_err(|e| fail(e.to_string()))?;
        if attempt.schema != 1
            || attempt.descriptor_sha256 != digest
            || attempt.label != *label
            || attempt.plan_sha256 != plan.reference.sha256
        {
            return Err(fail("attempt belongs to another calibration plan".into()));
        }
        let evidence = match attempt.outcome {
            AttemptOutcome::Captured { evidence } => evidence,
            AttemptOutcome::Failed { error } => {
                return Err(fail(format!("attempt failed: {error}")));
            }
        };
        let arm = projected_arm(plan);
        let files = read_run(run, &arm, &evidence).map_err(fail)?;
        check_run_record(&plan.bytes, &files).map_err(fail)?;
        let rebuilt = derive_arm_for_output(target.output, target.mode_millihz, &arm, &files)
            .map_err(fail)?;
        let verified = verify(
            target,
            &ArmFiles {
                arm: &arm,
                plan: &files.plan,
                environment: &files.environment,
                environment_receipts: &files.environment_receipts,
                load: None,
            },
        )
        .map_err(fail)?;
        check_target(target, &verified).map_err(fail)?;
        let environment = environment_record(&verified.capture, &verified.expect).map_err(fail)?;
        check_measured_interval(&verified, rebuilt.run).map_err(fail)?;
        let DerivedArm::Cost(cost) = rebuilt.arm else {
            return Err(fail("calibration run did not rebuild as a cost arm".into()));
        };
        environments.push(comparable(&environment));
        arms.push(serde_json::json!({
            "label": label,
            "attempt_sha256": crate::sha256(&attempt_bytes),
            "evidence": evidence,
            "environment": environment,
            "run": rebuilt.run,
            "cost": cost.report,
        }));
        costs.push(*cost);
    }
    if environments[0] != environments[1] {
        return Err("bytes and sealed runs differ in static or scheduling environment".into());
    }
    let comparison = crate::shell_timing_compare::compare_source_cost(&costs[1], &costs[0])?;
    Ok(serde_json::json!({
        "schema": 1,
        "calibration_sha256": digest,
        "scope": "t338 paired files panel cpu-only calibration; report only, not a campaign condition or gate",
        "target": target,
        "arms": arms,
        "comparison": comparison,
    }))
}

pub fn run(arguments: &[String]) -> Result<Vec<String>, String> {
    match arguments {
        [command, request, frozen] if command == "prepare-calibration" => {
            Ok(vec![prepare(Path::new(request), Path::new(frozen))?])
        }
        [command, frozen, digest, label, run] if command == "run-calibration-arm" => {
            run_arm(Path::new(frozen), digest, label, Path::new(run))
        }
        [command, frozen, digest, bytes, sealed] if command == "report-calibration" => {
            let value = report(
                Path::new(frozen),
                digest,
                [Path::new(bytes), Path::new(sealed)],
                &verify_environment,
            )?;
            Ok(vec![
                serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
            ])
        }
        _ => Err(USAGE.into()),
    }
}
