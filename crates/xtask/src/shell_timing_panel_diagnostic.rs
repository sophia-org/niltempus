//! Default-off ring versus sealed capture with an independently sampled CPU
//! window. The native runner still owns console, launch and restoration.
use crate::shell_timing::{DrawWorkload, ImageSource};
use crate::shell_timing_calibration::{absolute, read_bounded, write_new};
use crate::shell_timing_campaign::Condition;
use crate::shell_timing_campaign_observer::LiveCampaignObserver;
use crate::shell_timing_campaign_provenance::ProvenanceTarget;
use crate::shell_timing_native::{self, Artifact, CampaignObserver, Request};
use crate::shell_timing_process_binding::{
    Binder, BindingRecheck, Executables, ProcessBinding, ResolveOutcome,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CAP: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: u8,
    pub plan: Artifact,
    pub target: ProvenanceTarget,
    pub python: Artifact,
    pub sampler: Artifact,
    pub warmup: usize,
    pub counted: usize,
    pub interval_ms: u64,
    /// Schema 2 pins the installed sibling helper, including in the ring arm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<DrawWorkload>,
}

/// Deliberately narrow diagnostic. Neither historical comparator
/// instrumentation nor external-reference acceptance enters this path.
pub fn policy(d: &Descriptor, r: &Request, probe: u64) -> Result<(), String> {
    d.target.validate()?;
    let source_matches = match d.schema {
        1 => {
            d.raster.is_none()
                && d.workload.is_none()
                && r.reference_workload.is_none()
                && matches!(r.source, ImageSource::Isb | ImageSource::OwnedRing)
        }
        2 => {
            d.raster.is_some()
                && d.workload == r.reference_workload
                && matches!(
                    d.workload,
                    Some(DrawWorkload::Retained | DrawWorkload::Continuous | DrawWorkload::Bursty)
                )
                && matches!(
                    r.source,
                    ImageSource::OwnedRing | ImageSource::Draw | ImageSource::Mixed
                )
        }
        _ => false,
    };
    // The longer schema-2 window reduces counter quantization after the
    // inconclusive t341-vello-03 CPU gate. Frozen schema 1 stays unchanged.
    let window_matches = (d.counted, r.runtime_ms) == (2000, 60_000)
        || d.schema == 2 && (d.counted, r.runtime_ms) == (6000, 120_000);
    if !source_matches
        || !window_matches
        || d.warmup != 120
        || d.interval_ms != 5
        || Some(r.frames) != d.warmup.checked_add(d.counted)
        || r.instrumentation != "none"
        || r.external_timing.is_some()
        || !r.environment_readback
        || r.wire != "files"
        || probe != 0
        || r.output != d.target.output
        || r.mode_millihz != d.target.mode_millihz
    {
        return Err("panel diagnostic requires an observed files arm, 120 warmup, 2000 intervals/60 seconds (or schema 2: 6000/120 seconds), and 5 ms sampling".into());
    }
    Ok(())
}

fn artifact(value: &Artifact, cap: u64) -> Result<Vec<u8>, String> {
    absolute(&value.path)?;
    let bytes = read_bounded(&value.path, cap)?;
    if crate::sha256(&bytes) != value.sha256 {
        return Err(format!(
            "diagnostic artifact changed: {}",
            value.path.display()
        ));
    }
    Ok(bytes)
}

fn verify(d: &Descriptor) -> Result<Request, String> {
    let bytes = artifact(&d.plan, CAP)?;
    artifact(&d.python, 128 * CAP)?;
    artifact(&d.sampler, CAP)?;
    let (request, probe) = shell_timing_native::verify_panel_prepared(&bytes)?;
    policy(d, &request, probe)?;
    let config: serde_json::Value =
        serde_json::from_slice(&artifact(&request.artifacts["peer-config"], CAP)?)
            .map_err(|e| e.to_string())?;
    if d.schema == 1 {
        if config["height"] != 24 || config.get("workload").is_some_and(|v| v != "fresh") {
            return Err("panel diagnostic requires fresh 24-pixel content".into());
        }
    } else {
        if config["workload"] != serde_json::to_value(d.workload).map_err(|e| e.to_string())? {
            return Err("draw diagnostic workload differs from the peer config".into());
        }
        let raster = d.raster.as_ref().ok_or("missing raster helper")?;
        artifact(raster, 128 * CAP)?;
        let sibling = request.artifacts["sophia"]
            .path
            .parent()
            .ok_or("Session has no executable directory")?
            .join("sophia-draw-raster");
        if sibling.canonicalize().map_err(|e| e.to_string())?
            != raster.path.canonicalize().map_err(|e| e.to_string())?
        {
            return Err("pinned raster helper is not installed beside Session".into());
        }
    }
    Ok(request)
}

struct Sampler {
    child: Child,
}

