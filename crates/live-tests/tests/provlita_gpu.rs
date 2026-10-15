//! Explicit render-node gate: actual dock GPU raster, file upload and retained
//! resource custody. Output geometry is supplied; no KMS, input or presentation
//! completion is fabricated. Authentication has a separate paired-client gate.
use sha2::{Digest, Sha256};
use sophia_backend_live::LiveRenderDeviceIdentitySnapshot;
use sophia_config::*;
use sophia_protocol::*;
use sophia_runtime::*;
use sophia_session::shell_component_connections::ComponentConnectionKey;
use sophia_session::shell_component_launch::ShellComponentLaunch;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const OUTPUT: ContentOutputId = ContentOutputId {
    id: 2,
    generation: 1,
};
const FACTS: u64 = 3;
const LIMIT: Duration = Duration::from_secs(30);

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}
fn hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Snapshot only the explicitly named render node. The production launch owner
/// revalidates this device and constructs the exact GPU and sysfs grants.
fn device() -> LiveRenderDeviceIdentitySnapshot {
    assert_eq!(required("SOPHIA_DOCK_GPU_ARM"), "1");
    let node = PathBuf::from(required("SOPHIA_DOCK_RENDER_NODE"));
    assert!(node.is_absolute());
    let node = node.canonicalize().unwrap();
    let metadata = std::fs::symlink_metadata(&node).unwrap();
    assert!(metadata.file_type().is_char_device());
    let major = rustix::fs::major(metadata.rdev());
    let minor = rustix::fs::minor(metadata.rdev());
    let physical_device = PathBuf::from(format!("/sys/dev/char/{major}:{minor}/device"))
        .canonicalize()
        .unwrap();
    assert_eq!(
        physical_device.file_name().unwrap().to_str().unwrap(),
        required("SOPHIA_DOCK_PCI_BUS_ID")
    );
    let identity = LiveRenderDeviceIdentitySnapshot {
        node,
        device: metadata.dev(),
        inode: metadata.ino(),
        device_number: metadata.rdev(),
        physical_device,
    };
    assert!(identity.is_render_node(), "KMS nodes are forbidden");
    identity
}

fn catalog(epoch: u64) -> ShellPersistentCatalog {
    ShellPersistentCatalog {
        catalog: ShellApplicationCatalog {
            connection_epoch: epoch,
            generation: 1,
            entries: vec![ShellApplicationDescriptor {
                slot: 1,
                available: true,
                label: "Terminal".into(),
                keywords: String::new(),
            }],
        },
        identities: [(1, "registered:terminal".into())].into_iter().collect(),
    }
}

fn pixels(bundle: &ContentRenderBundle) -> String {
    assert!(!bundle.placements.is_empty(), "no raster placements");
    let mut digest = Sha256::new();
    for placement in &bundle.placements {
        let resource = bundle.resource(placement.resource).unwrap();
        let bytes = resource.bytes();
        assert_eq!(bytes.len() as u64, resource.description().total_bytes);
        assert!(bytes.len() > 1024);
        assert!(
            bytes.windows(4).any(|pair| pair != &bytes[..4]),
            "uniform raster"
        );
        digest.update(bytes);
    }
    format!("{:x}", digest.finalize())
}

