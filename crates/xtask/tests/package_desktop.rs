//! `cargo xtask package-desktop`: every refusal happens before anything is
//! staged or built, and the assembled release has exactly the schema-6
//! layout, manifest and SHA256SUMS (assembled from fixture inputs; the real
//! builds belong to the E4 slot run).
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use xtask::package_desktop::{
    COMMANDS, OPERATIONS_DOC, SESSIONS, SOPHIA_RETAINED, StagedTree, TOOLS, assemble, manifest,
    release_id, run, run_with,
};
use xtask::pins::{SOPHIA_REV, SOPHIA_URL};

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, repo, sha256, write_pair};

struct Inputs {
    dir: Dir,
    commits: String,
    digests: String,
    profile: String,
}

fn inputs(tag: &str) -> Inputs {
    let dir = Dir::new(tag);
    let ids = write_pair(&dir.0.join("wm-pair"));
    fs::create_dir(dir.0.join("sophia")).unwrap();
    fs::create_dir(dir.0.join("build")).unwrap();
    fs::set_permissions(dir.0.join("build"), fs::Permissions::from_mode(0o700)).unwrap();
    Inputs {
        commits: ids.commits.join(","),
        digests: ids.digests.join(","),
        profile: ids.profile.clone(),
        dir,
    }
}

const HOME: &str = "/fixture/cargo-home";

/// A stand-in integration checkout whose provisioning marker is valid for
/// its own Cargo.lock and HOME (the real one is used where no later step is
/// reached).
fn provisioned_repo(dir: &Path) -> std::path::PathBuf {
    let repo = dir.join("integration");
    fs::create_dir_all(repo.join(".provision")).unwrap();
    fs::write(repo.join("Cargo.lock"), "# fixture lock\n").unwrap();
    marker(&repo, SOPHIA_REV, &sha256(b"# fixture lock\n"), HOME);
    repo
}

fn marker(repo: &Path, rev: &str, lock: &str, home: &str) {
    fs::write(
        repo.join(".provision/accepted"),
        format!("url={SOPHIA_URL}\nrev={rev}\ncargo_lock_sha256={lock}\ncargo_home={home}\n"),
    )
    .unwrap();
}

impl Inputs {
    fn args(&self, changes: &[(&str, &str)]) -> Vec<String> {
        let d = &self.dir.0;
        let mut values = vec![
            ("sophia-root", d.join("sophia").display().to_string()),
            ("sophia-rev", SOPHIA_REV.to_owned()),
            ("wm-pair", d.join("wm-pair").display().to_string()),
            ("wm-pair-commits", self.commits.clone()),
            ("wm-pair-sha256", self.digests.clone()),
            ("wm-pair-profile-sha256", self.profile.clone()),
            ("wm-pair-c-sdk-rev", fixture::HAGIA_C_SDK_REV.to_owned()),
            ("build-dir", d.join("build").display().to_string()),
            ("out", d.join("release").display().to_string()),
        ];
        for (key, value) in changes {
            match values.iter_mut().find(|(k, _)| k == key) {
                Some(entry) => entry.1 = (*value).to_owned(),
                None => values.push((key, (*value).to_owned())),
            }
        }
        values
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("--{k}={v}"))
            .collect()
    }

    fn refused(&self, changes: &[(&str, &str)], message: &str) {
        self.refused_with(&repo(), None, changes, message);
    }

    fn refused_with(
        &self,
        integration: &Path,
        home: Option<&str>,
        changes: &[(&str, &str)],
        message: &str,
    ) {
        let error = run_with(integration, &self.args(changes), home).unwrap_err();
        assert!(error.contains(message), "{changes:?}: {error}");
        assert!(!self.dir.0.join("release").exists(), "{changes:?}");
        assert_eq!(
            fs::read_dir(self.dir.0.join("build")).unwrap().count(),
            0,
            "{changes:?} wrote into the build directory"
        );
    }
}

#[test]
fn options_are_exact_and_explicit() {
    let f = inputs("options");
    assert!(run(&repo(), &[]).unwrap_err().contains("is required"));
    f.refused(&[("out", "")], "--out is required");
    f.refused(&[("extra", "1")], "unknown option --extra");
    let mut repeated = f.args(&[]);
    repeated.push(repeated[0].clone());
    assert!(run(&repo(), &repeated).unwrap_err().contains("repeated"));
}

#[test]
fn a_relative_or_sibling_root_and_relative_paths_are_refused() {
    let f = inputs("relative");
    f.refused(
        &[("sophia-root", "../sophia")],
        "--sophia-root must be absolute",
    );
    f.refused(
        &[("sophia-root", "sophia")],
        "--sophia-root must be absolute",
    );
    f.refused(&[("wm-pair", "wm-pair")], "--wm-pair must be absolute");
    f.refused(&[("build-dir", "build")], "--build-dir must be absolute");
    f.refused(&[("out", "release")], "--out must be absolute");
}

