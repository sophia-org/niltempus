//! Bind a frozen campaign's retained arm evidence to the paired comparison
//! inputs. Every arm is rebuilt from its retained raw capture, never from the
//! derived `native-samples.json`, and any missing, failed or mismatched arm
//! refuses the whole condition.
//!
//! Environment and load facts come only through [`ArmProvenance`], a seam for
//! root-owned authenticated delivery. The raw session log is never
//! authentication and never supplies environment facts. The attended runner's
//! provider is [`crate::shell_timing_campaign_provenance::TrustedRunnerProvenance`];
//! supplied test providers prove import mechanics only.
use crate::shell_timing::{ImageSource, Window};
use crate::shell_timing_campaign::{
    Arm, AttemptOutcome, Condition, Manifest, Measurement, Variant, Workload as ArmWorkload,
    read_finished, verify_arm, verify_frozen,
};
use crate::shell_timing_compare::{ConditionReport, CostArm, LatencyArm, PairInput, Workload};
use crate::shell_timing_environment::{
    Capture, Environment, Expect, Interval, Vrr, environment_record,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

/// Each arm's run-native output directory, under `<campaign>/<arm>/`.
pub const RUN_DIRECTORY: &str = "run";
/// Evidence every captured arm names, with digests of the retained files.
pub const BASE_EVIDENCE: [&str; 5] = [
    "plan.json",
    "result.json",
    "session.log",
    "environment.json",
    "environment-receipts.json",
];
/// Named only by load arms.
pub const LOAD_EVIDENCE: &str = "load.json";
/// run-native's session capture cap.
const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORD_BYTES: u64 = 1024 * 1024;
const PROBE_BYTES: u64 = 4_194_304;

/// The retained files of one arm, each read once and checked against the
/// digest its attempt recorded.
pub struct RunFiles {
    pub plan: Vec<u8>,
    pub result: Vec<u8>,
    pub session_log: Vec<u8>,
    pub environment: Vec<u8>,
    pub environment_receipts: Vec<u8>,
    pub load: Option<Vec<u8>>,
}

/// The exact bytes a provider may judge. The session log is left out on
/// purpose: it is unauthenticated.
pub struct ArmFiles<'a> {
    pub arm: &'a Arm,
    pub plan: &'a [u8],
    pub environment: &'a [u8],
    pub environment_receipts: &'a [u8],
    pub load: Option<&'a [u8]>,
}

/// What a root-owned provider vouches for. Test fakes are supplied verdicts.
pub struct VerifiedEnvironment {
    /// Only records the provider authenticated, plus supplied `/proc` and
    /// `/sys` sources.
    pub capture: Capture,
    /// Host and peer pids come from supervised bindings, never log text.
    pub expect: Expect,
    /// `/proc/<pid>/stat` field 22, in clock ticks.
    pub host_starttime: u64,
    pub peer_starttime: u64,
    pub clock: MonotonicDomain,
}

/// Evidence that the peer journal, the host's kernel completion stamps and
/// the capture all read one unshifted CLOCK_MONOTONIC. The reference is Linux's
/// initial time namespace inode, not a read of PID 1. Host, peer and capturer
/// `/proc/<pid>/ns/time` reads must match it; the capturer's retained
/// `timens_offsets` monotonic offset must also be zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonotonicDomain {
    pub reference: u64,
    pub host: u64,
    pub peer: u64,
    pub capturer: u64,
    pub monotonic_offset_seconds: i64,
    pub monotonic_offset_nanoseconds: i64,
}

pub struct VerifiedLoad {
    pub workers: u8,
    pub interval: Interval,
    pub measurement: Interval,
    /// Each worker's utime+stime ticks between its two observations.
    pub worker_cpu_ticks: Vec<u64>,
}

