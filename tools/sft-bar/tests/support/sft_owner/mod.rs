//! Actual owner/copy-worker join; authentication and native completion are supplied.
use host_protocol::*;
use sophia_config::ShellComponentRole;
use sophia_runtime::*;
use sophia_session::shell_component_connections::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
mod authority;
mod host;

#[test]
fn both_protocols_present_identical_cpu_frames_and_drain_the_owner() {
    for height in [24, 512] {
        let legacy = run(height, false, false);
        let sft = run(height, true, false);
        assert_eq!(sft, legacy);
        let terminal = run(height, true, true);
        assert_eq!(terminal, legacy);
    }
}

fn run(height: u32, sft: bool, terminal_only: bool) -> Vec<Vec<u8>> {
    let directory = std::env::temp_dir().join(format!(
        "sft-bar-{}-{height}-{sft}-{terminal_only}",
        std::process::id()
    ));
    assert!(!directory.exists());
    let mut owner = ShellComponentConnections::new().unwrap();
    let slot = owner
        .add(
            "bar",
            ShellComponentRole::Bar,
            &directory,
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
    if sft {
        if terminal_only {
            owner.enable_terminal_frame_transactions().unwrap();
        } else {
            owner.enable_frame_transactions().unwrap();
        }
    }
    let key = owner.reserve_attempt(slot).unwrap();
    owner
        .begin_authenticated_negotiation(
            key,
            &ProtectionDomainEvidence {
                backend: ProtectionBackendKind::Bubblewrap,
                supervisor_pid: std::process::id(),
                peer_pid: std::process::id(),
                roles: [ProtectionDomainRole::MetadataShell].into_iter().collect(),
            },
            Duration::from_secs(10),
            ShellContentAdmissionPolicy::Granted {
                discrete_input: true,
            },
            Box::new(authority::Factory),
        )
        .unwrap();
    let peer = start(
        owner.socket_path(slot).unwrap().into(),
        height,
        sft,
        terminal_only,
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut host = host::Host::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        while !peer.is_finished() {
            assert!(
                Instant::now() < deadline,
                "reference peer progress deadline"
            );
            for (_, result) in owner.poll_negotiations(65536).into_iter().flatten() {
                assert_eq!(result.unwrap().selected_revision, if sft { 13 } else { 10 });
            }
            assert!(owner.turn_connected().iter().all(Option::is_none));
            if owner.phase(key) == Ok(ComponentConnectionPhase::Connected) {
                let now = owner.content_clock_msec();
                owner
                    .with_connection(key, |t| host.turn(t, now, height))
                    .unwrap();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }));
    owner.close(key).unwrap();
    // Disconnect also wakes a producer whose bounded wait outlived a failed assertion.
    let peer_result = peer.join().unwrap();
    let shown = std::mem::take(&mut host.shown);
    let demands = host.demands;
    drop(host);
    assert!(owner.collect().quiescent());
    owner.finish_after_backend_drop(()).unwrap();
    drop(owner);
    assert!(!directory.exists());
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
    peer_result.unwrap();
    assert_eq!(demands, if sft { 0 } else { 4 });
    assert_eq!(shown.len(), 4);
    shown
        .into_iter()
        .enumerate()
        .map(|(index, mut frame)| {
            assert_eq!(frame.len(), 1);
            let (_, bytes) = frame.pop().unwrap();
            let mut expected = vec![0; 1920 * height as usize * 4];
            crate::fill_cpu(&mut expected, index as u64 + 1);
            assert_eq!(bytes, expected);
            bytes
        })
        .collect()
}

fn start(
    socket: PathBuf,
    height: u32,
    sft: bool,
    terminal_only: bool,
) -> std::thread::JoinHandle<std::result::Result<(), String>> {
    std::thread::spawn(move || {
        use crate::*;
        use sophia_9p_client::pipeline::{Pipeline, PipelineLimits, Reply};
        let timeout = Duration::from_secs(10);
        let mut pipeline = Pipeline::over(
            UnixStream::connect(&socket).unwrap(),
            PipelineLimits::default(),
            timeout,
        )
        .unwrap();
        let (tag, afid) = pipeline.auth(&[], &[]).unwrap();
        assert!(matches!(
            pipeline.wait(tag, Instant::now() + timeout).unwrap(),
            Reply::Auth(_)
        ));
        let (tag, root) = pipeline.attach_with_afid(afid, &[], &[]).unwrap();
        assert!(matches!(
            pipeline.wait(tag, Instant::now() + timeout).unwrap(),
            Reply::Attach(_)
        ));
        let tag = pipeline.clunk(afid).unwrap();
        pipeline.wait(tag, Instant::now() + timeout).unwrap();
        let client = ShellConnection::from_attached(
            pipeline,
            root,
            ShellClientOptions {
                minimum_revision: if sft { 13 } else { 10 },
                maximum_revision: if sft { 13 } else { 10 },
                required_capabilities: SOPHIA_SHELL_CAPABILITY_DESCRIPTOR_SWITCHER
                    | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE
                    | SOPHIA_SHELL_CAPABILITY_CONTENT_DISCRETE_INPUT
                    | IMAGE_CAPABILITY_OWNED_RING
                    | if sft {
                        sophia_shell_protocol::shell_files::SHELL_FRAME_CAPABILITY
                    } else {
                        0
                    }
                    | if terminal_only {
                        sophia_shell_protocol::shell_files::SHELL_FRAME_TERMINAL_ONLY_CAPABILITY
                    } else {
                        0
                    },
                handshake_timeout: timeout,
            },
        )
        .map_err(|e| e.to_string())?;
        let mut wire = Wire { client };
        let config = Config {
            wire: "files".into(),
            source: "owned_ring".into(),
            producer: "cpu".into(),
            ring_slots: 2,
            output: 1,
            frames: 4,
            height,
            hold_msec: 0,
            frame_transactions: sft,
            terminal_only,
        };
        run_connected(&mut wire, &config, socket.as_os_str()).map_err(|e| e.to_string())
    })
}
