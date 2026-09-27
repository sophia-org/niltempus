// Provenance: the three real-Bemenu tests of Sophia
// crates/sophia-session/tests/shell_component_processes.rs (lines 272-419 and
// 807-886) at d20faf3709ae21d94491f7a628ac9a4a86619cdf, unchanged since
// 9fcaec782 (Sophia rule 13). Their assertions are verbatim; only the
// executable now comes from a bound, prepared artifact instead of
// SOPHIA_TEST_BEMENU, and the feature gate is the dependency's feature.
//! LIVE smoke (G2 U1-U3): the real Bemenu through Sophia's Session-layer
//! owners over the current IPC socket wire (the default component transport):
//! the production ShellComponentLaunch plan and protection, the aggregate
//! ShellComponentProcesses owner beside a protected fixture bar in the same
//! registry, and ShellComponentSession launch evidence with
//! LiveProductionVisualRuntime settlement. Opt-in and fail closed, like
//! bemenu_files:
//!
//!   SOPHIA_BEMENU_ARTIFACT=<output-dir> SOPHIA_BEMENU_SHA256=<binary sha256> \
//!   SOPHIA_BEMENU_COMMIT=<signed commit> nice -n 19 \
//!   cargo test -p live-tests --test bemenu_session_ipc -- --ignored --nocapture \
//!       --test-threads=1
//!
//! (One thread: the two protected_bemenu tests share their verbatim scratch
//! directory name, as in Sophia.)
//!
//! `protected_component_peer` is the fixture bar's child entry, run only by
//! the parent through the protected launcher.
use sophia_config::ShellComponentRole;
use sophia_runtime::{
    ProcessLaunchSpec, ProtectionDomainRole, ProtectionDomainSpec, ShellContentAdmissionPolicy,
};
use sophia_session::shell_component_connections::ComponentConnectionPhase;
use sophia_session::shell_component_processes::*;
use std::path::{Path, PathBuf};

#[allow(dead_code)] // Only the verified binary path is used here.
#[path = "support/bemenu_files/artifact.rs"]
mod artifact;
#[path = "support/component_processes/bemenu.rs"]
mod bemenu;
#[path = "support/component_processes/peer.rs"]
mod peer;

