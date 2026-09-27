// Provenance: the body of Sophia crates/sophia-runtime/tests/shell_bemenu_files.rs
// at 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13), extended (G2)
// with a neighbour bar, allocation and candidate shape assertions and a
// held-lease retirement phase. 9P-only: the IPC twin is retired with product
// IPC.
//! The live Bemenu gate body of bemenu_files (the 9P file export only). See
//! bemenu_files.rs for scope.
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::fixture::{self, Wire};
use crate::{artifact, sandbox};
use sophia_protocol::NativeLauncherInputKind;

// Harness liveness bounds, not performance requirements. The application keeps
// its own protocol and local failure deadlines unchanged.
const TOTAL: Duration = Duration::from_secs(120);
const PHASE: Duration = Duration::from_secs(30);

/// The private scratch root, removed however the gate ends. (Sophia's copy
/// came from its shell_files_oracle support module; only this guard was used.)
struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Service the owners until `done`, bounded per phase and overall. Owner errors,
/// early exit and oversized output end the gate with Bemenu's stderr.
fn until(
    f: &mut fixture::Fixture,
    peer: &mut sandbox::Peer,
    start: Instant,
    what: &str,
    mut done: impl FnMut(&fixture::Fixture) -> bool,
) {
    let phase = Instant::now();
    loop {
        peer.check();
        if let Err(error) = f.tick() {
            panic!("{what}: {error}\n{}", peer.stderr());
        }
        if done(f) {
            return;
        }
        assert!(
            phase.elapsed() < PHASE && start.elapsed() < TOTAL,
            "bemenu live gate: {what} not reached\n{}",
            peer.stderr()
        );
        std::thread::sleep(Duration::from_micros(100));
    }
}

