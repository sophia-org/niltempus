//! Bind a reviewed shell workload to exact files before any attended launch.
use crate::shell_timing::ImageSource;
use crate::{hex, output_file_native_run::process, sha256};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

const MAX_INPUT: u64 = 1024 * 1024;
const KEYS: &[&str] = &[
    "sophia",
    "peer",
    "wm",
    "factotum",
    "pam-helper",
    "preflight",
    "runner",
    "core",
    "profile",
    "peer-config",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: PathBuf,
    pub commit: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub sophia_source: Source,
    pub integration_source: Source,
    pub artifacts: BTreeMap<String, Artifact>,
    pub tty: String,
    pub display: u8,
    pub input_seat: String,
    pub runtime_ms: u64,
    pub wire: String,
    #[serde(default)]
    pub source: ImageSource,
    pub output: u64,
    pub frames: usize,
    pub mode_millihz: u64,
    pub instrumentation: String,
    /// Freeze the explicit environment observation flag into the host argv.
    /// Legacy single-arm capture plans leave this false.
    #[serde(default)]
    pub environment_readback: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema: u8,
    request: Request,
    argv: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerConfig {
    wire: String,
    #[serde(default)]
    source: ImageSource,
    output: u64,
    frames: usize,
    height: u64,
    hold_msec: u64,
    #[serde(default)]
    probe_bytes: u64,
    image: Option<ImageConfig>,
}

/// Caller bounds for the sealed SDK adapter. The peer verifies these against
/// the negotiated content limits; preparation binds their exact artifact bytes.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageConfig {
    max_resource_bytes: u64,
    max_width: u32,
    max_height: u32,
    max_admitted_outputs: u32,
    transfer_slots: u32,
    transfer_timeout_ms: u32,
}

impl ImageConfig {
    fn valid(&self) -> bool {
        self.max_resource_bytes != 0
            && self.max_width != 0
            && self.max_height != 0
            && self.max_admitted_outputs != 0
            && self.transfer_slots != 0
            && self.transfer_timeout_ms != 0
    }
}

fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds input bound", path.display()));
    }
    Ok(bytes)
}

fn absolute(path: &Path) -> Result<&str, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err("paths must be absolute without parent traversal".into());
    }
    path.to_str()
        .filter(|p| !p.chars().any(char::is_control))
        .ok_or_else(|| "invalid path encoding".into())
}

fn file_digest(path: &Path, limit: u64) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];
    let mut total = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > limit {
            return Err("artifact exceeds size bound".into());
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn git(source: &Source, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&source.path)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("source check failed: git {}", args.join(" ")));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

fn source(source: &Source) -> Result<(), String> {
    absolute(&source.path)?;
    if !hex(&source.commit, 40)
        || git(source, &["rev-parse", "HEAD"])?.trim() != source.commit
        || !git(source, &["status", "--porcelain"])?.is_empty()
    {
        return Err("source must be clean at the exact declared commit".into());
    }
    git(source, &["verify-commit", &source.commit])?;
    let top = PathBuf::from(git(source, &["rev-parse", "--show-toplevel"])?.trim());
    if top.canonicalize().map_err(|e| e.to_string())?
        != source.path.canonicalize().map_err(|e| e.to_string())?
    {
        return Err("source must name the repository root".into());
    }
    Ok(())
}

fn document(bytes: &[u8]) -> Result<kdl::KdlDocument, String> {
    let doc: kdl::KdlDocument = std::str::from_utf8(bytes)
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|e: kdl::KdlError| e.to_string())?;
    fn flat(doc: &kdl::KdlDocument) -> bool {
        doc.nodes()
            .iter()
            .all(|n| n.name().value() != "include" && n.children().is_none_or(flat))
    }
    if !flat(&doc) {
        return Err("expand all profile/core includes before preparation".into());
    }
    Ok(doc)
}

