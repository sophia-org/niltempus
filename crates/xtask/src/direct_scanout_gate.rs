// Provenance: moved from Sophia crates/sophia-conformance/src/direct_scanout_gate.rs
// at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). Changes: Sophia is the explicit pinned
// checkout SOPHIA_SOURCE (never this repository), the repository the archive
// binds. Nothing is built: the release binary and the exact pinned tree come
// from prepared physical inputs (SOPHIA_PHYSICAL_INPUTS with its manifest
// sha256 SOPHIA_PHYSICAL_INPUTS_SHA256, made by `xtask
// prepare-physical-inputs --sophia-features=atomic-scanout-live`), verified
// before the session and again before archiving; the direct-scanout fixtures
// (they stay in Sophia) are read from that staged tree. This repository is bound too; the session starts
// through this repository's tools/session/start_sophia_tty3.sh with absolute
// SOPHIA_BIN and SOPHIA_SESSION_PREFLIGHT; the TTY is SOPHIA_SESSION_TTY or the
// descriptor. Sophia's public direct_scanout, direct_scanout_archive and profile
// modules do the verification and archiving.
//! Development orchestration for the direct-scanout physical gate.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use sophia_conformance::{direct_scanout, direct_scanout_archive, profile};

/// The explicit inputs every run needs; nothing defaults to a checkout.
#[derive(Clone, Debug)]
pub struct Sources {
    /// This repository (the launcher lives here).
    pub integration: PathBuf,
    /// The pinned, signed, clean Sophia checkout.
    pub sophia: PathBuf,
    /// The absolute host checker handed to Sophia's launcher.
    pub preflight: PathBuf,
    /// The absolute recipe tool the launcher's adapter runs.
    pub recipes: PathBuf,
    /// The prepared physical inputs and their expected manifest sha256.
    pub inputs: PathBuf,
    pub inputs_sha256: String,
}

impl Sources {
    /// From SOPHIA_SOURCE, SOPHIA_SESSION_PREFLIGHT, SOPHIA_INTEGRATION_XTASK
    /// and SOPHIA_PHYSICAL_INPUTS, each absolute, and
    /// SOPHIA_PHYSICAL_INPUTS_SHA256 (no default for any).
    pub fn from_environment(integration: &Path) -> Result<Self, String> {
        let absolute = |name: &str, executable: bool| -> Result<PathBuf, String> {
            let value = std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| format!("{name} must be an absolute path (no default)"))?;
            if executable && !value.is_file() {
                return Err(format!("{name} must name an executable file"));
            }
            Ok(value)
        };
        Ok(Self {
            integration: integration.to_path_buf(),
            sophia: absolute("SOPHIA_SOURCE", false)?,
            preflight: absolute("SOPHIA_SESSION_PREFLIGHT", true)?,
            recipes: absolute("SOPHIA_INTEGRATION_XTASK", true)?,
            inputs: absolute("SOPHIA_PHYSICAL_INPUTS", false)?,
            inputs_sha256: std::env::var("SOPHIA_PHYSICAL_INPUTS_SHA256")
                .ok()
                .filter(|value| crate::hex(value, 64))
                .ok_or("SOPHIA_PHYSICAL_INPUTS_SHA256 must be the prepared manifest sha256 (no default)")?,
        })
    }
}

/// How long a cost run holds the overlay, in owner-loop ticks and in the
/// probe's own seconds.
///
/// The composed population only exists inside that window, and the promoted
/// client repaints on a cursor blink -- so a window sized for the transition
/// yields a handful of composed frames, which is a story rather than a
/// distribution.
const COST_HOLD_TICKS: u32 = 1_200;
const COST_HOLD_SECONDS: u32 = 35;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub hold_seconds: u32,
    pub workload: String,
    /// Whether the session drives an overlay over a directly scanned frame and
    /// the verification requires the return to composition.
    pub overlay_proof: bool,
    /// Whether the run measures what frames cost. Implies the overlay proof:
    /// the composed population only exists while the overlay is up, so
    /// without it there is nothing to compare a direct frame against.
    pub cost: bool,
    /// Whether the session moves a cursor over directly scanned frames.
    /// Independent of the overlay: this proof wants frames going *to* the
    /// plane throughout, not a return to composition in the middle.
    pub cursor: bool,
    /// Whether the cursor rides the atomic path rather than the legacy
    /// ioctl. Implies `cursor`: the proof is what shows the path works, so
    /// asking for the path without the proof would change how a session
    /// behaves and check nothing.
    pub atomic_cursor: bool,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            width: 2560,
            height: 1440,
            hold_seconds: 20,
            workload: "kitty".to_owned(),
            overlay_proof: false,
            cost: false,
            cursor: false,
            atomic_cursor: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateReport {
    pub source_commit: String,
    pub sophia_sha256: String,
    pub client: PathBuf,
    pub client_sha256: String,
    pub archive: PathBuf,
}

