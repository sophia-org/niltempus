//! Prepare an inspectable native-output run, then execute that exact plan from
//! its named text console. Preparation never opens a device or starts Session.
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::output_file_native::{self as proof, Bound, Layout, RunManifest, StageRecord};
use crate::{hex, read, sha256};

#[path = "output_file_native_run/process.rs"]
mod process;

/// Validate the generic wrapper's recorded console restoration without devices.
pub fn check_recovery(text: &str) -> Result<(), String> {
    process::check_recovery(text)
}

/// Compare the kernel's active VT with the plan, without opening a device.
pub fn check_foreground_tty(tty: &str, active: &str) -> Result<(), String> {
    let active = active.strip_suffix('\n').unwrap_or(active);
    let number = active
        .strip_prefix("tty")
        .ok_or("invalid active console record")?;
    let number = self::number(number)?;
    if !(1..=63).contains(&number) || active != format!("tty{number}") {
        return Err("invalid active console record".into());
    }
    if tty != format!("/dev/{active}") {
        return Err(format!(
            "Return to {tty}: the foreground console is /dev/{active}. Keep {tty} in the foreground until all four output stages finish."
        ));
    }
    Ok(())
}

const PLAN: &str = "run-plan.json";
const KEYS: &[&str] = &[
    "inputs",
    "inputs-manifest-sha256",
    "preparation",
    "preparation-sha256",
    "profile",
    "out",
    "tty",
    "display",
    "input-seat",
    "runtime-ms",
    "a-topology-epoch",
    "a-heads",
    "a-groups",
    "a-primary",
    "b-heads",
    "b-groups",
    "b-primary",
    "preflight",
    "preflight-sha256",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema: u32,
    integration_commit: String,
    options: BTreeMap<String, String>,
    argv: Vec<Vec<String>>,
}

pub fn run(repo: &Path, args: &[String]) -> Result<Vec<String>, String> {
    match args.split_first() {
        Some((command, rest)) if command == "prepare-run" => prepare(repo, rest),
        Some((command, rest)) if command == "run" => execute(repo, rest),
        _ => Err("expected output-file-native prepare-run|run".into()),
    }
}

fn options(args: &[String], keys: &[&str]) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    for arg in args {
        let (key, value) = arg
            .strip_prefix("--")
            .and_then(|s| s.split_once('='))
            .ok_or("every option requires --name=value")?;
        if !keys.contains(&key)
            || value.is_empty()
            || values.insert(key.to_owned(), value.to_owned()).is_some()
        {
            return Err(format!("unknown, empty or repeated option: {key}"));
        }
    }
    for key in keys {
        if !values.contains_key(*key) {
            return Err(format!("missing --{key}"));
        }
    }
    Ok(values)
}

fn number(value: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("expected unsigned decimal, got {value:?}"));
    }
    value.parse().map_err(|e| format!("{value:?}: {e}"))
}

fn layouts(values: &BTreeMap<String, String>) -> Result<(Layout, Layout), String> {
    let layout = |name: &str, epoch| Layout {
        epoch,
        heads: values[&format!("{name}-heads")].clone(),
        groups: values[&format!("{name}-groups")].clone(),
        primary: values[&format!("{name}-primary")].clone(),
    };
    Ok((
        layout("a", number(&values["a-topology-epoch"])?),
        layout("b", 0),
    ))
}