#[test]
fn an_unpinned_revision_is_refused() {
    let f = inputs("unpinned");
    f.refused(
        &[("sophia-rev", &"0".repeat(40))],
        "is not the pinned revision",
    );
    f.refused(&[("sophia-rev", "HEAD")], "is not the pinned revision");
}

#[test]
fn build_and_output_directories_stay_outside_sources_and_private() {
    let f = inputs("dirs");
    let inside_repo = repo().join("target/package-desktop-fixture");
    f.refused(
        &[("build-dir", inside_repo.to_str().unwrap())],
        "--build-dir must be outside every source tree",
    );
    let inside_sophia = f.dir.0.join("sophia/release");
    f.refused(
        &[("out", inside_sophia.to_str().unwrap())],
        "--out must be outside every source tree",
    );
    fs::set_permissions(f.dir.0.join("build"), fs::Permissions::from_mode(0o755)).unwrap();
    f.refused(&[], "--build-dir must be a private (0700) directory");
    fs::set_permissions(f.dir.0.join("build"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(f.dir.0.join("release")).unwrap();
    let error = run(&repo(), &f.args(&[])).unwrap_err();
    assert!(error.contains("--out already exists"), "{error}");
}

#[test]
fn a_pair_or_digest_mismatch_is_refused() {
    let f = inputs("pair");
    let (hagia, narthex) = f.commits.split_once(',').unwrap();
    let (hagia_sha, _) = f.digests.split_once(',').unwrap();
    f.refused(
        &[("wm-pair-commits", &format!("{narthex},{hagia}"))],
        "hagia commit mismatch",
    );
    f.refused(
        &[("wm-pair-sha256", &format!("{hagia_sha},{hagia_sha}"))],
        "narthex binary SHA-256 is not the expected one",
    );
    f.refused(&[("wm-pair-sha256", hagia_sha)], "must be HAGIA,NARTHEX");
    f.refused(
        &[("wm-pair-commits", &format!("{},{narthex}", "HEAD"))],
        "40 lowercase hex",
    );
    // Binary and unsigned manifest replaced together: self-consistent, but
    // not the operator's expected digest.
    let pair = f.dir.0.join("wm-pair");
    fs::write(pair.join("narthex"), "substituted\n").unwrap();
    f.refused(&[], "narthex binary SHA-256 is not the expected one");
    // The default profile and its manifest hash replaced together.
    let f = inputs("profile");
    let pair = f.dir.0.join("wm-pair");
    let replaced = b"schema 1\n// substituted\n";
    fs::write(pair.join("default.kdl"), replaced).unwrap();
    let text = fs::read_to_string(pair.join("wm-pair.manifest")).unwrap();
    fs::write(
        pair.join("wm-pair.manifest"),
        text.replace(&f.profile, &fixture::sha256(replaced)),
    )
    .unwrap();
    f.refused(&[], "default.kdl SHA-256 is not the expected one");
    f.refused(
        &[("wm-pair-profile-sha256", "")],
        "--wm-pair-profile-sha256 is required",
    );
    // The Hagia C SDK revision: required, well formed, and the pair's own.
    let f = inputs("sdk-rev");
    f.refused(
        &[("wm-pair-c-sdk-rev", "")],
        "--wm-pair-c-sdk-rev is required",
    );
    for bad in ["841563d", "HEAD", &"G".repeat(40)] {
        f.refused(
            &[("wm-pair-c-sdk-rev", bad)],
            "--wm-pair-c-sdk-rev must be 40 lowercase hex",
        );
    }
    f.refused(
        &[("wm-pair-c-sdk-rev", &"0".repeat(40))],
        "is not --wm-pair-c-sdk-rev",
    );
    // A manifest that disagrees with the files it describes.
    let f = inputs("manifest");
    let path = f.dir.0.join("wm-pair/wm-pair.manifest");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replace("default_profile_sha256=", "default_profile_sha256=0"),
    )
    .unwrap();
    f.refused(&[], "default_profile_sha256 is not the bound value");
    fs::write(path, format!("{text}extra=1\n")).unwrap();
    f.refused(&[], "unknown key");
}

#[test]
fn the_full_provisioning_marker_is_checked_before_any_build() {
    let f = inputs("marker");
    let integration = provisioned_repo(&f.dir.0);
    // A valid marker lets the run reach the next check (the Sophia source).
    f.refused_with(&integration, Some(HOME), &[], "Sophia checkout");
    f.refused_with(
        &integration,
        None,
        &[],
        "CARGO_HOME must name the provisioned",
    );
    f.refused_with(
        &integration,
        Some("/another/cargo-home"),
        &[],
        "CARGO_HOME is not the provisioned",
    );
    // A stale pin.
    marker(
        &integration,
        &"0".repeat(40),
        &sha256(b"# fixture lock\n"),
        HOME,
    );
    f.refused_with(&integration, Some(HOME), &[], "stale or malformed");
    // A stale lock: Cargo.lock changed since provisioning.
    marker(&integration, SOPHIA_REV, &sha256(b"# fixture lock\n"), HOME);
    fs::write(integration.join("Cargo.lock"), "# changed lock\n").unwrap();
    f.refused_with(&integration, Some(HOME), &[], "stale or malformed");
    // A different recorded home.
    fs::write(integration.join("Cargo.lock"), "# fixture lock\n").unwrap();
    marker(
        &integration,
        SOPHIA_REV,
        &sha256(b"# fixture lock\n"),
        "/elsewhere",
    );
    f.refused_with(
        &integration,
        Some(HOME),
        &[],
        "CARGO_HOME is not the provisioned",
    );
    // No marker at all.
    fs::remove_file(integration.join(".provision/accepted")).unwrap();
    f.refused_with(&integration, Some(HOME), &[], "accepted");
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=F", "-c", "user.email=f@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn a_dirty_sophia_source_is_refused_before_staging() {
    let f = inputs("dirty");
    let integration = provisioned_repo(&f.dir.0);
    let sophia = f.dir.0.join("sophia");
    git(&sophia, &["init", "-q"]);
    fs::write(sophia.join("file"), "one\n").unwrap();
    git(&sophia, &["add", "file"]);
    git(&sophia, &["commit", "-q", "-m", "fixture"]);
    fs::write(sophia.join("file"), "drift\n").unwrap();
    f.refused_with(
        &integration,
        Some(HOME),
        &[],
        "Sophia checkout must be clean",
    );
    // Clean, but not the pinned revision.
    git(&sophia, &["checkout", "-q", "--", "file"]);
    f.refused_with(&integration, Some(HOME), &[], "is not the pinned revision");
}

/// A committed integration checkout holding every file the release copies.
fn integration_checkout(dir: &Path) -> std::path::PathBuf {
    let checkout = dir.join("checkout");
    let real = repo();
    let mut paths = COMMANDS
        .iter()
        .map(|(_, source)| *source)
        .collect::<Vec<_>>();
    paths.extend(TOOLS.iter().map(|(path, _)| *path));
    paths.push(OPERATIONS_DOC);
    for path in paths {
        let target = checkout.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(real.join(path), &target).unwrap();
    }
    fs::write(checkout.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(checkout.join("ignored.txt"), "never packaged\n").unwrap();
    git(&checkout, &["init", "-q"]);
    git(&checkout, &["add", "-A"]);
    git(&checkout, &["commit", "-q", "-m", "fixture"]);
    checkout
}

#[test]
fn the_release_comes_from_the_staged_commit_not_the_checkout() {
    let dir = Dir::new("staged");
    let checkout = integration_checkout(&dir.0);
    let head = git(&checkout, &["rev-parse", "HEAD"]);
    let staged = StagedTree::unauthorized(&checkout, &head, "integration").unwrap();
    assert!(!staged.dir().join("ignored.txt").exists());
    assert!(!staged.dir().join(".git").exists());
    let original = fs::read(checkout.join("tools/installed/sophia-session")).unwrap();

    // The checkout changes after staging: tracked, committed and untracked.
    fs::write(
        checkout.join("tools/installed/sophia-session"),
        "tampered\n",
    )
    .unwrap();
    fs::write(checkout.join("tools/installed/sophia-stop"), "tampered\n").unwrap();
    git(&checkout, &["commit", "-q", "-am", "later"]);
    fs::write(checkout.join("tools/installed/untracked"), "x\n").unwrap();

    staged.reverify().unwrap();
    let mut assembly = fixture::assembly(&dir.0);
    assembly.repo = staged.dir().to_path_buf();
    assemble(&assembly).unwrap();
    assert_eq!(
        fs::read(assembly.out.join("bin/sophia-session")).unwrap(),
        original
    );
    assert_ne!(
        fs::read(assembly.out.join("bin/sophia-stop")).unwrap(),
        b"tampered\n"
    );

    // A staged tree changed after staging is refused.
    fs::write(
        staged.dir().join("tools/installed/sophia-session"),
        "late\n",
    )
    .unwrap();
    let error = staged.reverify().unwrap_err();
    assert!(error.contains("changed after staging"), "{error}");
}

fn listing(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            listing(root, &path, out);
        } else {
            out.insert(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
}

#[test]
fn the_release_layout_manifest_and_sums_are_exact() {
    let dir = Dir::new("layout");
    let assembly = fixture::assembly(&dir.0);
    let lines = assemble(&assembly).unwrap();
    assert!(
        lines[0].starts_with("desktop_release status=packaged "),
        "{lines:?}"
    );
    let out = &assembly.out;

    let mut expected = BTreeSet::new();
    for name in [
        "sophia",
        "sophia-wm-demo",
        "sophia-integration-xtask",
        "active-session-preflight",
        "hagia",
        "narthex",
    ] {
        expected.insert(format!("target/release/{name}"));
    }
    for (name, _) in COMMANDS {
        expected.insert(format!("bin/{name}"));
    }
    for (path, _) in TOOLS.iter().chain(SOPHIA_RETAINED.iter()) {
        expected.insert((*path).to_owned());
    }
    for (stem, ..) in SESSIONS {
        expected.insert(format!("share/wayland-sessions/{stem}.desktop"));
    }
    expected.insert("share/doc/sophia/operations.md".into());
    expected.insert("share/sophia-policy/hagia/default.kdl".into());
    let sealed = expected.clone();
    expected.insert("manifest".into());
    expected.insert("SHA256SUMS".into());
    let mut actual = BTreeSet::new();
    listing(out, out, &mut actual);
    assert_eq!(actual, expected);

    // SHA256SUMS covers exactly bin, share, target and tools, and verifies.
    let sums = fs::read_to_string(out.join("SHA256SUMS")).unwrap();
    let listed = sums
        .lines()
        .map(|l| l.split_once("  ").unwrap().1.to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(listed, sealed);
    let check = Command::new("sha256sum")
        .args(["-c", "--quiet", "SHA256SUMS"])
        .current_dir(out)
        .output()
        .unwrap();
    assert!(check.status.success(), "{check:?}");

    // The manifest is the schema-6 release manifest plus the pair and
    // integration identities, byte for byte.
    let pair = &assembly.pair;
    let expected_manifest = format!(
        "schema=6\nversion=0.1.0\ncommit={rev}\nrelease_id=0.1.0-{short}-{int_short}\n\
         built_at_utc=2026-09-27T00:00:00Z\nhagia_included=true\nhagia_source_commit={hc}\n\
         hagia_default_profile_sha256={ps}\nhagia_binary_sha256={hs}\n\
         hagia_shell_binary_sha256={ns}\nnarthex_source_commit={nc}\nintegration_commit={int}\n",
        rev = assembly.sophia_rev,
        short = &assembly.sophia_rev[..12],
        int = assembly.integration_commit,
        int_short = &assembly.integration_commit[..12],
        hc = pair.hagia_commit,
        ps = pair.profile_sha256,
        hs = pair.hagia_sha256,
        ns = pair.narthex_sha256,
        nc = pair.narthex_commit,
    );
    assert_eq!(
        fs::read_to_string(out.join("manifest")).unwrap(),
        expected_manifest
    );
    assert_eq!(manifest(&assembly), expected_manifest);
    assert_eq!(
        release_id(&assembly),
        format!(
            "0.1.0-{}-{}",
            &assembly.sophia_rev[..12],
            &assembly.integration_commit[..12]
        )
    );

    // Modes: commands and executables 0755, data 0644.
    let mode = |path: &str| fs::metadata(out.join(path)).unwrap().permissions().mode() & 0o777;
    for (name, _) in COMMANDS {
        assert_eq!(mode(&format!("bin/{name}")), 0o755, "{name}");
    }
    for (path, want) in TOOLS.iter().chain(SOPHIA_RETAINED.iter()) {
        assert_eq!(mode(path), *want, "{path}");
    }
    assert_eq!(mode("manifest"), 0o644);
    assert_eq!(mode("share/sophia-policy/hagia/default.kdl"), 0o644);
    let entry =
        fs::read_to_string(out.join("share/wayland-sessions/sophia-hagia.desktop")).unwrap();
    assert!(entry.contains("Exec=@SOPHIA_INSTALL_PREFIX@/current/bin/sophia-hagia-session\n"));
    // The Firefox primary probe a proof can select is packaged too.
    assert!(
        out.join("tools/fixtures/firefox_m10_primary_kitty_probe.sh")
            .is_file()
    );
    // The adapter and the host checker the installed session names exist.
    for path in [
        "tools/session/run_desktop_session.sh",
        "target/release/active-session-preflight",
        "target/release/sophia-integration-xtask",
        "tools/run_sophia_session.sh",
    ] {
        assert!(out.join(path).is_file(), "{path}");
    }
}

#[test]
fn a_missing_input_removes_the_partial_release() {
    let dir = Dir::new("partial");
    let assembly = fixture::assembly(&dir.0);
    fs::remove_file(assembly.sophia_tree.join("tools/stop_sophia_session.sh")).unwrap();
    let error = assemble(&assembly).unwrap_err();
    assert!(error.contains("stop_sophia_session.sh"), "{error}");
    assert!(!assembly.out.exists());
}
