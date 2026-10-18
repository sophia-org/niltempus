//! Fixed nine-arm bulk diagnostic. This is not the acceptance campaign.
//! Plans, order and attempts are frozen; a failed arm cannot be replaced.
//! Only the attended runner launches, and reports rederive retained raw data.
use crate::shell_timing::ImageSource;
use crate::shell_timing_calibration::{
    PlanRef, VerifiedPlan, absolute, attempt_error, check_target, comparable, create_private,
    read_bounded, write_new,
};
use crate::shell_timing_campaign::{
    Arm, AttemptOutcome, Condition, Measurement, Variant, Workload,
};
use crate::shell_timing_campaign_import::{
    ArmFiles, DerivedArm, VerifiedEnvironment, check_measured_interval, check_run_record,
    derive_arm_for_output, read_run, required_evidence,
};
use crate::shell_timing_campaign_observer::LiveCampaignObserver;
use crate::shell_timing_campaign_provenance::{ProvenanceTarget, verify_environment};
use crate::shell_timing_cost::{Bounds, Report};
use crate::shell_timing_environment::environment_record;
use crate::shell_timing_native::{
    Request, campaign_profile, launch_prerequisites, run_observed, stage_available,
    verify_prepared, verify_sources,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const CAP: u64 = 1024 * 1024;
const LOG_CAP: u64 = 64 * 1024 * 1024;
const DESCRIPTOR: &str = "cost-pilot.json";
const USAGE: &str = "usage: shell-timing prepare-cost-pilot REQUEST NEW_FROZEN | run-cost-pilot-arm FROZEN SHA ARM | report-cost-pilot FROZEN SHA";
pub const PLAN_IDS: [&str; 6] = [
    "cpu-ipc",
    "cpu-bytes",
    "cpu-sealed",
    "alloc-ipc",
    "alloc-bytes",
    "alloc-sealed",
];
/// Exact order, not selected after seeing results.
pub const ARM_IDS: [&str; 9] = [
    "cpu-1-ipc",
    "cpu-1-bytes",
    "cpu-1-sealed",
    "cpu-2-sealed",
    "cpu-2-bytes",
    "cpu-2-ipc",
    "alloc-1-ipc",
    "alloc-1-bytes",
    "alloc-1-sealed",
];
const PLAN_INDEX: [usize; 9] = [0, 1, 2, 2, 1, 0, 3, 4, 5];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PilotRequest {
    pub schema: u8,
    pub target: ProvenanceTarget,
    pub quiet_window: String,
    /// Exactly PLAN_IDS, each naming a prepared plan.
    pub plans: BTreeMap<String, PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: u8,
    pub target: ProvenanceTarget,
    pub quiet_window: String,
    pub plans: Vec<PlanRef>,
    pub order: Vec<String>,
}

fn declaration(schema: u8, target: &ProvenanceTarget, quiet: &str) -> Result<(), String> {
    target.validate()?;
    if schema != 1 || quiet.trim().is_empty() || quiet.len() > 4096 {
        return Err("cost pilot needs schema 1 and a bounded quiet-window declaration".into());
    }
    Ok(())
}

/// Sources and policy must match across instrumentation builds. Executable
/// hashes for Sophia/peer may differ only across the two instrumentation sets.
fn identity(request: &Request, across_modes: bool) -> Result<Vec<u8>, String> {
    let artifacts: Vec<_> = request
        .artifacts
        .iter()
        .filter(|(key, _)| {
            !(matches!(key.as_str(), "profile" | "peer-config")
                || across_modes && matches!(key.as_str(), "sophia" | "peer"))
        })
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

pub fn check_plans(
    target: &ProvenanceTarget,
    inputs: Vec<(PlanRef, Vec<u8>)>,
) -> Result<Vec<VerifiedPlan>, String> {
    target.validate()?;
    if inputs
        .iter()
        .map(|(p, _)| p.label.as_str())
        .collect::<Vec<_>>()
        != PLAN_IDS
    {
        return Err("cost pilot requires the six plans in canonical order".into());
    }
    let mut plans = Vec::new();
    for (index, (reference, bytes)) in inputs.into_iter().enumerate() {
        absolute(&reference.path)?;
        if reference.sha256 != crate::sha256(&bytes) {
            return Err("pilot plan digest differs".into());
        }
        let (request, probe) = verify_prepared(&bytes)?;
        let peer = &request.artifacts["peer-config"];
        let config = read_bounded(&peer.path, CAP)?;
        if crate::sha256(&config) != peer.sha256 {
            return Err("pilot peer configuration changed".into());
        }
        let config: serde_json::Value =
            serde_json::from_slice(&config).map_err(|e| e.to_string())?;
        let variant = index % 3;
        let wire = if variant == 0 { "ipc" } else { "files" };
        let source = if variant == 2 {
            ImageSource::Sealed
        } else {
            ImageSource::Bytes
        };
        let mode = if index < 3 { "cpu-only" } else { "rust-global" };
        if request.wire != wire
            || request.source != source
            || request.instrumentation != mode
            || request.frames != 1000
            || request.runtime_ms != 60_000
            || probe != 4_194_304
            || config["height"].as_u64() != Some(24)
            || !request.environment_readback
            || request.mode_millihz != target.mode_millihz
            || request.output != target.output
        {
            return Err(format!(
                "{} is not its declared 1000-frame bulk pilot arm",
                reference.label
            ));
        }
        verify_sources(&request)?;
        plans.push(VerifiedPlan {
            reference,
            bytes,
            request,
        });
    }
    for group in [0..3, 3..6] {
        let baseline = identity(&plans[group.start].request, false)?;
        for index in group {
            if identity(&plans[index].request, false)? != baseline {
                return Err(
                    "pilot plans differ beyond their transport within an instrumentation set"
                        .into(),
                );
            }
        }
    }
    if identity(&plans[0].request, true)? != identity(&plans[3].request, true)? {
        return Err("pilot instrumentation sets differ in sources or policy".into());
    }
    Ok(plans)
}

pub fn prepare(request: &Path, frozen: &Path) -> Result<String, String> {
    absolute(frozen)?;
    let request: PilotRequest =
        serde_json::from_slice(&read_bounded(request, CAP)?).map_err(|e| e.to_string())?;
    declaration(request.schema, &request.target, &request.quiet_window)?;
    if request.plans.len() != PLAN_IDS.len()
        || PLAN_IDS.iter().any(|id| !request.plans.contains_key(*id))
    {
        return Err("cost pilot requires exactly six named plans".into());
    }
    let inputs = PLAN_IDS
        .iter()
        .map(|id| {
            let path = &request.plans[*id];
            absolute(path)?;
            let bytes = read_bounded(path, CAP)?;
            Ok((
                PlanRef {
                    label: (*id).into(),
                    path: path.clone(),
                    sha256: crate::sha256(&bytes),
                },
                bytes,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let plans = check_plans(&request.target, inputs)?;
    let descriptor = Descriptor {
        schema: 1,
        target: request.target,
        quiet_window: request.quiet_window,
        plans: plans.into_iter().map(|p| p.reference).collect(),
        order: ARM_IDS.iter().map(|s| (*s).into()).collect(),
    };
    let bytes = serde_json::to_vec_pretty(&descriptor).map_err(|e| e.to_string())?;
    create_private(frozen)?;
    write_new(&frozen.join(DESCRIPTOR), &bytes)?;
    Ok(format!(
        "frozen bulk cost pilot {} sha256={} (diagnostic only)",
        frozen.display(),
        crate::sha256(&bytes)
    ))
}

pub fn verify_descriptor(
    frozen: &Path,
    digest: &str,
) -> Result<(Descriptor, Vec<VerifiedPlan>), String> {
    absolute(frozen)?;
    let bytes = read_bounded(&frozen.join(DESCRIPTOR), CAP)?;
    if !crate::hex(digest, 64) || crate::sha256(&bytes) != digest {
        return Err("cost pilot descriptor digest differs".into());
    }
    let d: Descriptor = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    declaration(d.schema, &d.target, &d.quiet_window)?;
    if d.order != ARM_IDS {
        return Err("cost pilot order differs".into());
    }
    let inputs = d
        .plans
        .iter()
        .map(|p| Ok((p.clone(), read_bounded(&p.path, CAP)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let plans = check_plans(&d.target, inputs)?;
    Ok((d, plans))
}

fn projected(index: usize, plan: &VerifiedPlan) -> Arm {
    Arm {
        id: ARM_IDS[index].into(),
        condition: Condition::Idle,
        pair: if (3..6).contains(&index) { 2 } else { 1 },
        workload: Workload::Bulk,
        measurement: if index < 6 {
            Measurement::CpuOnly
        } else {
            Measurement::RustGlobal
        },
        variant: match PLAN_INDEX[index] % 3 {
            0 => Variant::A,
            1 => Variant::B,
            _ => Variant::C,
        },
        plan_sha256: plan.reference.sha256.clone(),
        peer_config_sha256: plan.request.artifacts["peer-config"].sha256.clone(),
        frames: 1000,
        warmup: 0,
        runtime_ms: 60_000,
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    schema: u8,
    pilot_sha256: String,
    arm: String,
    index: usize,
    plan_sha256: String,
    previous_result_sha256: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Finished {
    schema: u8,
    claim_sha256: String,
    outcome: AttemptOutcome,
}

fn claim(digest: &str, index: usize, plan: &VerifiedPlan, previous: Option<String>) -> Claim {
    Claim {
        schema: 1,
        pilot_sha256: digest.into(),
        arm: ARM_IDS[index].into(),
        index,
        plan_sha256: plan.reference.sha256.clone(),
        previous_result_sha256: previous,
    }
}

fn finished(frozen: &Path, expected: &Claim) -> Result<(Finished, String), String> {
    let dir = frozen.join(&expected.arm);
    let bytes = read_bounded(&dir.join("claim.json"), CAP)?;
    let retained: Claim = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if retained != *expected {
        return Err("pilot attempt claim differs from its predecessor or plan".into());
    }
    let result = read_bounded(&dir.join("attempt.json"), CAP)?;
    let record: Finished = serde_json::from_slice(&result).map_err(|e| e.to_string())?;
    if record.schema != 1 || record.claim_sha256 != crate::sha256(&bytes) {
        return Err("pilot attempt result differs from claim".into());
    }
    if let AttemptOutcome::Failed { error } = &record.outcome {
        return Err(format!("{} failed: {error}", expected.arm));
    }
    Ok((record, crate::sha256(&result)))
}

fn history(
    frozen: &Path,
    digest: &str,
    plans: &[VerifiedPlan],
    count: usize,
) -> Result<Option<String>, String> {
    let mut previous = None;
    for index in 0..count {
        let expected = claim(digest, index, &plans[PLAN_INDEX[index]], previous);
        previous = Some(finished(frozen, &expected)?.1);
    }
    Ok(previous)
}

pub fn run_arm(frozen: &Path, digest: &str, id: &str) -> Result<Vec<String>, String> {
    let (descriptor, plans) = verify_descriptor(frozen, digest)?;
    let index = ARM_IDS
        .iter()
        .position(|v| *v == id)
        .ok_or("unknown cost pilot arm")?;
    let plan = &plans[PLAN_INDEX[index]];
    let previous = history(frozen, digest, &plans, index)?;
    let dir = frozen.join(id);
    let stage = dir.join("run");
    let plan_path = absolute(&plan.reference.path)?.to_owned();
    let stage_path = absolute(&stage)?.to_owned();
    launch_prerequisites(&plan.request)?;
    stage_available(&dir)?;
    let expected = claim(digest, index, plan, previous);
    let bytes = serde_json::to_vec(&expected).map_err(|e| e.to_string())?;
    create_private(&dir)?;
    write_new(&dir.join("claim.json"), &bytes)?;
    let mut observer = LiveCampaignObserver::new(Condition::Idle, descriptor.target.card);
    let result = run_observed(
        &[
            "run-native".into(),
            plan_path,
            plan.reference.sha256.clone(),
            stage_path,
        ],
        &mut observer,
    )
    .and_then(|mut lines| {
        let mut evidence = BTreeMap::new();
        for name in required_evidence(Condition::Idle) {
            let cap = if name == "session.log" { LOG_CAP } else { CAP };
            evidence.insert(
                name.into(),
                crate::sha256(&read_bounded(&stage.join(name), cap)?),
            );
        }
        lines.push(format!(
            "{id} captured; report-cost-pilot qualifies the complete nine-arm pilot"
        ));
        Ok((lines, evidence))
    });
    let (outcome, reply) = match result {
        Ok((lines, evidence)) => (AttemptOutcome::Captured { evidence }, Ok(lines)),
        Err(error) => (
            AttemptOutcome::Failed {
                error: attempt_error(&error),
            },
            Err(error),
        ),
    };
    let recorded = Finished {
        schema: 1,
        claim_sha256: crate::sha256(&bytes),
        outcome,
    };
    let write = serde_json::to_vec_pretty(&recorded)
        .map_err(|e| e.to_string())
        .and_then(|b| write_new(&dir.join("attempt.json"), &b));
    match (write, reply) {
        (Ok(()), result) => result,
        (Err(error), Ok(_)) => Err(format!("pilot attempt record failed: {error}")),
        (Err(write), Err(error)) => Err(format!("{error}; pilot attempt record failed: {write}")),
    }
}

/// Cross-bound differences are an interval; same-side subtractions are not.
fn difference(candidate: &Bounds, baseline: &Bounds) -> serde_json::Value {
    serde_json::json!({"lower": i128::from(candidate.lower) - i128::from(baseline.upper),
        "upper": i128::from(candidate.upper) - i128::from(baseline.lower)})
}

pub fn comparisons(costs: &[Report]) -> Result<serde_json::Value, String> {
    if costs.len() != 9 {
        return Err("cost pilot needs all nine reports".into());
    }
    let mut rows = Vec::new();
    for (block, a, b, c) in [("cpu-1", 0, 1, 2), ("cpu-2", 5, 4, 3), ("alloc-1", 6, 7, 8)] {
        let metric = |index: usize| -> Result<Bounds, String> {
            let report = &costs[index];
            let n = report
                .normalized
                .as_ref()
                .ok_or("pilot cost is not normalized")?;
            if report
                .diagnostics
                .as_ref()
                .is_none_or(|d| d.counter_schema != 3)
            {
                return Err("pilot requires schema-3 process and fault counters".into());
            }
            let mode = if index < 6 { "cpu-only" } else { "rust-global" };
            if report.instrumentation != mode
                || n.frame_count != 1000
                || n.presentation_bytes != 1000 * 1920 * 24 * 4
                || n.transfer_probe_bytes != 1000 * 4_194_304
                || n.source
                    != if matches!(index, 2 | 3 | 8) {
                        ImageSource::Sealed
                    } else {
                        ImageSource::Bytes
                    }
                || n.transferred_bytes != n.presentation_bytes + n.transfer_probe_bytes
                || n.uploaded_bytes != n.transferred_bytes
            {
                return Err("pilot reports differ from the declared bulk workload or mode".into());
            }
            let bounds = if index < 6 {
                n.cpu_ns_per_uploaded_mib
            } else {
                n.allocation_operations_per_frame
                    .ok_or("pilot allocation arm has no counts")?
            };
            if bounds.lower > bounds.upper {
                return Err("pilot metric bounds are inverted".into());
            }
            Ok(bounds)
        };
        for (name, x, y) in [
            ("bytes_vs_ipc", b, a),
            ("sealed_vs_ipc", c, a),
            ("sealed_vs_bytes", c, b),
        ] {
            let candidate = metric(x)?;
            let baseline = metric(y)?;
            rows.push(serde_json::json!({"block":block, "comparison":name, "outcome":"reported_only", "process_scope":"session_and_peer_combined",
                "measure":if a < 6 {"cpu_ns_per_logical_mib"} else {"allocation_operations_per_frame"},
                "candidate":candidate,"baseline":baseline,"difference":difference(&candidate,&baseline)}));
        }
    }
    let spread = [("ipc",0,5),("bytes",1,4),("sealed",2,3)].into_iter().map(|(role,first,second)| {
        let a = costs[first].normalized.as_ref().unwrap().cpu_ns_per_uploaded_mib;
        let b = costs[second].normalized.as_ref().unwrap().cpu_ns_per_uploaded_mib;
        serde_json::json!({"variant":role,"first":a,"second":b,"second_minus_first":difference(&b,&a)})
    }).collect::<Vec<_>>();
    Ok(
        serde_json::json!({"comparisons":rows,"cpu_block_variation":spread,
        "scope":"reported only; counter bounds are not confidence intervals; one allocation observation per variant"}),
    )
}

type EnvironmentCheck<'a> =
    &'a dyn Fn(&ProvenanceTarget, &ArmFiles<'_>) -> Result<VerifiedEnvironment, String>;

pub fn report(
    frozen: &Path,
    digest: &str,
    verify: EnvironmentCheck<'_>,
) -> Result<serde_json::Value, String> {
    let (descriptor, plans) = verify_descriptor(frozen, digest)?;
    let mut previous = None;
    let mut costs = Vec::new();
    let mut arms = Vec::new();
    let mut environment_value = None;
    for index in 0..9 {
        let plan = &plans[PLAN_INDEX[index]];
        let expected = claim(digest, index, plan, previous);
        let (record, result_digest) = finished(frozen, &expected)?;
        previous = Some(result_digest.clone());
        let AttemptOutcome::Captured { evidence } = record.outcome else {
            unreachable!()
        };
        let arm = projected(index, plan);
        let files = read_run(&frozen.join(&arm.id), &arm, &evidence)?;
        check_run_record(&plan.bytes, &files)?;
        let rebuilt = derive_arm_for_output(
            descriptor.target.output,
            descriptor.target.mode_millihz,
            &arm,
            &files,
        )?;
        let verified = verify(
            &descriptor.target,
            &ArmFiles {
                arm: &arm,
                plan: &files.plan,
                environment: &files.environment,
                environment_receipts: &files.environment_receipts,
                load: None,
            },
        )?;
        check_target(&descriptor.target, &verified)?;
        let env = environment_record(&verified.capture, &verified.expect)?;
        check_measured_interval(&verified, rebuilt.run)?;
        let value = comparable(&env);
        if environment_value.as_ref().is_some_and(|old| old != &value) {
            return Err("pilot arms differ in static or scheduling environment".into());
        }
        environment_value = Some(value);
        let DerivedArm::Cost(cost) = rebuilt.arm else {
            return Err("pilot arm is not a cost capture".into());
        };
        arms.push(
            serde_json::json!({"id":arm.id,"attempt_sha256":result_digest,"evidence":evidence,
            "environment":env,"run":rebuilt.run,"cost":cost.report}),
        );
        costs.push(cost.report);
    }
    Ok(
        serde_json::json!({"schema":1,"pilot_sha256":digest,"target":descriptor.target,"arms":arms,
        "summary":comparisons(&costs)?,"scope":"t331 bulk diagnostic; not a campaign condition, B9 verdict or transport selection"}),
    )
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    match args {
        [command, request, frozen] if command == "prepare-cost-pilot" => {
            Ok(vec![prepare(Path::new(request), Path::new(frozen))?])
        }
        [command, frozen, digest, arm] if command == "run-cost-pilot-arm" => {
            run_arm(Path::new(frozen), digest, arm)
        }
        [command, frozen, digest] if command == "report-cost-pilot" => {
            let value = report(Path::new(frozen), digest, &verify_environment)?;
            Ok(vec![
                serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
            ])
        }
        _ => Err(USAGE.into()),
    }
}
