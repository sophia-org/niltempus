//! `cargo xtask assemble-nix`: the release a Nix build assembles from
//! prebuilt inputs is byte for byte the release `package-desktop` assembles
//! from the same files, and malformed inputs are refused before anything is
//! written.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use xtask::nix_assembly::run;
use xtask::package_desktop::{Assembly, assemble};

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::Dir;

/// The fixture assembly, with a Sophia workspace manifest for its version.
fn assembly(root: &Path) -> Assembly {
    let assembly = fixture::assembly(root);
    fs::write(
        assembly.sophia_tree.join("Cargo.toml"),
        "[workspace.package]\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    assembly
}

fn arguments(a: &Assembly, out: &Path) -> Vec<String> {
    let path = |p: &Path| p.to_str().unwrap().to_owned();
    [
        ("out", path(out)),
        ("built-at-utc", a.built_at_utc.clone()),
        ("repo", path(&a.repo)),
        ("integration-commit", a.integration_commit.clone()),
        ("sophia-tree", path(&a.sophia_tree)),
        ("sophia-rev", a.sophia_rev.clone()),
        ("sophia", path(&a.binaries.sophia)),
        ("factotum", path(&a.binaries.factotum)),
        ("pam-helper", path(&a.binaries.pam_helper)),
        ("xtask", path(&a.binaries.xtask)),
        ("preflight", path(&a.binaries.preflight)),
        ("hagia", path(&a.pair.hagia)),
        ("hagia-commit", a.pair.hagia_commit.clone()),
        ("narthex", path(&a.pair.narthex)),
        ("narthex-commit", a.pair.narthex_commit.clone()),
        ("profile", path(&a.pair.profile)),
        ("c-sdk-manifest", path(&a.pair.hagia_c_sdk_manifest)),
        ("c-sdk-rev", a.pair.hagia_c_sdk_revision.clone()),
    ]
    .into_iter()
    .map(|(key, value)| format!("--{key}={value}"))
    .collect()
}

/// Every file under `root`, relative, with its bytes and mode.
fn contents(root: &Path) -> BTreeMap<String, (Vec<u8>, u32)> {
    use std::os::unix::fs::PermissionsExt;
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                pending.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                files.insert(
                    relative,
                    (fs::read(&path).unwrap(), meta.permissions().mode() & 0o7777),
                );
            }
        }
    }
    files
}

#[test]
fn a_nix_assembly_equals_the_package_desktop_assembly() {
    let dir = Dir::new("nix-equal");
    let mut assembly = assembly(&dir.0);
    let nix_out = dir.0.join("nix-release");
    let lines = run(&arguments(&assembly, &nix_out)).unwrap();
    assert!(
        lines[0].starts_with("desktop_release status=packaged "),
        "{lines:?}"
    );
    assembly.out = dir.0.join("release");
    assemble(&assembly).unwrap();
    let (nix, packaged) = (contents(&nix_out), contents(&assembly.out));
    assert!(nix.contains_key("manifest") && nix.contains_key("SHA256SUMS"));
    assert_eq!(nix, packaged);
}

#[test]
fn malformed_inputs_are_refused_before_any_output() {
    let dir = Dir::new("nix-refuse");
    let assembly = assembly(&dir.0);
    let out = dir.0.join("nix-release");
    let valid = arguments(&assembly, &out);
    let with = |key: &str, value: &str| -> Vec<String> {
        valid
            .iter()
            .map(|arg| {
                if arg.starts_with(&format!("--{key}=")) {
                    format!("--{key}={value}")
                } else {
                    arg.clone()
                }
            })
            .collect()
    };
    let cases: Vec<(Vec<String>, &str)> = vec![
        (
            with("sophia", "relative/sophia"),
            "--sophia must be absolute",
        ),
        (
            with("hagia", "/nonexistent/hagia"),
            "--hagia is not a regular file",
        ),
        (with("repo", "/nonexistent"), "--repo is not a directory"),
        (
            with("sophia-rev", "HEAD"),
            "--sophia-rev must be 40 lowercase hex",
        ),
        (
            with("c-sdk-rev", &"A".repeat(40)),
            "--c-sdk-rev must be 40 lowercase hex",
        ),
        (
            with("built-at-utc", "2026-10-04 00:00:00"),
            "--built-at-utc must be",
        ),
        (valid[1..].to_vec(), "--out is required"),
        (
            valid
                .iter()
                .cloned()
                .chain(["--extra=1".to_owned()])
                .collect(),
            "unknown or empty option --extra",
        ),
    ];
    for (args, expected) in cases {
        let error = run(&args).unwrap_err();
        assert!(error.contains(expected), "{expected:?} not in {error:?}");
        assert!(!out.exists(), "output written for {expected:?}");
    }
    fs::create_dir(&out).unwrap();
    assert!(run(&valid).unwrap_err().contains("--out already exists"));
}

#[test]
fn a_c_sdk_revision_the_manifest_does_not_name_leaves_no_release() {
    let dir = Dir::new("nix-sdk");
    let assembly = assembly(&dir.0);
    let out = dir.0.join("nix-release");
    let mut args = arguments(&assembly, &out);
    let other = "0123456789abcdef0123456789abcdef01234567";
    for arg in &mut args {
        if arg.starts_with("--c-sdk-rev=") {
            *arg = format!("--c-sdk-rev={other}");
        }
    }
    assert!(run(&args).is_err());
    assert!(!out.exists());
}
