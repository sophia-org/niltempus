//! Bind a reviewed shell workload to exact files before any attended launch.
use crate::shell_timing::{DrawSource, DrawWorkload, ImageSource, WorkloadMode};
use crate::{hex, output_file_native_run::process, sha256};
use serde::{Deserialize, Serialize};
mod external;
pub use external::ExternalTiming;
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
    /// Explicit new timing policies; absent preserves historical plans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_timing: Option<ExternalTiming>,
    /// Schema-6 reference-client workload; absent preserves every
    /// historical peer-config plan. When present, the peer-config
    /// artifact is the reference client's own config
    /// (wire/output/frames/source/workload), validated by
    /// `validate_reference` instead of the legacy `PeerConfig` shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_workload: Option<DrawWorkload>,
    /// Explicit producer binding: absent means the historical CPU
    /// producer (GPU denied). `gpu` permits and requires the direct
    /// grant for the GPU peers (I/S/G); `dma_buf` necessarily requires
    /// it. Frozen into the plan like every other request field.
    #[serde(default, skip_serializing_if = "ProducerKind::is_cpu")]
    pub producer: ProducerKind,
}

/// Who renders the peer's pixels.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProducerKind {
    #[default]
    Cpu,
    Gpu,
}

impl ProducerKind {
    fn is_cpu(&self) -> bool {
        matches!(self, Self::Cpu)
    }
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
    /// Absent means the historical fresh workload. Explicit JSON null is
    /// invalid (peer parity): the enum itself refuses it, not an Option.
    #[serde(default)]
    workload: WorkloadMode,
    /// Absent means no pool. A present value must be an integer: explicit
    /// null is invalid (peer parity), so this rejects null instead of
    /// collapsing it to absence.
    #[serde(default, deserialize_with = "present_u64")]
    retained_resources: Option<u64>,
    #[serde(default, deserialize_with = "present_u64")]
    interval_msec: Option<u64>,
    #[serde(default, deserialize_with = "present_u64")]
    ring_slots: Option<u64>,
    image: Option<ImageConfig>,
}

fn present_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    u64::deserialize(deserializer).map(Some)
}

/// The reference client's own configuration, exactly as
/// `shell_draw_reference` parses it: no other keys exist. `source`
/// uses the journal spellings (draw/ring/mixed); `Request::source`
/// uses the runner spellings (draw/owned_ring/mixed) and the two are
/// reconciled only through `DrawSource::image_source`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceConfig {
    wire: String,
    output: u64,
    frames: usize,
    source: DrawSource,
    workload: DrawWorkload,
    #[serde(default = "reference_hold_ms")]
    hold_ms: u64,
}

fn reference_hold_ms() -> u64 {
    5000
}

fn validate_reference(request: &Request, bytes: &[u8]) -> Result<(), String> {
    let Some(workload) = request.reference_workload else {
        return Err("reference validation requires a declared workload".into());
    };
    if request.external_timing.is_some() {
        return Err("external timing is not defined for reference workloads".into());
    }
    let config: ReferenceConfig = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if config.wire != "files"
        || config.output != request.output
        || config.output == 0
        || config.frames != request.frames
        || config.workload != workload
        || config.source.image_source() != request.source
        || !(1000..=120_000).contains(&config.hold_ms)
    {
        return Err("reference configuration differs from the declared workload".into());
    }
    Ok(())
}

