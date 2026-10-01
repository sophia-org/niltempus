//! The output-file native verifier against synthetic sealed inputs, a
//! synthetic Sophia preparation and four fabricated stage logs. Each negative
//! control changes one thing. Nothing here runs Sophia, the peer, Hagia or
//! hardware.
use std::cell::Cell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde_json::{Value, json};
use xtask::output_file_native::{
    EXPORT_TESTS, Layout, Paths, Request, RunManifest, SESSION_TEST, STAGES, StageRecord, Verified,
    check_argv, peer_argv, verify_sealed, verify_stage_record,
};
use xtask::physical_inputs::{header_for_tests, seal, verify, write_env_for_tests};
use xtask::pins;

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, NIM_CONFIG_SHA256, NIM_STDLIB_SHA256, repo, reviewed_deps, sha256};

#[path = "support/output_native/regressions.rs"]
mod regressions;

const PROFILE: &str = "integration/output-native.kdl";
const PROFILE_TEXT: &str = "output profile\n";
const PEER_BYTES: &[u8] = b"generic peer";
const HAGIA_COMMIT: &str = "3333333333333333333333333333333333333333";
/// Session lifecycle records, printed bare (session_println). Completion
/// carries a subset of its declared fields; the verifier needs no more.
const RUNNING: &str = "sophia_live_session schema=7 status=running display=:95 terminal=xterm runtime=persistent authority_capacity=64 input_capacity=64 control_capacity=64 native_presentation=enabled physical_input=enabled pointer_proof=disabled secondary_terminal=disabled wm_policy=external namespace_profile=desktop namespace_request_capabilities=0 namespace_publish_capabilities=0";
const COMPLETE: &str = "sophia_live_session schema=19 status=bounded_complete display=:95 elapsed_msec=60012 startup_ready_msec=not_requested session_ticks=1200 authority_batches_dropped=0 native_presentation=enabled native_in_flight=false native_cleanup_pending=false physical_input=enabled wm_policy=external namespace_profile=desktop";
const PREFIX: &str = "2026-09-30T12:00:00.000000Z  INFO sophia_session::live_session: ";

fn integration() -> String {
    "2".repeat(40)
}

fn layouts() -> (Layout, Layout) {
    (
        Layout {
            epoch: 2,
            heads: "1:1:normal:disabled".into(),
            groups: "1@0,0,2560x1440=1/exact".into(),
            primary: "0".into(),
        },
        Layout {
            epoch: 0,
            heads: "1:2:normal:disabled".into(),
            groups: "1@0,0,1920x1080=1/exact".into(),
            primary: "0".into(),
        },
    )
}

#[derive(Clone, Copy)]
enum State {
    A,
    B,
}

/// A traced Session record.
fn traced(record: &str) -> String {
    format!("{PREFIX}{record}")
}

fn peer(stage: &str, event: &str, t: u64, fields: &str) -> String {
    let fields = if fields.is_empty() {
        String::new()
    } else {
        format!(" {fields}")
    };
    format!("sophia_output_proof schema=1 stage={stage} event={event} t={t}{fields}")
}

/// One complete KMS and owner readback set.
fn set(stage: &str, t: u64, connection: u64, txn: u64, base: u64, state: State) -> Vec<String> {
    let (mode, width, height, refresh, crtc_w, crtc_h) = match state {
        State::A => (
            "2560,1440,60,241500,2608,2640,2720,1443,1448,1481,0,0,10",
            2560,
            1440,
            59951,
            2560,
            1440,
        ),
        State::B => (
            "1920,1080,60,148500,2008,2052,2200,1084,1089,1125,0,0,5",
            1920,
            1080,
            60000,
            1920,
            1080,
        ),
    };
    let common = format!(
        "stage={stage} t={t} connection_epoch={connection} base_topology_epoch={base} transaction={txn}"
    );
    let properties = format!(
        "connector.CRTC_ID:41,crtc.ACTIVE:1,plane.CRTC_H:{crtc_h},plane.CRTC_ID:41,plane.CRTC_W:{crtc_w},plane.CRTC_X:0,plane.CRTC_Y:0,plane.SRC_H:{},plane.SRC_W:{},plane.SRC_X:0,plane.SRC_Y:0,plane.rotation:1",
        crtc_h << 16,
        crtc_w << 16
    );
    vec![
        traced(&format!(
            "sophia_output_kms_readback schema=1 {common} head=1 card=0 connector=40 crtc=41 plane=42 mode={mode} properties={properties}"
        )),
        traced(&format!(
            "sophia_output_kms_readback schema=1 {common} heads=1 complete=true"
        )),
        traced(&format!(
            "sophia_output_owner_readback schema=1 {common} head=1 enabled=true output=1 native_width={width} native_height={height} native_scale=1 scale=1 refresh_millihz={refresh} transform=Normal mapping=Exact vrr=Disabled"
        )),
        traced(&format!(
            "sophia_output_owner_readback schema=1 {common} output=1 logical_width={width} logical_height={height} scale=1"
        )),
        traced(&format!(
            "sophia_output_owner_readback schema=1 {common} heads=1 outputs=1 complete=true"
        )),
    ]
}

