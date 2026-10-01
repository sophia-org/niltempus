//! Custody: the integration commit, the sealed inputs and staged SDK, the
//! Sophia preparation, the declared layouts, the Session argv and the run
//! manifest. The runner uses the same checks before and after its launches.
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use super::{
    EXPORT_TESTS, MAX_LOG_BYTES, MAX_MANIFEST_BYTES, MAX_PEER_DEADLINE_MS, MAX_PREPARED_BYTES,
    PREPARED, PROFILE_OWNERS, RUN_MANIFEST, SESSION_TEST, STAGES, path_text, real_dir, regular,
    unsigned,
};
use crate::physical_inputs::{self, Sealed};
use crate::records::{self, Record};
use crate::{c_sdk_pin, hex, pins, read, sha256};

/// The canonical absolute paths a Session argv must name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub sophia: String,
    pub hagia: String,
    pub profile: String,
    pub peer: String,
}

/// Inputs and preparation bound to one integration commit, before any
/// physical launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub integration_commit: String,
    pub inputs_sha256: String,
    pub preparation_sha256: String,
    pub sophia_commit: String,
    pub sdk_revision: String,
    pub hagia_commit: String,
    /// `OWNER/PATH` under the inputs' `profiles/`.
    pub profile: String,
    pub profile_sha256: String,
    pub peer_sha256: String,
    pub paths: Paths,
}

fn git(repo: &Path, args: &[&str]) -> Result<(bool, String), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git {args:?}: {e}"))?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    ))
}

/// This checkout, clean, at a signed HEAD.
pub fn integration_commit(repo: &Path) -> Result<String, String> {
    let (ok, status) = git(repo, &["status", "--porcelain", "--untracked-files=all"])?;
    if !ok || !status.is_empty() {
        return Err("the integration checkout must be clean".into());
    }
    let (ok, head) = git(repo, &["rev-parse", "HEAD"])?;
    if !ok || !hex(&head, 40) {
        return Err("cannot resolve the integration HEAD".into());
    }
    if !git(repo, &["verify-commit", &head])?.0 {
        return Err(format!(
            "integration HEAD {head} is not a verified signed commit"
        ));
    }
    Ok(head)
}

/// Bind sealed inputs, the staged SDK and a preparation to `integration`,
/// resolving the profile and the paths a Session argv must use.
pub fn bind(
    repo: &Path,
    inputs: &Path,
    inputs_sha256: &str,
    preparation: &Path,
    preparation_sha256: &str,
    profile: &str,
    integration: &str,
) -> Result<Bound, String> {
    let sealed = physical_inputs::verify(inputs, inputs_sha256)?;
    let sdk = c_sdk_pin::verify(
        &sealed
            .out
            .join(physical_inputs::SOPHIA_TREE)
            .join("vendor/c-desktop-sdk"),
        repo,
    )?;
    bind_sealed(
        repo,
        &sealed,
        &sdk,
        preparation,
        preparation_sha256,
        profile,
        integration,
    )
}