/// The whole gate on the 9P file wire.
pub fn run(wire: Wire) {
    let label = match wire {
        Wire::Files => "files",
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = std::env::temp_dir().join(format!("sophia-bemenu-{label}-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let scratch = Scratch(root);
    std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o700)).unwrap();

    // Identity first: nothing executes before every binding check passes.
    let artifact = artifact::load(&repo, &scratch.0.join("bin"));
    let fonts = sandbox::fonts(&repo, &scratch.0.join("fonts"));
    let mut f = fixture::Fixture::new(&scratch.0, wire);
    let socket = f.socket_path().to_path_buf();
    let mut peer = sandbox::launch(&scratch.0, &artifact.binary, &socket, &fonts, wire);
    let start = Instant::now();
    peer.verify_domain(&artifact.binary, &socket, &fonts, wire);
    f.authorize(peer.evidence());
    // A neighbouring bar on a second transport in the same registry; its
    // connection and exact content usage must survive Bemenu's lifecycle.
    f.connect_neighbour(&scratch.0);
    let neighbour_usage = f.neighbour_usage();

    // a negotiation; b catalog and output-fact objects; c opening 7.
    until(&mut f, &mut peer, start, "negotiation", |f| f.live());
    f.publish();
    f.open(7);

    // d the peer's allocation and its actual Cairo raster upload.
    until(&mut f, &mut peer, start, "first raster", |f| {
        f.prepared().is_some()
    });
    let first = f.prepared().unwrap();
    assert_eq!((first.opening, first.revision), (7, 1));
    assert_eq!((first.rows.as_slice(), first.selected), (&[1, 2, 3][..], 1));
    for pixels in &first.pixels {
        assert!(
            pixels.iter().any(|b| *b != pixels[0]),
            "uploaded raster is uniform"
        );
    }
    let first_pixels = first.pixels.clone();
    assert_eq!(f.granted.len(), 1);

    // e Prepared -> Presented (scripted) -> the owner's exact focus.
    let focus = f.present();
    assert_eq!(
        (
            focus.opening,
            focus.catalog_generation,
            focus.interaction_generation,
            focus.state_revision,
            focus.allocation
        ),
        (7, 1, 1, 1, f.granted[0])
    );

    // f a text edit: rows and pixels both change in the next candidate.
    f.input(NativeLauncherInputKind::Text, "br");
    until(
        &mut f,
        &mut peer,
        start,
        "text ack and edited raster",
        |f| f.input_acks == 1 && f.prepared().is_some_and(|s| s.revision == 2),
    );
    let edited = f.prepared().unwrap();
    assert_eq!((edited.opening, edited.rows.as_slice()), (7, &[2][..]));
    assert_eq!(edited.selected, 2);
    assert_ne!(
        edited.pixels, first_pixels,
        "text edit left the pixels unchanged"
    );
    let focus = f.present();
    assert_eq!((focus.opening, focus.state_revision), (7, 2));

    // g keyboard Accept: exactly one activation, admitted by scripted Session.
    f.input(NativeLauncherInputKind::Accept, "");
    until(&mut f, &mut peer, start, "keyboard activation", |f| {
        f.input_acks == 2 && !f.activations.is_empty()
    });
    let activation = f.activations[0];
    assert_eq!((activation.cause, activation.slot), (1, 2));
    assert_eq!(activation.event.binding.opening, 7);

    // h close, invalidation and the peer's resource retirement.
    f.close(7);
    until(&mut f, &mut peer, start, "opening 7 settled", |f| {
        f.settled(7)
    });

    // i reopen: a fresh allocation and raster with the query reset. The
    // renderer keeps this candidate's lease across close (held-lease phase).
    f.hold_next = true;
    f.open(8);
    until(&mut f, &mut peer, start, "reopened raster", |f| {
        f.prepared().is_some_and(|s| s.opening == 8)
    });
    let reopened = f.prepared().unwrap();
    assert_eq!(
        (reopened.revision, reopened.rows.as_slice()),
        (1, &[1, 2, 3][..])
    );
    assert_eq!(reopened.selected, 1);
    assert_eq!(f.granted.len(), 2);
    let focus = f.present();
    assert_eq!((focus.opening, focus.allocation), (8, f.granted[1]));
    f.close(8);
    // While the renderer still holds the lease, the peer's retired bytes stay
    // charged as retiring and the closed owners cannot settle.
    until(
        &mut f,
        &mut peer,
        start,
        "opening 8 retiring under a held lease",
        |f| f.retiring() > 0,
    );
    assert!(!f.settled(8), "closed owners settled while a lease is held");
    f.release();
    until(&mut f, &mut peer, start, "opening 8 settled", |f| {
        f.settled(8)
    });
    assert_eq!(
        f.neighbour_usage(),
        neighbour_usage,
        "the launcher's lifecycle changed the neighbour's content usage"
    );

    // j graceful stop, then the owners must hold nothing.
    assert_eq!(
        (
            f.input_acks,
            f.unmatched_acks,
            f.activations.len(),
            f.admissions,
            f.pointer
        ),
        (2, 0, 1, 1, 0)
    );
    let (stdout, stderr) = peer.stop();
    f.cleanup();
    assert!(stdout.is_empty(), "bemenu stdout: {stdout}");
    let lines = stderr.lines().collect::<Vec<_>>();
    let negotiated = format!(
        "bemenu_native status=negotiated revision=7 epoch={}{}",
        fixture::EPOCH,
        wire.negotiated_suffix()
    );
    let announced = lines
        .iter()
        .copied()
        .filter(|l| l.contains("status=negotiated"))
        .collect::<Vec<_>>();
    assert_eq!(announced, [negotiated.as_str()], "{stderr}");
    assert!(!stderr.contains("status=failed"), "{stderr}");
    // Bemenu is 9P-only: the negotiated line names the file wire.
    assert!(
        negotiated.ends_with(" wire=9p"),
        "the negotiated record must carry wire=9p: {negotiated}"
    );
    assert_eq!(
        lines.last(),
        Some(&"bemenu_native status=stopped result=0"),
        "{stderr}"
    );
    println!(
        "sophia_bemenu_{label} status=pass bemenu={} binary_sha256={} signer={} sdk={} \
         openings=2 candidates={} edits=1 activations=1 fonts=isolated neighbour=unchanged \
         held_lease=retired",
        artifact.commit,
        artifact.sha256,
        artifact.signer,
        artifact.sdk,
        f.shown.len()
    );
}