fn prelude(stage: &str, with_b: bool) -> Vec<String> {
    // These owner records follow the attended validate log, including the
    // startup apply that precedes the peer's own physical work in every stage.
    let mut lines = vec![
        traced(
            "sophia_live_output_authority schema=2 status=apply_started transaction=18446744073709551615 heads=2 cards=ordered published=false",
        ),
        traced(
            "sophia_live_output_authority schema=2 status=first_presented transaction=18446744073709551615 outputs=2 published=false rollback_retained=true",
        ),
        traced(
            "sophia_live_output_authority schema=3 status=settled_locally transaction=18446744073709551615 outcome=Committed topology_epoch=2 reason=\"desktop profile startup\" preserved_topology=false",
        ),
        traced(
            "sophia_live_output_authority schema=2 status=committed_snapshot_published transaction=2 topology_epoch=2 transport_published=true",
        ),
        traced(
            "sophia_live_output_authority schema=2 status=committed transaction=18446744073709551615 topology_epoch=2 outputs=2 policy_required=false input=quarantined",
        ),
    ];
    lines.extend(set("baseline", 100, 0, 0, 2, State::A));
    lines.extend([
        peer(
            stage,
            "start",
            1000,
            &format!(
                "a_topology_epoch=2 a_heads=1 b={}",
                if with_b { "yes" } else { "no" }
            ),
        ),
        traced("sophia_live_output_authority schema=1 status=connected epoch=1"),
        peer(stage, "negotiated", 1001, "epoch=1 capabilities=3"),
        peer(
            stage,
            "topology",
            1002,
            "topology_epoch=2 qid=9 heads=1 groups=1 match=a",
        ),
        peer(stage, "ready", 1003, "topology_epoch=2"),
    ]);
    lines
}

fn epilogue(base: u64) -> Vec<String> {
    let mut lines = vec![
        traced("sophia_live_output_supervisor schema=1 status=exited peer=4242 code=0 signal=none"),
        traced("sophia_live_output_supervisor schema=1 status=pause_requested peer=4242"),
        traced(
            "sophia_live_output_authority schema=1 status=disconnected epoch=1 preserved_topology=true",
        ),
    ];
    lines.extend(set("peer_exit", 900, 0, 0, base, State::A));
    lines
}

fn validate_log() -> Vec<String> {
    let s = "validate";
    let mut lines = prelude(s, true);
    lines.extend([
        peer(s, "submit", 1004, "layout=b txn=1 intent=validate-only base_topology_epoch=2"),
        peer(s, "submitted", 1005, "ticket=1"),
        traced("sophia_live_output_authority schema=1 status=settled transaction=1 outcome=Validated topology_epoch=2"),
        peer(s, "outcome", 1006, "txn=1 kind=validated reason=0 topology_epoch=2"),
        peer(s, "pass", 1007, ""),
    ]);
    lines.extend(epilogue(2));
    lines
}

fn reject_log() -> Vec<String> {
    let s = "reject";
    let mut lines = prelude(s, false);
    lines.extend([
        peer(s, "reject-candidate", 1004, "head=1 absent_mode=3"),
        peer(s, "submit", 1005, "layout=a-unknown-mode txn=1 intent=apply base_topology_epoch=2"),
        traced("sophia_live_output_authority schema=1 status=rejected transaction=1 phase=admission reason=\"output candidate failed semantic admission\""),
        peer(s, "outcome", 1006, "txn=1 kind=rejected reason=7 topology_epoch=2"),
        peer(s, "pass", 1007, ""),
    ]);
    lines.extend(epilogue(2));
    lines
}

fn commit(txn: u64, base: u64, t: u64) -> Vec<String> {
    let epoch = base + 1;
    let publication = txn + 2;
    vec![
        traced(&format!(
            "sophia_live_output_authority schema=2 status=first_presented transaction={txn} outputs=1 published=false rollback_retained=true"
        )),
        traced(&format!(
            "sophia_live_output_authority schema=1 status=settled transaction={txn} outcome=Committed topology_epoch={epoch}"
        )),
        traced(&format!(
            "sophia_live_output_authority schema=2 status=committed_snapshot_published transaction={publication} topology_epoch={epoch} transport_published=true"
        )),
        traced(&format!(
            "sophia_live_output_authority schema=2 status=committed transaction={txn} topology_epoch={epoch} outputs=1 policy_required=false input=quarantined"
        )),
        peer(
            "commit-restore",
            "outcome",
            t,
            &format!("txn={txn} kind=committed reason=0 topology_epoch={epoch}"),
        ),
    ]
}

fn apply(txn: u64, base: u64, from: State, to: State, t: u64) -> Vec<String> {
    let mut lines = vec![traced(&format!(
        "sophia_live_output_authority schema=2 status=apply_started transaction={txn} heads=1 cards=ordered published=false"
    ))];
    lines.extend(set("before", t, 1, txn, base, from));
    lines.extend(set("applied", t + 1, 1, txn, base, to));
    lines.extend(set("installed", t + 2, 1, txn, base, to));
    lines
}