/// [`bind`] once the sealed inputs and the staged SDK revision are known.
/// Exposed so tests can supply synthetic sealed inputs.
pub fn bind_sealed(
    repo: &Path,
    sealed: &Sealed,
    sdk_revision: &str,
    preparation: &Path,
    preparation_sha256: &str,
    profile: &str,
    integration: &str,
) -> Result<Bound, String> {
    let header = &sealed.records;
    if header_field(header, "integration", "commit") != Some(integration) {
        return Err("the physical inputs were not prepared from this integration commit".into());
    }
    let sophia_commit = header_field(header, "sophia", "commit").ok_or("no sophia record")?;
    if sophia_commit != pins::SOPHIA_REV || sealed.sophia_commit != pins::SOPHIA_REV {
        return Err("the physical inputs are not the pinned Sophia revision".into());
    }
    if header_field(header, "sophia", "features") != Some("native-session") {
        return Err("the physical inputs were not built with native-session".into());
    }
    if pinned_sdk_revision(repo)? != sdk_revision {
        return Err("the staged C SDK is not the pinned revision".into());
    }
    let hagia_commit = header
        .iter()
        .find(|r| {
            r.kind == "product"
                && r.fields
                    .first()
                    .is_some_and(|(k, v)| k == "name" && v == "hagia")
        })
        .and_then(|r| r.fields.iter().find(|(k, _)| k == "commit"))
        .map(|(_, v)| v.clone())
        .ok_or("the physical inputs carry no Hagia product")?;
    let files = sealed_files(sealed)?;
    for binary in ["bin/sophia", "bin/hagia"] {
        sealed_file(&files, binary)?;
    }
    check_profile(profile)?;
    let profile_sha256 = sealed_file(&files, &format!("profiles/{profile}"))?.to_owned();
    let peer_sha256 = verify_preparation(preparation, preparation_sha256, sdk_revision)?;
    Ok(Bound {
        integration_commit: integration.to_owned(),
        inputs_sha256: sealed.sha256.clone(),
        preparation_sha256: preparation_sha256.to_owned(),
        sophia_commit: sophia_commit.to_owned(),
        sdk_revision: sdk_revision.to_owned(),
        hagia_commit,
        profile: profile.to_owned(),
        profile_sha256,
        peer_sha256,
        paths: Paths {
            sophia: path_text(&sealed.out.join("bin/sophia"))?,
            hagia: path_text(&sealed.out.join("bin/hagia"))?,
            profile: path_text(&sealed.out.join("profiles").join(profile))?,
            peer: path_text(&preparation.join("peer"))?,
        },
    })
}

fn header_field<'r>(records: &'r [Record], kind: &str, key: &str) -> Option<&'r str> {
    records
        .iter()
        .find(|r| r.kind == kind)?
        .fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The manifest's `file` records. [`Sealed`] keeps only the header;
/// [`physical_inputs::verify`] has already re-derived every file record, so
/// this re-reads that manifest bound to the same digest.
fn sealed_files(sealed: &Sealed) -> Result<Vec<Record>, String> {
    let bytes = read(&sealed.out.join(physical_inputs::MANIFEST))?;
    if sha256(&bytes) != sealed.sha256 {
        return Err("the physical inputs manifest changed after verification".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "the inputs manifest is not UTF-8")?;
    Ok(records::parse_text(&text)?
        .into_iter()
        .filter(|r| r.kind == "file")
        .collect())
}

/// The sealed sha256 of one output file.
fn sealed_file<'r>(records: &'r [Record], path: &str) -> Result<&'r str, String> {
    records
        .iter()
        .filter(|r| r.kind == "file")
        .find(|r| {
            r.fields
                .first()
                .is_some_and(|(k, v)| k == "path" && v == path)
        })
        .and_then(|r| r.fields.iter().find(|(k, _)| k == "sha256"))
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| format!("the physical inputs do not seal {path}"))
}

fn check_profile(profile: &str) -> Result<(), String> {
    records::relative_path(profile)?;
    match profile.split_once('/') {
        Some((owner, _)) if PROFILE_OWNERS.contains(&owner) => Ok(()),
        _ => Err(format!(
            "profile {profile:?} is not OWNER/PATH with owner in {PROFILE_OWNERS:?}"
        )),
    }
}

fn pinned_sdk_revision(repo: &Path) -> Result<String, String> {
    let bytes = read(&repo.join(pins::SDK_MANIFEST))?;
    if sha256(&bytes) != pins::SDK_MANIFEST_SHA256 {
        return Err(format!(
            "{} differs from its pinned digest",
            pins::SDK_MANIFEST
        ));
    }
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    manifest["revision"]
        .as_str()
        .filter(|r| hex(r, 40))
        .map(str::to_owned)
        .ok_or_else(|| "the pinned SDK manifest has no revision".into())
}

// ---- preparation ----