/// Validate the exact workload without executing any binary or opening devices.
pub fn validate(request: &Request) -> Result<Vec<String>, String> {
    if request.artifacts.len() != KEYS.len()
        || KEYS.iter().any(|k| !request.artifacts.contains_key(*k))
    {
        return Err(
            "expected sophia, peer, wm, factotum, pam-helper, preflight, runner, core, profile and peer-config artifacts"
                .into(),
        );
    }
    let tty = request
        .tty
        .strip_prefix("/dev/tty")
        .and_then(|n| n.parse::<u8>().ok())
        .filter(|n| (1..=63).contains(n))
        .ok_or("invalid console")?;
    if request.tty != format!("/dev/tty{tty}")
        || !(90..=99).contains(&request.display)
        || !(60_000..=600_000).contains(&request.runtime_ms)
        || !(1..=10_000).contains(&request.frames)
        || !matches!(request.mode_millihz, 60_000 | 120_000)
        || request.output == 0
        || !matches!(request.wire.as_str(), "files" | "ipc")
        || !matches!(
            request.instrumentation.as_str(),
            "none" | "rust-global" | "cpu-only" | "profile" | "progress"
        )
        || (request.instrumentation != "none" && request.runtime_ms > 60_000)
        || request.input_seat.is_empty()
        || request.input_seat.len() > 64
        || !request
            .input_seat
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        return Err("invalid bounded workload, mode, console, display or seat".into());
    }
    let mut texts = BTreeMap::new();
    for (key, artifact) in &request.artifacts {
        absolute(&artifact.path)?;
        if !hex(&artifact.sha256, 64) {
            return Err(format!("invalid {key} digest"));
        }
        let executable = matches!(
            key.as_str(),
            "sophia" | "peer" | "wm" | "factotum" | "pam-helper" | "preflight" | "runner"
        );
        let metadata = fs::metadata(&artifact.path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || (executable && metadata.permissions().mode() & 0o111 == 0) {
            return Err(format!("invalid {key} artifact"));
        }
        if executable {
            if file_digest(&artifact.path, 1024 * MAX_INPUT)? != artifact.sha256 {
                return Err(format!("{key} artifact changed"));
            }
        } else {
            let bytes = bounded(&artifact.path, MAX_INPUT)?;
            if sha256(&bytes) != artifact.sha256 {
                return Err(format!("{key} artifact changed"));
            }
            texts.insert(key.as_str(), bytes);
        }
    }
    let peer: PeerConfig =
        serde_json::from_slice(&texts["peer-config"]).map_err(|e| e.to_string())?;
    if peer.wire != request.wire
        || peer.source != request.source
        || (peer.source == ImageSource::Sealed && request.wire != "files")
        || match peer.source {
            ImageSource::Sealed => !peer.image.as_ref().is_some_and(ImageConfig::valid),
            ImageSource::Bytes => peer.image.is_some(),
        }
        || peer.output != request.output
        || peer.frames != request.frames
        || !(1..=128).contains(&peer.height)
        || !matches!(peer.probe_bytes, 0 | 4194304)
        || peer.hold_msec != request.runtime_ms
    {
        return Err(
            "peer configuration differs from the declared workload or full-runtime hold".into(),
        );
    }
    document(&texts["core"])?;
    let profile = document(&texts["profile"])?;
    let sessions: Vec<_> = profile
        .nodes()
        .iter()
        .filter(|n| n.name().value() == "session")
        .collect();
    if sessions.len() != 1 {
        return Err("profile requires exactly one session node".into());
    }
    let session = sessions[0].children().ok_or("empty session")?;
    let components: Vec<_> = session
        .nodes()
        .iter()
        .filter(|n| n.name().value() == "shell-component")
        .collect();
    if components.len() != 1 || components[0].get(1).and_then(|v| v.as_string()) != Some("bar") {
        return Err("timing workload requires exactly one bar component".into());
    }
    let component = components[0].children().ok_or("missing component fields")?;
    let field = |name: &str| -> Result<&kdl::KdlNode, String> {
        let nodes: Vec<_> = component
            .nodes()
            .iter()
            .filter(|n| n.name().value() == name)
            .collect();
        if nodes.len() != 1 {
            return Err(format!("missing or repeated component {name}"));
        }
        Ok(nodes[0])
    };
    let wire = if request.wire == "files" {
        "9p2000.L"
    } else {
        "comparison-ipc"
    };
    for (name, expected) in [
        ("executable", absolute(&request.artifacts["peer"].path)?),
        ("config", absolute(&request.artifacts["peer-config"].path)?),
        ("transport", wire),
        ("gpu", "denied"),
    ] {
        if field(name)?.get(0).and_then(|v| v.as_string()) != Some(expected) {
            return Err(format!("wrong component {name}"));
        }
    }
    let startups: Vec<_> = session
        .nodes()
        .iter()
        .filter(|n| n.name().value() == "startup")
        .collect();
    if startups.len() != 1 || !startups[0].entries().is_empty() || startups[0].children().is_some()
    {
        return Err("profile must explicitly select empty startup".into());
    }
    let mut argv = vec![
        absolute(&request.artifacts["sophia"].path)?.into(),
        "session".into(),
        "run".into(),
        "--session-mode=normal".into(),
        "--native-scanout".into(),
        format!("--display=:{}", request.display),
        format!("--input-seat={}", request.input_seat),
        format!("--max-runtime-ms={}", request.runtime_ms),
        format!("--config={}", absolute(&request.artifacts["core"].path)?),
        format!(
            "--desktop-profile={}",
            absolute(&request.artifacts["profile"].path)?
        ),
        format!("--wm-process={}", absolute(&request.artifacts["wm"].path)?),
        format!(
            "--factotum-agent={}",
            absolute(&request.artifacts["factotum"].path)?
        ),
        format!(
            "--factotum-pam-helper={}",
            absolute(&request.artifacts["pam-helper"].path)?
        ),
    ];
    if request.environment_readback {
        argv.push("--output-environment-readback".into());
    }
    Ok(argv)
}