pub trait ArmProvenance {
    fn environment(&self, files: &ArmFiles<'_>) -> Result<VerifiedEnvironment, String>;
    /// Called only for load arms, after the environment verified.
    fn load(
        &self,
        files: &ArmFiles<'_>,
        environment: &VerifiedEnvironment,
    ) -> Result<VerifiedLoad, String>;
}

/// One arm rebuilt from its retained capture.
pub enum DerivedArm {
    Latency(Box<LatencyArm>),
    Cost(Box<CostArm>),
}

/// A rebuilt arm and the span its own timestamps cover: the earliest peer
/// upload or offer to the latest kernel completion or terminal.
pub struct Rebuilt {
    pub arm: DerivedArm,
    pub run: Interval,
}

pub struct ImportedArm {
    pub id: String,
    pub attempt_result_sha256: String,
    pub evidence: BTreeMap<String, String>,
    pub environment: Environment,
    pub load: Option<VerifiedLoad>,
}

pub struct ImportedCondition {
    pub campaign_schema: u8,
    pub condition: Condition,
    pub pairs: Vec<PairInput>,
    pub arms: Vec<ImportedArm>,
}

/// Import every arm of one condition, or refuse the condition.
pub fn import_condition(
    campaign: &Path,
    digest: &str,
    condition: Condition,
    provenance: &dyn ArmProvenance,
) -> Result<ImportedCondition, String> {
    let manifest = verify_frozen(campaign, digest)?;
    let mut derived = BTreeMap::new();
    let mut arms = Vec::new();
    for arm in manifest.arms.iter().filter(|a| a.condition == condition) {
        let fail = |e: String| format!("arm {}: {e}", arm.id);
        verify_arm(campaign, digest, &arm.id).map_err(fail)?;
        let (outcome, attempt_result_sha256) =
            read_finished(campaign, digest, &arm.id).map_err(fail)?;
        let evidence = match outcome {
            AttemptOutcome::Captured { evidence } => evidence,
            AttemptOutcome::Failed { error } => {
                return Err(fail(format!("attempt failed: {error}")));
            }
        };
        let arm_directory = campaign.join(&arm.id);
        let run = read_run(&arm_directory, arm, &evidence).map_err(fail)?;
        let frozen = bounded(&arm_directory.join("plan.json"), MAX_RECORD_BYTES).map_err(fail)?;
        check_run_record(&frozen, &run).map_err(fail)?;
        let rebuilt = derive_arm(&manifest, arm, &run).map_err(fail)?;
        let files = ArmFiles {
            arm,
            plan: &run.plan,
            environment: &run.environment,
            environment_receipts: &run.environment_receipts,
            load: run.load.as_deref(),
        };
        let verified = provenance.environment(&files).map_err(fail)?;
        check_expect(&manifest, &verified.expect).map_err(fail)?;
        let environment = environment_record(&verified.capture, &verified.expect).map_err(fail)?;
        let load = match arm.condition {
            Condition::Load => {
                let load = provenance.load(&files, &verified).map_err(fail)?;
                check_load(&manifest, &verified.expect, &load).map_err(fail)?;
                Some(load)
            }
            Condition::Idle => None,
        };
        check_measured_interval(&verified, rebuilt.run).map_err(fail)?;
        derived.insert(arm.id.clone(), rebuilt.arm);
        arms.push(ImportedArm {
            id: arm.id.clone(),
            attempt_result_sha256,
            evidence,
            environment,
            load,
        });
    }
    let pairs = pair_inputs(&manifest, condition, derived)?;
    Ok(ImportedCondition {
        campaign_schema: manifest.schema,
        condition,
        pairs,
        arms,
    })
}

/// The gates are `condition_report`'s alone.
pub fn condition_verdict(imported: &ImportedCondition) -> Result<ConditionReport, String> {
    match imported.campaign_schema {
        2 => crate::shell_timing_compare::condition_report(&imported.pairs),
        3 => crate::shell_timing_compare::isb_condition_report(&imported.pairs),
        _ => Err("unknown campaign schema".into()),
    }
}

/// The names an arm must record: exactly the base set, plus the load record
/// on load arms only.
pub fn required_evidence(condition: Condition) -> BTreeSet<&'static str> {
    let mut names: BTreeSet<_> = BASE_EVIDENCE.into_iter().collect();
    if condition == Condition::Load {
        names.insert(LOAD_EVIDENCE);
    }
    names
}

/// Read each named run file once, bounded, and check its recorded digest.
/// An idle arm with a load record on disk is refused even if unnamed.
pub fn read_run(
    arm_directory: &Path,
    arm: &Arm,
    evidence: &BTreeMap<String, String>,
) -> Result<RunFiles, String> {
    let required = required_evidence(arm.condition);
    let named: BTreeSet<&str> = evidence.keys().map(String::as_str).collect();
    if named != required {
        return Err("captured evidence names differ from the required set".into());
    }
    let run = arm_directory.join(RUN_DIRECTORY);
    if arm.condition == Condition::Idle
        && std::fs::symlink_metadata(run.join(LOAD_EVIDENCE)).is_ok()
    {
        return Err("idle arm retains a load record".into());
    }
    let file = |name: &str, limit: u64| -> Result<Vec<u8>, String> {
        let bytes = bounded(&run.join(name), limit)?;
        if crate::sha256(&bytes) != evidence[name] {
            return Err(format!("{name} differs from its recorded digest"));
        }
        Ok(bytes)
    };
    Ok(RunFiles {
        plan: file("plan.json", MAX_RECORD_BYTES)?,
        result: file("result.json", MAX_RECORD_BYTES)?,
        session_log: file("session.log", MAX_SESSION_BYTES)?,
        environment: file("environment.json", MAX_RECORD_BYTES)?,
        environment_receipts: file("environment-receipts.json", MAX_RECORD_BYTES)?,
        load: match arm.condition {
            Condition::Load => Some(file(LOAD_EVIDENCE, MAX_RECORD_BYTES)?),
            Condition::Idle => None,
        },
    })
}

