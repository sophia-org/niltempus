//! DIAGNOSTIC ONLY (G2 run6b): why real Bemenu, launched from Session's own
//! component launch plan over IPC, never connects in
//! joined_bemenu_evidence_requires_exact_current_negotiation. Production code
//! is unchanged. The ProcessLaunchSpec comes from Session's public
//! ShellComponentLaunch::prepare, printed verbatim (program, argv, env, domain);
//! the diagnostic launch differs only by an outer /usr/bin/sh that records the
//! child's stdout, stderr and exit status into one extra read-write directory,
//! because the production domain sends the child's stdio to /dev/null. A real
//! ShellComponentTransport listens on the same socket over IPC. It asserts
//! nothing about Bemenu; it prints evidence. Gated: SOPHIA_DIAG_SESSION_LAUNCH=1
//! plus the usual bound artifact variables.
use sophia_config::{ShellComponentConfig, ShellComponentRole, ShellGpuMode};
use sophia_protocol::ContentGrant;
use sophia_runtime::*;
use sophia_session::shell_component_connections::ComponentConnectionKey;
use sophia_session::shell_component_launch::ShellComponentLaunch;
use std::path::Path;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "support/bemenu_files/artifact.rs"]
mod artifact;

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
    let mut transport = ShellComponentTransport::bind_for_supervised_uid(
        root.join("menu"),
        rustix::process::geteuid().as_raw(),
    )
    .unwrap();
    let socket = transport.socket_path().to_path_buf();
    let grant = ContentGrant {
        connection_epoch: 1,
        content_grant_epoch: 1,
    };
    let key = ComponentConnectionKey { slot: 0, grant };
    let (spec, gpu) = plan.prepare(key, &socket).unwrap();
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
    let mut supervisor = ProcessSupervisor::new(SupervisedProcessKind::Shell, wrapped);
    let started = supervisor.apply(SupervisorCommand::StartProcess {
        process: SupervisedProcessKind::Shell,
        delay: Duration::ZERO,
    });
    println!("diag launch start={started:?}");
    let mut registry = ContentEpochRegistry::new(64 * 1024 * 1024).unwrap();
    if let Some(evidence) = supervisor.protection_evidence().cloned() {
        println!(
            "diag launch evidence backend={:?} roles={:?}",
            evidence.backend, evidence.roles
        );
        println!(
            "diag authorize={:?}",
            transport.authorize_protected_peer(&evidence)
        );
        println!(
            "diag reserve={:?}",
            transport.reserve_content_with_profile(
                &mut registry,
                ContentLimits::prototype(grant),
                ContentStoreProfile::NativeLauncher,
            )
        );
        println!(
            "diag begin_negotiation={:?}",
            transport.begin_negotiation(
                &registry,
                grant.connection_epoch,
                Duration::from_secs(10),
                ShellContentAdmissionPolicy::Granted {
                    discrete_input: true
                },
            )
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut outcome = "timeout";
        while Instant::now() < deadline {
            match transport.poll_negotiation(&mut registry, 65536) {
                Ok(Some(_)) => {
                    outcome = "negotiated";
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    println!("diag negotiation error={error:?}");
                    outcome = "error";
                    break;
                }
            }
            if let Ok(Some(event)) = supervisor.poll() {
                println!("diag process event={event:?}");
                outcome = "exited";
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        println!("diag negotiation outcome={outcome}");
    }
    let _ = supervisor.terminate();
    std::thread::sleep(Duration::from_millis(200));
    for name in ["status", "stdout", "stderr"] {
        let text = std::fs::read_to_string(capture.join(name)).unwrap_or_else(|e| format!("<{e}>"));
        println!("diag child {name}:\n{text}");
    }
    let _ = transport.disconnect(&mut registry);
    let _ = std::fs::remove_dir_all(&root);
}