impl Sampler {
    fn finish(mut self) -> Result<(), String> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            // A direct unreaped child cannot have its PID reused here.
            rustix::process::kill_process(
                rustix::process::Pid::from_raw(self.child.id() as i32)
                    .ok_or("invalid sampler pid")?,
                rustix::process::Signal::TERM,
            )
            .map_err(|e| e.to_string())?;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("CPU sampler failed: {status}"))
                };
            }
            if Instant::now() >= deadline {
                return Err("CPU sampler did not finalize after TERM".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

struct Observer<'a> {
    descriptor: &'a Descriptor,
    descriptor_bytes: &'a [u8],
    environment: LiveCampaignObserver,
    stage: Option<PathBuf>,
    sampler: Option<Sampler>,
    raster_binding: Option<ProcessBinding>,
    raster_ended: bool,
    raster_first_bound: Option<u64>,
    raster_last_bound: Option<u64>,
    raster_first_ended: Option<u64>,
    source: ImageSource,
}

impl Observer<'_> {
    /// Bind the helper by executable identity and its owned supervisor chain.
    /// End-of-run exit is recorded; cpuwin refuses any exit inside its window.
    fn observe_raster(&mut self, wrapper_pid: u32) -> Result<bool, String> {
        let Some(raster) = &self.descriptor.raster else {
            return Ok(true);
        };
        let Some(host) = self.environment.live_binding() else {
            return Ok(false);
        };
        let binder = Binder::system();
        // Stamp before the walk: an alive observation proves life at some
        // point after this timestamp, never through the walk's end.
        let recheck_started = crate::shell_timing_environment_capture::monotonic_usec()?;
        if let Some(binding) = &self.raster_binding {
            if !self.raster_ended && binder.recheck(binding)? == BindingRecheck::Ended {
                let ended = crate::shell_timing_environment_capture::monotonic_usec()?;
                self.raster_first_ended = Some(ended);
                write_new(
                    &self
                        .stage
                        .as_ref()
                        .ok_or("stage missing")?
                        .join("cpu-raster-ended.json"),
                    &serde_json::to_vec(&serde_json::json!({
                        "monotonic_usec": ended
                    }))
                    .map_err(|e| e.to_string())?,
                )?;
                self.raster_ended = true;
            }
            if !self.raster_ended
                && let ResolveOutcome::Bound(current) = binder.resolve(
                    wrapper_pid,
                    Executables {
                        host: &host.host_executable.prepared_path,
                        host_argv: &host.host_argv,
                        supervisor: &host.supervisor_executable.prepared_path,
                        peer: &raster.path,
                    },
                )?
                && (current.peer != binding.peer || current.supervisor != binding.supervisor)
            {
                return Err("raster helper was replaced during the capture".into());
            }
            if !self.raster_ended {
                self.raster_last_bound = Some(recheck_started);
            }
            return Ok(true);
        }
        if binder.recheck(host)? == BindingRecheck::Ended {
            return Ok(false);
        }
        match binder.resolve(
            wrapper_pid,
            Executables {
                host: &host.host_executable.prepared_path,
                host_argv: &host.host_argv,
                supervisor: &host.supervisor_executable.prepared_path,
                peer: &raster.path,
            },
        )? {
            ResolveOutcome::Pending { .. } => Ok(self.source == ImageSource::OwnedRing),
            ResolveOutcome::Bound(binding) => {
                if self.source == ImageSource::OwnedRing {
                    return Err("ring comparison unexpectedly launched a raster helper".into());
                }
                if binding.host != host.host {
                    return Err("raster helper belongs to another Session".into());
                }
                write_new(
                    &self
                        .stage
                        .as_ref()
                        .ok_or("stage missing")?
                        .join("cpu-raster-binding.json"),
                    &serde_json::to_vec_pretty(&binding).map_err(|e| e.to_string())?,
                )?;
                let bound = crate::shell_timing_environment_capture::monotonic_usec()?;
                self.raster_first_bound = Some(bound);
                self.raster_last_bound = Some(recheck_started);
                self.raster_binding = Some(*binding);
                Ok(true)
            }
        }
    }
}