impl Probe {
    pub fn from_arguments(arguments: &[String]) -> Result<Self, String> {
        let mut overlay_proof = false;
        let mut cost = false;
        let mut cursor = false;
        let mut atomic_cursor = false;
        let arguments = arguments
            .iter()
            .filter(|argument| match argument.as_str() {
                "--overlay-proof" => {
                    overlay_proof = true;
                    false
                }
                "--cost" => {
                    cost = true;
                    false
                }
                "--cursor" => {
                    cursor = true;
                    false
                }
                "--atomic-cursor" => {
                    atomic_cursor = true;
                    false
                }
                _ => true,
            })
            .cloned()
            .collect::<Vec<_>>();
        let arguments = arguments.as_slice();
        if arguments.len() > 4 {
            return Err(
                "direct-scanout run accepts WIDTH HEIGHT HOLD WORKLOAD as positional arguments"
                    .to_owned(),
            );
        }
        let mut probe = Self::default();
        if let Some(value) = arguments.first() {
            probe.width = positive(value, "width")?;
        }
        if let Some(value) = arguments.get(1) {
            probe.height = positive(value, "height")?;
        }
        if let Some(value) = arguments.get(2) {
            probe.hold_seconds = positive(value, "hold")?;
        }
        if let Some(value) = arguments.get(3) {
            probe.workload = value.to_owned();
        }
        if !["kitty", "glxgears", "vkcube", "xterm"].contains(&probe.workload.as_str()) {
            return Err(format!(
                "workload must be kitty, glxgears, vkcube, or xterm, got {:?}",
                probe.workload
            ));
        }
        probe.cost = cost;
        probe.cursor = cursor || atomic_cursor;
        probe.atomic_cursor = atomic_cursor;
        // A cost run is an overlay run that holds the window open long
        // enough to have a composed population, so asking for one implies
        // the other rather than requiring both to be spelled.
        probe.overlay_proof = overlay_proof || cost;
        if cost {
            probe.hold_seconds = probe.hold_seconds.max(COST_HOLD_SECONDS);
        }
        Ok(probe)
    }
}

pub fn run_probe(
    sources: &Sources,
    sophia_root: &Path,
    sophia_binary: &Path,
    probe: &Probe,
    client: Option<&Path>,
) -> Result<(), String> {
    profile::check_every_profile(&[])?;
    let mut command = Command::new(
        sources
            .integration
            .join("tools/session/start_sophia_tty3.sh"),
    );
    command
        .current_dir(&sources.integration)
        .env("SOPHIA_ROOT", sophia_root)
        .env("SOPHIA_BIN", sophia_binary)
        .env("SOPHIA_SESSION_PREFLIGHT", &sources.preflight)
        .env("SOPHIA_INTEGRATION_XTASK", &sources.recipes)
        .env("SOPHIA_TTY_PROFILE", "standalone")
        .env("SOPHIA_SESSION_VERBOSE_TRACE", "true")
        .env("SOPHIA_ENABLE_DIRECT_SCANOUT", "1")
        .env("SOPHIA_STANDALONE_WORKLOAD", &probe.workload)
        .env("SOPHIA_STANDALONE_WIDTH", probe.width.to_string())
        .env("SOPHIA_STANDALONE_HEIGHT", probe.height.to_string())
        .env(
            "SOPHIA_STANDALONE_HOLD_SECONDS",
            probe.hold_seconds.to_string(),
        );
    if probe.overlay_proof {
        command.env("SOPHIA_DIRECT_OVERLAY_PROOF", "1");
    }
    if probe.cursor {
        command.env("SOPHIA_DIRECT_CURSOR_PROOF", "1");
    }
    if probe.atomic_cursor {
        command.env("SOPHIA_ATOMIC_CURSOR", "1");
    }
    if probe.cost {
        command.env(
            "SOPHIA_DIRECT_OVERLAY_HOLD_TICKS",
            COST_HOLD_TICKS.to_string(),
        );
    }
    if let Some(client) = client {
        command
            .env("SOPHIA_STANDALONE_APP_BIN", client)
            .env("SOPHIA_DIRECT_SCANOUT_CLIENT_BIN", client);
    }
    let status = command
        .status()
        .map_err(|error| format!("could not start the direct-scanout session: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the direct-scanout session exited with {status}"))
    }
}