/// Match Factotum's helper custody rule before an attended launch. A Nix store
/// file can be immutable while an ancestor is group-writable; hashes alone
/// cannot establish the root-owned path that Factotum requires.
pub fn validate_pam_helper(path: &Path) -> Result<(), String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    for entry in canonical.ancestors() {
        let metadata = fs::metadata(entry).map_err(|e| e.to_string())?;
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(format!(
                "PAM helper requires root-owned, protected file and ancestors: {}",
                entry.display()
            ));
        }
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?
        .write_all(bytes)
        .map_err(|e| e.to_string())
}

/// Preserve every client record; normalize only the host's logging prefix.
/// The selected generic peer is the sole JSON writer in this workload.
pub fn split_capture(text: &str) -> Result<(String, String), String> {
    if text.len() > 64 * MAX_INPUT as usize || !text.ends_with('\n') {
        return Err("oversized or truncated session capture".into());
    }
    let mut client = String::new();
    let mut host = String::new();
    for (index, line) in text.lines().enumerate() {
        // Profiling emits five phase rows beside every counter sample. Keep
        // room for the bounded sampler and client journal within the 64 MiB cap.
        if index >= 500_000 || line.len() > 65536 {
            return Err("capture record bound exceeded".into());
        }
        let line = line.trim_start();
        if line.starts_with('{') {
            // Parsing here catches interleaved writes instead of repairing or
            // discarding a damaged offer. collect checks the complete schema.
            serde_json::from_str::<serde_json::Value>(line).map_err(|e| e.to_string())?;
            client.push_str(line);
            client.push('\n');
        } else if let Some(at) = line.find("sophia_shell_native_") {
            host.push_str(&line[at..]);
            host.push('\n');
        }
    }
    if client.is_empty() || host.is_empty() {
        return Err("capture lacks client or native evidence".into());
    }
    Ok((client, host))
}

