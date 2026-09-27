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
    COMMANDS, SESSIONS, SOPHIA_RETAINED, TOOLS, assemble, manifest, release_id, run,
};
use xtask::pins::SOPHIA_REV;

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, repo, write_pair};

struct Inputs {
    dir: Dir,
    commits: String,
    digests: String,
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
        dir,
    }
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
        let error = run(&repo(), &self.args(changes)).unwrap_err();
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
fn a_dirty_sophia_source_is_refused_before_staging() {
    let f = inputs("dirty");
    let sophia = f.dir.0.join("sophia");
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&sophia)
                .args(["-c", "user.name=F", "-c", "user.email=f@example.com"])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        )
    };
    git(&["init", "-q"]);
    fs::write(sophia.join("file"), "one\n").unwrap();
    git(&["add", "file"]);
    git(&["commit", "-q", "-m", "fixture"]);
    fs::write(sophia.join("file"), "drift\n").unwrap();
    f.refused(&[], "Sophia checkout must be clean");
    // Clean, but not the pinned revision.
    git(&["checkout", "-q", "--", "file"]);
    f.refused(&[], "is not the pinned revision");
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
