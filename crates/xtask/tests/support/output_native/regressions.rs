//! Startup and publication identity regressions from the attended validate run.
use super::*;

#[test]
fn attended_validate_log_passes_without_rewriting_its_records() {
    // Extracted recognized records, byte-for-byte, from the retained failed
    // verifier run t253-native-run-2808fcc-01/run/validate/session.log.
    // Session 170d606b6 and the peer passed; integration 2808fcc misclassified
    // startup's apply as an effect of ValidateOnly. No hardware runs here.
    // Full source SHA256: 32ee21bbf78e93f3869e6c1ba38987e8be458a3f43517894116f10b190cac634.
    let text = include_str!("attended-validate.log");
    assert_eq!(
        sha256(text.as_bytes()),
        "d0152ae0fa23aa338f5f693724f1d6cd0783372fa4793803e0493b8c687819ac"
    );
    let dir = Dir::new("attended-output-validate");
    fs::create_dir(dir.0.join("validate")).unwrap();
    fs::write(dir.0.join("validate/session.log"), text).unwrap();
    let (mut a, mut b) = layouts();
    a.heads = "1:260:normal:disabled,2:513:normal:disabled".into();
    b.heads = "1:257:normal:disabled,2:513:normal:disabled".into();
    a.groups = "1@0,0,2560x1440=1/fit;2@2560,0,1920x1080=2/fit".into();
    b.groups = a.groups.clone();
    let paths = Paths {
        sophia: "/sealed/bin/sophia".into(),
        hagia: "/sealed/bin/hagia".into(),
        profile: "/sealed/profiles/integration/baseline.kdl".into(),
        peer: "/preparation/peer".into(),
    };
    let mut argv = vec![
        paths.sophia.clone(),
        "--display=:91".into(),
        "--native-scanout".into(),
        "--session-mode=normal".into(),
        "--max-runtime-ms=60000".into(),
        format!("--desktop-profile={}", paths.profile),
        format!("--wm-process={}", paths.hagia),
        format!("--output-process={}", paths.peer),
        "--output-proof-readback".into(),
    ];
    argv.extend(
        peer_argv("validate", &a, &b)
            .unwrap()
            .into_iter()
            .map(|arg| format!("--output-process-arg={arg}")),
    );
    let record = StageRecord {
        name: "validate".into(),
        log: "validate/session.log".into(),
        size: text.len() as u64,
        sha256: sha256(text.as_bytes()),
        session_exit: 0,
        argv,
    };
    verify_stage_record(&record, &a, &b, &paths, &dir.0).unwrap();
}

#[test]
fn startup_is_bound_to_the_baseline_and_cannot_hide_runtime_effects() {
    for (tag, edit) in [
        ("missing", 0),
        ("epoch", 1),
        ("late", 2),
        ("duplicate", 3),
        ("runtime", 4),
    ] {
        let mut fixture = Fixture::new(tag);
        let log = fixture.log("validate");
        let index = log
            .iter()
            .position(|l| l.contains("status=committed transaction=184467"))
            .unwrap();
        match edit {
            0 => {
                log.remove(index);
            }
            1 => {
                log[index] = log[index].replace("topology_epoch=2", "topology_epoch=3");
            }
            2 => {
                let row = log.remove(index);
                log.insert(log.len() - 1, row);
            }
            3 => {
                log.insert(index, log[index].clone());
            }
            _ => {
                log.insert(index, traced("sophia_live_output_authority schema=2 status=first_presented transaction=99 outputs=1"));
            }
        }
        assert!(fixture.stage("validate").is_err(), "{tag}");
    }
}

#[test]
fn publication_ids_are_independent_and_every_publication_is_accounted_for() {
    let mut fixture = Fixture::new("independent-publication-ids");
    replace(
        fixture.log("commit-restore"),
        "status=committed_snapshot_published transaction=3 ",
        "transaction=3",
        "transaction=91",
    );
    replace(
        fixture.log("commit-restore"),
        "status=committed_snapshot_published transaction=4 ",
        "transaction=4",
        "transaction=107",
    );
    fixture.stage("commit-restore").unwrap();
    for stage in ["validate", "reject", "peer-death"] {
        let mut fixture = Fixture::new(stage);
        let log = fixture.log(stage);
        log.insert(log.len() - 1, traced("sophia_live_output_authority schema=2 status=committed_snapshot_published transaction=99 topology_epoch=3 transport_published=true"));
        assert!(
            fixture
                .stage(stage)
                .unwrap_err()
                .contains("snapshot publications")
        );
    }
    for (tag, from, to) in [
        ("reused-id", "transaction=4", "transaction=3"),
        ("zero-id", "transaction=4", "transaction=0"),
        ("epoch", "topology_epoch=4", "topology_epoch=7"),
        (
            "unpublished",
            "transport_published=true",
            "transport_published=false",
        ),
    ] {
        let mut fixture = Fixture::new(tag);
        replace(
            fixture.log("commit-restore"),
            "status=committed_snapshot_published transaction=4 ",
            from,
            to,
        );
        assert!(fixture.stage("commit-restore").is_err(), "{tag}");
    }
}

#[test]
fn peer_records_may_overtake_owner_publication_logging() {
    let mut fixture = Fixture::new("worker-peer-overtake");
    let log = fixture.log("commit-restore");
    let at = log
        .iter()
        .position(|l| l.contains("status=committed_snapshot_published transaction=3 "))
        .unwrap();
    let publication = log.remove(at);
    let commit = log.remove(at);
    let topology = log
        .iter()
        .position(|l| l.contains("event=topology") && l.contains("match=b"))
        .unwrap();
    log.splice(topology + 1..topology + 1, [publication, commit]);
    fixture.stage("commit-restore").unwrap();
}

#[test]
fn publication_must_follow_its_owner_settlement_and_precede_commit() {
    for stage in ["validate", "commit-restore"] {
        for before in [true, false] {
            let mut fixture = Fixture::new("publication-order");
            let log = fixture.log(stage);
            let id = if stage == "validate" { 2 } else { 3 };
            let marker = format!("status=committed_snapshot_published transaction={id} ");
            let at = log.iter().position(|l| l.contains(&marker)).unwrap();
            let row = log.remove(at);
            log.insert(if before { at - 1 } else { at + 1 }, row);
            assert!(fixture.stage(stage).unwrap_err().contains("bracketed"));
        }
    }
}
