//! Verify one attended output-file native proof run offline
//! (`cargo xtask output-file-native verify`).
//!
//! ```text
//! cargo xtask output-file-native verify \
//!     --inputs=/ABS/PHYSICAL-INPUTS --inputs-manifest-sha256=<64> \
//!     --preparation=/ABS/SOPHIA-PREPARED --preparation-sha256=<64> \
//!     --run=/ABS/RUN --run-manifest-sha256=<64>
//! ```
//!
//! Reads files only: no device, discovery, build or network. Identity comes
//! from existing custody. [`physical_inputs::verify`] binds Sophia, Hagia, the
//! profile and the binaries; [`c_sdk_pin::verify`] binds the staged tree's
//! vendored C SDK to this repository's pin. Sophia's `prepared.json` binds
//! the generic peer and its device-free harnesses to the same Sophia commit
//! and SDK. `run.manifest`, written by the runner with [`RunManifest`],
//! attests the four Session runs (validate, reject, commit-restore, peer
//! death): each log's digest, exit status and exact argv. It is checked
//! against the sealed inputs and the logs; it cannot prove what was
//! executed, only that the attestation, inputs and evidence agree.
//!
//! The runner shares this custody: [`bind`] before any physical launch,
//! [`check_argv`] on each Session argv it builds (from [`peer_argv`]), then
//! [`RunManifest::render`] and [`verify`] afterwards.
//!
//! Each stage log holds Session output with the supervised peer's inherited
//! stdout, in arrival order. Only named records are read; the peer's exit
//! and its socket disconnect may appear in either order. Limits this gate
//! keeps: global origins and the primary output are bound only by the
//! declared layouts and the peer's snapshot matches; KMS and owner rows are
//! not pixels; head disabling, routing changes and leaks outside the selected
//! objects are not qualified.
use std::path::{Path, PathBuf};

use crate::physical_inputs::{self, Sealed};
use crate::{c_sdk_pin, hex, read};

mod custody;
mod evidence;
mod verdict;

use custody::read_log;
pub use custody::{
    Bound, Layout, Paths, RunManifest, StageRecord, bind, bind_sealed, check_argv,
    integration_commit, peer_argv,
};
use verdict::verify_stage;

pub const RUN_MANIFEST: &str = "run.manifest";
pub const PREPARED: &str = "prepared.json";
/// Stage names in run order, with the peer's `--stage` for each.
pub const STAGES: [(&str, &str); 4] = [
    ("validate", "validate"),
    ("reject", "reject"),
    ("commit-restore", "commit-restore"),
    ("peer-death", "apply-await-termination"),
];
// Provenance: Sophia crates/xtask/src/output_file_native_proof.rs
// (EXPORT_TESTS, SESSION_TEST) at the t253 candidate. The preparation must
// have run exactly these device-free cases.
pub const EXPORT_TESTS: [&str; 13] = [
    "proof::fixtures_are_valid_and_b_differs_verifiably",
    "validate_stage_receives_exactly_one_validated_outcome",
    "reject_stage_unknown_mode_is_refused_at_transport_admission",
    "commit_restore_stage_commits_b_then_restores_a",
    "apply_await_termination_stage_is_ended_by_its_supervisor",
    "baseline_waits_below_the_declared_epoch_then_proceeds",
    "baseline_failures_exit_three_without_submitting",
    "outcome_before_termination_exits_five",
    "unterminated_peer_exits_six_at_its_deadline",
    "refused_arguments_exit_two_before_connecting",
    "outcome_with_the_wrong_topology_epoch_exits_four",
    "scripted_source_without_reuse_carries_commit_restore",
    "reused_publication_qid_exits_four",
];
pub const SESSION_TEST: &str = "live_session::reload::tests::desktop_launch_reload::output_file_recovery::native_session::native_proof_peer_uses_session_supervision_and_owner_settlement";
pub const PROFILE_OWNERS: [&str; 4] = ["sophia", "hagia", "narthex", "integration"];
/// Session's local startup transaction.
const STARTUP_TRANSACTION: u64 = u64::MAX;
const MAX_LOG_BYTES: u64 = 64 << 20;
const MAX_MANIFEST_BYTES: u64 = 1 << 20;
const MAX_PREPARED_BYTES: u64 = 64 << 10;
const MAX_PEER_DEADLINE_MS: u64 = 120_000;

/// The validated command line.
#[derive(Clone, Debug)]
pub struct Request {
    pub inputs: PathBuf,
    pub inputs_sha256: String,
    pub preparation: PathBuf,
    pub preparation_sha256: String,
    pub run: PathBuf,
    pub run_sha256: String,
}

impl Request {
    /// The options after `verify`, each exactly once.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        const KEYS: [&str; 6] = [
            "--inputs",
            "--inputs-manifest-sha256",
            "--preparation",
            "--preparation-sha256",
            "--run",
            "--run-manifest-sha256",
        ];
        let mut values: [Option<&str>; 6] = [None; 6];
        for arg in args {
            let (key, value) = arg
                .split_once('=')
                .ok_or_else(|| format!("expected --key=value, not {arg:?}"))?;
            let index = KEYS
                .iter()
                .position(|k| *k == key)
                .ok_or_else(|| format!("unknown option {key}"))?;
            if values[index].replace(value).is_some() {
                return Err(format!("{key} given twice"));
            }
        }
        let get = |i: usize| values[i].ok_or_else(|| format!("{} is required", KEYS[i]));
        let path = |i: usize| -> Result<PathBuf, String> {
            let path = PathBuf::from(get(i)?);
            if !path.is_absolute() {
                return Err(format!("{} must be absolute", KEYS[i]));
            }
            Ok(path)
        };
        let digest = |i: usize| -> Result<String, String> {
            let value = get(i)?;
            if !hex(value, 64) {
                return Err(format!("{} must be 64 lowercase hex", KEYS[i]));
            }
            Ok(value.to_owned())
        };
        Ok(Self {
            inputs: path(0)?,
            inputs_sha256: digest(1)?,
            preparation: path(2)?,
            preparation_sha256: digest(3)?,
            run: path(4)?,
            run_sha256: digest(5)?,
        })
    }
}