fn commit_restore_log() -> Vec<String> {
    let s = "commit-restore";
    let mut lines = prelude(s, true);
    lines.push(peer(
        s,
        "submit",
        1004,
        "layout=b txn=1 intent=apply base_topology_epoch=2",
    ));
    lines.extend(apply(1, 2, State::A, State::B, 200));
    lines.extend(set("presented", 203, 1, 1, 2, State::B));
    lines.extend(commit(1, 2, 1005));
    lines.push(peer(
        s,
        "topology",
        1006,
        "topology_epoch=3 qid=10 heads=1 groups=1 match=b",
    ));
    lines.push(peer(
        s,
        "submit",
        1007,
        "layout=a txn=2 intent=apply base_topology_epoch=3",
    ));
    lines.extend(apply(2, 3, State::B, State::A, 300));
    lines.extend(set("presented", 303, 1, 2, 3, State::A));
    lines.extend(commit(2, 3, 1008));
    lines.push(peer(
        s,
        "topology",
        1009,
        "topology_epoch=4 qid=11 heads=1 groups=1 match=a",
    ));
    lines.push(peer(s, "pass", 1010, ""));
    lines.extend(epilogue(4));
    lines
}

fn peer_death_log() -> Vec<String> {
    let s = "apply-await-termination";
    let cause = "reason=\"output peer disconnected at epoch 1\"";
    let mut lines = prelude(s, true);
    lines.push(peer(
        s,
        "submit",
        1004,
        "layout=b txn=1 intent=apply base_topology_epoch=2",
    ));
    lines.push(peer(s, "awaiting-termination", 1005, "txn=1"));
    let mut apply = apply(1, 2, State::A, State::B, 200);
    // The request comes between the applied and installed readback.
    let installed = apply.split_off(apply.len() - 5);
    lines.extend(apply);
    lines.push(traced("sophia_output_peer_loss_proof schema=1 status=termination_requested epoch=1 transaction=1 peer=4242 boundary=all_cards_applied deadline_ms=5000"));
    lines.extend(installed);
    lines.extend([
        // The disconnect is recorded before the exit here; either order passes.
        traced("sophia_live_output_authority schema=1 status=disconnected epoch=1 preserved_topology=true"),
        traced("sophia_live_output_supervisor schema=1 status=exited peer=4242 code=none signal=15"),
        traced("sophia_live_output_supervisor schema=1 status=pause_requested peer=4242"),
        traced(&format!("sophia_live_output_authority schema=2 status=cancellation_observed transaction=1 phase=AwaitingFirstPresentation {cause} published=false")),
    ]);
    lines.extend(set("restored", 240, 1, 1, 2, State::A));
    lines.extend([
        traced(&format!("sophia_live_output_authority schema=2 status=rolled_back transaction=1 card=0 {cause} published=false input=enabled")),
        traced(&format!("sophia_live_output_authority schema=3 status=settled_locally transaction=1 outcome=RolledBack topology_epoch=2 {cause} preserved_topology=true")),
        traced("sophia_output_peer_loss_proof schema=1 status=passed epoch=1 transaction=1 disconnected=true terminated=true restored=true"),
    ]);
    lines.extend(set("peer_exit", 900, 0, 0, 2, State::A));
    lines
}

fn session_argv(stage: &str) -> Vec<String> {
    let (a, b) = layouts();
    let mut argv = vec![
        "{inputs}/bin/sophia".to_owned(),
        "--display=:95".into(),
        "--native-scanout".into(),
        "--session-mode=normal".into(),
        "--max-runtime-ms=60000".into(),
        format!("--desktop-profile={{inputs}}/profiles/{PROFILE}"),
        "--wm-process={inputs}/bin/hagia".into(),
        "--output-process={prep}/peer".into(),
        "--output-proof-readback".into(),
    ];
    if stage == "peer-death" {
        argv.push("--output-proof-peer-loss-after-apply".into());
    }
    for arg in peer_argv(stage, &a, &b).unwrap() {
        argv.push(format!("--output-process-arg={arg}"));
    }
    argv.push("--output-process-arg=--deadline-ms=30000".into());
    argv
}

fn prepared_value() -> Value {
    json!({
        "schema": 1,
        "source": {
            "sophia_commit": pins::SOPHIA_REV,
            "sdk_revision": pinned_sdk(),
            "sdk_manifest_sha256": pins::SDK_MANIFEST_SHA256,
        },
        "peer_sha256": "",
        "harness_sha256": "",
        "session_harness_sha256": "",
        "rust_profile": "release",
        "cargo_jobs": 1,
        "nice": 19,
        "c_flags": "-std=c99 -O2 -Wall -Wextra -Werror -pedantic -UNDEBUG",
        "export_tests": EXPORT_TESTS,
        "session_test": SESSION_TEST,
        "device_hidden": true,
        "session_sockets_hidden": true,
        "network_hidden": true,
        "native_acceptance": false,
        "limits": "supplied physical observations",
    })
}

