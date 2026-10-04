//! Development commands for the desktop integration repository.
use std::path::{Path, PathBuf};

const USAGE: &str = "usage:
  xtask prepare-bemenu-artifact SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
  xtask prepare-product-artifact lom|provlita|hagia SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR --build-dir=/ABS [--nim-deps=/ABS --nim-deps-sha256=SHA]
  xtask prepare-wm-pair --hagia REPO COMMIT --narthex REPO COMMIT NEW-OUTPUT-DIR --build-dir=/ABS --hagia-nim-deps=/ABS --hagia-nim-deps-sha256=SHA --narthex-nim-deps=/ABS --narthex-nim-deps-sha256=SHA --hagia-c-sdk-rev=REV
  xtask prepare-physical-inputs --sophia-root=/ABS --build-dir=/ABS --out=/ABS/NEW --sophia-features=F [--sophia-packages=P] [--hagia=... --narthex=...] [--profile=OWNER:PATH ...]
  xtask prepare-physical-inputs verify --out=/ABS --manifest-sha256=SHA
  xtask nim-deps draft --store=/ABS --source=/ABS --commit=SHA --product=hagia|narthex --nim=/ABS --nim-lib=/ABS --gcc=/ABS --bwrap=/ABS --build-dir=/ABS --pin=NAME=VERSION ... --out=/ABS/NEW
  xtask package-desktop --sophia-root=/ABS --sophia-rev=SHA --wm-pair=/ABS --wm-pair-commits=H,N --wm-pair-sha256=H,N --wm-pair-profile-sha256=SHA --wm-pair-c-sdk-rev=REV --build-dir=/ABS --out=/ABS/NEW
  xtask direct-scanout-gate [WIDTH HEIGHT HOLD WORKLOAD] [--overlay-proof] [--cost] [--cursor] [--atomic-cursor]
  xtask verify-archives [--legacy]
  xtask output-file-native verify --inputs=/ABS --inputs-manifest-sha256=SHA --preparation=/ABS --preparation-sha256=SHA --run=/ABS --run-manifest-sha256=SHA
  xtask output-file-native prepare-run ...   (explicit inputs and layouts: docs/output-file-native.md)
  xtask output-file-native run --plan=/ABS/run-plan.json --plan-sha256=SHA --out=/ABS/NEW
  xtask verify-release /ABS/RELEASE-DIR --c-sdk-rev=<40 lowercase hex>   (read-only; runs without the checkout)
  xtask verify-c-sdk /ABS/SNAPSHOT --revision=<40 lowercase hex>   (read-only; runs without the checkout)
  xtask desktop-comparison install-reference|prepare|prepare-soak|cursor-theme|gate|status|attest|preflight|qualify|capture|finalize|replay|workload|verify|report ...
  xtask session-recipe prepare-arguments|prepare-inputs|stage-proofs|prepare-environment --name=value ... -- [session arguments]
  xtask check-pins
  xtask check-provision
  xtask dock profile LOM LOM_CONFIG BEMENU PROVLITA DOCK_CONFIG
  xtask dock verify HOST_LOG
  xtask panel --probe --renderer=software --quickshell=/ABS --wm=/ABS --sophia=/ABS [--display=:N] [--output=/ABS/NEW]
  xtask panel [--renderer=gpu|software] [--quickshell=/ABS] [--output=/ABS/NEW]   (operator-only: attaches to the current session)
  xtask panel verify SESSION_LOG
  xtask audit-pins ABSOLUTE-SOPHIA-REPO";

