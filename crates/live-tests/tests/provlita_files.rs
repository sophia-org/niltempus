//! Device-free, protected Provlita executable join against the production
//! file export. Admission is supplied from real supervisor evidence, not a
//! Session factotum launch. No GPU grant is supplied: successful negotiation
//! must end at that boundary, without claiming content or native acceptance.
use sha2::{Digest, Sha256};
use sophia_protocol::{ContentGrant, ContentLimits};
use sophia_runtime::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GPU_REFUSAL: &str = "Sophia did not grant direct GPU access to this shell process";
const LIMIT: Duration = Duration::from_secs(15);

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn digest(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

fn log(path: &Path) -> String {
    if !path.exists() {
        return String::new();
    }
    assert!(std::fs::metadata(path).unwrap().len() < 64 * 1024);
    std::fs::read_to_string(path).unwrap()
}

#[test]
#[ignore = "requires explicit SOPHIA_PROVLITA_BINARY, SHA256, CONFIG and CONFIG_SHA256; no GPU"]
fn protected_dock_negotiates_files_then_refuses_missing_gpu_grant() {
    let binary = PathBuf::from(required("SOPHIA_PROVLITA_BINARY"));
    let config = PathBuf::from(required("SOPHIA_PROVLITA_CONFIG"));
    let binary_hash = required("SOPHIA_PROVLITA_SHA256");
    let config_hash = required("SOPHIA_PROVLITA_CONFIG_SHA256");
    assert!(binary.is_absolute() && config.is_absolute());
    assert_eq!(digest(&binary), binary_hash);
    assert_eq!(digest(&config), config_hash);
    let root = Scratch(std::env::temp_dir().join(format!(
        "provlita-files-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    )));
    std::fs::create_dir(&root.0).unwrap();
    // Each process gets a new epoch and export. The second launch must recover
    // after the first refusal, and neither may leave content reservations.
    let mut epochs = ContentEpochRegistry::new(64 * 1024 * 1024).unwrap();
    for epoch in [61, 62] {
        let directory = root.0.join(epoch.to_string());
        std::fs::create_dir(&directory).unwrap();
        let logs = directory.join("logs");
        std::fs::create_dir(&logs).unwrap();
        let stdout = logs.join("stdout");
        let stderr = logs.join("stderr");
        let mut owner = ShellComponentTransport::bind_for_supervised_uid(
            directory.join("export"),
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
        let domain = ProtectionDomainSpec::bubblewrap([ProtectionDomainRole::MetadataShell])
            .unwrap()
            .inherited_fds([])
            .unwrap()
            .path(ProtectionPath::read_only(binary.parent().unwrap()))
            .unwrap()
            .path(ProtectionPath::read_only(config.parent().unwrap()))
            .unwrap()
            .path(ProtectionPath::read_only(
                owner.socket_path().parent().unwrap(),
            ))
            .unwrap()
            .path(ProtectionPath::read_write(&logs))
            .unwrap();
        // Positional arguments preserve paths; env -i excludes ambient display,
        // GPU, legacy-wire and launch-auth grants. The supervisor owns the PID.
        let spec = ProcessLaunchSpec::new("/usr/bin/sh")
            .arg("-c")
            .arg("exec /usr/bin/env -i PATH=/usr/bin SOPHIA_SHELL_9P_SOCKET=\"$2\" SOPHIA_SHELL_CONFIG=\"$3\" SOPHIA_SHELL_BAR_THICKNESS=64 \"$1\" --serve >\"$4\" 2>\"$5\"")
            .arg("provlita-join").arg(&binary).arg(owner.socket_path())
            .arg(&config).arg(&stdout).arg(&stderr).protection_domain(domain);
        let mut peer = ProcessSupervisor::new(SupervisedProcessKind::Shell, spec);
        peer.apply(SupervisorCommand::StartProcess {
            process: SupervisedProcessKind::Shell,
            delay: Duration::ZERO,
        })
        .unwrap();
        let evidence = peer.protection_evidence().unwrap().clone();
        assert_eq!(evidence.backend, ProtectionBackendKind::Bubblewrap);
        assert_eq!(peer.peer_id(), Some(evidence.peer_pid));
        owner.authorize_protected_peer(&evidence).unwrap();
        let grant = ContentGrant {
            connection_epoch: epoch,
            content_grant_epoch: epoch,
        };
        owner
            .reserve_content_with_profile(
                &mut epochs,
                ContentLimits::prototype(grant),
                ContentStoreProfile::PersistentCatalog,
            )
            .unwrap();
        owner
            .begin_file_negotiation(
                &epochs,
                epoch,
                LIMIT,
                ShellContentAdmissionPolicy::Granted {
                    discrete_input: true,
                },
            )
            .unwrap();
        let deadline = Instant::now() + LIMIT;
        let mut negotiated = false;
        loop {
            assert!(Instant::now() < deadline, "join deadline: {}", log(&stderr));
            if !negotiated {
                if let Some(welcome) = owner
                    .poll_negotiation(&mut epochs, 64 * 1024)
                    .unwrap_or_else(|e| panic!("negotiation: {e}; {}", log(&stderr)))
                {
                    assert_eq!(welcome.selected_revision, 8);
                    negotiated = true;
                }
            } else {
                // The peer may close immediately after the final response.
                if let Err(error) = owner.service_owner_turn(&mut epochs) {
                    eprintln!("provlita export turn during shutdown: {error}");
                }
            }
            if peer.poll().unwrap() == Some(SupervisorEvent::ProcessExited) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let out = log(&stdout);
        let err = log(&stderr);
        assert!(negotiated, "exited before negotiation: {err}");
        let expected =
            format!("provlita_shell_transport schema=1 wire=9p2000.L revision=8 epoch={epoch}");
        assert_eq!(
            out.lines().filter(|line| *line == expected).count(),
            1,
            "{out}"
        );
        assert!(err.contains(GPU_REFUSAL), "wrong refusal: {err}");
        assert!(
            !out.contains("provlita_gpu"),
            "unexpected GPU admission: {out}"
        );
        owner.disconnect(&mut epochs).unwrap();
        epochs.collect();
        assert!(epochs.accounting().quiescent(), "content custody retained");
        println!(
            "provlita_export_join epoch={epoch} negotiated=true gpu=refused reclaimed=true authentication=supplied native=false binary_sha256={binary_hash}"
        );
    }
    assert_eq!(digest(&binary), binary_hash);
    assert_eq!(digest(&config), config_hash);
}