/// Pure argv construction: no environment defaults, devices or subprocesses.
pub fn arguments(
    values: &BTreeMap<String, String>,
    paths: &proof::Paths,
) -> Result<Vec<Vec<String>>, String> {
    for key in KEYS {
        if !values.contains_key(*key) {
            return Err(format!("missing {key}"));
        }
    }
    let tty = values["tty"]
        .strip_prefix("/dev/tty")
        .ok_or("tty must be /dev/ttyN")?;
    if !(1..=63).contains(&number(tty)?) {
        return Err("tty must be /dev/tty1..63".into());
    }
    let display = values["display"]
        .strip_prefix(':')
        .ok_or("display must be :N")?;
    let display = number(display)?;
    if !(90..=99).contains(&display) {
        return Err("attended proof uses a private display :90..:99".into());
    }
    let seat = &values["input-seat"];
    if seat.len() > 64
        || seat.is_empty()
        || !seat
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        return Err("invalid explicit input seat".into());
    }
    let runtime = number(&values["runtime-ms"])?;
    if !(60_000..=600_000).contains(&runtime) {
        return Err("runtime-ms must be 60000..600000 per stage".into());
    }
    let (a, b) = layouts(values)?;
    proof::STAGES
        .iter()
        .map(|(stage, _)| {
            let mut argv = vec![
                paths.sophia.clone(),
                "session".into(),
                "run".into(),
                "--session-mode=normal".into(),
                "--native-scanout".into(),
                format!("--display={}", values["display"]),
                format!("--input-seat={seat}"),
                format!("--max-runtime-ms={runtime}"),
                format!(
                    "--config={}/sophia-tree/tools/config/sophia/core.kdl",
                    values["inputs"]
                ),
                format!("--desktop-profile={}", paths.profile),
                format!("--wm-process={}", paths.hagia),
                format!("--output-process={}", paths.peer),
                "--output-proof-readback".into(),
            ];
            if *stage == "peer-death" {
                argv.push("--output-proof-peer-loss-after-apply".into());
            }
            argv.extend(
                proof::peer_argv(stage, &a, &b)?
                    .into_iter()
                    .map(|a| format!("--output-process-arg={a}")),
            );
            // Keep the peer's deadline below the outer Session bound, including
            // time for startup and the five-second departure guard.
            argv.push("--output-process-arg=--deadline-ms=30000".into());
            proof::check_argv(stage, &argv, &a, &b, paths)?;
            Ok(argv)
        })
        .collect()
}

fn bind(
    repo: &Path,
    values: &BTreeMap<String, String>,
    integration: &str,
) -> Result<Bound, String> {
    for key in ["inputs", "preparation", "preflight", "out"] {
        let path = Path::new(&values[key]);
        if !path.is_absolute() {
            return Err(format!("{key} must be absolute"));
        }
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!("{key} cannot contain parent traversal"));
        }
    }
    let preflight = Path::new(&values["preflight"]);
    if !hex(&values["preflight-sha256"], 64)
        || sha256(&read(preflight)?) != values["preflight-sha256"]
        || rustix::fs::access(preflight, rustix::fs::Access::EXEC_OK).is_err()
    {
        return Err("preflight executable or digest differs".into());
    }
    proof::bind(
        repo,
        Path::new(&values["inputs"]),
        &values["inputs-manifest-sha256"],
        Path::new(&values["preparation"]),
        &values["preparation-sha256"],
        &values["profile"],
        integration,
    )
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes).map_err(|e| e.to_string())
}