/// Sophia's `prepared.json`: its digest, its exact schema, its source bound
/// to the pinned Sophia and SDK, its device-free test set, and every
/// artifact digest recomputed. Returns the peer's sha256.
fn verify_preparation(dir: &Path, expected: &str, sdk_revision: &str) -> Result<String, String> {
    real_dir(dir)?;
    if !hex(expected, 64) {
        return Err("the preparation sha256 must be 64 lowercase hex".into());
    }
    let bytes = regular(&dir.join(PREPARED), MAX_PREPARED_BYTES)?;
    if sha256(&bytes) != expected {
        return Err("prepared.json does not match its expected sha256".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("prepared.json: {e}"))?;
    let object = value.as_object().ok_or("prepared.json is not an object")?;
    let keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_keys = BTreeSet::from([
        "schema",
        "source",
        "peer_sha256",
        "harness_sha256",
        "session_harness_sha256",
        "rust_profile",
        "cargo_jobs",
        "nice",
        "c_flags",
        "export_tests",
        "session_test",
        "device_hidden",
        "session_sockets_hidden",
        "network_hidden",
        "native_acceptance",
        "limits",
    ]);
    if keys != expected_keys {
        return Err(format!("prepared.json has fields {keys:?}"));
    }
    let source = value["source"]
        .as_object()
        .ok_or("prepared.json source is not an object")?;
    if source.keys().map(String::as_str).collect::<BTreeSet<_>>()
        != BTreeSet::from(["sophia_commit", "sdk_revision", "sdk_manifest_sha256"])
    {
        return Err("prepared.json source has unexpected fields".into());
    }
    let checks = [
        (value["schema"] == 1, "schema is not 1"),
        (
            value["source"]["sophia_commit"] == pins::SOPHIA_REV,
            "source.sophia_commit is not the pinned Sophia revision",
        ),
        (
            value["source"]["sdk_revision"] == sdk_revision,
            "source.sdk_revision is not the pinned SDK",
        ),
        (
            value["source"]["sdk_manifest_sha256"] == pins::SDK_MANIFEST_SHA256,
            "source.sdk_manifest_sha256 is not the pinned SDK manifest",
        ),
        (
            value["rust_profile"] == "release",
            "rust_profile is not release",
        ),
        // What preparation recorded: 1 and 19 in preparations before Sophia
        // builds ran at the caller's priority and parallelism.
        (
            value["cargo_jobs"].as_u64().is_some_and(|jobs| jobs > 0),
            "cargo_jobs is not a positive integer",
        ),
        (
            value["nice"]
                .as_i64()
                .is_some_and(|nice| (-20..=19).contains(&nice)),
            "nice is not a nice value",
        ),
        (value["c_flags"].is_string(), "c_flags is not a string"),
        (value["limits"].is_string(), "limits is not a string"),
        (value["device_hidden"] == true, "devices were not hidden"),
        (
            value["session_sockets_hidden"] == true,
            "session sockets were not hidden",
        ),
        (value["network_hidden"] == true, "network was not hidden"),
        (
            value["native_acceptance"] == false,
            "a device-free preparation cannot claim native acceptance",
        ),
        (
            value["session_test"] == SESSION_TEST,
            "session_test is not the Session fixture",
        ),
    ];
    if let Some((_, error)) = checks.iter().find(|(ok, _)| !ok) {
        return Err(format!("prepared.json: {error}"));
    }
    let tests = value["export_tests"]
        .as_array()
        .ok_or("export_tests is not a list")?
        .iter()
        .map(|t| t.as_str().ok_or("export_tests holds a non-string"))
        .collect::<Result<Vec<_>, _>>()?;
    if tests.len() != EXPORT_TESTS.len()
        || tests.iter().copied().collect::<BTreeSet<_>>()
            != EXPORT_TESTS.into_iter().collect::<BTreeSet<_>>()
    {
        return Err("prepared.json export_tests is not the exact device-free set".into());
    }
    let mut peer_sha256 = String::new();
    for (file, key) in [
        ("peer", "peer_sha256"),
        ("harness", "harness_sha256"),
        ("session-harness", "session_harness_sha256"),
    ] {
        let recorded = value[key]
            .as_str()
            .filter(|d| hex(d, 64))
            .ok_or_else(|| format!("prepared.json {key} is not a sha256"))?;
        let actual = sha256(&regular(&dir.join(file), u64::MAX)?);
        if actual != recorded {
            return Err(format!("prepared {file} does not match {key}"));
        }
        if file == "peer" {
            peer_sha256 = actual;
        }
    }
    Ok(peer_sha256)
}

// ---- layouts, argv and the run manifest ----

/// One declared layout, as the peer's compact arguments. Only layout a has
/// a topology epoch: the committed profile startup epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub epoch: u64,
    pub heads: String,
    pub groups: String,
    pub primary: String,
}