/// Normalize environment log prefixes without interpreting facts. Keep
/// duplicates and unknown environment names so the M5 checker can refuse
/// them. This does not authenticate the writer; PID custody is runner evidence.
pub fn environment_capture(text: &str) -> Result<String, String> {
    if text.len() > 64 * MAX_INPUT as usize || !text.ends_with('\n') {
        return Err("oversized or truncated session capture".into());
    }
    let mut environment = String::new();
    for (index, line) in text.lines().enumerate() {
        if index >= 500_000 || line.len() > 65_536 {
            return Err("capture record bound exceeded".into());
        }
        let line = line.trim_start();
        if line.starts_with('{') {
            // A peer's JSON string mentioning a record is not a host record.
            continue;
        }
        if let Some(at) = line.find("sophia_shell_environment_") {
            environment.push_str(&line[at..]);
            environment.push('\n');
        }
    }
    if environment.is_empty() {
        return Err("capture lacks host environment records".into());
    }
    Ok(environment)
}

/// Reject a valid journal from a different arm before joining native samples.
pub fn validate_captured_workload(
    metrics: &crate::shell_timing::Journal,
    request: &Request,
    probe_bytes: u64,
) -> Result<(), String> {
    if metrics.probe_bytes != probe_bytes
        || metrics.source != request.source
        || metrics.wire != request.wire
    {
        return Err("captured upload workload differs from prepared configuration".into());
    }
    Ok(())
}

/// Compare profile policy across campaign arms while allowing only the peer
/// executable/config paths and selected wire to differ. All other KDL values,
/// properties and nodes remain part of the comparison.
pub fn campaign_profile(request: &Request) -> Result<String, String> {
    validate(request)?;
    let checked = |key: &str| -> Result<Vec<u8>, String> {
        let artifact = request
            .artifacts
            .get(key)
            .ok_or("missing campaign artifact")?;
        let bytes = bounded(&artifact.path, MAX_INPUT)?;
        if sha256(&bytes) != artifact.sha256 {
            return Err(format!("{key} artifact changed"));
        }
        Ok(bytes)
    };
    let peer: PeerConfig =
        serde_json::from_slice(&checked("peer-config")?).map_err(|e| e.to_string())?;
    if peer.height != 24 {
        return Err("campaign panel height must be 24".into());
    }
    let mut profile = document(&checked("profile")?)?;
    let session = profile
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "session")
        .and_then(|n| n.children_mut().as_mut())
        .ok_or("missing session")?;
    let component = session
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "shell-component")
        .and_then(|n| n.children_mut().as_mut())
        .ok_or("missing component")?;
    for field in component.nodes_mut() {
        if matches!(field.name().value(), "executable" | "config" | "transport") {
            // validate() already checked these values against prepared artifacts.
            // Retain extra entries/properties so they cannot be hidden here.
            let entry = field
                .entries_mut()
                .iter_mut()
                .find(|entry| entry.name().is_none())
                .ok_or("missing component value")?;
            *entry.value_mut() = kdl::KdlValue::String("campaign-arm-value".into());
        }
    }
    profile.autoformat_no_comments();
    Ok(profile.to_string())
}

/// Exact sealed adapter policy; campaign C arms may not silently change it.
pub fn campaign_image_bounds(request: &Request) -> Result<Option<String>, String> {
    let artifact = request
        .artifacts
        .get("peer-config")
        .ok_or("missing peer config")?;
    let bytes = bounded(&artifact.path, MAX_INPUT)?;
    if sha256(&bytes) != artifact.sha256 {
        return Err("peer configuration changed".into());
    }
    let peer: PeerConfig = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    peer.image
        .map(|image| serde_json::to_string(&image).map_err(|e| e.to_string()))
        .transpose()
}

/// Validate the clean signed source identities bound into a prepared request.
pub fn verify_sources(request: &Request) -> Result<(), String> {
    source(&request.sophia_source)?;
    source(&request.integration_source)
}

/// Recheck a prepared plan without launching it. Campaign preparation uses the
/// same artifact and argv checks as an individual native run.
pub fn verify_prepared(bytes: &[u8]) -> Result<(Request, u64), String> {
    if bytes.len() > MAX_INPUT as usize {
        return Err("plan exceeds input bound".into());
    }
    let plan: Plan = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if plan.schema != 1 || validate(&plan.request)? != plan.argv {
        return Err("plan arguments differ".into());
    }
    let peer_bytes = bounded(&plan.request.artifacts["peer-config"].path, MAX_INPUT)?;
    // Bind the second read too: never accept workload metadata from a changed file.
    if sha256(&peer_bytes) != plan.request.artifacts["peer-config"].sha256 {
        return Err("peer configuration changed during verification".into());
    }
    let peer: PeerConfig = serde_json::from_slice(&peer_bytes).map_err(|e| e.to_string())?;
    Ok((plan.request, peer.probe_bytes))
}

