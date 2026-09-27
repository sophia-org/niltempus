// Provenance: moved from Sophia crates/xtask/src/panel.rs (the runner) and
// crates/sophia-conformance/src/panel.rs (the verifier) at
// de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13), with
// tools/fixtures/quickshell_sophia and the three controls now in
// tests/panel.rs. Changes: the isolated probe runs an explicit prepared
// Sophia binary (--sophia=/ABS or SOPHIA_BIN), never a source tree's
// target/debug; the fixtures come from this repository; the probe-mode
// session is spawned in its own process group under the bounded
// `wait_logged` custody; lines are returned rather than printed, except the
// two printed before a run starts. The verifier is unchanged apart from
// using the public `sophia_conformance::record::after_marker`, which the
// private helper it called forwarded to. Live mode is operator-only: it
// attaches to the current session's DISPLAY, and no gate here runs it (every
// gate unsets DISPLAY, which live mode refuses). See docs/quickshell.md.
//! Opt-in X11 reference client launcher. Physical session ownership stays outside xtask.
//! The verifier accepts the isolated, fixed-geometry X11 reference fixture;
//! CPU commits and work-area transitions are not physical presentation evidence.
use sophia_conformance::record::after_marker as record_after_marker;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The session's own --max-runtime-ms is 15 s; this bounds the whole run.
const PROBE_LIMIT: Duration = Duration::from_secs(60);