impl Layout {
    fn check(&self, name: &str) -> Result<(), String> {
        for (what, value) in [("heads", &self.heads), ("groups", &self.groups)] {
            if value.is_empty()
                || value.len() > 4096
                || value.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(format!(
                    "layout {name} {what} is empty, too long or has whitespace"
                ));
            }
        }
        if unsigned(&self.primary, "layout primary")? >= 16 {
            return Err(format!("layout {name} primary is out of range"));
        }
        if (name == "a") != (self.epoch != 0) {
            return Err(format!(
                "layout {name}: only layout a has a nonzero topology epoch"
            ));
        }
        Ok(())
    }
}

/// The peer's argv for a stage (without the optional `--deadline-ms`).
pub fn peer_argv(stage: &str, a: &Layout, b: &Layout) -> Result<Vec<String>, String> {
    let peer_stage = STAGES
        .iter()
        .find(|(name, _)| *name == stage)
        .map(|(_, peer)| *peer)
        .ok_or_else(|| format!("unknown stage {stage:?}"))?;
    a.check("a")?;
    b.check("b")?;
    let mut args = vec![
        format!("--stage={peer_stage}"),
        format!("--a-topology-epoch={}", a.epoch),
        format!("--a-heads={}", a.heads),
        format!("--a-groups={}", a.groups),
        format!("--a-primary={}", a.primary),
    ];
    if stage != "reject" {
        args.extend([
            format!("--b-heads={}", b.heads),
            format!("--b-groups={}", b.groups),
            format!("--b-primary={}", b.primary),
        ]);
    }
    Ok(args)
}

/// A Session argv for `stage`: the sealed program, WM, profile and peer;
/// native normal mode, a positive runtime bound and the readback control,
/// each exactly once; the peer-loss control only for peer death; no other
/// output control; the peer argv exactly [`peer_argv`], plus at most one
/// `--deadline-ms`; exactly one private `--display` (:90..:99). Other
/// Session arguments (seat, config) are the runner's.
pub fn check_argv(
    stage: &str,
    argv: &[String],
    a: &Layout,
    b: &Layout,
    paths: &Paths,
) -> Result<(), String> {
    let expected_peer = peer_argv(stage, a, b)?;
    let (program, args) = argv.split_first().ok_or("empty argv")?;
    if *program != paths.sophia {
        return Err(format!("program {program:?} is not the sealed bin/sophia"));
    }
    let count = |flag: &str| args.iter().filter(|a| *a == flag).count();
    let values = |key: &str| {
        let prefix = format!("{key}=");
        args.iter()
            .filter_map(|a| a.strip_prefix(&prefix).map(str::to_owned))
            .collect::<Vec<_>>()
    };
    let one = |key: &str| -> Result<String, String> {
        match (values(key).as_slice(), count(key)) {
            ([value], 0) => Ok(value.clone()),
            _ => Err(format!("{key}= must appear exactly once")),
        }
    };
    let death = stage == "peer-death";
    for (flag, wanted) in [
        ("--native-scanout", 1),
        ("--output-proof-readback", 1),
        ("--output-proof-peer-loss-after-apply", usize::from(death)),
    ] {
        if count(flag) != wanted || !values(flag).is_empty() {
            return Err(format!("{flag} must appear {wanted} time(s), bare"));
        }
    }
    if one("--session-mode")? != "normal" {
        return Err("--session-mode must be normal".into());
    }
    // Session defaults to the daily :77 without --display; attended proofs
    // name one private display, which its lifecycle records must repeat.
    let display = one("--display")?;
    if !display
        .strip_prefix(':')
        .and_then(|n| unsigned(n, "--display").ok())
        .is_some_and(|n| (90..=99).contains(&n))
    {
        return Err("--display must be one private display :90..:99".into());
    }
    if unsigned(&one("--max-runtime-ms")?, "--max-runtime-ms")? == 0 {
        return Err("--max-runtime-ms must be positive".into());
    }
    for (key, expected) in [
        ("--desktop-profile", &paths.profile),
        ("--wm-process", &paths.hagia),
        ("--output-process", &paths.peer),
    ] {
        if &one(key)? != expected {
            return Err(format!("{key} is not the sealed or prepared path"));
        }
    }
    const KNOWN: [&str; 2] = [
        "--output-proof-readback",
        "--output-proof-peer-loss-after-apply",
    ];
    if let Some(other) = args.iter().find(|a| {
        (a.starts_with("--output-proof") && !KNOWN.contains(&a.as_str()))
            || (a.starts_with("--output-process")
                && !a.starts_with("--output-process=")
                && !a.starts_with("--output-process-arg="))
    }) {
        return Err(format!("unexpected output control {other:?}"));
    }
    let mut peer = values("--output-process-arg");
    let deadlines = peer
        .iter()
        .filter(|a| a.starts_with("--deadline-ms="))
        .cloned()
        .collect::<Vec<_>>();
    if deadlines.len() > 1 {
        return Err("the peer takes at most one --deadline-ms".into());
    }
    if let Some(deadline) = deadlines.first() {
        let ms = unsigned(&deadline["--deadline-ms=".len()..], "--deadline-ms")?;
        if ms == 0 || ms > MAX_PEER_DEADLINE_MS {
            return Err("peer --deadline-ms must be 1..120000".into());
        }
        peer.retain(|a| a != deadline);
    }
    if peer != expected_peer {
        return Err(format!("peer argv {peer:?} is not {expected_peer:?}"));
    }
    Ok(())
}

