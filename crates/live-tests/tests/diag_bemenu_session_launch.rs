//! DIAGNOSTIC ONLY (G2 run6b): why real Bemenu, launched from Session's own
//! component launch plan over IPC, does not complete negotiation in
//! joined_bemenu_evidence_requires_exact_current_negotiation. Production code
//! is unchanged. The ProcessLaunchSpec comes from Session's public
//! ShellComponentLaunch::prepare and is printed verbatim (program, argv, env,
//! protection domain). The diagnostic launch differs only by an outer
//! /usr/bin/sh that records the child's stdout, stderr and exit status into
//! one extra read-write directory, because the production domain sends the
//! child's stdio to /dev/null. Instead of Session's transport, a plain
//! listener on the same socket path timestamps each phase so they can be told
//! apart: (a) no accept, (b) accept but no hello bytes, (c) partial or
//! invalid hello, (d) early exit. Session's own deadline is 5 s
//! (shell_component_processes.rs:116, one deadline for accept and hello); the
//! diagnostic keeps observing to 20 s so a late hello is still seen. It also
//! samples the Bemenu process (CPU time, open font files) as progress
//! evidence. It asserts nothing about Bemenu. Gated:
//! SOPHIA_DIAG_SESSION_LAUNCH=1 plus the usual bound artifact variables.
use sophia_config::{ShellComponentConfig, ShellComponentRole, ShellGpuMode};
use sophia_protocol::{ContentGrant, decode_shell_v1_client_hello_frame};
use sophia_runtime::*;
use sophia_session::shell_component_connections::ComponentConnectionKey;
use sophia_session::shell_component_launch::ShellComponentLaunch;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "support/bemenu_files/artifact.rs"]
mod artifact;

const SESSION_DEADLINE: Duration = Duration::from_secs(5);
const OBSERVE: Duration = Duration::from_secs(20);

/// A process whose executable is `binary`, with its CPU ticks and the number
/// of open files under a fonts directory.
fn sample(binary: &Path) -> Option<(u32, u64, usize)> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if std::fs::read_link(entry.path().join("exe")).ok().as_deref() != Some(binary) {
            continue;
        }
        let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
        let fields = stat.rsplit_once(") ")?.1.split(' ').collect::<Vec<_>>();
        let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
        let fonts = std::fs::read_dir(entry.path().join("fd"))
            .map(|fds| {
                fds.flatten()
                    .filter(|fd| {
                        std::fs::read_link(fd.path())
                            .is_ok_and(|target| target.to_string_lossy().contains("/fonts"))
                    })
                    .count()
            })
            .unwrap_or(0);
        return Some((pid, ticks, fonts));
    }
    None
}