pub fn run(root: &Path, arguments: &[String]) -> Result<Vec<String>, String> {
    let mut renderer = "gpu";
    let mut probe = false;
    let mut sophia = std::env::var_os("SOPHIA_BIN").map(PathBuf::from);
    let mut quickshell = std::env::var_os("SOPHIA_QUICKSHELL_BIN").map(PathBuf::from);
    let mut wm = std::env::var_os("SOPHIA_HAGIA_BIN").map(PathBuf::from);
    let mut display = None;
    let mut directory = None;
    for argument in arguments {
        if argument == "--probe" {
            probe = true;
        } else if let Some(value) = argument.strip_prefix("--renderer=") {
            renderer = value;
        } else if let Some(value) = argument.strip_prefix("--quickshell=") {
            quickshell = Some(PathBuf::from(value));
        } else if let Some(value) = argument.strip_prefix("--sophia=") {
            sophia = Some(PathBuf::from(value));
        } else if let Some(value) = argument.strip_prefix("--wm=") {
            wm = Some(PathBuf::from(value));
        } else if let Some(value) = argument.strip_prefix("--display=") {
            display = Some(value.to_owned());
        } else if let Some(value) = argument.strip_prefix("--output=") {
            directory = Some(PathBuf::from(value));
        } else {
            return Err(format!("unknown panel option {argument:?}"));
        }
    }
    if !matches!(renderer, "gpu" | "software") {
        return Err("panel renderer must be gpu or software".to_owned());
    }
    if probe && renderer != "software" {
        return Err(
            "the isolated probe requires --renderer=software; use live mode for GPU".to_owned(),
        );
    }
    if !probe && (display.is_some() || wm.is_some() || sophia.is_some()) {
        return Err(
            "--display, --wm and --sophia belong to --probe; live mode inherits the current session"
                .to_owned(),
        );
    }
    if !probe && std::env::var_os("DISPLAY").is_none() {
        return Err(
            "live mode needs DISPLAY and the current session's Xauthority environment".to_owned(),
        );
    }
    let quickshell = quickshell
        .ok_or("set SOPHIA_QUICKSHELL_BIN or --quickshell=/absolute/path")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let wm = if probe {
        let wm = wm
            .ok_or("--probe needs --wm=/absolute/path/to/hagia")?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let sophia =
            sophia.ok_or("--probe needs a prepared Sophia binary: --sophia=/ABS or SOPHIA_BIN")?;
        if !sophia.is_absolute() || !sophia.is_file() {
            return Err(format!(
                "--sophia must be an absolute prepared binary: {}",
                sophia.display()
            ));
        }
        Some((wm, sophia))
    } else {
        None
    };
    let directory = directory.unwrap_or_else(|| {
        std::env::temp_dir().join(format!(
            "sophia-panel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ))
    });
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|error| error.to_string())?;
    let directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let fixture = root.join("tools/fixtures/quickshell_sophia/shell.qml");
    let fixture_copy = directory.join("shell.qml");
    private_copy(&fixture, &fixture_copy)?;
    let mut identity = format!(
        "schema=1\nmode={}\nrequested_renderer={renderer}\nquickshell={}\n",
        if probe { "isolated" } else { "live" },
        quickshell.display()
    );
    for path in [&quickshell, &fixture_copy] {
        identity.push_str(&digest(path)?);
    }
    let version = Command::new(&quickshell)
        .arg("--version")
        .output()
        .map_err(|error| error.to_string())?;
    identity.push_str(&String::from_utf8_lossy(&version.stdout));
    identity.push_str(&String::from_utf8_lossy(&version.stderr));
    let mut command = if let Some((wm, sophia)) = wm {
        identity.push_str(&digest(&sophia)?);
        identity.push_str(&digest(&wm)?);
        let core = directory.join("core.kdl");
        let desktop = directory.join("desktop.kdl");
        private_copy(
            &root.join("tools/fixtures/quickshell_sophia/core.kdl"),
            &core,
        )?;
        private_copy(
            &root.join("tools/fixtures/quickshell_sophia/desktop.kdl"),
            &desktop,
        )?;
        let mut command = Command::new(sophia);
        command
            .args([
                "session",
                "run",
                "--no-input",
                "--session-mode=normal",
                "--session-start=panel",
                "--session-app=terminal=/usr/bin/xterm",
                "--session-app=browser=/usr/bin/firefox",
                "--session-action-app=terminal=terminal",
                "--session-action-app=browser=browser",
                "--session-start=terminal",
                "--session-app-arg=terminal=-cm",
                "--session-app-arg=terminal=-dc",
                "--session-app-arg=terminal=-fn",
                "--session-app-arg=terminal=6x13",
                "--session-app-arg=terminal=-e",
                "--session-app-arg=terminal=/bin/sh",
                "--session-app-arg=terminal=-c",
                "--session-app-arg=terminal=printf 'Sophia panel witness\n'; sleep 12",
                "--max-runtime-ms=15000",
                "--wm-interface=sophia_wm_v1",
            ])
            .arg(format!("--config={}", core.display()))
            .arg(format!("--desktop-profile={}", desktop.display()))
            .arg(format!("--wm-process={}", wm.display()))
            .arg(format!("--session-app=panel={}", quickshell.display()))
            .arg("--session-app-arg=panel=--path")
            .arg(format!(
                "--session-app-arg=panel={}",
                fixture_copy.display()
            ))
            .arg(format!(
                "--display={}",
                display.unwrap_or_else(|| format!(":{}", 20000 + std::process::id() % 30000))
            ))
            .env("SOPHIA_PANEL_EXERCISE", "1");
        command
    } else {
        let mut command = Command::new(quickshell);
        command
            .arg("--path")
            .arg(&fixture_copy)
            .env_remove("SOPHIA_PANEL_EXERCISE");
        command
    };
    command
        .env_remove("WAYLAND_DISPLAY")
        .env("QT_QPA_PLATFORM", "xcb")
        .env("QSG_INFO", "1")
        .env(
            "QT_LOGGING_RULES",
            "qt.scenegraph.general=true;qt.rhi.general=true",
        );
    if renderer == "software" {
        command
            .env("QT_QUICK_BACKEND", "software")
            .env_remove("QSG_RHI_BACKEND");
    } else {
        command
            .env_remove("QT_QUICK_BACKEND")
            .env("QSG_RHI_BACKEND", "opengl")
            .env_remove("LIBGL_ALWAYS_SOFTWARE");
    }
    identity.push_str(&format!("command={command:?}\n"));
    fs::write(directory.join("identity.txt"), identity).map_err(|error| error.to_string())?;
    let log_path = directory.join("session.log");
    let log = File::create(&log_path).map_err(|error| error.to_string())?;
    println!("Panel evidence: {}", directory.display());
    println!(
        "Requested renderer: {renderer}. Inspect session.log for the actual device/backend; this is not GPU acceptance."
    );
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|error| error.to_string())?)
        .stderr(log);
    if !probe {
        let status = command.status().map_err(|error| error.to_string())?;
        fs::write(directory.join("exit.txt"), format!("{status}\n"))
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!(
                "panel run failed: {status}; see {}",
                directory.display()
            ));
        }
        return Ok(Vec::new());
    }
    let child = command
        .process_group(0)
        .spawn()
        .map_err(|error| error.to_string())?;
    let result =
        crate::bemenu_artifact::wait_logged(child, &log_path, PROBE_LIMIT, "panel session");
    fs::write(
        directory.join("exit.txt"),
        format!(
            "{}\n",
            result.as_ref().err().map_or("success", String::as_str)
        ),
    )
    .map_err(|error| error.to_string())?;
    if let Err(error) = result {
        return Err(format!(
            "panel run failed: {error}; see {}",
            directory.display()
        ));
    }
    let log = fs::read_to_string(&log_path).map_err(|error| error.to_string())?;
    let verdict = verify(&log)?;
    fs::write(directory.join("verdict.txt"), &verdict).map_err(|error| error.to_string())?;
    Ok(vec![verdict])
}