/// One attested Session run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageRecord {
    pub name: String,
    /// `NAME/session.log` under the run directory.
    pub log: String,
    pub size: u64,
    pub sha256: String,
    pub session_exit: u64,
    pub argv: Vec<String>,
}

/// `run.manifest`, in the records encoding: one `output-file-native-run`
/// header, layouts a and b, the four stages in order, then each stage's
/// argv in index order. [`RunManifest::render`] is the runner's writer and
/// [`RunManifest::read`] the verifier's reader; both refuse the same shapes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunManifest {
    pub integration_commit: String,
    pub inputs_sha256: String,
    pub preparation_sha256: String,
    pub profile: String,
    pub a: Layout,
    pub b: Layout,
    pub stages: Vec<StageRecord>,
}

impl RunManifest {
    fn check(&self) -> Result<(), String> {
        if !hex(&self.integration_commit, 40)
            || !hex(&self.inputs_sha256, 64)
            || !hex(&self.preparation_sha256, 64)
        {
            return Err("run.manifest header is malformed".into());
        }
        check_profile(&self.profile)?;
        self.a.check("a")?;
        self.b.check("b")?;
        if self.stages.len() != STAGES.len() {
            return Err("run.manifest must record all four stages".into());
        }
        for (stage, (name, _)) in self.stages.iter().zip(STAGES) {
            if stage.name != name
                || stage.log != format!("{name}/session.log")
                || !hex(&stage.sha256, 64)
                || stage.argv.is_empty()
            {
                return Err(format!("run.manifest stage {name} is malformed"));
            }
        }
        Ok(())
    }

    pub fn render(&self) -> Result<String, String> {
        self.check()?;
        let mut out = vec![
            Record::of("output-file-native-run")
                .with("schema", "1")
                .with("integration_commit", &self.integration_commit)
                .with("inputs_manifest_sha256", &self.inputs_sha256)
                .with("preparation_sha256", &self.preparation_sha256)
                .with("profile", &self.profile),
            Record::of("layout")
                .with("name", "a")
                .with("topology_epoch", self.a.epoch.to_string())
                .with("heads", &self.a.heads)
                .with("groups", &self.a.groups)
                .with("primary", &self.a.primary),
            Record::of("layout")
                .with("name", "b")
                .with("heads", &self.b.heads)
                .with("groups", &self.b.groups)
                .with("primary", &self.b.primary),
        ];
        for (order, stage) in self.stages.iter().enumerate() {
            out.push(
                Record::of("stage")
                    .with("name", &stage.name)
                    .with("order", (order + 1).to_string())
                    .with("log", &stage.log)
                    .with("size", stage.size.to_string())
                    .with("sha256", &stage.sha256)
                    .with("session_exit", stage.session_exit.to_string()),
            );
        }
        for stage in &self.stages {
            for (index, value) in stage.argv.iter().enumerate() {
                out.push(
                    Record::of("argv")
                        .with("stage", &stage.name)
                        .with("index", index.to_string())
                        .with("value", value),
                );
            }
        }
        records::render(&out)
    }