fn pinned_sdk() -> String {
    let bytes = fs::read(repo().join(pins::SDK_MANIFEST)).unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["revision"]
        .as_str()
        .unwrap()
        .to_owned()
}

struct Fixture {
    dir: Dir,
    inputs: PathBuf,
    inputs_sha256: String,
    count: Cell<usize>,
    logs: Vec<Vec<String>>,
    argv: Vec<Vec<String>>,
    exits: [u64; 4],
    prepared: Value,
    peer_bytes: Vec<u8>,
    tamper_log_after_manifest: bool,
    integration: String,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = Dir::new(&format!("output-file-native-{tag}"));
        let inputs = dir.0.join("inputs");
        for sub in ["bin", "profiles/integration", "nim-deps", "sophia-tree/sub"] {
            fs::create_dir_all(inputs.join(sub)).unwrap();
        }
        fs::write(inputs.join("bin/sophia"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(inputs.join("bin/sophia"), fs::Permissions::from_mode(0o555)).unwrap();
        fs::write(inputs.join("profiles").join(PROFILE), PROFILE_TEXT).unwrap();
        fs::write(inputs.join("sophia-tree/a.txt"), "a\n").unwrap();
        let tree = xtask::git_tree::inventory(&inputs.join("sophia-tree"))
            .unwrap()
            .tree;
        let deps = reviewed_deps("hagia", HAGIA_COMMIT);
        fs::write(inputs.join("nim-deps/hagia.manifest"), &deps).unwrap();
        fs::write(inputs.join("bin/hagia"), "#!/bin/sh\n").unwrap();
        let deps_sha = sha256(deps.as_bytes());
        let header = header_for_tests(
            &integration(),
            &tree,
            &[(
                "hagia",
                HAGIA_COMMIT,
                deps_sha.as_str(),
                NIM_CONFIG_SHA256,
                NIM_STDLIB_SHA256,
            )],
        );
        write_env_for_tests(&inputs, &header).unwrap();
        let manifest = seal(&inputs, header).unwrap();
        fs::write(inputs.join("physical-inputs.manifest"), &manifest).unwrap();
        Self {
            dir,
            inputs,
            inputs_sha256: sha256(manifest.as_bytes()),
            count: Cell::new(0),
            logs: [
                validate_log(),
                reject_log(),
                commit_restore_log(),
                peer_death_log(),
            ]
            .into_iter()
            .map(|mut log| {
                // Session prints running before its owner loop settles
                // startup, and completion after bounded shutdown.
                log.insert(0, RUNNING.to_owned());
                log.push(COMPLETE.to_owned());
                log
            })
            .collect(),
            argv: STAGES.iter().map(|(name, _)| session_argv(name)).collect(),
            exits: [0; 4],
            prepared: prepared_value(),
            peer_bytes: PEER_BYTES.to_vec(),
            tamper_log_after_manifest: false,
            integration: integration(),
        }
    }

    fn log(&mut self, stage: &str) -> &mut Vec<String> {
        let index = STAGES.iter().position(|(name, _)| *name == stage).unwrap();
        &mut self.logs[index]
    }

    /// Write a fresh preparation and run, then verify them.
    fn check(&self) -> Result<Verified, String> {
        let n = self.count.get();
        self.count.set(n + 1);
        let root = self.dir.0.join(format!("attempt-{n}"));
        let prep = root.join("prep");
        let run = root.join("run");
        fs::create_dir_all(&prep).unwrap();
        fs::create_dir_all(&run).unwrap();
        let mut prepared = self.prepared.clone();
        for (file, key, bytes) in [
            ("peer", "peer_sha256", PEER_BYTES),
            ("harness", "harness_sha256", b"harness".as_slice()),
            (
                "session-harness",
                "session_harness_sha256",
                b"session harness".as_slice(),
            ),
        ] {
            if prepared[key] == "" {
                prepared[key] = json!(sha256(bytes));
            }
            let written = if file == "peer" {
                self.peer_bytes.as_slice()
            } else {
                bytes
            };
            fs::write(prep.join(file), written).unwrap();
        }
        let prepared = serde_json::to_vec_pretty(&prepared).unwrap();
        fs::write(prep.join("prepared.json"), &prepared).unwrap();
        let inputs = self.inputs.display().to_string();
        let prep_text = prep.display().to_string();
        let (a, b) = layouts();
        let mut stages = Vec::new();
        for (index, (name, _)) in STAGES.iter().enumerate() {
            let text = format!("{}\n", self.logs[index].join("\n"));
            fs::create_dir_all(run.join(name)).unwrap();
            fs::write(run.join(name).join("session.log"), &text).unwrap();
            stages.push(StageRecord {
                name: (*name).to_owned(),
                log: format!("{name}/session.log"),
                size: text.len() as u64,
                sha256: sha256(text.as_bytes()),
                session_exit: self.exits[index],
                argv: self.argv[index]
                    .iter()
                    .map(|arg| {
                        arg.replace("{inputs}", &inputs)
                            .replace("{prep}", &prep_text)
                    })
                    .collect(),
            });
        }
        let manifest = RunManifest {
            integration_commit: self.integration.clone(),
            inputs_sha256: self.inputs_sha256.clone(),
            preparation_sha256: sha256(&prepared),
            profile: PROFILE.into(),
            a,
            b,
            stages,
        }
        .render()?;
        fs::write(run.join("run.manifest"), &manifest).unwrap();
        if self.tamper_log_after_manifest {
            let path = run.join("validate/session.log");
            let mut text = fs::read_to_string(&path).unwrap();
            text.push('\n');
            fs::write(path, text).unwrap();
        }
        let sealed = verify(&self.inputs, &self.inputs_sha256).unwrap();
        let request = Request {
            inputs: self.inputs.clone(),
            inputs_sha256: self.inputs_sha256.clone(),
            preparation: prep,
            preparation_sha256: sha256(&prepared),
            run,
            run_sha256: sha256(manifest.as_bytes()),
        };
        verify_sealed(&repo(), &sealed, &pinned_sdk(), &request, &integration())
    }

    /// As the runner does after one stage: only that stage's log exists and
    /// no run.manifest has been written.
    fn stage(&self, name: &str) -> Result<(), String> {
        let n = self.count.get();
        self.count.set(n + 1);
        let run = self.dir.0.join(format!("stage-{n}"));
        fs::create_dir_all(run.join(name)).unwrap();
        let index = STAGES.iter().position(|(stage, _)| *stage == name).unwrap();
        let text = format!("{}\n", self.logs[index].join("\n"));
        fs::write(run.join(name).join("session.log"), &text).unwrap();
        let inputs = self.inputs.display().to_string();
        let prep = self.dir.0.join("prep").display().to_string();
        let paths = Paths {
            sophia: format!("{inputs}/bin/sophia"),
            hagia: format!("{inputs}/bin/hagia"),
            profile: format!("{inputs}/profiles/{PROFILE}"),
            peer: format!("{prep}/peer"),
        };
        let record = StageRecord {
            name: name.to_owned(),
            log: format!("{name}/session.log"),
            size: text.len() as u64,
            sha256: sha256(text.as_bytes()),
            session_exit: self.exits[index],
            argv: self.argv[index]
                .iter()
                .map(|arg| arg.replace("{inputs}", &inputs).replace("{prep}", &prep))
                .collect(),
        };
        let (a, b) = layouts();
        verify_stage_record(&record, &a, &b, &paths, &run)
    }
}

fn refused(tag: &str, edit: impl FnOnce(&mut Fixture), expected: &str) {
    let mut fixture = Fixture::new(tag);
    edit(&mut fixture);
    let error = fixture.check().expect_err(tag);
    assert!(error.contains(expected), "{tag}: {error}");
}

fn replace(lines: &mut [String], contains: &str, from: &str, to: &str) {
    let mut hit = false;
    for line in lines.iter_mut().filter(|l| l.contains(contains)) {
        assert!(line.contains(from), "{line} lacks {from}");
        *line = line.replace(from, to);
        hit = true;
    }
    assert!(hit, "no line contains {contains}");
}

fn remove(lines: &mut Vec<String>, contains: &str) {
    let before = lines.len();
    lines.retain(|l| !l.contains(contains));
    assert!(lines.len() < before, "no line contains {contains}");
}

#[test]
fn synthetic_run_passes_and_binds_every_identity() {
    let fixture = Fixture::new("pass");
    let verified = fixture.check().unwrap();
    let bound = &verified.bound;
    assert_eq!(
        verified.stages,
        ["validate", "reject", "commit-restore", "peer-death"]
    );
    assert_eq!(verified.epoch_a, 2);
    assert_eq!(bound.sophia_commit, pins::SOPHIA_REV);
    assert_eq!(bound.sdk_revision, pinned_sdk());
    assert_eq!(bound.hagia_commit, HAGIA_COMMIT);
    assert_eq!(bound.profile, PROFILE);
    assert_eq!(bound.profile_sha256, sha256(PROFILE_TEXT.as_bytes()));
    assert_eq!(bound.peer_sha256, sha256(PEER_BYTES));
    assert_eq!(
        bound.paths.hagia,
        format!("{}/bin/hagia", fixture.inputs.display())
    );
}

/// Positive control: field order within a record is not significant.
#[test]
fn reordered_fields_are_accepted() {
    let mut fixture = Fixture::new("reordered");
    for log in &mut fixture.logs {
        for line in log.iter_mut() {
            let Some(at) = line
                .find("sophia_output_")
                .or_else(|| line.find("sophia_live_output_"))
                .or_else(|| line.find("sophia_live_session "))
            else {
                continue;
            };
            let rest = &line[at..];
            if rest.contains('"') {
                continue;
            }
            let mut tokens = rest.split(' ').collect::<Vec<_>>();
            // Keep the marker and schema first; reverse every field after.
            let tail = tokens.split_off(2);
            tokens.extend(tail.into_iter().rev());
            *line = format!("{}{}", &line[..at], tokens.join(" "));
        }
    }
    assert!(fixture.logs[3].iter().any(|l| l.ends_with("status=passed")));
    fixture.check().unwrap();
}

#[test]
fn run_manifest_round_trips_and_refuses_unknown_profile_owners() {
    let fixture = Fixture::new("manifest");
    let (a, b) = layouts();
    let manifest = RunManifest {
        integration_commit: integration(),
        inputs_sha256: fixture.inputs_sha256.clone(),
        preparation_sha256: "4".repeat(64),
        profile: PROFILE.into(),
        a,
        b,
        stages: STAGES
            .iter()
            .map(|(name, _)| StageRecord {
                name: (*name).to_owned(),
                log: format!("{name}/session.log"),
                size: 1,
                sha256: "5".repeat(64),
                session_exit: 0,
                argv: vec!["/x/bin/sophia".into(), "--value=a b\"c".into()],
            })
            .collect(),
    };
    let text = manifest.render().unwrap();
    assert_eq!(RunManifest::parse(&text).unwrap(), manifest);
    let mut other = manifest.clone();
    other.profile = "product/output.kdl".into();
    assert!(other.render().unwrap_err().contains("owner"));
    let mut escaping = manifest;
    escaping.profile = "integration/../x".into();
    assert!(escaping.render().is_err());
}

#[test]
fn argv_checks_are_shared_with_the_runner() {
    let (a, b) = layouts();
    let paths = Paths {
        sophia: "/in/bin/sophia".into(),
        hagia: "/in/bin/hagia".into(),
        profile: format!("/in/profiles/{PROFILE}"),
        peer: "/prep/peer".into(),
    };
    let argv = |stage: &str| {
        session_argv(stage)
            .into_iter()
            .map(|arg| arg.replace("{inputs}", "/in").replace("{prep}", "/prep"))
            .collect::<Vec<_>>()
    };
    for (stage, _) in STAGES {
        check_argv(stage, &argv(stage), &a, &b, &paths).unwrap();
    }
    let mut twice = argv("validate");
    twice.push("--output-process-arg=--deadline-ms=10".into());
    assert!(
        check_argv("validate", &twice, &a, &b, &paths)
            .unwrap_err()
            .contains("deadline")
    );
    let mut extra = argv("validate");
    extra.push("--output-proof-rollback-after-apply".into());
    assert!(
        check_argv("validate", &extra, &a, &b, &paths)
            .unwrap_err()
            .contains("output control")
    );
    let reject = peer_argv("reject", &a, &b).unwrap();
    assert!(!reject.iter().any(|arg| arg.starts_with("--b-")));
    let mut bad_b = b.clone();
    bad_b.epoch = 3;
    assert!(peer_argv("validate", &a, &bad_b).is_err());
}

#[test]
fn absent_or_different_restoration_is_refused() {
    refused(
        "no-restored",
        |f| remove(f.log("peer-death"), "stage=restored"),
        "restored readback set",
    );
    refused(
        "restored-differs",
        |f| {
            let log = f.log("peer-death");
            let at = log
                .iter()
                .position(|l| l.contains("stage=restored"))
                .unwrap();
            let wrong = set("restored", 240, 1, 1, 2, State::B);
            log.splice(at..at + 5, wrong);
        },
        "restored display differs",
    );
    refused(
        "presented-while-held",
        |f| {
            let log = f.log("peer-death");
            let at = log
                .iter()
                .position(|l| l.contains("status=termination_requested"))
                .unwrap();
            let presented = set("presented", 230, 1, 1, 2, State::B);
            log.splice(at + 6..at + 6, presented);
        },
        "presented",
    );
}

#[test]
fn interrupted_or_unproven_proofs_are_refused() {
    refused(
        "runtime-deadline",
        |f| {
            f.log("peer-death").push(traced(
                "sophia_output_peer_loss_proof schema=1 status=failed reason=session_runtime_deadline restoration=unproven",
            ))
        },
        "failure record",
    );
    refused(
        "no-verdict",
        |f| remove(f.log("peer-death"), "status=passed"),
        "peer-loss verdict",
    );
    refused(
        "two-verdicts",
        |f| {
            let log = f.log("peer-death");
            let verdict = log
                .iter()
                .find(|l| l.contains("status=passed"))
                .unwrap()
                .clone();
            // Inside the Session's lifetime: before its completion record.
            let end = log.len() - 1;
            log.insert(end, verdict);
        },
        "peer-loss verdict",
    );
    refused(
        "verdict-before-restoration",
        |f| {
            let log = f.log("peer-death");
            let at = log
                .iter()
                .position(|l| l.contains("status=passed"))
                .unwrap();
            let verdict = log.remove(at);
            let restored = log
                .iter()
                .position(|l| l.contains("stage=restored"))
                .unwrap();
            log.insert(restored, verdict);
        },
        "out of order",
    );
    refused(
        "peer-pass-in-death",
        |f| {
            let log = f.log("peer-death");
            let end = log.len() - 1;
            log.insert(end, peer("apply-await-termination", "pass", 2000, ""));
        },
        "verdict",
    );
    refused(
        "clean-exit-in-death",
        |f| {
            replace(
                f.log("peer-death"),
                "status=exited",
                "code=none signal=15",
                "code=0 signal=none",
            )
        },
        "TERM or KILL",
    );
    refused(
        "other-transaction-rolled-back",
        |f| {
            replace(
                f.log("peer-death"),
                "status=rolled_back",
                "transaction=1",
                "transaction=2",
            )
        },
        "rollback",
    );
    refused("death-exit", |f| f.exits[3] = 1, "Session exited");
}

#[test]
fn epochs_transactions_and_publications_must_match() {
    refused(
        "validated-epoch",
        |f| {
            replace(
                f.log("validate"),
                "status=settled transaction=1",
                "topology_epoch=2",
                "topology_epoch=3",
            )
        },
        "settle Validated",
    );
    refused(
        "startup-epoch",
        |f| {
            replace(
                f.log("reject"),
                "desktop profile startup",
                "topology_epoch=2",
                "topology_epoch=1",
            )
        },
        "startup",
    );
    refused(
        "owner-rejection",
        |f| {
            replace(
                f.log("reject"),
                "status=rejected",
                "reason=\"output candidate failed semantic admission\"",
                "error=unknown mode for head 1",
            )
        },
        "transport admission refusal",
    );
    refused(
        "reused-qid",
        |f| {
            replace(
                f.log("commit-restore"),
                "topology_epoch=4 qid=11",
                "qid=11",
                "qid=10",
            )
        },
        "Qid reused",
    );
    refused(
        "no-timing-change",
        |f| {
            let log = f.log("commit-restore");
            let at = log
                .iter()
                .position(|l| l.contains("stage=applied") && l.contains("transaction=1"))
                .unwrap();
            log.splice(at..at + 5, set("applied", 201, 1, 1, 2, State::A));
        },
        "mode timing",
    );
    refused(
        "selection-changed",
        |f| {
            replace(
                f.log("commit-restore"),
                "stage=installed t=202 connection_epoch=1 base_topology_epoch=2 transaction=1 head=1 card=0",
                "crtc=41",
                "crtc=51",
            )
        },
        "selections",
    );
    refused(
        "exit-differs-from-baseline",
        |f| {
            let log = f.log("validate");
            let at = log
                .iter()
                .position(|l| l.contains("stage=peer_exit"))
                .unwrap();
            log.splice(at..at + 5, set("peer_exit", 900, 0, 0, 2, State::B));
        },
        "differs from the baseline",
    );
}

#[test]
fn evidence_shapes_are_strict() {
    refused(
        "kms-footer-missing",
        |f| {
            let log = f.log("validate");
            let at = log
                .iter()
                .position(|l| {
                    l.contains("stage=baseline")
                        && l.contains("heads=1 complete=true")
                        && l.contains("kms")
                })
                .unwrap();
            log.remove(at);
        },
        "no KMS row footer",
    );
    refused(
        "kms-footer-count",
        |f| {
            let log = f.log("validate");
            let at = log
                .iter()
                .position(|l| {
                    l.contains("stage=baseline")
                        && l.contains("heads=1 complete=true")
                        && l.contains("kms")
                })
                .unwrap();
            log[at] = log[at].replace("heads=1", "heads=2");
        },
        "do not match its footers",
    );
    refused(
        "owner-extra-field",
        |f| {
            replace(
                f.log("validate"),
                "native_scale=1 scale=1 refresh_millihz=59951",
                "vrr=Disabled",
                "vrr=Disabled extra=1",
            )
        },
        "are not exactly",
    );
    refused(
        "escape",
        |f| {
            replace(
                f.log("validate"),
                "status=connected",
                "epoch=1",
                "epoch=1 \u{1b}[0m",
            )
        },
        "escape",
    );
}

#[test]
fn custody_and_attestation_are_bound() {
    refused(
        "tampered-log",
        |f| f.tamper_log_after_manifest = true,
        "does not match run.manifest",
    );
    refused(
        "peer-hash",
        |f| f.peer_bytes = b"other peer".to_vec(),
        "peer_sha256",
    );
    refused(
        "native-acceptance",
        |f| f.prepared["native_acceptance"] = json!(true),
        "native acceptance",
    );
    refused(
        "export-tests",
        |f| {
            f.prepared["export_tests"].as_array_mut().unwrap().pop();
        },
        "export_tests",
    );
    refused(
        "sdk",
        |f| f.prepared["source"]["sdk_revision"] = json!("9".repeat(40)),
        "sdk_revision",
    );
    refused(
        "program",
        |f| f.argv[0][0] = "/usr/bin/sophia".into(),
        "sealed bin/sophia",
    );
    refused(
        "loss-flag-outside-death",
        |f| f.argv[0].push("--output-proof-peer-loss-after-apply".into()),
        "--output-proof-peer-loss-after-apply",
    );
    refused(
        "peer-argv",
        |f| f.argv[2].retain(|a| !a.starts_with("--output-process-arg=--b-primary")),
        "peer argv",
    );
    refused(
        "integration",
        |f| f.integration = "4".repeat(40),
        "integration",
    );
}

#[test]
fn session_lifecycle_brackets_every_stage() {
    refused(
        "no-completion",
        |f| remove(f.log("peer-death"), "status=bounded_complete"),
        "Session completion record",
    );
    refused(
        "two-running",
        |f| {
            let log = f.log("validate");
            log.insert(1, RUNNING.to_owned());
        },
        "Session running record",
    );
    refused(
        "completion-display",
        |f| {
            replace(
                f.log("reject"),
                "status=bounded_complete",
                "display=:95",
                "display=:96",
            )
        },
        "display and controls",
    );
    refused(
        "running-without-physical-input",
        |f| {
            replace(
                f.log("reject"),
                "status=running",
                "physical_input=enabled",
                "physical_input=disabled",
            )
        },
        "display and controls",
    );
    refused(
        "cleanup-pending",
        |f| {
            replace(
                f.log("commit-restore"),
                "status=bounded_complete",
                "native_cleanup_pending=false",
                "native_cleanup_pending=true",
            )
        },
        "cleanup pending",
    );
    refused(
        "in-flight-not-boolean",
        |f| {
            replace(
                f.log("validate"),
                "status=bounded_complete",
                "native_in_flight=false",
                "native_in_flight=0",
            )
        },
        "boolean",
    );
    refused(
        "proof-schema",
        |f| {
            replace(
                f.log("validate"),
                "status=bounded_complete",
                "schema=19",
                "schema=18",
            )
        },
        "ordinary native schema",
    );
    refused(
        "completion-before-work",
        |f| {
            let log = f.log("validate");
            let complete = log.pop().unwrap();
            let at = log.iter().position(|l| l.contains("event=pass")).unwrap();
            log.insert(at, complete);
        },
        "bracket",
    );
    refused(
        "running-after-startup",
        |f| {
            let log = f.log("peer-death");
            let running = log.remove(0);
            let at = log
                .iter()
                .rposition(|l| l.contains("stage=baseline"))
                .unwrap();
            log.insert(at + 1, running);
        },
        "bracket",
    );
    refused(
        "no-display",
        |f| f.argv[1].retain(|a| !a.starts_with("--display=")),
        "--display",
    );
    refused(
        "daily-display",
        |f| {
            for arg in f.argv[0]
                .iter_mut()
                .filter(|a| a.as_str() == "--display=:95")
            {
                *arg = "--display=:77".into();
            }
        },
        "private display",
    );
}

/// The runner verifies each stage before launching the next: a stage whose
/// peer or proof failed stops the run even though Session exited 0.
#[test]
fn each_stage_verifies_alone_and_a_failed_first_stage_stops_the_run() {
    let fixture = Fixture::new("per-stage");
    for (name, _) in STAGES {
        fixture
            .stage(name)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
    let mut failed = Fixture::new("per-stage-peer-failed");
    let log = failed.log("validate");
    let at = log.iter().position(|l| l.contains("event=pass")).unwrap();
    log[at] = peer("validate", "fail", 1007, "exit=4 reason=unexpected-outcome");
    let error = failed.stage("validate").unwrap_err();
    assert!(error.starts_with("validate log:"), "{error}");
    let mut proof = Fixture::new("per-stage-proof-failed");
    proof.log("validate").push(traced(
        "sophia_output_readback_proof schema=1 status=failed transaction=1 reason=readback",
    ));
    assert!(
        proof
            .stage("validate")
            .unwrap_err()
            .contains("failure record")
    );
    let mut exited = Fixture::new("per-stage-exit");
    exited.exits[0] = 1;
    assert!(
        exited
            .stage("validate")
            .unwrap_err()
            .contains("Session exited")
    );
    let mut argv = Fixture::new("per-stage-argv");
    argv.argv[0].push("--output-proof-peer-loss-after-apply".into());
    assert!(
        argv.stage("validate")
            .unwrap_err()
            .starts_with("validate argv:")
    );
}

/// Preparation records its actual jobs and nice value. Historical
/// preparations (1 at 19, the fixture's default) and current ones (the
/// caller's parallelism at its priority) verify; values no preparation can
/// record do not.
#[test]
fn preparation_jobs_and_nice_are_recorded_values() {
    for (tag, jobs, nice) in [("parallel", 16, 0), ("negative", 2, -5), ("bounds", 1, -20)] {
        let mut fixture = Fixture::new(tag);
        fixture.prepared["cargo_jobs"] = json!(jobs);
        fixture.prepared["nice"] = json!(nice);
        fixture.check().unwrap();
    }
    for (tag, key, value, expected) in [
        ("zero-jobs", "cargo_jobs", json!(0), "cargo_jobs"),
        ("negative-jobs", "cargo_jobs", json!(-1), "cargo_jobs"),
        ("fractional-jobs", "cargo_jobs", json!(1.5), "cargo_jobs"),
        ("text-jobs", "cargo_jobs", json!("8"), "cargo_jobs"),
        ("nice-high", "nice", json!(20), "nice"),
        ("nice-low", "nice", json!(-21), "nice"),
        ("text-nice", "nice", json!("19"), "nice"),
        ("null-nice", "nice", Value::Null, "nice"),
    ] {
        refused(tag, |f| f.prepared[key] = value, expected);
    }
}
