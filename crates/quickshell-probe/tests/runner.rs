//! The runner against stub clients: no X client is needed to prove that a
//! client which never connects is refused, that the deadline and the process
//! group bound it, and that the socket and evidence are handled.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use quickshell_probe::Renderer;
use quickshell_probe::runner::{Probe, run};

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "quickshell-probe-test-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    dir
}

fn stub(dir: &Path, body: &str) -> (PathBuf, PathBuf) {
    let client = dir.join("client");
    std::fs::write(&client, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = dir.join("shell.qml");
    std::fs::write(&config, "// stub\n").unwrap();
    (client, config)
}

fn probe(dir: &Path, client: PathBuf, config: PathBuf, deadline: Duration) -> Probe {
    Probe {
        renderer: Renderer::Software,
        quickshell: client,
        config,
        render_node: None,
        out: dir.join("evidence"),
        deadline,
    }
}

#[test]
fn a_client_that_never_connects_is_refused_at_the_deadline() {
    let dir = scratch("never");
    // The stub records its display and arguments, then waits past the
    // deadline without opening the display.
    let (client, config) = stub(
        &dir,
        "printf '%s %s %s\\n' \"$DISPLAY\" \"$1\" \"$QT_QUICK_BACKEND\"; exec sleep 30",
    );
    let started = Instant::now();
    let refusal = run(&probe(&dir, client, config.clone(), Duration::from_secs(1))).unwrap_err();
    // Deadline, TERM grace and teardown, not the stub's 30 seconds.
    assert!(started.elapsed() < Duration::from_secs(15), "{refusal}");
    assert!(refusal.contains("status=refused"), "{refusal}");
    assert!(refusal.contains("client_never_connected"), "{refusal}");
    assert!(refusal.contains("end=stopped_at_deadline"), "{refusal}");
    let evidence = dir.join("evidence");
    let stdout = std::fs::read_to_string(evidence.join("client.stdout")).unwrap();
    let display = stdout.split_whitespace().next().unwrap().to_owned();
    assert!(display.starts_with(':'), "{stdout}");
    assert!(stdout.contains(" --path software"), "{stdout}");
    // The socket is gone once the probe returns.
    assert!(!Path::new(&format!("/tmp/.X11-unix/X{}", &display[1..])).exists());
    let verdict = std::fs::read_to_string(evidence.join("verdict.txt")).unwrap();
    assert!(verdict.contains("client_never_connected"), "{verdict}");
    let identity = std::fs::read_to_string(evidence.join("identity.txt")).unwrap();
    assert!(identity.contains("renderer=software"), "{identity}");
    assert!(identity.contains(&format!("config={}", config.display())));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_client_that_exits_without_connecting_is_refused() {
    let dir = scratch("exits");
    let (client, config) = stub(&dir, "exit 0");
    let refusal = run(&probe(&dir, client, config, Duration::from_secs(10))).unwrap_err();
    assert!(refusal.contains("client_never_connected"), "{refusal}");
    assert!(refusal.contains("end=exited:0"), "{refusal}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inputs_are_refused_before_anything_runs() {
    let dir = scratch("inputs");
    let (client, config) = stub(&dir, "exit 0");
    let mut gpu = probe(&dir, client.clone(), config.clone(), Duration::from_secs(1));
    gpu.renderer = Renderer::Gpu;
    assert!(run(&gpu).unwrap_err().contains("explicit --render-node"));
    gpu.render_node = Some(PathBuf::from("/dev/dri/card0"));
    assert!(run(&gpu).unwrap_err().contains("renderDN"));
    let mut software = probe(&dir, client.clone(), config.clone(), Duration::from_secs(1));
    software.render_node = Some(PathBuf::from("/dev/dri/renderD128"));
    assert!(run(&software).unwrap_err().contains("no render node"));
    let relative = probe(
        &dir,
        PathBuf::from("client"),
        config,
        Duration::from_secs(1),
    );
    assert!(run(&relative).unwrap_err().contains("absolute"));
    // Nothing was created for a refused input.
    assert!(!dir.join("evidence").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