    /// `RUN/run.manifest`, bound to `expected` before it is parsed.
    pub fn read(run: &Path, expected: &str) -> Result<Self, String> {
        real_dir(run)?;
        let bytes = regular(&run.join(RUN_MANIFEST), MAX_MANIFEST_BYTES)?;
        if !hex(expected, 64) || sha256(&bytes) != expected {
            return Err("run.manifest does not match --run-manifest-sha256".into());
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| "run.manifest is not UTF-8")?;
        Self::parse(text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let parsed = records::parse_text(text)?;
        let mut records = parsed.iter();
        let mut next = |kind: &str| {
            records
                .next()
                .filter(|r| r.kind == kind)
                .ok_or_else(|| format!("run.manifest: expected a {kind} record"))
        };
        let header = next("output-file-native-run")?.expect(&[
            "schema",
            "integration_commit",
            "inputs_manifest_sha256",
            "preparation_sha256",
            "profile",
        ])?;
        if header[0] != "1" {
            return Err("run.manifest schema is not 1".into());
        }
        let a =
            next("layout")?.expect(&["name", "topology_epoch", "heads", "groups", "primary"])?;
        let b = next("layout")?.expect(&["name", "heads", "groups", "primary"])?;
        if a[0] != "a" || b[0] != "b" {
            return Err("run.manifest layouts must be a then b".into());
        }
        let mut stages = Vec::new();
        for (order, (name, _)) in STAGES.iter().enumerate() {
            let stage = next("stage")?.expect(&[
                "name",
                "order",
                "log",
                "size",
                "sha256",
                "session_exit",
            ])?;
            if stage[0] != *name || stage[1] != (order + 1).to_string() {
                return Err(format!("run.manifest stage {} must be {name}", order + 1));
            }
            stages.push(StageRecord {
                name: (*name).to_owned(),
                log: stage[2].to_owned(),
                size: unsigned(stage[3], "stage size")?,
                sha256: stage[4].to_owned(),
                session_exit: unsigned(stage[5], "session_exit")?,
                argv: Vec::new(),
            });
        }
        let mut current = 0;
        for record in records {
            if record.kind != "argv" {
                return Err(format!("run.manifest: unexpected {} record", record.kind));
            }
            let argv = record.expect(&["stage", "index", "value"])?;
            while current < stages.len() && stages[current].name != argv[0] {
                current += 1;
            }
            let stage = stages
                .get_mut(current)
                .ok_or("run.manifest argv records are not grouped in stage order")?;
            if unsigned(argv[1], "argv index")? != stage.argv.len() as u64 {
                return Err(format!(
                    "run.manifest {} argv indices are not consecutive",
                    stage.name
                ));
            }
            stage.argv.push(argv[2].to_owned());
        }
        let manifest = Self {
            integration_commit: header[1].to_owned(),
            inputs_sha256: header[2].to_owned(),
            preparation_sha256: header[3].to_owned(),
            profile: header[4].to_owned(),
            a: Layout {
                epoch: unsigned(a[1], "layout a topology_epoch")?,
                heads: a[2].to_owned(),
                groups: a[3].to_owned(),
                primary: a[4].to_owned(),
            },
            b: Layout {
                epoch: 0,
                heads: b[1].to_owned(),
                groups: b[2].to_owned(),
                primary: b[3].to_owned(),
            },
            stages,
        };
        manifest.check()?;
        Ok(manifest)
    }
}

pub(super) fn read_log(run: &Path, stage: &StageRecord) -> Result<String, String> {
    let bytes = regular(&run.join(&stage.log), MAX_LOG_BYTES)?;
    if bytes.len() as u64 != stage.size || sha256(&bytes) != stage.sha256 {
        return Err(format!("{} does not match run.manifest", stage.log));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8", stage.log))
}