fn private_copy(source: &Path, destination: &Path) -> Result<(), String> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .map_err(|error| error.to_string())?;
    std::io::copy(
        &mut File::open(source).map_err(|error| error.to_string())?,
        &mut output,
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn digest(path: &Path) -> Result<String, String> {
    let result = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;
    if !result.status.success() {
        return Err(format!("cannot fingerprint {}", path.display()));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

pub fn verify(log: &str) -> Result<String, String> {
    if log.len() > 4 * 1024 * 1024 {
        return Err("panel evidence exceeds 4 MiB".to_owned());
    }
    let mut panel_generations = BTreeSet::new();
    let mut popup_generations = BTreeSet::new();
    let mut panel_surface = None;
    let mut popup_surface = None;
    let mut last_seen = BTreeMap::new();
    let mut last_popup_sample = 0;
    let mut last_panel_sample = 0;
    let mut reservation_stage = 0;
    let mut clean = false;
    let mut protocol_clean = false;
    let mut software = false;
    let mut isolated = false;
    let mut terminal = false;
    for line in log.lines() {
        if line.starts_with("Error:")
            || line.contains("status=rejected reason=")
            || line.contains("status=restart_requested")
            || line.contains("status=detected source=owner_loop")
        {
            return Err("panel evidence contains a session or policy failure".to_owned());
        }
        software |= line.contains("Loading backend software");
        let fields = line
            .split_whitespace()
            .filter_map(|field| field.split_once('='))
            .collect::<BTreeMap<_, _>>();
        if record_after_marker(line, "sophia_live_session schema=").is_some() {
            isolated |= fields.get("native_presentation") == Some(&"disabled")
                && fields.get("physical_input") == Some(&"disabled")
                && fields.get("wm_policy") == Some(&"external");
        }
        if record_after_marker(line, "sophia_live_cpu_surface ").is_some() {
            if fields.get("schema") != Some(&"1") || fields.get("truncated") != Some(&"false") {
                return Err("unsupported or truncated CPU surface evidence".to_owned());
            }
            let value = |name| fields.get(name).ok_or_else(|| format!("missing {name}"));
            let number = |name| -> Result<u64, String> {
                value(name)?.parse().map_err(|_| format!("invalid {name}"))
            };
            let seq = number("seq")?;
            let width = number("width")?;
            let height = number("height")?;
            let x = number("x")?;
            let y = number("y")?;
            let generation = number("buffer_generation")?;
            let surface = *value("surface")?;
            last_seen
                .entry(surface)
                .and_modify(|last: &mut u64| *last = (*last).max(seq))
                .or_insert(seq);
            if fields.get("visual_detail") == Some(&"true") {
                if (x, y, width, height) == (0, 0, 1280, 32) {
                    if panel_surface.is_some_and(|previous| previous != surface) {
                        return Err("panel identity changed during exercise".to_owned());
                    }
                    panel_surface = Some(surface);
                    panel_generations.insert(generation);
                    last_panel_sample = last_panel_sample.max(seq);
                } else if (x, y, width, height) == (1032, 32, 240, 112) {
                    if popup_surface.is_some_and(|previous| previous != surface) {
                        return Err("popup identity changed during exercise".to_owned());
                    }
                    popup_surface = Some(surface);
                    popup_generations.insert(generation);
                    last_popup_sample = last_popup_sample.max(seq);
                } else if y >= 32 && width > 240 && height > 112 {
                    terminal = true;
                }
            }
        }
        if record_after_marker(line, "sophia_live_work_area ").is_some()
            && fields.get("output") == Some(&"1")
        {
            if fields.get("schema") != Some(&"1") || fields.get("shell_reservations") != Some(&"0")
            {
                return Err("unexpected work-area evidence".to_owned());
            }
            let expected = if reservation_stage % 2 == 0 {
                ("32", "688", "1")
            } else {
                ("0", "720", "0")
            };
            if reservation_stage < 4
                && fields.get("y") == Some(&expected.0)
                && fields.get("height") == Some(&expected.1)
                && fields.get("app_reservations") == Some(&expected.2)
                && fields.get("x") == Some(&"0")
                && fields.get("width") == Some(&"1280")
            {
                reservation_stage += 1;
            }
        }
        if record_after_marker(line, "sophia_live_session_cleanup ").is_some() {
            clean |= fields.get("status") == Some(&"clean")
                && fields.get("namespace") == Some(&"revoked")
                && fields.get("app_groups") == Some(&"0")
                && fields.get("frontend_workers") == Some(&"0");
        }
        if record_after_marker(line, "sophia_live_session_protocol_error_tally ").is_some() {
            if fields.get("status") != Some(&"clean") || fields.get("count") != Some(&"0") {
                return Err("panel session reported X11 protocol errors".to_owned());
            }
            protocol_clean = true;
        }
    }
    if let Some(surface) = popup_surface {
        last_popup_sample = last_seen.get(surface).copied().unwrap_or(last_popup_sample);
    }
    if !isolated
        || !software
        || !clean
        || !protocol_clean
        || !terminal
        || panel_generations.len() < 3
        || popup_generations.len() < 2
        || last_panel_sample <= last_popup_sample
        || reservation_stage != 4
    {
        return Err(format!(
            "incomplete panel evidence: isolated={isolated} software={software} cleanup={clean} protocol={protocol_clean} terminal={terminal} panel_updates={} popup_updates={} reservation_steps={reservation_stage}",
            panel_generations.len(),
            popup_generations.len()
        ));
    }
    Ok("sophia_panel_probe schema=1 status=accepted content=cpu_committed reservation_cycle=complete popup=updated_then_withdrawn physical_input=unproven gpu_presentation=unproven".to_owned())
}