#[test]
#[ignore = "explicit dock binary/config hashes and exact render-node GPU arm required"]
fn dock_gpu_upload_survives_revocation_until_consumer_release() {
    let identity = device();
    let binary = PathBuf::from(required("SOPHIA_PROVLITA_BINARY"));
    let config = PathBuf::from(required("SOPHIA_PROVLITA_CONFIG"));
    assert!(binary.is_absolute() && config.is_absolute());
    let binary_hash = required("SOPHIA_PROVLITA_SHA256");
    let config_hash = required("SOPHIA_PROVLITA_CONFIG_SHA256");
    assert_eq!(hash(&binary), binary_hash);
    assert_eq!(hash(&config), config_hash);
    let scratch = Scratch(std::env::temp_dir().join(format!("dock-gpu-{}-{}",
        std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())));
    std::fs::create_dir(&scratch.0).unwrap();
    let launch = ShellComponentLaunch::new(
        ShellComponentConfig {
            id: "dock".into(),
            role: ShellComponentRole::Dock,
            executable: binary.clone(),
            config: Some(config.clone()),
            gpu: ShellGpuMode::Direct,
            reservation: Some(ShellComponentReservation {
                edge: ShellComponentEdge::Bottom,
                max_thickness: 64,
            }),
            transport: ShellTransportSelection::NineP2000L,
        },
        None,
        Some(identity),
        Path::new("/usr/bin/bwrap"),
    )
    .unwrap();
    let mut registry = ContentEpochRegistry::new(64 * 1024 * 1024).unwrap();
    let mut retained: Option<(ShellComponentTransport, ContentRenderBundle, String)> = None;
    for epoch in [71, 72] {
        let grant = ContentGrant {
            connection_epoch: epoch,
            content_grant_epoch: epoch,
        };
        let directory = scratch.0.join(epoch.to_string());
        std::fs::create_dir(&directory).unwrap();
        let mut transport = ShellComponentTransport::bind_for_supervised_uid(
            directory.join("export"),
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
        transport
            .reserve_content_with_profile(
                &mut registry,
                ContentLimits::prototype(grant),
                ContentStoreProfile::PersistentCatalog,
            )
            .unwrap();
        let (mut spec, evidence) = launch
            .prepare(
                ComponentConnectionKey { slot: 0, grant },
                transport.socket_path(),
            )
            .unwrap();
        assert_eq!(evidence.as_ref().unwrap().epoch, epoch);
        // Preserve the real renderer's evidence in the caller's log.
        spec.protection_domain = Some(
            spec.protection_domain
                .take()
                .unwrap()
                .inherited_fds([1, 2])
                .unwrap(),
        );
        let mut process = ProcessSupervisor::new(SupervisedProcessKind::Shell, spec);
        process
            .apply(SupervisorCommand::StartProcess {
                process: SupervisedProcessKind::Shell,
                delay: Duration::ZERO,
            })
            .unwrap();
        transport
            .authorize_protected_peer(process.protection_evidence().unwrap())
            .unwrap();
        transport
            .begin_file_negotiation(
                &registry,
                epoch,
                LIMIT,
                ShellContentAdmissionPolicy::Granted {
                    discrete_input: true,
                },
            )
            .unwrap();
        let start = Instant::now();
        let mut connected = false;
        let mut permit = 0;
        let mut allocation_id = 0;
        let catalog = catalog(epoch);
        let held = loop {
            assert!(start.elapsed() < LIMIT, "dock content deadline");
            assert_ne!(
                process.poll().unwrap(),
                Some(SupervisorEvent::ProcessExited),
                "dock exited before upload"
            );
            let now = start.elapsed().as_millis() as u64 + 1;
            if !connected {
                if let Some(welcome) = transport.poll_negotiation(&mut registry, 65536).unwrap() {
                    assert_eq!(welcome.selected_revision, 8);
                    connected = true;
                    transport
                        .publish_catalog(&registry, TransactionId::from_raw(1), &catalog)
                        .unwrap();
                    transport
                        .publish_content_output_facts(
                            &mut registry,
                            TransactionId::from_raw(2),
                            FACTS,
                            vec![ContentOutputFactsEntry {
                                output: OUTPUT,
                                local_width: 1280,
                                local_height: 720,
                                scale_numerator: 1,
                                scale_denominator: 1,
                                scale_generation: 1,
                            }],
                        )
                        .unwrap();
                }
            } else {
                transport.service_owner_turn(&mut registry).unwrap();
                transport
                    .service_content_resources(&mut registry, now)
                    .unwrap();
                transport
                    .service_content_allocation_requests(&mut registry, &[], now)
                    .unwrap();
                while let Some((_, request)) = transport.next_content_allocation_request(&registry)
                {
                    assert_eq!((request.role, request.edge, request.operation), (1, 3, 1));
                    assert_eq!(request.output, OUTPUT);
                    assert!(request.desired_width > 0 && request.desired_width <= 1280);
                    assert!(request.desired_height > 0 && request.desired_height <= 64);
                    allocation_id += 1;
                    let y = 720 - request.desired_height;
                    transport
                        .grant_content_allocation(
                            &mut registry,
                            request.allocation_request_id,
                            ContentAllocationSnapshot {
                                native_opening: None,
                                output: OUTPUT,
                                allocation: ContentAllocationId {
                                    id: allocation_id,
                                    generation: 1,
                                },
                                scale_generation: 1,
                                scale_numerator: 1,
                                scale_denominator: 1,
                                role: 1,
                                edge: 3,
                                margins: ContentMargins::default(),
                                logical: ContentLogicalRect {
                                    x: 0,
                                    y: y as i32,
                                    width: request.desired_width,
                                    height: request.desired_height,
                                },
                                pixel: ContentPixelRect {
                                    x: 0,
                                    y: y as i32,
                                    width: request.desired_width,
                                    height: request.desired_height,
                                },
                                parent: ContentAllocationId::default(),
                                anchor_parent_rect: ContentPixelRect::default(),
                                allowed_reservation_extent: 64,
                            },
                            &[],
                        )
                        .unwrap();
                }
                let allocations = transport.content_allocation_snapshots(&registry);
                transport
                    .service_content_demands(&mut registry, &[OUTPUT], &allocations)
                    .unwrap();
                while let Some((tx, demand)) = transport.next_content_demand(&registry) {
                    permit += 1;
                    transport
                        .grant_content_demand(&mut registry, tx, demand.output, permit, now)
                        .unwrap();
                }
                let context = ContentCandidateContext {
                    output: OUTPUT,
                    facts_generation: FACTS,
                    interaction_generation: 1,
                    allocations: &allocations,
                };
                transport
                    .service_catalog_candidates(&mut registry, &[context], &catalog.catalog, now)
                    .unwrap();
                if let Some((_, generation)) = transport.next_content_submission(&registry) {
                    let bundle = transport
                        .connection(&mut registry)
                        .begin_catalog_submission(generation, context, &catalog.catalog, now)
                        .unwrap();
                    assert_eq!(bundle.grant, grant);
                    assert_eq!(
                        bundle
                            .persistent_catalog
                            .as_ref()
                            .unwrap()
                            .catalog_generation,
                        1
                    );
                    assert!(bundle.native_launcher.is_none());
                    assert!(
                        !bundle.targets.is_empty(),
                        "authorized terminal has no target"
                    );
                    break bundle;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let digest = pixels(&held);
        // A fresh process must upload while the old consumer still owns pixels.
        if let Some((mut old_owner, old, old_hash)) = retained.take() {
            assert_ne!(old.grant, held.grant);
            assert_eq!(pixels(&old), old_hash);
            assert_eq!(registry.accounting().retired_epochs, 1);
            assert!(
                matches!(
                    old_owner.content_renderer_failed(
                        &mut registry,
                        held.grant,
                        OUTPUT,
                        held.candidate_generation
                    ),
                    Err(ShellTransportError::WrongContentGrant)
                ),
                "old owner settled live replacement"
            );
            // This consumer never presents. Explicitly fail its submitted work;
            // dropping pixels alone cannot settle a pending submission.
            old_owner
                .content_renderer_failed(&mut registry, old.grant, OUTPUT, old.candidate_generation)
                .unwrap();
            assert_eq!(registry.accounting().retired_epochs, 1);
            drop(old);
            registry.collect();
            assert_eq!(registry.accounting().retired_epochs, 0);
        }
        transport.disconnect(&mut registry).unwrap();
        process.request_termination().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !process.poll_termination().unwrap() {
            assert!(Instant::now() < deadline, "dock reap deadline");
            std::thread::sleep(Duration::from_millis(2));
        }
        registry.collect();
        assert_eq!(registry.accounting().retired_epochs, 1);
        assert!(registry.accounting().memory.resident > 0);
        assert_eq!(pixels(&held), digest);
        println!(
            "dock_gpu_content epoch={epoch} uploaded=true retained_after_revoke=true pixels_sha256={digest} native=false authentication=supplied binary_sha256={binary_hash}"
        );
        retained = Some((transport, held, digest));
    }
    let (mut owner, last, _) = retained.take().unwrap();
    owner
        .content_renderer_failed(&mut registry, last.grant, OUTPUT, last.candidate_generation)
        .unwrap();
    assert_eq!(registry.accounting().retired_epochs, 1);
    drop(last);
    registry.collect();
    assert!(registry.accounting().quiescent());
    assert_eq!(hash(&binary), binary_hash);
    assert_eq!(hash(&config), config_hash);
    println!("dock_gpu_content replacement=true reclaimed=true native=false");
}