/// Attended campaign hooks. Preparation happens only after console, source,
/// artifact and runner checks, with a new private stage directory. `finish`
/// must retain evidence and stop owned load workers even when launch fails.
pub trait CampaignObserver {
    fn prepare(&mut self, request: &Request, stage: &Path) -> Result<PathBuf, String>;
    fn observe(&mut self, wrapper_pid: u32, logs: &Path) -> Result<(), String>;
    fn finish(&mut self, stage: &Path) -> Result<(), String>;
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    run_inner(args, None)
}

/// Uses exactly the same attended launch and restoration path as run-native.
pub fn run_observed(
    args: &[String],
    observer: &mut dyn CampaignObserver,
) -> Result<Vec<String>, String> {
    if args.first().map(String::as_str) != Some("run-native") {
        return Err("campaign observation requires run-native".into());
    }
    run_inner(args, Some(observer))
}

fn run_inner(
    args: &[String],
    mut observer: Option<&mut dyn CampaignObserver>,
) -> Result<Vec<String>, String> {
    match args {
        [command, input, output] if command == "prepare-native" => {
            let request: Request = serde_json::from_slice(&bounded(Path::new(input), MAX_INPUT)?)
                .map_err(|e| e.to_string())?;
            let argv = validate(&request)?;
            validate_pam_helper(&request.artifacts["pam-helper"].path)?;
            source(&request.sophia_source)?;
            source(&request.integration_source)?;
            let bytes = serde_json::to_vec_pretty(&Plan {
                schema: 1,
                request,
                argv,
            })
            .map_err(|e| e.to_string())?;
            absolute(Path::new(output))?;
            write(Path::new(output), &bytes)?;
            Ok(vec![format!(
                "prepared {} sha256={}",
                output,
                sha256(&bytes)
            )])
        }
        [command, path, digest, output] if command == "run-native" => {
            let bytes = bounded(Path::new(path), MAX_INPUT)?;
            if !hex(digest, 64) || sha256(&bytes) != *digest {
                return Err("plan digest differs".into());
            }
            let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if plan.schema != 1 || validate(&plan.request)? != plan.argv {
                return Err("plan arguments differ".into());
            }
            if plan.request.environment_readback != observer.is_some() {
                return Err(
                    "environment readback requires the campaign observer and its frozen flag"
                        .into(),
                );
            }
            process::attended_with(&plan.request.tty, "SOPHIA_SHELL_TIMING_ARM")?;
            validate_pam_helper(&plan.request.artifacts["pam-helper"].path)?;
            source(&plan.request.sophia_source)?;
            source(&plan.request.integration_source)?;
            git(
                &plan.request.sophia_source,
                &["ls-files", "--error-unmatch", "tools/run_sophia_session.sh"],
            )?;
            if std::env::current_exe()
                .map_err(|e| e.to_string())?
                .canonicalize()
                .map_err(|e| e.to_string())?
                != plan.request.artifacts["runner"]
                    .path
                    .canonicalize()
                    .map_err(|e| e.to_string())?
            {
                return Err("run with the prepared runner executable".into());
            }
            let stage = Path::new(output);
            absolute(stage)?;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(stage)
                .map_err(|e| e.to_string())?;
            write(&stage.join("plan.json"), &bytes)?;
            let wrapper = plan
                .request
                .sophia_source
                .path
                .join("tools/run_sophia_session.sh");
            let mut result = (|| {
                let destination = match observer.as_deref_mut() {
                    Some(observer) => Some(observer.prepare(&plan.request, stage)?),
                    None => None,
                };
                let mut environment = vec![("SOPHIA_RUN_REAL_ATOMIC_SCANOUT_SMOKE", "1")];
                if plan.request.wire == "ipc" {
                    environment.push(("SOPHIA_COMPARATOR_ADMISSION", "protected-legacy-baseline"));
                }
                if let Some(destination) = &destination {
                    if destination.parent() != Some(stage) || destination.file_name().is_none() {
                        return Err(
                            "environment receiver must be inside the private run directory".into(),
                        );
                    }
                    environment.push(("SOPHIA_OUTPUT_ENVIRONMENT_ARM", "1"));
                    environment.push(("SOPHIA_OUTPUT_ENVIRONMENT_SOCKET", absolute(destination)?));
                }
                process::run_bound_observed(
                    &process::BoundSession {
                        wrapper: &wrapper,
                        argv: &plan.argv,
                        preflight: absolute(&plan.request.artifacts["preflight"].path)?,
                        tty: &plan.request.tty,
                        runtime: plan.request.runtime_ms,
                        label: "shell-timing",
                        arm: "SOPHIA_SHELL_TIMING_ARM",
                        environment: &environment,
                    },
                    stage,
                    |pid, logs| match observer.as_deref_mut() {
                        Some(observer) => observer.observe(pid, logs),
                        None => Ok(()),
                    },
                )
            })();
            if let Some(observer) = observer
                && let Err(error) = observer.finish(stage)
            {
                result = Err(match result {
                    Ok(()) => format!("campaign observation finalization failed: {error}"),
                    Err(previous) => {
                        format!("{previous}; campaign observation finalization failed: {error}")
                    }
                });
            }
            let record = serde_json::json!({"schema":1,"status":if result.is_ok() {"captured_only"} else {"failed"},"error":result.as_ref().err()});
            write(
                &stage.join("result.json"),
                &serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?,
            )?;
            result?;
            let capture = bounded(&stage.join("session.log"), 64 * MAX_INPUT)?;
            let (client, host) =
                split_capture(std::str::from_utf8(&capture).map_err(|e| e.to_string())?)?;
            write(&stage.join("client.ndjson"), client.as_bytes())?;
            write(&stage.join("native.log"), host.as_bytes())?;
            let metrics = crate::shell_timing::collect_metrics(&client, plan.request.frames)?;
            let peer: PeerConfig = serde_json::from_slice(&bounded(
                &plan.request.artifacts["peer-config"].path,
                MAX_INPUT,
            )?)
            .map_err(|e| e.to_string())?;
            validate_captured_workload(&metrics, &plan.request, peer.probe_bytes)?;
            let offers = &metrics.offers;
            let samples = crate::shell_timing::join(
                &host,
                offers,
                plan.request.mode_millihz,
                plan.request.frames,
            )?;
            let counters = std::str::from_utf8(&capture).map_err(|e| e.to_string())?;
            let progress = if plan.request.instrumentation == "progress" {
                Some(crate::shell_progress::collect(counters)?)
            } else {
                if counters.contains("sophia_comparator_progress ") {
                    return Err("progress diagnostics cannot supply cost or latency arms".into());
                }
                None
            };
            let costs = if plan.request.instrumentation == "progress" {
                None
            } else if plan.request.instrumentation != "none" {
                let report = crate::shell_timing_cost::measure_journal(
                    counters,
                    counters,
                    &client,
                    plan.request.frames,
                )?;
                if report.instrumentation != plan.request.instrumentation {
                    return Err("counter mode differs from prepared instrumentation".into());
                }
                Some(report)
            } else {
                crate::shell_timing_cost::for_mode(&plan.request.instrumentation, counters, offers)?
            };
            let report = serde_json::json!({"schema":2,"verdict":"joined_only","instrumentation":plan.request.instrumentation,"offered":offers.len(),"samples":samples,"costs":costs,"journal":metrics,"progress":progress});
            write(
                &stage.join("native-samples.json"),
                &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
            )?;
            Ok(vec![format!(
                "captured and joined {} (not a B9 verdict)",
                output
            )])
        }
        _ => Err(
            "expected prepare-native REQUEST NEW_PLAN or run-native PLAN SHA256 NEW_RUN_DIR".into(),
        ),
    }
}