/// What a passing run established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    pub bound: Bound,
    pub epoch_a: u64,
    pub stages: Vec<String>,
}

/// `cargo xtask output-file-native verify ...`: binds this checkout's clean,
/// signed HEAD as the integration commit, then verifies the run.
pub fn run(repo: &Path, args: &[String]) -> Result<Vec<String>, String> {
    let [command, rest @ ..] = args else {
        return Err("usage: output-file-native verify --inputs=... (see module docs)".into());
    };
    if command != "verify" {
        return Err(format!("unknown output-file-native command {command:?}"));
    }
    let request = Request::parse(rest)?;
    let integration = integration_commit(repo)?;
    let verified = verify(repo, &request, &integration)?;
    if integration_commit(repo)? != integration {
        return Err("integration checkout changed during verification".into());
    }
    let bound = &verified.bound;
    Ok(vec![format!(
        "output-file-native: PASS stages={} integration={} sophia={} sdk={} hagia={} profile={} profile_sha256={} peer={} epoch_a={}",
        verified.stages.len(),
        bound.integration_commit,
        bound.sophia_commit,
        bound.sdk_revision,
        bound.hagia_commit,
        bound.profile,
        bound.profile_sha256,
        bound.peer_sha256,
        verified.epoch_a,
    )])
}

/// Verify the whole run.
pub fn verify(repo: &Path, request: &Request, integration: &str) -> Result<Verified, String> {
    let sealed = physical_inputs::verify(&request.inputs, &request.inputs_sha256)?;
    let sdk = c_sdk_pin::verify(
        &sealed
            .out
            .join(physical_inputs::SOPHIA_TREE)
            .join("vendor/c-desktop-sdk"),
        repo,
    )?;
    verify_sealed(repo, &sealed, &sdk, request, integration)
}

/// [`verify`] once the sealed inputs and the staged SDK revision are known.
/// Exposed so tests can supply synthetic sealed inputs.
pub fn verify_sealed(
    repo: &Path,
    sealed: &Sealed,
    sdk_revision: &str,
    request: &Request,
    integration: &str,
) -> Result<Verified, String> {
    let manifest = RunManifest::read(&request.run, &request.run_sha256)?;
    if manifest.integration_commit != integration
        || manifest.inputs_sha256 != request.inputs_sha256
        || manifest.preparation_sha256 != request.preparation_sha256
    {
        return Err("run.manifest names other inputs, preparation or integration commit".into());
    }
    let bound = bind_sealed(
        repo,
        sealed,
        sdk_revision,
        &request.preparation,
        &request.preparation_sha256,
        &manifest.profile,
        integration,
    )?;
    let mut stages = Vec::new();
    for stage in &manifest.stages {
        verify_stage_record(stage, &manifest.a, &manifest.b, &bound.paths, &request.run)?;
        stages.push(stage.name.clone());
    }
    Ok(Verified {
        bound,
        epoch_a: manifest.a.epoch,
        stages,
    })
}

/// One attested stage, exactly as [`verify`] checks it inside a run: a known
/// stage name and its canonical log path, its argv against the bound paths
/// and declared layouts, a zero Session exit, its log's size and digest
/// under `run`, and its verdict. The runner calls this after each stage and
/// before launching the next, so a failed stage stops the physical run.
pub fn verify_stage_record(
    stage: &StageRecord,
    a: &Layout,
    b: &Layout,
    paths: &Paths,
    run: &Path,
) -> Result<(), String> {
    if !STAGES.iter().any(|(name, _)| *name == stage.name)
        || stage.log != format!("{}/session.log", stage.name)
        || !hex(&stage.sha256, 64)
    {
        return Err(format!("stage record {:?} is malformed", stage.name));
    }
    check_argv(&stage.name, &stage.argv, a, b, paths)
        .map_err(|e| format!("{} argv: {e}", stage.name))?;
    // Peer death too: ordinary bounded shutdown after the proof passed.
    if stage.session_exit != 0 {
        return Err(format!(
            "{} Session exited {}",
            stage.name, stage.session_exit
        ));
    }
    real_dir(run)?;
    let text = read_log(run, stage)?;
    verify_stage(stage, a, &text).map_err(|e| format!("{} log: {e}", stage.name))
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("non-UTF-8 path {}", path.display()))
}

/// A regular file, not a link, within a size bound.
fn regular(path: &Path, bound: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if meta.len() > bound {
        return Err(format!("{} exceeds {bound} bytes", path.display()));
    }
    read(path)
}

fn real_dir(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        return Err(format!(
            "{} must be an absolute real directory",
            path.display()
        ));
    }
    Ok(())
}

/// Canonical unsigned decimal.
fn unsigned(value: &str, what: &str) -> Result<u64, String> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(format!(
            "{what} is not a canonical unsigned integer: {value:?}"
        ));
    }
    value
        .parse()
        .map_err(|_| format!("{what} does not fit in u64: {value:?}"))
}