/// A regular file, not a link, read with a hard size bound. The leaf is
/// opened without following links and without blocking, and the type is
/// taken from the opened descriptor, so a swapped leaf cannot be read.
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use rustix::fs::OFlags;
    use std::os::unix::fs::OpenOptionsExt;
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
    let metadata = file
        .metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.file_type().is_file() {
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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RunResult {
    schema: u8,
    status: String,
    error: Option<String>,
}

/// The run used the frozen plan and run-native finished capturing.
pub fn check_run_record(frozen_plan: &[u8], run: &RunFiles) -> Result<(), String> {
    if run.plan != frozen_plan {
        return Err("run plan differs from the frozen plan".into());
    }
    let result: RunResult = serde_json::from_slice(&run.result).map_err(|e| e.to_string())?;
    if result.schema != 1 || result.status != "captured_only" || result.error.is_some() {
        return Err("run did not finish capturing".into());
    }
    Ok(())
}

fn identity(variant: Variant) -> (&'static str, ImageSource) {
    match variant {
        Variant::A => ("ipc", ImageSource::Bytes),
        Variant::B => ("files", ImageSource::Bytes),
        Variant::C => ("files", ImageSource::Isb),
    }
}

fn workload(workload: ArmWorkload) -> (Workload, u64) {
    match workload {
        ArmWorkload::Panel => (Workload::Panel, 0),
        ArmWorkload::Bulk => (Workload::Bulk, PROBE_BYTES),
    }
}

/// Campaign completions must carry kernel page-flip timestamps.
fn kernel_completions_only(host: &str) -> Result<(), String> {
    for line in host.lines() {
        let mut tokens = line.split(' ');
        if tokens.next() != Some("sophia_shell_native_completion") {
            continue;
        }
        let tokens: BTreeSet<&str> = tokens.collect();
        if !tokens.contains("timestamp_source=kernel")
            || !tokens.contains("missing_kernel_timestamp=0")
        {
            return Err("native completion lacks a kernel timestamp".into());
        }
    }
    Ok(())
}

/// Rebuild one arm from its retained session capture.
pub fn derive_arm(manifest: &Manifest, arm: &Arm, run: &RunFiles) -> Result<Rebuilt, String> {
    derive_arm_for_output(manifest.environment.output, manifest.mode_millihz, arm, run)
}

/// Rebuild a frozen arm for an explicit output and nominal mode. Calibration
/// callers validate their own two-arm descriptor before using this shared path;
/// this does not validate or substitute for a campaign manifest.
pub fn derive_arm_for_output(
    output: u64,
    mode_millihz: u64,
    arm: &Arm,
    run: &RunFiles,
) -> Result<Rebuilt, String> {
    let text = std::str::from_utf8(&run.session_log).map_err(|e| e.to_string())?;
    let (client, host) = crate::shell_timing_native::split_capture(text)?;
    let metrics = crate::shell_timing::collect_metrics(&client, arm.frames)?;
    let (wire, source) = identity(arm.variant);
    let (workload, probe_bytes) = workload(arm.workload);
    if metrics.workload != crate::shell_timing::WorkloadMode::Fresh {
        return Err(
            "campaign arms declare fresh workloads; a retained journal cannot supply one".into(),
        );
    }
    if metrics.wire != wire || metrics.source != source || metrics.probe_bytes != probe_bytes {
        return Err("captured upload workload differs from the arm".into());
    }
    if metrics.offers.iter().any(|offer| offer.output != output) {
        return Err("captured output differs from the frozen output".into());
    }
    if text.contains("sophia_comparator_progress ") {
        return Err("progress diagnostics cannot supply cost or latency arms".into());
    }
    kernel_completions_only(&host)?;
    let samples = crate::shell_timing::join(&host, &metrics.offers, mode_millihz, arm.frames)?;
    let start = metrics
        .uploads
        .iter()
        .map(|u| u.upload.begin_usec)
        .chain(metrics.offers.iter().map(|o| o.submitted_monotonic_usec))
        .min()
        .ok_or("run has no uploads or offers")?;
    let end = samples
        .iter()
        .map(|s| s.completed_monotonic_usec)
        .chain(metrics.terminal_usec.iter().copied())
        .max()
        .ok_or("run has no completions")?;
    let run_interval = Interval {
        start_usec: start,
        end_usec: end,
    };
    let label = arm.id.clone();
    let arm = match arm.measurement {
        Measurement::Latency => {
            let measured = arm
                .frames
                .checked_sub(arm.warmup)
                .filter(|&n| n > 0)
                .ok_or("warmup must leave measured frames")?;
            let window = Window {
                warmup: arm.warmup,
                measured,
            };
            let latency =
                crate::shell_timing::latency_window(&host, &client, mode_millihz, window)?;
            DerivedArm::Latency(Box::new(LatencyArm {
                label,
                wire: wire.into(),
                source,
                workload,
                latency,
            }))
        }
        Measurement::CpuOnly | Measurement::RustGlobal => {
            let expected = match arm.measurement {
                Measurement::CpuOnly => "cpu-only",
                _ => "rust-global",
            };
            let report =
                crate::shell_timing_cost::measure_journal(text, text, &client, arm.frames)?;
            if report.instrumentation != expected {
                return Err("counter mode differs from the arm".into());
            }
            DerivedArm::Cost(Box::new(CostArm {
                label,
                wire: wire.into(),
                source,
                workload,
                report,
            }))
        }
    };
    Ok(Rebuilt {
        arm,
        run: run_interval,
    })
}

/// The provider's expectation must be the frozen campaign's target.
pub fn check_expect(manifest: &Manifest, expect: &Expect) -> Result<(), String> {
    let target = &manifest.environment;
    let vrr = if manifest.vrr { Vrr::On } else { Vrr::Off };
    if expect.connector != manifest.connector
        || expect.width != manifest.width
        || expect.height != manifest.height
        || expect.refresh_millihz != manifest.mode_millihz
        || expect.vrr != vrr
        || expect.card != target.card
        || expect.output != target.output
        || expect.scale_milli != target.scale_milli
    {
        return Err("environment expectation differs from the frozen campaign".into());
    }
    Ok(())
}

/// The load the provider qualified covered this arm's measurement.
pub fn check_load(manifest: &Manifest, expect: &Expect, load: &VerifiedLoad) -> Result<(), String> {
    let (measured, interval) = (load.measurement, load.interval);
    if load.workers != manifest.load_workers
        || measured != expect.measurement
        || interval.start_usec > measured.start_usec
        || interval.end_usec < measured.end_usec
    {
        return Err("qualified load does not cover the measurement".into());
    }
    Ok(())
}

/// The run's own span lies inside the declared measurement, compared only
/// when every timestamp source is proven to read one unshifted clock.
pub fn check_measured_interval(
    verified: &VerifiedEnvironment,
    run: Interval,
) -> Result<(), String> {
    let clock = verified.clock;
    let inodes = [clock.reference, clock.host, clock.peer, clock.capturer];
    if clock.reference != crate::shell_timing_process_binding::INITIAL_TIME_NAMESPACE_INODE
        || inodes.iter().any(|&inode| inode != clock.reference)
        || clock.monotonic_offset_seconds != 0
        || clock.monotonic_offset_nanoseconds != 0
    {
        return Err("measured interval clock domain is unproven".into());
    }
    let measured = verified.expect.measurement;
    if run.start_usec > run.end_usec
        || run.start_usec < measured.start_usec
        || run.end_usec > measured.end_usec
    {
        return Err("run timestamps fall outside the declared measurement".into());
    }
    Ok(())
}

/// Group a condition's rebuilt arms into its three pairs, in manifest order.
pub fn pair_inputs(
    manifest: &Manifest,
    condition: Condition,
    mut derived: BTreeMap<String, DerivedArm>,
) -> Result<Vec<PairInput>, String> {
    let arms: Vec<&Arm> = manifest
        .arms
        .iter()
        .filter(|a| a.condition == condition)
        .collect();
    let ids: BTreeSet<&str> = arms.iter().map(|a| a.id.as_str()).collect();
    if derived.len() != ids.len() || !derived.keys().all(|id| ids.contains(id.as_str())) {
        return Err("rebuilt arms differ from the condition's arms".into());
    }
    let mut pairs = Vec::new();
    for index in 1..=3u8 {
        let mut pair = PairInput {
            index: usize::from(index),
            order: Vec::new(),
            latency: Vec::new(),
            cost: Vec::new(),
        };
        for arm in arms.iter().filter(|a| a.pair == index) {
            pair.order.push(arm.id.clone());
            match derived.remove(&arm.id).ok_or("rebuilt arm missing")? {
                DerivedArm::Latency(latency) => pair.latency.push(*latency),
                DerivedArm::Cost(cost) => pair.cost.push(*cost),
            }
        }
        pairs.push(pair);
    }
    if !derived.is_empty() {
        return Err("an arm belongs to no pair".into());
    }
    Ok(pairs)
}