impl CampaignObserver for Observer<'_> {
    fn prepare(
        &mut self,
        request: &Request,
        argv: &[String],
        stage: &Path,
    ) -> Result<PathBuf, String> {
        // Retain the exact descriptor before any fallible monitor setup, and
        // only after the native runner has checked the attended console.
        write_new(&stage.join("panel-diagnostic.json"), self.descriptor_bytes)?;
        let socket = self.environment.prepare(request, argv, stage)?;
        self.stage = Some(stage.to_owned());
        Ok(socket)
    }

    fn observe(&mut self, wrapper_pid: u32, logs: &Path) -> Result<(), String> {
        self.environment.observe(wrapper_pid, logs)?;
        if !self.observe_raster(wrapper_pid)? {
            return Ok(());
        }
        if self.sampler.is_some() {
            // The sampler may finish normally after the observed processes
            // exit. Its final status and window coverage are checked separately.
            return Ok(());
        }
        let Some(binding) = self.environment.live_binding() else {
            return Ok(());
        };
        let stage = self.stage.as_ref().ok_or("diagnostic stage not prepared")?;
        artifact(&self.descriptor.python, 128 * CAP)?;
        artifact(&self.descriptor.sampler, CAP)?;
        if let Some(raster) = &self.descriptor.raster {
            artifact(raster, 128 * CAP)?;
        }
        let log = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join("cpu-sampler.log"))
            .map_err(|e| e.to_string())?;
        write_new(
            &stage.join("cpu-binding.json"),
            &serde_json::to_vec_pretty(binding).map_err(|e| e.to_string())?,
        )?;
        let mut command = Command::new(&self.descriptor.python.path);
        command
            .arg("-I")
            .arg(&self.descriptor.sampler.path)
            .arg("sample");
        if let Some(binding) = &self.raster_binding {
            command.arg("--pid").arg(format!(
                "raster={}:{}",
                binding.peer.pid, binding.peer.start_time_ticks
            ));
        }
        let child = command
            .arg("--out")
            .arg(stage.join("cpu-window.jsonl"))
            .args(["--seconds", "180", "--interval-ms", "5"])
            .arg("--pid")
            .arg(format!(
                "session={}:{}",
                binding.host.pid, binding.host.start_time_ticks
            ))
            .arg("--pid")
            .arg(format!(
                "peer={}:{}",
                binding.peer.pid, binding.peer.start_time_ticks
            ))
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| e.to_string())?;
        self.sampler = Some(Sampler { child });
        Ok(())
    }

    fn finish(&mut self, stage: &Path) -> Result<(), String> {
        // Always finalize both owners, including on a failed launch.
        let sampled = self
            .sampler
            .take()
            .ok_or("CPU sampler never started".to_owned())
            .and_then(Sampler::finish);
        let environment = self.environment.finish(stage);
        if let Some(binding) = &self.raster_binding {
            write_new(&stage.join("cpu-raster-lifetime.json"), &serde_json::to_vec_pretty(&serde_json::json!({
                "pid": binding.peer.pid,
                "starttime": binding.peer.start_time_ticks,
                "first_bound_usec": self.raster_first_bound,
                "last_bound_usec": self.raster_last_bound,
                "first_ended_usec": self.raster_first_ended,
                "scope": "one raster helper; require first_bound <= window start and last_bound >= window end"
            })).map_err(|e| e.to_string())?)?;
        }
        match (sampled, environment) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(a), Err(b)) => Err(format!("{a}; {b}")),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }
}

fn checked_descriptor(path: &str, digest: &str) -> Result<(Vec<u8>, Descriptor, Request), String> {
    let bytes = read_bounded(Path::new(path), CAP)?;
    if crate::sha256(&bytes) != digest {
        return Err("diagnostic descriptor changed".into());
    }
    let d: Descriptor = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let request = verify(&d)?;
    Ok((bytes, d, request))
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    if let [command, descriptor, digest] = args
        && command == "check-panel-diagnostic"
    {
        let (_, _, request) = checked_descriptor(descriptor, digest)?;
        shell_timing_native::verify_sources(&request)?;
        shell_timing_native::validate_pam_helper(&request.artifacts["pam-helper"].path)?;
        return Ok(vec![
            "panel diagnostic verified; no stage created or Session launched".into(),
        ]);
    }
    let [command, descriptor, digest, stage] = args else {
        return Err("expected check-panel-diagnostic DESCRIPTOR SHA256 or capture-panel-diagnostic DESCRIPTOR SHA256 NEW_RUN_DIR".into());
    };
    if command != "capture-panel-diagnostic" {
        return Err("unknown panel diagnostic command".into());
    }
    let (bytes, d, request) = checked_descriptor(descriptor, digest)?;
    let mut observer = Observer {
        environment: LiveCampaignObserver::new(Condition::Idle, d.target.card.clone()),
        descriptor: &d,
        descriptor_bytes: &bytes,
        stage: None,
        sampler: None,
        raster_binding: None,
        raster_ended: false,
        raster_first_bound: None,
        raster_last_bound: None,
        raster_first_ended: None,
        source: request.source,
    };
    // This repeats all source, runner image and seated-console checks before
    // creating the run directory or starting either Session or the sampler.
    let mut result = shell_timing_native::run_observed(
        &[
            "run-native".into(),
            absolute(&d.plan.path)?.into(),
            d.plan.sha256.clone(),
            stage.clone(),
        ],
        &mut observer,
    )?;
    result.push("CPU samples retained; window coverage and attribution remain unproven until cpuwin analyze succeeds. Scope: Session, peer, and the bound raster helper when present; excludes supervisor/wrapper and other processes.".into());
    Ok(result)
}