fn new_directory(repo: &Path, path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.starts_with(repo) {
        return Err("output must be absolute and outside checkout".into());
    }
    let parent = path
        .parent()
        .ok_or("output has no parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent.starts_with(repo.canonicalize().map_err(|e| e.to_string())?) {
        return Err("output resolves into checkout".into());
    }
    let path = parent.join(path.file_name().ok_or("output has no name")?);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn prepare(repo: &Path, args: &[String]) -> Result<Vec<String>, String> {
    let values = options(args, KEYS)?;
    let integration = proof::integration_commit(repo)?;
    let bound = bind(repo, &values, &integration)?;
    let argv = arguments(&values, &bound.paths)?;
    let output = new_directory(repo, Path::new(&values["out"]))?;
    let plan = Plan {
        schema: 1,
        integration_commit: integration.clone(),
        options: values,
        argv,
    };
    let bytes = serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?;
    if proof::integration_commit(repo)? != integration {
        return Err("integration changed during preparation".into());
    }
    write_new(&output.join(PLAN), &bytes)?;
    Ok(vec![format!(
        "output-file-native: prepared plan={} sha256={} native_acceptance=false",
        output.join(PLAN).display(),
        sha256(&bytes)
    )])
}

fn execute(repo: &Path, args: &[String]) -> Result<Vec<String>, String> {
    let options = options(args, &["plan", "plan-sha256", "out"])?;
    let plan_path = Path::new(&options["plan"]);
    if !plan_path.is_absolute() || fs::metadata(plan_path).map_err(|e| e.to_string())?.len() > 65536
    {
        return Err("plan must be absolute and at most 64 KiB".into());
    }
    let bytes = read(plan_path)?;
    if !hex(&options["plan-sha256"], 64) || sha256(&bytes) != options["plan-sha256"] {
        return Err("run plan digest differs".into());
    }
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if plan.schema != 1 {
        return Err("unsupported run plan schema".into());
    }
    // Re-parse the fixed option set so an edited plan cannot omit a required key.
    let raw = plan
        .options
        .iter()
        .map(|(k, v)| format!("--{k}={v}"))
        .collect::<Vec<_>>();
    let values = self::options(&raw, KEYS)?;
    let integration = proof::integration_commit(repo)?;
    if integration != plan.integration_commit {
        return Err("run plan belongs to another integration commit".into());
    }
    let bound = bind(repo, &values, &integration)?;
    if arguments(&values, &bound.paths)? != plan.argv {
        return Err("run plan argv differs from sealed inputs and declared layouts".into());
    }
    process::attended(&values["tty"])?;
    let output = new_directory(repo, Path::new(&options["out"]))?;
    write_new(&output.join(PLAN), &bytes)?;
    let result = stages(repo, &plan, &bound, &output);
    let record = serde_json::json!({"schema":1,"passed":result.is_ok(),"error":result.as_ref().err(),
        "plan_sha256":options["plan-sha256"],"integration_commit":integration});
    write_new(
        &output.join("run-result.json"),
        &serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?,
    )?;
    result
}

fn stages(repo: &Path, plan: &Plan, bound: &Bound, output: &Path) -> Result<Vec<String>, String> {
    let (a, b) = layouts(&plan.options)?;
    let mut manifest = RunManifest {
        integration_commit: bound.integration_commit.clone(),
        inputs_sha256: bound.inputs_sha256.clone(),
        preparation_sha256: bound.preparation_sha256.clone(),
        profile: bound.profile.clone(),
        a,
        b,
        stages: Vec::new(),
    };
    for ((name, _), argv) in proof::STAGES.iter().zip(&plan.argv) {
        if bind(repo, &plan.options, &bound.integration_commit)? != *bound {
            return Err("bound inputs changed before stage".into());
        }
        let stage = output.join(name);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&stage)
            .map_err(|e| e.to_string())?;
        write_new(
            &stage.join("argv.json"),
            &serde_json::to_vec_pretty(argv).map_err(|e| e.to_string())?,
        )?;
        eprintln!(
            "Output proof stage {name}: {} on {}. Emergency recovery: Ctrl-Alt-Backspace. Do not switch VTs unless recovering. Logs: {}",
            plan.options["display"],
            plan.options["tty"],
            stage.display()
        );
        process::session(&plan.options, argv, &stage)?;
        let log = read(&stage.join("session.log"))?;
        let record = StageRecord {
            name: (*name).into(),
            log: format!("{name}/session.log"),
            size: log.len() as u64,
            sha256: sha256(&log),
            session_exit: 0,
            argv: argv.clone(),
        };
        // A clean Session exit does not imply the peer or physical proof passed.
        // Refuse this stage before granting the next stage any device custody.
        proof::verify_stage_record(&record, &manifest.a, &manifest.b, &bound.paths, output)?;
        manifest.stages.push(record);
        eprintln!(
            "Output proof: {name} passed ({}/4). Keep {} in the foreground until the final PASS; input verification between stages may take a moment.",
            manifest.stages.len(),
            plan.options["tty"]
        );
        if bind(repo, &plan.options, &bound.integration_commit)? != *bound
            || proof::integration_commit(repo)? != bound.integration_commit
        {
            return Err("bound inputs changed after stage".into());
        }
    }
    let text = manifest.render()?;
    write_new(&output.join(proof::RUN_MANIFEST), text.as_bytes())?;
    let digest = sha256(text.as_bytes());
    let request = proof::Request {
        inputs: plan.options["inputs"].clone().into(),
        inputs_sha256: bound.inputs_sha256.clone(),
        preparation: plan.options["preparation"].clone().into(),
        preparation_sha256: bound.preparation_sha256.clone(),
        run: output.into(),
        run_sha256: digest.clone(),
    };
    proof::verify(repo, &request, &bound.integration_commit)?;
    Ok(vec![format!(
        "output-file-native: PASS stages=4 run={} run_manifest_sha256={digest}",
        output.display()
    )])
}