impl PeerConfig {
    /// Validated workload declaration: retained needs files, a sealed or ring
    /// source and an explicit 2..=8 pool; fresh forbids a pool declaration.
    /// Tile geometry is deliberately not plan-checked here: neither the
    /// request nor this config carries the output/allocation width, so the
    /// package analyzer must bind every retained upload's width to
    /// floor(allocation_width / N) rounded down even, its height to this
    /// config's height, and placements to slots inside that width.
    fn workload(&self) -> Result<(WorkloadMode, Option<u64>), String> {
        let mode = self.workload;
        let retained = mode.check(self.retained_resources)?;
        if self.source == ImageSource::DmaBuf {
            // GPU DMA-BUF uploads are fresh-only: bounds come from the
            // negotiated content limits, never from config image bounds,
            // a probe, a pool or a pacing interval.
            if self.wire != "files"
                || mode != WorkloadMode::Fresh
                || self.probe_bytes != 0
                || self.ring_slots.is_some()
                || self.interval_msec.is_some()
                || self.image.is_some()
            {
                return Err(
                    "dma_buf requires the files wire, a fresh workload and no image/probe/pool keys"
                        .into(),
                );
            }
        }
        if self.source == ImageSource::OwnedRing {
            // Retained ring tiles are vsync-paced, as the peer refuses an interval.
            if self.wire != "files"
                || self.probe_bytes != 0
                || (mode == WorkloadMode::Retained && self.interval_msec.is_some())
                || !self
                    .ring_slots
                    .is_some_and(|slots| (2..=4).contains(&slots))
            {
                return Err(
                    "owned ring requires a file panel, ring_slots in 2..=4 and no paced retained pool"
                        .into(),
                );
            }
        } else if self.ring_slots.is_some() {
            return Err("ring_slots requires owned_ring source".into());
        }
        if mode == WorkloadMode::Retained
            && (self.wire != "files"
                || !matches!(self.source, ImageSource::Isb | ImageSource::OwnedRing))
        {
            return Err("retained workload requires files wire and a sealed or ring source".into());
        }
        Ok((mode, retained))
    }
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
    if request.reference_workload.is_some() {
        if request.wire != "files"
            || !matches!(
                request.source,
                ImageSource::Draw | ImageSource::OwnedRing | ImageSource::Mixed
            )
        {
            return Err("reference workloads require files wire and a draw/ring source".into());
        }
        validate_reference(request, &texts["peer-config"])?;
    } else {
        let peer: PeerConfig =
            serde_json::from_slice(&texts["peer-config"]).map_err(|e| e.to_string())?;
        peer.workload()?;
        external::validate(request, &peer)?;
        if peer.wire != request.wire
            || peer.source != request.source
            || (peer.source == ImageSource::Isb && request.wire != "files")
            || match peer.source {
                ImageSource::Isb => !peer.image.as_ref().is_some_and(ImageConfig::valid),
                ImageSource::Bytes | ImageSource::OwnedRing | ImageSource::DmaBuf => {
                    peer.image.is_some()
                }
                // Reference sources only validate through the
                // reference_workload branch above.
                ImageSource::Draw | ImageSource::Mixed => true,
            }
            || (peer.source == ImageSource::OwnedRing && request.instrumentation != "none")
            || peer.output != request.output
            || peer.frames != request.frames
            || !(1..=512).contains(&peer.height)
            || !matches!(peer.probe_bytes, 0 | 4194304)
            || peer.hold_msec != request.runtime_ms
        {
            return Err(
                "peer configuration differs from the declared workload or full-runtime hold".into(),
            );
        }
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
    // The producer binding, not the source, decides the grant: GPU
    // peers (I/S/G) require direct, CPU peers stay denied, and a
    // DMA-BUF source without the GPU binding is refused outright.
    let gpu = match (request.producer, request.source) {
        (ProducerKind::Gpu, _) => "direct",
        (ProducerKind::Cpu, ImageSource::DmaBuf) => {
            return Err("dma_buf requires the gpu producer binding".into());
        }
        (ProducerKind::Cpu, _) => "denied",
    };
    for (name, expected) in [
        ("executable", absolute(&request.artifacts["peer"].path)?),
        ("config", absolute(&request.artifacts["peer-config"].path)?),
        ("transport", wire),
        ("gpu", gpu),
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
/// The journal's workload mode and pool size must match the prepared
/// peer-config exactly; a fresh plan never accepts a retained journal.
pub fn validate_captured_workload(
    metrics: &crate::shell_timing::Journal,
    request: &Request,
    probe_bytes: u64,
    workload: WorkloadMode,
    retained_resources: Option<u64>,
) -> Result<(), String> {
    workload.check(retained_resources)?;
    if metrics.probe_bytes != probe_bytes
        || metrics.source != request.source
        || metrics.wire != request.wire
        || metrics.workload != workload
        || metrics.retained_resources != retained_resources
        || metrics.interval_msec
            != request
                .external_timing
                .as_ref()
                .and_then(ExternalTiming::interval)
    {
        return Err("captured upload workload differs from prepared configuration".into());
    }
    Ok(())
}

/// Validate the collected journal against the same config used at preparation.
/// Schema 6 never passes through the legacy image-peer config decoder.
pub fn validate_captured_config(
    metrics: &crate::shell_timing::Journal,
    request: &Request,
    config: &[u8],
) -> Result<(), String> {
    if let Some(workload) = request.reference_workload {
        validate_reference(request, config)?;
        let config: ReferenceConfig = serde_json::from_slice(config).map_err(|e| e.to_string())?;
        let evidence = metrics
            .draw
            .as_ref()
            .ok_or("reference journal lacks draw evidence")?;
        if metrics.schema != 6
            || metrics.wire != request.wire
            || metrics.source != request.source
            || evidence.source.image_source() != request.source
            || evidence.workload != workload
            || evidence.hold_usec < config.hold_ms * 1000
            || metrics.offers.len() != request.frames
            || metrics
                .offers
                .iter()
                .any(|offer| offer.output != request.output)
        {
            return Err("captured reference workload differs from prepared configuration".into());
        }
        return Ok(());
    }
    if metrics.draw.is_some() {
        return Err("reference journal supplied for a legacy image plan".into());
    }
    let peer: PeerConfig = serde_json::from_slice(config).map_err(|e| e.to_string())?;
    let (workload, retained) = peer.workload()?;
    validate_captured_workload(metrics, request, peer.probe_bytes, workload, retained)?;
    if metrics.ring.as_ref().map(|ring| ring.registration.slots) != peer.ring_slots
        || metrics.source == ImageSource::OwnedRing
            && metrics
                .uploads
                .iter()
                .any(|copy| u64::from(copy.upload.height) != peer.height)
    {
        return Err("captured ring geometry differs from prepared configuration".into());
    }
    Ok(())
}

/// A reference peer must timestamp submission and receipt in the kernel
/// completion's clock domain. Corrected vblank timestamps can lead receipt
/// (about 0.5 ms in d4p-03); allow one nominal refresh after the terminal.
/// This is a clock-consistency check, not a latency acceptance gate.
pub fn validate_reference_timestamps(
    metrics: &crate::shell_timing::Journal,
    samples: &[crate::shell_timing::Sample],
    mode_millihz: u64,
) -> Result<(), String> {
    if metrics.schema != 6 {
        return Ok(());
    }
    if mode_millihz == 0 {
        return Err("reference clock check requires a nonzero refresh rate".into());
    }
    let refresh_usec = 1_000_000_000u64.div_ceil(mode_millihz);
    if samples.len() != metrics.offers.len() || samples.len() != metrics.terminal_usec.len() {
        return Err("reference completion timestamp count differs".into());
    }
    for ((sample, offer), observed) in samples
        .iter()
        .zip(&metrics.offers)
        .zip(&metrics.terminal_usec)
    {
        if (
            sample.connection_epoch,
            sample.content_grant_epoch,
            sample.output,
            sample.candidate_generation,
        ) != (
            offer.connection_epoch,
            offer.content_grant_epoch,
            offer.output,
            offer.candidate_generation,
        ) || sample.completed_monotonic_usec < offer.submitted_monotonic_usec
            || sample.completed_monotonic_usec > observed.saturating_add(refresh_usec)
        {
            return Err(
                "reference kernel completion is outside the submit/terminal clock window".into(),
            );
        }
    }
    Ok(())
}

/// Compare profile policy across campaign arms while allowing only the peer
/// executable/config paths and selected wire to differ. All other KDL values,
/// properties and nodes remain part of the comparison.
pub fn campaign_profile(request: &Request) -> Result<String, String> {
    profile_for_height(request, 24)
}

/// Diagnostic pilots may declare a larger presented surface; campaign callers
/// retain their fixed 24px contract through campaign_profile.
pub(crate) fn profile_for_height(request: &Request, height: u32) -> Result<String, String> {
    if !matches!(height, 24 | 512) {
        return Err("unsupported pilot surface height".into());
    }
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
    if peer.height != u64::from(height) {
        return Err(if height == 24 {
            "campaign panel height must be 24"
        } else {
            "pilot surface height must be 512"
        }
        .into());
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
    verify_prepared_for(bytes, false)
}

/// The panel diagnostic supports reference-client plans; legacy campaigns do not.
pub fn verify_panel_prepared(bytes: &[u8]) -> Result<(Request, u64), String> {
    verify_prepared_for(bytes, true)
}

fn verify_prepared_for(bytes: &[u8], reference: bool) -> Result<(Request, u64), String> {
    if bytes.len() > MAX_INPUT as usize {
        return Err("plan exceeds input bound".into());
    }
    let plan: Plan = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if plan.request.reference_workload.is_some() && !reference {
        return Err("reference plans require the panel diagnostic".into());
    }
    if plan.schema != 1 || validate(&plan.request)? != plan.argv {
        return Err("plan arguments differ".into());
    }
    let peer_bytes = bounded(&plan.request.artifacts["peer-config"].path, MAX_INPUT)?;
    // Bind the second read too: never accept workload metadata from a changed file.
    if sha256(&peer_bytes) != plan.request.artifacts["peer-config"].sha256 {
        return Err("peer configuration changed during verification".into());
    }
    let probe_bytes = if plan.request.reference_workload.is_some() {
        validate_reference(&plan.request, &peer_bytes)?;
        0 // The reference config has no probe field or probe workload.
    } else {
        let peer: PeerConfig = serde_json::from_slice(&peer_bytes).map_err(|e| e.to_string())?;
        peer.probe_bytes
    };
    Ok((plan.request, probe_bytes))
}

/// Attended campaign hooks. Preparation happens only after console, source,
/// artifact and runner checks, with a new private stage directory. `finish`
/// must retain evidence and stop owned load workers even when launch fails.
pub trait CampaignObserver {
    fn prepare(
        &mut self,
        request: &Request,
        host_argv: &[String],
        stage: &Path,
    ) -> Result<PathBuf, String>;
    fn observe(&mut self, wrapper_pid: u32, logs: &Path) -> Result<(), String>;
    fn finish(&mut self, stage: &Path) -> Result<(), String>;
}

pub fn run(args: &[String]) -> Result<Vec<String>, String> {
    run_inner(args, None)
}

/// Read-only launch prerequisites, shared with the campaign entry point so
/// routine preflight refusals do not consume its single-use arm claim.
pub(crate) fn launch_prerequisites(request: &Request) -> Result<(), String> {
    process::attended_with(&request.tty, "SOPHIA_SHELL_TIMING_ARM")?;
    validate_pam_helper(&request.artifacts["pam-helper"].path)?;
    source(&request.sophia_source)?;
    source(&request.integration_source)?;
    git(
        &request.sophia_source,
        &["ls-files", "--error-unmatch", "tools/run_sophia_session.sh"],
    )?;
    if std::env::current_exe()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?
        != request.artifacts["runner"]
            .path
            .canonicalize()
            .map_err(|e| e.to_string())?
    {
        return Err("run with the prepared runner executable".into());
    }
    let running = fs::metadata("/proc/self/exe").map_err(|e| e.to_string())?;
    let prepared = fs::metadata(&request.artifacts["runner"].path).map_err(|e| e.to_string())?;
    if (running.dev(), running.ino()) != (prepared.dev(), prepared.ino())
        || file_digest(Path::new("/proc/self/exe"), 1024 * MAX_INPUT)?
            != request.artifacts["runner"].sha256
    {
        return Err("running image differs from the prepared runner artifact".into());
    }
    Ok(())
}

pub(crate) fn stage_available(stage: &Path) -> Result<(), String> {
    absolute(stage)?;
    match fs::symlink_metadata(stage) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
        Ok(_) => Err("run directory already exists".into()),
    }
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
            launch_prerequisites(&plan.request)?;
            let stage = Path::new(output);
            stage_available(stage)?;
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
                    Some(observer) => Some(observer.prepare(&plan.request, &plan.argv, stage)?),
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
            let peer_bytes = bounded(&plan.request.artifacts["peer-config"].path, MAX_INPUT)?;
            if sha256(&peer_bytes) != plan.request.artifacts["peer-config"].sha256 {
                return Err("peer configuration changed during capture".into());
            }
            validate_captured_config(&metrics, &plan.request, &peer_bytes)?;
            let offers = &metrics.offers;
            let samples = crate::shell_timing::join(
                &host,
                offers,
                plan.request.mode_millihz,
                plan.request.frames,
            )?;
            validate_reference_timestamps(&metrics, &samples, plan.request.mode_millihz)?;
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
            let mut report = serde_json::json!({"schema":2,"verdict":"joined_only","instrumentation":plan.request.instrumentation,"offered":offers.len(),"samples":samples,"costs":costs,"journal":metrics,"progress":progress});
            if let Some(timing) = &plan.request.external_timing {
                report["external_timing"] =
                    serde_json::to_value(timing).map_err(|e| e.to_string())?;
                report["external_measurement_status"] =
                    "frame_join_only_sampling_window_unproven".into();
            }
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