pub fn run_gate(sources: &Sources) -> Result<GateReport, String> {
    run_gate_with(sources, &Probe::default())
}

pub fn run_gate_with(sources: &Sources, probe: &Probe) -> Result<GateReport, String> {
    require_tty3()?;
    let repo = sources.sophia.as_path();
    let integration = sources.integration.as_path();
    let pinned = crate::pins::SOPHIA_REV;
    let source_commit = git_output(repo, &["rev-parse", "HEAD"])?;
    if source_commit != pinned {
        return Err(format!(
            "Sophia HEAD {source_commit} is not the pinned revision {pinned}"
        ));
    }
    if !git_output(repo, &["status", "--short"])?.is_empty() {
        return Err("Sophia worktree must be clean before the physical proof".to_owned());
    }
    if !git_status(repo, &["verify-commit", &source_commit])? {
        return Err("physical-proof HEAD lacks a valid signature".to_owned());
    }
    let integration_commit = git_output(integration, &["rev-parse", "HEAD"])?;
    if !git_output(
        integration,
        &["status", "--porcelain", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err("Integration worktree must be clean before the physical proof".to_owned());
    }
    if !git_status(integration, &["verify-commit", &integration_commit])? {
        return Err("Integration HEAD lacks a valid signature".to_owned());
    }
    // A signed commit on a clean tree is the whole identity requirement.
    //
    // The gate used to also demand HEAD equal the locally known
    // origin/master, which added a push to every commit-gate-run cycle
    // without adding anything the archive binds to: the archive names the
    // commit, the commit is signed, and re-verification checks both against
    // this repository -- none of which involves a remote. Where the commit
    // has been pushed is a publishing question, not an evidence one, and a
    // run made before pushing is bound exactly as tightly as one made after.

    let client = std::env::var_os("SOPHIA_STANDALONE_APP_BIN")
        .map(PathBuf::from)
        .map_or_else(|| find_program("kitty"), Ok)?;
    // The prepared inputs: verified against their manifest, built from this
    // pinned revision with atomic-scanout-live, from this integration commit.
    let sealed = crate::physical_inputs::verify(&sources.inputs, &sources.inputs_sha256)?;
    let field = |kind: &str, key: &str| {
        sealed
            .records
            .iter()
            .find(|record| record.kind == kind)
            .and_then(|record| record.fields.iter().find(|(k, _)| k == key))
            .map(|(_, value)| value.as_str())
    };
    if sealed.sophia_commit != source_commit
        || field("sophia", "features") != Some("atomic-scanout-live")
        || field("integration", "commit") != Some(integration_commit.as_str())
    {
        return Err(
            "the prepared inputs are not this Sophia commit with atomic-scanout-live from this integration commit"
                .to_owned(),
        );
    }
    let sophia_root = sources.inputs.join(crate::physical_inputs::SOPHIA_TREE);
    let sophia = sources.inputs.join("bin/sophia");
    let core = sophia_root.join("tools/fixtures/direct_scanout_core.kdl");
    let desktop = sophia_root.join("tools/fixtures/direct_scanout_desktop.kdl");
    if git_output(repo, &["rev-parse", "HEAD"])? != source_commit
        || !git_output(repo, &["status", "--short"])?.is_empty()
        || git_output(integration, &["rev-parse", "HEAD"])? != integration_commit
        || !git_output(
            integration,
            &["status", "--porcelain", "--untracked-files=all"],
        )?
        .is_empty()
    {
        return Err("Sophia or integration source identity changed".to_owned());
    }
    if !git_status(repo, &["verify-commit", &source_commit])? {
        return Err("Sophia signature no longer verifies".to_owned());
    }

    let sophia_sha256 = direct_scanout_archive::sha256(&sophia)?;
    let client_sha256 = direct_scanout_archive::sha256(&client)?;

    let state_home = state_home()?;
    let session_log = state_home.join("sophia/standalone-session/session.log");
    match fs::remove_file(&session_log) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "could not remove prior evidence {}: {error}",
                session_log.display()
            ));
        }
    }
    run_probe(sources, &sophia_root, &sophia, probe, Some(&client))?;
    if !session_log.is_file() {
        return Err(format!(
            "the direct-scanout session produced no evidence: {}",
            session_log.display()
        ));
    }

    let evidence = std::env::var_os("SOPHIA_DIRECT_SCANOUT_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/sophia-direct-scanout.log"));
    direct_scanout_archive::bind_evidence(&direct_scanout_archive::BindEvidence {
        session_log: &session_log,
        evidence: &evidence,
        source_commit: &source_commit,
        sophia_binary: &sophia,
        client_binary: &client,
        core_config: &core,
        desktop_profile: &desktop,
    })?;
    direct_scanout::verify_standalone_logs_proving(
        &[evidence.display().to_string()],
        probe.overlay_proof,
        probe.cost,
        probe.cursor,
    )?;
    // The prepared inputs are verified again before archiving.
    crate::physical_inputs::verify(&sources.inputs, &sources.inputs_sha256)?;
    let run_root = std::env::var_os("SOPHIA_DIRECT_SCANOUT_RUN_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_home.join("sophia/promotion/direct-scanout-runs"));
    let archive = direct_scanout_archive::create_archive(&direct_scanout_archive::CreateArchive {
        repo,
        evidence: &evidence,
        run_root: &run_root,
        sophia_binary: &sophia,
        client_binary: &client,
    })?;
    Ok(GateReport {
        source_commit,
        sophia_sha256,
        client,
        client_sha256,
        archive,
    })
}

