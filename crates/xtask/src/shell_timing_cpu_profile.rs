//! Attended, diagnostic user-space call-stack capture. Cost counters collected
//! under perf are not acceptance measurements. The normal native runner owns
//! launch, identity, console restoration and environment evidence unchanged.
use crate::shell_timing_calibration::{absolute, create_private, read_bounded, write_new};
use crate::shell_timing_campaign::{Arm, Condition, Measurement, Variant, Workload};
use crate::shell_timing_campaign_import::{
    ArmFiles, DerivedArm, check_measured_interval, check_run_record, derive_arm_for_output,
    read_run, required_evidence,
};
use crate::shell_timing_campaign_observer::LiveCampaignObserver;
use crate::shell_timing_campaign_provenance::{ProvenanceTarget, verify_environment};
use crate::shell_timing_environment::{Interval, environment_record};
use crate::shell_timing_native::{self, Artifact, Request};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

const CAP: u64 = 1024 * 1024;
const PERF_CAP: u64 = 512 * CAP;
const SCOPE: &str = "diagnostic user-space samples only; no kernel CPU attribution, no cost or latency gate verdict; sampling perturbs execution";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: u8,
    pub plan: Artifact,
    pub perf: Artifact,
    pub target: ProvenanceTarget,
}

fn digest(path: &Path, cap: u64) -> Result<String, String> {
    Ok(crate::sha256(&read_bounded(path, cap)?))
}

/// This projection only reuses raw-evidence checking; it is not a campaign arm.
pub fn projected(request: &Request, probe: u64, plan_sha256: &str) -> Result<Arm, String> {
    if request.instrumentation != "cpu-only"
        || !request.environment_readback
        || request.runtime_ms != 60_000
        || request.frames != 1000
    {
        return Err(
            "CPU profiling requires an observed 1000-frame, 60-second cpu-only plan".into(),
        );
    }
    let variant = match (request.wire.as_str(), request.source) {
        ("ipc", crate::shell_timing::ImageSource::Bytes) => Variant::A,
        ("files", crate::shell_timing::ImageSource::Bytes) => Variant::B,
        ("files", crate::shell_timing::ImageSource::Isb) => Variant::C,
        _ => return Err("unsupported profiling transport".into()),
    };
    let workload = match probe {
        0 => Workload::Panel,
        4_194_304 => Workload::Bulk,
        _ => return Err("unsupported profiling probe".into()),
    };
    Ok(Arm {
        id: "cpu-profile".into(),
        condition: Condition::Idle,
        pair: 1,
        workload,
        measurement: Measurement::CpuOnly,
        variant,
        plan_sha256: plan_sha256.into(),
        peer_config_sha256: request
            .artifacts
            .get("peer-config")
            .ok_or("missing peer configuration")?
            .sha256
            .clone(),
        frames: request.frames,
        warmup: 0,
        runtime_ms: request.runtime_ms,
    })
}

fn verify(profile: &Profile) -> Result<(Vec<u8>, Request, Arm), String> {
    if profile.schema != 1 {
        return Err("unknown CPU profile schema".into());
    }
    profile.target.validate()?;
    absolute(&profile.plan.path)?;
    absolute(&profile.perf.path)?;
    let bytes = read_bounded(&profile.plan.path, CAP)?;
    if crate::sha256(&bytes) != profile.plan.sha256
        || digest(&profile.perf.path, 128 * CAP)? != profile.perf.sha256
    {
        return Err("profile plan or perf executable changed".into());
    }
    let (request, probe) = shell_timing_native::verify_prepared(&bytes)?;
    if request.output != profile.target.output
        || request.mode_millihz != profile.target.mode_millihz
    {
        return Err("profile target differs from plan".into());
    }
    let arm = projected(&request, probe, &profile.plan.sha256)?;
    Ok((bytes, request, arm))
}

/// Fixed sampling policy. Inheritance captures threads/descendants; reports
/// select bound outer TGIDs, never names or the peer's inner PID namespace.
pub fn record_arguments(
    data: &Path,
    runner: &Path,
    descriptor: &Path,
    sha: &str,
    stage: &Path,
) -> Vec<String> {
    vec![
        "record".into(),
        "--event=cpu-clock:u".into(),
        "--freq=199".into(),
        "--call-graph=dwarf,8192".into(),
        "--clockid=mono".into(),
        "--timestamp".into(),
        "--output".into(),
        data.display().to_string(),
        "--".into(),
        runner.display().to_string(),
        "shell-timing".into(),
        "capture-cpu-profile".into(),
        descriptor.display().to_string(),
        sha.into(),
        stage.display().to_string(),
    ]
}