#[test]
#[ignore = "diagnostic: needs SOPHIA_DIAG_SESSION_LAUNCH=1 and the bound Bemenu artifact"]
fn diag_session_launch_plan_for_real_bemenu_over_ipc() {
    assert_eq!(
        std::env::var("SOPHIA_DIAG_SESSION_LAUNCH").as_deref(),
        Ok("1"),
        "diagnostic is opt-in"
    );
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = std::env::temp_dir().join(format!("diag-session-launch-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let loaded = artifact::load(&repo, &root.join("bin"));
    let plan = ShellComponentLaunch::new(
        ShellComponentConfig {
            id: "menu".into(),
            role: ShellComponentRole::ApplicationLauncher,
            executable: loaded.binary.clone(),
            config: None,
            reservation: None,
            gpu: ShellGpuMode::Denied,
            transport: Default::default(),
        },
        None,
        None,
    )
    .unwrap();
    let socket_dir = root.join("menu");
    std::fs::create_dir(&socket_dir).unwrap();
    let socket: PathBuf = socket_dir.join("shell.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let grant = ContentGrant {
        connection_epoch: 1,
        content_grant_epoch: 1,
    };
    let key = ComponentConnectionKey { slot: 0, grant };
    let (spec, gpu) = plan.prepare(key, &socket).unwrap();
    let spec_program = PathBuf::from(&spec.program);
    println!("diag plan program={:?}", spec.program);
    println!("diag plan args={:?}", spec.args);
    for (name, value) in &spec.environment {
        println!("diag plan env {}={:?}", name.to_string_lossy(), value);
    }
    println!("diag plan process_group={}", spec.process_group);
    println!("diag plan gpu_evidence={}", gpu.is_some());
    println!("diag plan domain={:?}", spec.protection_domain);

    // Diagnostic launch: the same spec and domain, wrapped to keep stdio.
    let capture = root.join("capture");
    std::fs::create_dir(&capture).unwrap();
    let domain = spec
        .protection_domain
        .clone()
        .expect("production plan is protected")
        .path(ProtectionPath::read_write(&capture))
        .unwrap();
    let mut wrapped = ProcessLaunchSpec::new("/usr/bin/sh")
        .arg("-c")
        .arg("\"$@\" >\"$0/stdout\" 2>\"$0/stderr\"; echo $? >\"$0/status\"")
        .arg(&capture)
        .arg(&spec.program);
    for arg in &spec.args {
        wrapped = wrapped.arg(arg);
    }
    for (name, value) in &spec.environment {
        wrapped = wrapped.env(name, value);
    }
    if spec.process_group {
        wrapped = wrapped.process_group();
    }
    let wrapped = wrapped.protection_domain(domain);
    let t0 = Instant::now();
    let mut supervisor = ProcessSupervisor::new(SupervisedProcessKind::Shell, wrapped);
    let started = supervisor.apply(SupervisorCommand::StartProcess {
        process: SupervisedProcessKind::Shell,
        delay: Duration::ZERO,
    });
    println!("diag t={:?} launch start={started:?}", t0.elapsed());
    if let Some(evidence) = supervisor.protection_evidence() {
        println!(
            "diag launch evidence backend={:?} roles={:?} peer_pid={}",
            evidence.backend, evidence.roles, evidence.peer_pid
        );
    }
    println!("diag session deadline at t={SESSION_DEADLINE:?} (accept and hello together)");
    let mut stream = None;
    let mut received = Vec::new();
    let mut first_bytes = None;
    let mut exited = None;
    let mut last_sample = Instant::now() - Duration::from_secs(1);
    while t0.elapsed() < OBSERVE {
        if stream.is_none() {
            match listener.accept() {
                Ok((accepted, _)) => {
                    println!("diag t={:?} accepted", t0.elapsed());
                    accepted.set_nonblocking(true).unwrap();
                    stream = Some(accepted);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => println!("diag t={:?} accept error={error}", t0.elapsed()),
            }
        }
        if let Some(connection) = stream.as_mut() {
            let mut buffer = [0u8; 4096];
            match connection.read(&mut buffer) {
                Ok(0) => {
                    println!(
                        "diag t={:?} peer closed after {} bytes",
                        t0.elapsed(),
                        received.len()
                    );
                    stream = None;
                }
                Ok(n) => {
                    if first_bytes.is_none() {
                        first_bytes = Some(t0.elapsed());
                        println!("diag t={:?} first bytes n={n}", t0.elapsed());
                    }
                    received.extend_from_slice(&buffer[..n]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => println!("diag t={:?} read error={error}", t0.elapsed()),
            }
        }
        if last_sample.elapsed() >= Duration::from_millis(250) {
            last_sample = Instant::now();
            match sample(&spec_program) {
                Some((pid, ticks, fonts)) => println!(
                    "diag t={:?} bemenu pid={pid} cpu_ticks={ticks} open_font_files={fonts}",
                    t0.elapsed()
                ),
                None => println!("diag t={:?} bemenu process not found", t0.elapsed()),
            }
        }
        if exited.is_none()
            && let Ok(Some(event)) = supervisor.poll()
        {
            println!("diag t={:?} process event={event:?}", t0.elapsed());
            exited = Some(t0.elapsed());
        }
        if exited.is_some() && stream.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    println!(
        "diag received bytes={} first_bytes_at={first_bytes:?}",
        received.len()
    );
    match decode_shell_v1_client_hello_frame(&received) {
        Ok(hello) => println!("diag hello decoded={hello:?}"),
        Err(error) => println!(
            "diag hello not decoded={error:?} bytes={:02x?}",
            &received[..received.len().min(64)]
        ),
    }
    let phase = match (first_bytes, exited) {
        (Some(at), _) if at <= SESSION_DEADLINE => "hello bytes before the session deadline",
        (Some(_), _) => "hello bytes only after the session deadline",
        (None, Some(_)) => "exited without sending bytes",
        (None, None) => "no bytes within the observation window",
    };
    println!("diag phase={phase}");
    let _ = supervisor.terminate();
    std::thread::sleep(Duration::from_millis(200));
    for name in ["status", "stdout", "stderr"] {
        let text = std::fs::read_to_string(capture.join(name)).unwrap_or_else(|e| format!("<{e}>"));
        println!("diag child {name}:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