fn positive(value: &str, name: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{name} must be a positive integer, got {value:?}"))
}

/// Which terminal this run targets: SOPHIA_SESSION_TTY when set, otherwise the
/// terminal this process is attached to, read from the descriptor rather than
/// from a subprocess.
///
/// `Command::output` closes the child's stdin, so asking `tty(1)` what our
/// terminal is asks it about a null descriptor: it answered "not a tty" and
/// exited 1 on a real tty3, which refused the gate before it could start.
/// The descriptor is right here; nothing needs to be asked.
fn current_terminal() -> Result<PathBuf, String> {
    if let Some(explicit) = std::env::var_os("SOPHIA_SESSION_TTY") {
        return Ok(PathBuf::from(explicit));
    }
    std::fs::read_link("/proc/self/fd/0")
        .map_err(|error| format!("could not read this process's terminal: {error}"))
}

/// Whether this terminal is the one the gate runs on.
///
/// Split from the descriptor read so the decision is testable without a
/// terminal: the read is one syscall and the decision is the rule.
pub fn is_gate_terminal(terminal: &Path) -> bool {
    terminal == Path::new("/dev/tty3")
}

fn require_tty3() -> Result<(), String> {
    if !std::io::stdin().is_terminal() {
        return Err("switch to tty3, log in, and run: tools/direct_scanout_gate.sh".to_owned());
    }
    let terminal = current_terminal()?;
    if is_gate_terminal(&terminal) {
        Ok(())
    } else {
        Err(format!(
            "switch to tty3, log in, and run: tools/direct_scanout_gate.sh (current terminal: {})",
            terminal.display()
        ))
    }
}

fn state_home() -> Result<PathBuf, String> {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(state));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".local/state"))
        .ok_or_else(|| "HOME and XDG_STATE_HOME are unset".to_owned())
}

fn find_program(name: &str) -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH").ok_or("PATH is unset")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("the direct-scanout gate requires {name}"))
}

fn git_output(repo: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        return Err(format!("git exited with {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|error| format!("git emitted non-UTF-8: {error}"))
}

fn git_status(repo: &Path, arguments: &[&str]) -> Result<bool, String> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .status()
        .map(|status| status.success())
        .map_err(|error| format!("could not run git: {error}"))
}