fn main() -> std::process::ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match run(&arguments) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[String]) -> Result<Vec<String>, String> {
    // Commands an installed release runs come first and never look up this
    // repository: the packaged binary must work with its build checkout gone
    // (tests/installed_xtask.rs runs it with the checkout hidden).
    if arguments.first().map(String::as_str) == Some("session-recipe") {
        xtask::session::run(&arguments[1..]).map_err(|e| e.to_string())?;
        return Ok(Vec::new());
    }
    // Read-only release verification for the installer: no repository.
    if arguments.first().map(String::as_str) == Some("verify-release") {
        return xtask::release_verify::run(&arguments[1..]);
    }
    // A Nix build's release step: every input is a store path, and the
    // build checkout this binary came from does not exist.
    if arguments.first().map(String::as_str) == Some("assemble-nix") {
        return xtask::nix_assembly::run(&arguments[1..]);
    }
    // Product builders take explicit signed sources and private build inputs.
    // The installed component updater must not need this build checkout.
    if arguments.first().map(String::as_str) == Some("prepare-product-artifact") {
        return xtask::product_artifact::run(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("verify-c-sdk") {
        let [_, path, revision] = arguments else {
            return Err("usage: xtask verify-c-sdk /ABS/SNAPSHOT --revision=40hex".into());
        };
        let revision = revision
            .strip_prefix("--revision=")
            .ok_or("missing SDK revision")?;
        if !Path::new(path).is_absolute() {
            return Err("SDK snapshot must be absolute".into());
        }
        let snapshot = xtask::c_sdk_pin::verify_vendored(Path::new(path), revision)?;
        return Ok(vec![format!(
            "c_sdk_verification schema=1 status=pass revision={} manifest_sha256={}",
            snapshot.revision, snapshot.manifest_sha256
        )]);
    }
    let repo = workspace_root()?;
    match arguments.first().map(String::as_str) {
        Some("prepare-bemenu-artifact") => xtask::bemenu_artifact::run(&repo, &arguments[1..]),
        Some("prepare-wm-pair") => xtask::wm_pair::run(&arguments[1..]),
        Some("prepare-physical-inputs") => xtask::physical_inputs::run(&repo, &arguments[1..]),
        Some("nim-deps") => xtask::nim_deps::run(&arguments[1..]),
        Some("package-desktop") => xtask::package_desktop::run(&repo, &arguments[1..]),
        Some("direct-scanout-gate") => gate_direct_scanout(&repo, &arguments[1..]),
        Some("verify-archives") => xtask::verify_archives::run(&repo, &arguments[1..]),
        Some("output-file-native")
            if matches!(
                arguments.get(1).map(String::as_str),
                Some("prepare-run" | "run")
            ) =>
        {
            xtask::output_file_native_run::run(&repo, &arguments[1..])
        }
        Some("output-file-native") => xtask::output_file_native::run(&repo, &arguments[1..]),
        Some("desktop-comparison") => run_desktop_comparison(&repo, &arguments[1..]),
        // Moved from Sophia crates/xtask/src/main.rs:53-70 at 9fcaec782.
        Some("dock") => match &arguments[1..] {
            [command, paths @ ..] if command == "profile" => Ok(xtask::dock::profile(paths)?
                .lines()
                .map(str::to_owned)
                .collect()),
            [command, path] if command == "verify" => {
                use std::io::Read;
                let mut text = String::new();
                std::fs::File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(64 * 1024 * 1024 + 1)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
                Ok(vec![xtask::dock::verify(&text)?])
            }
            _ => Err(
                "usage: xtask dock profile LOM CONFIG BEMENU PROVLITA CONFIG | dock verify LOG"
                    .into(),
            ),
        },
        // Moved from Sophia crates/xtask/src/main.rs (`panel` and
        // `conformance verify panel`) at the pin.
        Some("panel") => match &arguments[1..] {
            [command, path] if command == "verify" => {
                use std::io::Read;
                let mut text = String::new();
                std::fs::File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(4 * 1024 * 1024 + 1)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
                Ok(vec![xtask::panel::verify(&text)?])
            }
            rest => xtask::panel::run(&repo, rest),
        },
        Some("check-pins") if arguments.len() == 1 => xtask::pins::check(&repo),
        Some("check-provision") if arguments.len() == 1 => xtask::pins::check_provision(&repo),
        Some("audit-pins") => match &arguments[1..] {
            [sophia] => xtask::pins::audit(&repo, Path::new(sophia)),
            _ => Err(USAGE.into()),
        },
        _ => Err(USAGE.into()),
    }
}

// Moved from Sophia crates/xtask/src/main.rs (run_desktop_comparison and
// gate_desktop_comparison) at de776c68afdf9a133818f86917893c3362dc9fb7 (the
// pin) (Sophia rule 13). Changes: nothing is built here. The Sophia candidate
// and the Hagia/Narthex pair are explicit prepared binaries (the
// prepare-physical-inputs helper, PENDING), never a source tree's target;
// Sophia's candidate commit comes from the explicit SOPHIA_SOURCE and this
// repository is bound too.
fn run_desktop_comparison(repo: &Path, arguments: &[String]) -> Result<Vec<String>, String> {
    use desktop_comparison as dc;
    let pid = |value: &str| {
        value
            .parse::<u32>()
            .map_err(|_| "session supervisor PID is not an integer".to_owned())
    };
    match arguments {
        [command, source, prefix] if command == "install-reference" => {
            dc::install_reference(repo, Path::new(source), Path::new(prefix))
        }
        [command, run] if command == "prepare" => dc::prepare(repo, Path::new(run)),
        [command, run] if command == "prepare-soak" => {
            dc::prepare_optional_soak(repo, Path::new(run))
        }
        [command, root] if command == "cursor-theme" => {
            dc::write_x11_core_cursor_theme(Path::new(root))
        }
        [command, run] if command == "gate" => gate_desktop_comparison(repo, Path::new(run)),
        [command, run] if command == "status" => dc::status(repo, Path::new(run)),
        [command, run] if command == "preflight" => dc::preflight(repo, Path::new(run)),
        [command, run] if command == "capture" => dc::capture_next(repo, Path::new(run)),
        [command, run] if command == "qualify" => dc::qualify(repo, Path::new(run)),
        [command, run] if command == "finalize" => dc::finalize_next(repo, Path::new(run)),
        [command, run, supervisor] if command == "attest" => {
            dc::attest_session_auto(repo, Path::new(run), pid(supervisor)?)
        }
        [command, run, supervisor, crtc] if command == "attest" => {
            let crtc = crtc
                .parse::<u64>()
                .map_err(|_| "session CRTC is not an integer".to_owned())?;
            dc::attest_session(repo, Path::new(run), pid(supervisor)?, crtc)
        }
        [command, run, attempt] if command == "replay" => {
            dc::replay_attempt(Path::new(run), Path::new(attempt))
                .map(|replay| vec![replay.sample_record])
        }
        [command, kind, seconds] if command == "workload" && kind == "kitty-stream" => {
            let seconds = seconds
                .parse::<u64>()
                .map_err(|_| "kitty-stream duration is not an integer".to_owned())?;
            dc::run_stream(seconds).map(|()| Vec::new())
        }
        [command, run] if command == "verify" => dc::verify(repo, Path::new(run)),
        [command, run] if command == "report" => dc::report(repo, Path::new(run)),
        [command, ..] => Err(format!(
            "desktop-comparison {command:?} has invalid arguments; expected install-reference, prepare, prepare-soak, cursor-theme, gate, status, attest, preflight, qualify, capture, finalize, replay, verify, or report"
        )),
        [] => Err("desktop-comparison needs install-reference, prepare, prepare-soak, cursor-theme, gate, status, attest, preflight, qualify, capture, finalize, replay, verify, or report".to_owned()),
    }
}

fn gate_desktop_comparison(repo: &Path, run: &Path) -> Result<Vec<String>, String> {
    use desktop_comparison as dc;
    dc::status(repo, run)?;
    dc::require_candidate_checkout(repo, run)?;
    dc::verify_host_tool_versions()?;
    dc::verify_prepared_binaries(repo, run)?;
    let adapter = repo.join("tools/desktop_comparison_tty3.sh");
    if !adapter.is_file() {
        return Err(format!(
            "desktop-comparison TTY adapter is missing: {}",
            adapter.display()
        ));
    }
    let xtask = std::env::current_exe()
        .map_err(|error| format!("could not identify the running xtask: {error}"))?;
    let status = std::process::Command::new(&adapter)
        .arg(run)
        .env("SOPHIA_DESKTOP_COMPARISON_XTASK", xtask)
        .status()
        .map_err(|error| format!("could not start {}: {error}", adapter.display()))?;
    if status.success() {
        Ok(Vec::new())
    } else {
        Err(format!(
            "desktop-comparison one-row gate exited with {status}"
        ))
    }
}

// Moved from Sophia crates/xtask/src/main.rs (gate_direct_scanout) at
// de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13), with
// explicit sources (xtask::direct_scanout_gate::Sources).
fn gate_direct_scanout(repo: &Path, arguments: &[String]) -> Result<Vec<String>, String> {
    use xtask::direct_scanout_gate::{Probe, Sources, run_gate_with};
    // Parsed by `Probe`, which owns the argument vocabulary: the gate and the
    // probe run the same session, and two spellings of the same options would
    // let them drift.
    let probe = Probe::from_arguments(arguments)?;
    let sources = Sources::from_environment(repo)?;
    println!("Building and running the exact physical-proof binary...");
    if probe.overlay_proof {
        println!("Overlay proof: the session will open an overlay over a direct frame.");
    }
    if probe.cost {
        println!("Cost run: the overlay holds long enough to measure composed frames.");
    }
    if probe.cursor {
        println!("Cursor proof: the session moves a cursor over directly scanned frames.");
    }
    if probe.atomic_cursor {
        println!("Atomic cursor: the cursor rides a plane rather than the legacy ioctl.");
    }
    let report = run_gate_with(&sources, &probe)?;
    Ok(vec![
        format!("Sophia commit:  {}", report.source_commit),
        format!("Sophia binary:  {}", report.sophia_sha256),
        format!(
            "Client:         {} ({})",
            report.client.display(),
            report.client_sha256
        ),
        format!("Direct scanout gate passed: {}", report.archive.display()),
    ])
}

fn workspace_root() -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::canonicalize(&root).map_err(|e| format!("{}: {e}", root.display()))
}