/// CLOCK_MONOTONIC absolute seconds, with exact microsecond precision.
pub fn time_filter(interval: &Interval) -> Result<String, String> {
    if interval.start_usec >= interval.end_usec {
        return Err("empty profiling interval".into());
    }
    let seconds = |v: u64| format!("{}.{:06}", v / 1_000_000, v % 1_000_000);
    Ok(format!(
        "{},{}",
        seconds(interval.start_usec),
        seconds(interval.end_usec)
    ))
}

/// Keep thread identity in the histogram key: filtering by TGID alone does
/// not prevent perf from merging shared-symbol callchains across processes.
/// Relative percentages use only the selected process's samples. All entries
/// remain visible so the sample total can be checked against `perf script`.
pub fn report_arguments(data: &Path, pid: u32, interval: &Interval) -> Result<Vec<String>, String> {
    if pid == 0 {
        return Err("invalid profile process".into());
    }
    Ok(vec![
        "report".into(),
        "--stdio".into(),
        "--stdio-color=never".into(),
        "--no-children".into(),
        "--show-nr-samples".into(),
        "--percent-limit=0".into(),
        "--percentage=relative".into(),
        "--sort=pid,dso,symbol".into(),
        "--call-graph=fractal,0,callee".into(),
        format!("--pid={pid}"),
        format!("--time={}", time_filter(interval)?),
        "--input".into(),
        data.display().to_string(),
    ])
}

/// Inspect the exact perf report header. Loss makes attribution incomplete.
pub fn check_losses(text: &str) -> Result<(), String> {
    let values = text
        .lines()
        .filter_map(|line| line.strip_prefix("# Total Lost Samples: "))
        .collect::<Vec<_>>();
    if values != ["0"] {
        return Err("perf loss count is missing, duplicated or nonzero".into());
    }
    Ok(())
}

/// The unthresholded self rows must account for exactly the independently
/// filtered raw samples. Callchain annotations are not additional samples.
pub fn check_report_samples(text: &str, sample_count: usize) -> Result<(), String> {
    let mut total = 0usize;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if !fields.next().is_some_and(|value| value.ends_with('%')) {
            continue;
        }
        let count = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or("invalid perf histogram sample row")?;
        let thread = fields
            .next()
            .and_then(|value| value.split_once(':'))
            .and_then(|(pid, _)| pid.parse::<u32>().ok());
        if thread.is_none_or(|pid| pid == 0) {
            return Err("perf histogram lacks thread identity".into());
        }
        total = total
            .checked_add(count)
            .ok_or("perf histogram sample count overflow")?;
    }
    if sample_count == 0 || total != sample_count {
        return Err("perf histogram differs from the selected process's raw sample count".into());
    }
    Ok(())
}

