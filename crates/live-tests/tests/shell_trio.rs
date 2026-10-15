//! Three actual clients share bounded content custody. No physical presentation
//! or input: completion and launcher typing are explicitly supplied by the host.
#[path = "support/trio_peer.rs"]
mod peer;
use peer::Peer;
use sha2::{Digest, Sha256};
use sophia_backend_live::LiveRenderDeviceIdentitySnapshot;
use sophia_config::*;
use sophia_runtime::ContentEpochRegistry;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}
fn artifact(name: &str) -> PathBuf {
    let path = PathBuf::from(required(name));
    assert!(path.is_absolute());
    let hash = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
    assert_eq!(hash, required(&format!("{name}_SHA256")));
    println!(
        "shell_artifact name={name} path={} sha256={hash}",
        path.display()
    );
    path
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn selection(name: &str, role: ShellComponentRole) -> ShellComponentConfig {
    let config = (role != ShellComponentRole::ApplicationLauncher)
        .then(|| artifact(&format!("SOPHIA_TRIO_{name}_CONFIG")));
    ShellComponentConfig {
        id: name.to_lowercase(),
        role,
        executable: artifact(&format!("SOPHIA_TRIO_{name}_BINARY")),
        config,
        gpu: if role == ShellComponentRole::ApplicationLauncher {
            ShellGpuMode::Denied
        } else {
            ShellGpuMode::Direct
        },
        reservation: match role {
            ShellComponentRole::Bar => Some(ShellComponentReservation {
                edge: ShellComponentEdge::Top,
                max_thickness: 24,
            }),
            ShellComponentRole::Dock => Some(ShellComponentReservation {
                edge: ShellComponentEdge::Bottom,
                max_thickness: 64,
            }),
            _ => None,
        },
        transport: ShellTransportSelection::NineP2000L,
    }
}

fn turn(peers: &mut [&mut Peer], registry: &mut ContentEpochRegistry, start: Instant) {
    let now = start.elapsed().as_millis() as u64 + 1;
    for peer in peers {
        peer.tick(registry, now)
            .unwrap_or_else(|e| panic!("{:?}: {e:?}", peer.role));
    }
    std::thread::sleep(Duration::from_millis(2));
}

fn device() -> LiveRenderDeviceIdentitySnapshot {
    assert_eq!(required("SOPHIA_TRIO_GPU_ARM"), "1");
    let node = PathBuf::from(required("SOPHIA_TRIO_RENDER_NODE"))
        .canonicalize()
        .unwrap();
    let metadata = std::fs::symlink_metadata(&node).unwrap();
    assert!(metadata.file_type().is_char_device());
    let physical = PathBuf::from(format!(
        "/sys/dev/char/{}:{}/device",
        rustix::fs::major(metadata.rdev()),
        rustix::fs::minor(metadata.rdev())
    ))
    .canonicalize()
    .unwrap();
    assert_eq!(
        physical.file_name().unwrap().to_str().unwrap(),
        required("SOPHIA_TRIO_PCI_BUS_ID")
    );
    let identity = LiveRenderDeviceIdentitySnapshot {
        node,
        device: metadata.dev(),
        inode: metadata.ino(),
        device_number: metadata.rdev(),
        physical_device: physical,
    };
    assert!(identity.is_render_node());
    identity
}

#[test]
#[ignore = "explicit three-client artifacts, hashes and exact render-node arm required"]
fn bar_and_launcher_progress_through_stalled_dock_and_replacement() {
    let identity = device();
    let bar_selection = selection("LOM", ShellComponentRole::Bar);
    let menu_selection = selection("BEMENU", ShellComponentRole::ApplicationLauncher);
    let dock_selection = selection("PROVLITA", ShellComponentRole::Dock);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        Scratch(std::env::temp_dir().join(format!("shell-trio-{}-{stamp}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    let mut registry = ContentEpochRegistry::with_active_capacity(64 * 1024 * 1024, 3).unwrap();
    let mut bar = Peer::launch(
        bar_selection,
        &identity,
        &root.0.join("bar"),
        &mut registry,
        81,
    );
    let mut menu = Peer::launch(
        menu_selection,
        &identity,
        &root.0.join("menu"),
        &mut registry,
        82,
    );
    let mut dock = Peer::launch(
        dock_selection.clone(),
        &identity,
        &root.0.join("dock"),
        &mut registry,
        83,
    );
    let start = Instant::now();
    while bar.frames == 0 || menu.frames == 0 || dock.frames == 0 {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "initial trio content deadline"
        );
        turn(&mut [&mut bar, &mut menu, &mut dock], &mut registry, start);
    }
    let original = (bar.grant, menu.grant);
    let before = (bar.frames, menu.frames);
    dock.pause(true);
    dock.fill_stalled_catalog(&registry);
    menu.edit(&mut registry, "t");
    let deadline = Instant::now() + Duration::from_secs(5);
    while bar.frames <= before.0 || menu.frames <= before.1 {
        assert!(
            Instant::now() < deadline,
            "neighbors did not progress during stalled dock"
        );
        turn(&mut [&mut bar, &mut menu, &mut dock], &mut registry, start);
    }
    println!(
        "trio_isolation phase=stalled_dock bar_frames={} menu_frames={} native=false",
        bar.frames, menu.frames
    );
    dock.revoke(&mut registry);
    assert!(dock.held.is_some());
    assert_eq!(registry.accounting().retired_epochs, 1);
    let before = (bar.frames, menu.frames);
    let mut replacement = Peer::launch(
        dock_selection,
        &identity,
        &root.0.join("replacement"),
        &mut registry,
        84,
    );
    menu.edit(&mut registry, "e");
    let deadline = Instant::now() + Duration::from_secs(5);
    while replacement.frames == 0 || bar.frames <= before.0 || menu.frames <= before.1 {
        assert!(
            Instant::now() < deadline,
            "neighbors/replacement content deadline"
        );
        turn(
            &mut [&mut bar, &mut menu, &mut replacement],
            &mut registry,
            start,
        );
    }
    assert_eq!(original, (bar.grant, menu.grant));
    assert_ne!(dock.grant, replacement.grant);
    let old = dock.held.take().unwrap();
    for placement in &old.placements {
        assert!(!old.resource(placement.resource).unwrap().bytes().is_empty());
    }
    drop(old);
    registry.collect();
    assert_eq!(registry.accounting().retired_epochs, 0);
    println!(
        "trio_isolation phase=replacement bar_frames={} menu_frames={} dock_frames={} native=false",
        bar.frames, menu.frames, replacement.frames
    );
    for peer in [&mut bar, &mut menu, &mut replacement] {
        peer.revoke(&mut registry);
        peer.held.take();
    }
    registry.collect();
    assert!(registry.accounting().quiescent());
    println!("trio_isolation reclaimed=true admission=supplied presentation=supplied native=false");
    // Inputs stay bound throughout the run, not only at launch.
    for name in [
        "SOPHIA_TRIO_LOM_BINARY",
        "SOPHIA_TRIO_LOM_CONFIG",
        "SOPHIA_TRIO_BEMENU_BINARY",
        "SOPHIA_TRIO_PROVLITA_BINARY",
        "SOPHIA_TRIO_PROVLITA_CONFIG",
    ] {
        assert!(Path::new(&artifact(name)).is_file());
    }
}

#[test]
#[ignore = "explicit baseline bar/menu artifacts, hashes and exact render-node arm required"]
fn two_component_baseline_keeps_bar_progress_during_menu_replacement() {
    let identity = device();
    let bar_selection = selection("LOM", ShellComponentRole::Bar);
    let menu_selection = selection("BEMENU", ShellComponentRole::ApplicationLauncher);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = Scratch(
        std::env::temp_dir().join(format!("shell-baseline-{}-{stamp}", std::process::id())),
    );
    std::fs::create_dir(&root.0).unwrap();
    let mut registry = ContentEpochRegistry::with_active_capacity(64 * 1024 * 1024, 2).unwrap();
    let mut bar = Peer::launch_baseline(
        bar_selection,
        &identity,
        &root.0.join("bar"),
        &mut registry,
        91,
    );
    let mut menu = Peer::launch_baseline(
        menu_selection.clone(),
        &identity,
        &root.0.join("menu"),
        &mut registry,
        92,
    );
    let start = Instant::now();
    while bar.frames == 0 || menu.frames == 0 {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "baseline content deadline"
        );
        turn(&mut [&mut bar, &mut menu], &mut registry, start);
    }
    let before = (bar.frames, menu.frames);
    menu.edit(&mut registry, "t");
    let deadline = Instant::now() + Duration::from_secs(5);
    while bar.frames <= before.0 || menu.frames <= before.1 {
        assert!(Instant::now() < deadline, "baseline edit deadline");
        turn(&mut [&mut bar, &mut menu], &mut registry, start);
    }
    menu.revoke(&mut registry);
    assert_eq!(registry.accounting().retired_epochs, 1);
    let before = bar.frames;
    let grant = bar.grant;
    let mut replacement = Peer::launch_baseline(
        menu_selection,
        &identity,
        &root.0.join("replacement"),
        &mut registry,
        93,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while bar.frames <= before || replacement.frames == 0 {
        assert!(Instant::now() < deadline, "baseline replacement deadline");
        turn(&mut [&mut bar, &mut replacement], &mut registry, start);
    }
    assert_eq!(bar.grant, grant);
    assert_ne!(menu.grant, replacement.grant);
    let old = menu.held.take().unwrap();
    for placement in &old.placements {
        assert!(!old.resource(placement.resource).unwrap().bytes().is_empty());
    }
    drop(old);
    registry.collect();
    assert_eq!(registry.accounting().retired_epochs, 0);
    println!(
        "baseline_content bar_frames={} menu_frames={} replacement_frames={} native=false",
        bar.frames, menu.frames, replacement.frames
    );
    for peer in [&mut bar, &mut replacement] {
        peer.revoke(&mut registry);
        peer.held.take();
    }
    registry.collect();
    assert!(registry.accounting().quiescent());
    for name in [
        "SOPHIA_TRIO_LOM_BINARY",
        "SOPHIA_TRIO_LOM_CONFIG",
        "SOPHIA_TRIO_BEMENU_BINARY",
    ] {
        artifact(name);
    }
    println!(
        "baseline_content reclaimed=true admission=supplied presentation=supplied native=false"
    );
}
