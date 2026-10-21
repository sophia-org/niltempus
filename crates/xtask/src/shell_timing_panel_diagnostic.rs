//! Default-off ring versus sealed capture with an independently sampled CPU
//! window. The native runner still owns console, launch and restoration.
use crate::shell_timing::ImageSource;
use crate::shell_timing_calibration::{absolute, read_bounded, write_new};
use crate::shell_timing_campaign::Condition;
use crate::shell_timing_campaign_observer::LiveCampaignObserver;
use crate::shell_timing_campaign_provenance::ProvenanceTarget;
use crate::shell_timing_native::{self, Artifact, CampaignObserver, Request};
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
}

/// Deliberately narrow first diagnostic. Neither historical comparator
/// instrumentation nor external-reference acceptance enters this path.
pub fn policy(d: &Descriptor, r: &Request, probe: u64) -> Result<(), String> {
    d.target.validate()?;
    if d.schema != 1
        || d.warmup != 120
        || d.counted != 2000
        || d.interval_ms != 5
        || r.frames != 2120
        || r.runtime_ms != 60_000
        || r.instrumentation != "none"
        || r.external_timing.is_some()
        || !r.environment_readback
        || r.wire != "files"
        || !matches!(r.source, ImageSource::Isb | ImageSource::OwnedRing)
        || probe != 0
        || r.output != d.target.output
        || r.mode_millihz != d.target.mode_millihz
    {
        return Err("panel diagnostic requires an observed fresh files arm, 120+2000 frames, 60 seconds and 5 ms sampling".into());
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
    let (request, probe) = shell_timing_native::verify_prepared(&bytes)?;
    policy(d, &request, probe)?;
    let config: serde_json::Value =
        serde_json::from_slice(&artifact(&request.artifacts["peer-config"], CAP)?)
            .map_err(|e| e.to_string())?;
    if config["height"] != 24 || config.get("workload").is_some_and(|v| v != "fresh") {
        return Err("panel diagnostic requires fresh 24-pixel content".into());
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
        let log = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join("cpu-sampler.log"))
            .map_err(|e| e.to_string())?;
        write_new(
            &stage.join("cpu-binding.json"),
            &serde_json::to_vec_pretty(binding).map_err(|e| e.to_string())?,
        )?;
        let child = Command::new(&self.descriptor.python.path)
            .arg("-I")
            .arg(&self.descriptor.sampler.path)
            .arg("sample")
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
        match (sampled, environment) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(a), Err(b)) => Err(format!("{a}; {b}")),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    }
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    let [command, descriptor, digest, stage] = args else {
        return Err("expected capture-panel-diagnostic DESCRIPTOR SHA256 NEW_RUN_DIR".into());
    };
    if command != "capture-panel-diagnostic" {
        return Err("unknown panel diagnostic command".into());
    }
    let bytes = read_bounded(Path::new(descriptor), CAP)?;
    if crate::sha256(&bytes) != *digest {
        return Err("diagnostic descriptor changed".into());
    }
    let d: Descriptor = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    verify(&d)?;
    let mut observer = Observer {
        environment: LiveCampaignObserver::new(Condition::Idle, d.target.card.clone()),
        descriptor: &d,
        descriptor_bytes: &bytes,
        stage: None,
        sampler: None,
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
    result.push("CPU samples retained; window coverage and attribution remain unproven until cpuwin analyze succeeds. Scope: Session and peer, excluding supervisor/wrapper and other processes.".into());
    Ok(result)
}
