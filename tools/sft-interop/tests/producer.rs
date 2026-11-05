//! Real Lom CPU rendering/service against Sophia's Session file/image owners.
//! Afid verdicts, allocation policy and renderer completion are supplied here.
use sophia_config::ShellComponentRole;
use sophia_protocol::*;
use sophia_runtime::*;
use sophia_session::shell_component_connections::*;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

mod support;
use support::{authority, host, peer};

#[test]
fn lom_cpu_regions_match_legacy_through_the_real_sft_owner() {
    let legacy = run(10);
    let frames = run(13);
    assert_eq!(frames, legacy, "SFT preserves every rendered region byte");
}

fn run(revision: u16) -> Vec<Vec<Vec<u8>>> {
    let directory = std::env::temp_dir().join(format!("sft-lom-{}-{revision}", std::process::id()));
    assert!(!directory.exists(), "refuse a stale fixture directory");
    let mut owner = ShellComponentConnections::new().unwrap();
    let slot = owner
        .add(
            "bar",
            ShellComponentRole::Bar,
            &directory,
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
    if revision == 13 {
        owner.enable_frame_transactions().unwrap();
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
    let stop = Arc::new(AtomicBool::new(false));
    let producer = peer::start(
        owner.socket_path(slot).unwrap().into(),
        directory.clone(),
        stop.clone(),
        revision,
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut host = host::Host::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        while !producer.is_finished() {
            assert!(
                Instant::now() < deadline,
                "Lom/owner progress deadline: {} frames",
                host.shown.len()
            );
            for (_, result) in owner.poll_negotiations(65536).into_iter().flatten() {
                assert_eq!(result.unwrap().selected_revision, revision);
            }
            assert!(owner.turn_connected().iter().all(Option::is_none));
            if owner.phase(key) == Ok(ComponentConnectionPhase::Connected) {
                let now = owner.content_clock_msec();
                owner
                    .with_connection(key, |transport| host.turn(transport, now))
                    .unwrap();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        host.assert_frames(revision);
    }));
    stop.store(true, Ordering::Relaxed);
    // Keep serving until the producer drops its connection and real CPU worker.
    while !producer.is_finished() && Instant::now() < deadline {
        let _ = owner.turn_connected();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(producer.is_finished(), "producer stop is bounded");
    let peer_result = producer.join().unwrap();
    owner.close(key).unwrap();
    let frames = host
        .shown
        .iter()
        .map(|frame| frame.iter().map(|(_, bytes)| bytes.clone()).collect())
        .collect();
    drop(host);
    assert!(owner.collect().quiescent());
    owner.finish_after_backend_drop(()).unwrap();
    drop(owner);
    assert!(!directory.exists());
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
    assert_eq!(
        peer_result.unwrap(),
        4,
        "producer consumed all Presented records"
    );
    frames
}