/// The verified private copy of the prepared Bemenu artifact (identity
/// before anything executes), in a private directory per test.
fn bemenu_binary(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let private = std::env::temp_dir().join(format!(
        "sophia-bemenu-session-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let loaded = artifact::load(&repo, &private);
    loaded.binary
}

#[test]
#[ignore = "child entry invoked only by protected parent fixture"]
fn protected_component_peer() {
    peer::run();
}

#[test]
#[ignore = "requires explicit Bemenu executable and nested device-hidden namespaces"]
fn selected_bemenu_negotiates_through_production_protection() {
    protected_bemenu(|_, _| {});
}

#[test]
#[ignore = "requires explicit Bemenu executable and nested device-hidden namespaces"]
fn protected_bemenu_uploads_catalog_pixels_to_real_content_stores() {
    protected_bemenu(bemenu::exercise);
}

fn protected_bemenu(
    exercise: impl FnOnce(
        &mut ShellComponentProcesses,
        sophia_session::shell_component_connections::ComponentConnectionKey,
    ),
) {
    use sophia_session::shell_component_launch::ShellComponentLaunch;
    use std::time::{Duration, Instant};
    // Binding, not a bare path: the prepared artifact, its commit object and
    // the operator's expected SHA-256 (see support/bemenu_files/artifact.rs).
    let executable = bemenu_binary("protected");
    let directory = std::env::temp_dir().join(format!("protected-bemenu-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut owner = ShellComponentProcesses::new().unwrap();
    // A real protected fixture bar retains pixels in the same aggregate owner.
    // It does not render through Lom/GPU; the launcher below is the actual C app.
    let bar_slot = owner
        .add(
            "bar",
            ShellComponentRole::Bar,
            &directory.join("bar"),
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
    let bar = owner
        .start(
            bar_slot,
            |key, socket| {
                let domain =
                    ProtectionDomainSpec::bubblewrap([ProtectionDomainRole::MetadataShell])
                        .map_err(|e| e.to_string())?
                        .path(sophia_runtime::ProtectionPath::read_only(
                            socket.parent().unwrap(),
                        ))
                        .map_err(|e| e.to_string())?;
                Ok(ProcessLaunchSpec::new(std::env::current_exe().unwrap())
                    .arg("protected_component_peer")
                    .arg("--exact")
                    .arg("--ignored")
                    .env("SOPHIA_FIXTURE_ROLE", key.slot.to_string())
                    .protection_domain(domain))
            },
            ShellContentAdmissionPolicy::Granted {
                discrete_input: true,
            },
        )
        .unwrap();
    let bar_pixels = peer::receive_resource(&mut owner, bar);
    let slot = owner
        .add(
            "menu",
            ShellComponentRole::ApplicationLauncher,
            &directory.join("menu"),
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
    let plan = ShellComponentLaunch::new(
        sophia_config::ShellComponentConfig {
            id: "menu".into(),
            role: ShellComponentRole::ApplicationLauncher,
            executable: executable.into(),
            config: None,
            reservation: None,
            gpu: sophia_config::ShellGpuMode::Denied,
            transport: Default::default(),
        },
        None,
        None,
    )
    .unwrap();
    let key = owner
        .start(
            slot,
            |key, socket| {
                let (spec, gpu) = plan.prepare(key, socket).map_err(|e| e.to_string())?;
                assert!(gpu.is_none());
                Ok(spec)
            },
            ShellContentAdmissionPolicy::Granted {
                discrete_input: true,
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while owner.phase(key).unwrap() != ComponentConnectionPhase::Connected {
        let visit = owner.visit(64 * 1024);
        for (_, result) in visit.negotiations.into_iter().flatten() {
            result.unwrap();
        }
        assert!(
            owner.process_retained(key),
            "Bemenu exited before negotiation"
        );
        assert!(Instant::now() < deadline, "Bemenu negotiation timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(owner.process_retained(key));
    owner
        .with_connection(key, |connection| {
            assert!(connection.supports_native_launcher());
            assert_eq!(connection.content_grant(), Some(key.grant));
        })
        .unwrap();
    exercise(&mut owner, key);
    assert_ne!(bar.grant, key.grant);
    assert!(owner.process_retained(bar));
    assert_eq!(
        owner.phase(bar).unwrap(),
        ComponentConnectionPhase::Connected
    );
    assert_eq!(bar_pixels.bytes(), &[1, 2, 3, 255]);
    owner.request_stop(key).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while owner.process_retained(key) {
        owner.visit(1024);
        assert!(Instant::now() < deadline, "Bemenu stop timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(owner.process_retained(bar));
    owner.request_stop(bar).unwrap();
    while owner.process_retained(bar) {
        owner.visit(1024);
        assert!(Instant::now() < deadline, "bar stop timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(bar_pixels);
    assert!(owner.finish_after_backend_drop(()).unwrap().1.quiescent());
    drop(owner);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires explicit device-hidden protected Bemenu binary"]
fn joined_bemenu_evidence_requires_exact_current_negotiation() {
    use sophia_session::shell_component_session::ShellComponentSession;
    use std::time::{Duration, Instant};
    let binary = bemenu_binary("joined");
    let root = std::env::temp_dir().join(format!("joined-bemenu-evidence-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let selection = sophia_config::ShellComponentConfig {
        id: "menu".into(),
        role: ShellComponentRole::ApplicationLauncher,
        executable: binary.into(),
        config: None,
        reservation: None,
        gpu: sophia_config::ShellGpuMode::Denied,
        transport: Default::default(),
    };
    let mut owner = ShellComponentSession::prepare(
        &[selection],
        0,
        None,
        &root,
        ShellContentAdmissionPolicy::Granted {
            discrete_input: true,
        },
    )
    .unwrap();
    let outputs = [sophia_engine::HeadlessOutput {
        id: sophia_protocol::OutputId::from_raw(1),
        size: sophia_protocol::Size {
            width: 64,
            height: 64,
        },
        scale: 1,
    }];
    let mut runtime =
        sophia_backend_live::LiveProductionVisualRuntime::new(&outputs, None).unwrap();
    owner.set_presentation_available(true).unwrap();
    let mut previous = None;
    for _ in 0..2 {
        let key = owner.start(0).unwrap();
        assert!(
            owner.launch_evidence(key).is_err(),
            "spawn is not negotiation"
        );
        if let Some(old) = previous {
            assert!(owner.launch_evidence(old).is_err());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while owner.phase(key).unwrap() != ComponentConnectionPhase::Connected {
            let visit = owner.poll(64 * 1024).unwrap();
            for (_, result) in visit.negotiations.into_iter().flatten() {
                result.unwrap();
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let (role, gpu) = owner.launch_evidence(key).unwrap();
        assert_eq!(role, ShellComponentRole::ApplicationLauncher);
        assert!(gpu.is_none(), "launcher cannot inherit a GPU grant");
        let mut wrong = key;
        wrong.grant.content_grant_epoch += 1;
        assert!(owner.launch_evidence(wrong).is_err());
        owner.stop(key).unwrap();
        assert!(owner.launch_evidence(key).is_err());
        while owner.process_retained(key) {
            owner.poll(64 * 1024).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(owner.settle_revocations(Some(&mut runtime)).unwrap(), 1);
        previous = Some(key);
    }
    owner.request_shutdown().unwrap();
    owner.settle_revocations(Some(&mut runtime)).unwrap();
    assert!(owner.finish_after_backend_drop(()).unwrap().1.quiescent());
    drop(owner);
    std::fs::remove_dir_all(root).unwrap();
}