/// `perf script -G --ns -F pid,time,event,ip,sym,dso`: one sample per line.
/// Count the selected outer TGID and exact monotonic interval independently
/// of perf's filters. Unknown leaf symbols remain an explicit count.
pub fn sample_counts(text: &str, pid: u32, interval: &Interval) -> Result<(usize, usize), String> {
    time_filter(interval)?;
    if pid == 0 || text.len() > 64 * CAP as usize {
        return Err("invalid profile sample input".into());
    }
    let mut count = 0;
    let mut unknown = 0;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 6
            || fields[0].parse::<u32>().ok() != Some(pid)
            || fields[2] != "cpu-clock:u:"
        {
            return Err("perf sample identity or event differs".into());
        }
        let (seconds, nanos) = fields[1]
            .trim_end_matches(':')
            .split_once('.')
            .ok_or("invalid perf timestamp")?;
        if seconds.is_empty()
            || !seconds.bytes().all(|b| b.is_ascii_digit())
            || nanos.len() != 9
            || !nanos.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("invalid perf timestamp".into());
        }
        let stamp = seconds
            .parse::<u128>()
            .map_err(|_| "invalid perf timestamp")?
            .checked_mul(1_000_000_000)
            .and_then(|v| v.checked_add(nanos.parse::<u128>().ok()?))
            .ok_or("invalid perf timestamp")?;
        if stamp < u128::from(interval.start_usec) * 1000
            || stamp > u128::from(interval.end_usec) * 1000
        {
            return Err("perf sample lies outside the selected interval".into());
        }
        count += 1;
        unknown += usize::from(line.contains("[unknown]"));
    }
    if count == 0 {
        return Err("no samples for the bound process in the measured interval".into());
    }
    Ok((count, unknown))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    write_new(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Finished {
    schema: u8,
    profile_sha256: String,
    success: bool,
    error: Option<String>,
    evidence: BTreeMap<String, String>,
    perf_sha256: Option<String>,
}

fn load(path: &Path, sha: &str) -> Result<Profile, String> {
    let bytes = read_bounded(path, CAP)?;
    if crate::sha256(&bytes) != sha {
        return Err("CPU profile descriptor changed".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    match args {
        [command, request, output] if command == "run-cpu-profile" => {
            // All routine refusals precede the single-use directory. Never
            // modify a plan or build an executable from this entry point.
            let bytes = read_bounded(Path::new(request), CAP)?;
            let profile: Profile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let (_, plan, _) = verify(&profile)?;
            shell_timing_native::launch_prerequisites(&plan)?;
            let output = Path::new(output);
            absolute(output)?;
            shell_timing_native::stage_available(output)?;
            create_private(output)?;
            let descriptor = output.join("profile.json");
            write_new(&descriptor, &bytes)?;
            let sha = crate::sha256(&bytes);
            let stage = output.join("run");
            let data = output.join("perf.data");
            let log = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output.join("perf-record.log"))
                .map_err(|e| e.to_string())?;
            let argv = record_arguments(
                &data,
                &plan.artifacts["runner"].path,
                &descriptor,
                &sha,
                &stage,
            );
            write_json(&output.join("perf-command.json"), &argv)?;
            let result = (|| {
                let status = Command::new(&profile.perf.path)
                    .args(&argv)
                    .stderr(log)
                    .status()
                    .map_err(|e| e.to_string())?;
                if !status.success() {
                    return Err(format!("perf/native capture failed: {status}"));
                }
                let perf_sha256 = digest(&data, PERF_CAP)?;
                let evidence = required_evidence(Condition::Idle)
                    .into_iter()
                    .map(|name| Ok((name.into(), digest(&stage.join(name), 64 * CAP)?)))
                    .collect::<Result<BTreeMap<_, _>, String>>()?;
                Ok((evidence, perf_sha256))
            })();
            let finished = Finished {
                schema: 1,
                profile_sha256: sha,
                success: result.is_ok(),
                error: result.as_ref().err().cloned(),
                evidence: result.as_ref().map(|(e, _)| e.clone()).unwrap_or_default(),
                perf_sha256: result.as_ref().ok().map(|(_, s)| s.clone()),
            };
            write_json(&output.join("profile-result.json"), &finished)?;
            result?;
            Ok(vec![format!(
                "captured {}; run report-cpu-profile before interpretation ({SCOPE})",
                output.display()
            )])
        }
        [command, descriptor, sha, stage] if command == "capture-cpu-profile" => {
            let profile = load(Path::new(descriptor), sha)?;
            verify(&profile)?;
            let mut observer = LiveCampaignObserver::new(Condition::Idle, profile.target.card);
            // The normal runner repeats all attended and identity checks.
            shell_timing_native::run_observed(
                &[
                    "run-native".into(),
                    request_path(&profile.plan)?,
                    profile.plan.sha256,
                    stage.into(),
                ],
                &mut observer,
            )
        }
        [command, directory] if command == "report-cpu-profile" => report(Path::new(directory)),
        _ => Err("expected run-cpu-profile REQUEST NEW_DIR | report-cpu-profile DIR".into()),
    }
}
fn request_path(artifact: &Artifact) -> Result<String, String> {
    Ok(absolute(&artifact.path)?.into())
}

fn report(directory: &Path) -> Result<Vec<String>, String> {
    let finished: Finished =
        serde_json::from_slice(&read_bounded(&directory.join("profile-result.json"), CAP)?)
            .map_err(|e| e.to_string())?;
    if finished.schema != 1 || !finished.success || finished.error.is_some() {
        return Err("CPU profile attempt failed".into());
    }
    let profile = load(&directory.join("profile.json"), &finished.profile_sha256)?;
    let (frozen, request, arm) = verify(&profile)?;
    let data = directory.join("perf.data");
    if Some(digest(&data, PERF_CAP)?) != finished.perf_sha256 {
        return Err("perf data changed".into());
    }
    let run = read_run(directory, &arm, &finished.evidence)?;
    check_run_record(&frozen, &run)?;
    let rebuilt = derive_arm_for_output(
        profile.target.output,
        profile.target.mode_millihz,
        &arm,
        &run,
    )?;
    let verified = verify_environment(
        &profile.target,
        &ArmFiles {
            arm: &arm,
            plan: &run.plan,
            environment: &run.environment,
            environment_receipts: &run.environment_receipts,
            load: None,
        },
    )?;
    let environment = environment_record(&verified.capture, &verified.expect)?;
    check_measured_interval(&verified, rebuilt.run)?;
    let DerivedArm::Cost(cost) = rebuilt.arm else {
        return Err("expected CPU counters".into());
    };
    // Include pixel preparation before the upload offer: the peer's frame
    // start/end counter window covers every frame, including its first canvas.
    // The combined cost's outer bounds additionally bracket Session samples.
    let interval = Interval {
        start_usec: cost.report.whole_workload.outer_start_usec,
        end_usec: cost.report.whole_workload.outer_end_usec,
    };
    check_measured_interval(&verified, interval)?;
    let time = time_filter(&interval)?;
    let mut reports = BTreeMap::new();
    for (role, pid) in [
        ("session", verified.expect.host_pid),
        ("peer", verified.expect.peer_pid),
    ] {
        let argv = report_arguments(&data, pid, &interval)?;
        let result = Command::new(&profile.perf.path)
            .env("LC_ALL", "C")
            .args(&argv)
            .output()
            .map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err(format!(
                "perf report failed: {}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
        let output = String::from_utf8(result.stdout).map_err(|e| e.to_string())?;
        check_losses(&output)?;
        let sample_argv = vec![
            "script".into(),
            "-G".into(),
            "--ns".into(),
            "--fields=pid,time,event,ip,sym,dso".into(),
            format!("--pid={pid}"),
            format!("--time={time}"),
            "--input".into(),
            data.display().to_string(),
        ];
        let samples = Command::new(&profile.perf.path)
            .env("LC_ALL", "C")
            .args(&sample_argv)
            .output()
            .map_err(|e| e.to_string())?;
        if !samples.status.success() {
            return Err("perf sample export failed".into());
        }
        let (sample_count, unknown_leaf_samples) = sample_counts(
            std::str::from_utf8(&samples.stdout).map_err(|e| e.to_string())?,
            pid,
            &interval,
        )?;
        check_report_samples(&output, sample_count)?;
        reports.insert(role, serde_json::json!({"pid":pid, "argv":argv,
            "sample_argv":sample_argv, "sample_count":sample_count, "unknown_leaf_samples":unknown_leaf_samples,
            "sample_stderr":String::from_utf8(samples.stderr).map_err(|e| e.to_string())?,
            "stdout":output,
            "stderr":String::from_utf8(result.stderr).map_err(|e| e.to_string())?}));
    }
    Ok(vec![serde_json::to_string_pretty(&serde_json::json!({
        "schema":2, "scope":SCOPE, "profile_sha256":finished.profile_sha256,
        "perf_sha256":finished.perf_sha256, "wire":request.wire, "source":request.source,
        "interval":interval, "environment":environment, "cost_diagnostic":cost.report,
        "reports":reports,
        "interpretation":"Review sample counts, lost-event warnings and unresolved stacks before attribution. Histogram rows retain thread identity within the selected outer TGID and must total its independently checked raw samples. Relative self percentages use that process's samples, not combined CPU. Fractal callchain percentages are conditional shares of their parent branch, not process CPU percentages; callchains overlap. Schema 1 reports grouped shared symbols across processes and must be regenerated from retained perf.data. No kernel samples were collected."
    })).map_err(|e| e.to_string())?])
}
