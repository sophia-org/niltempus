//! A client's vendored C SDK snapshot, bound to an explicitly supplied
//! revision (Hagia's, for the WM pair): the manifest names it, upstream.commit
//! hashes to it with source/'s tree, and source/ is exactly the manifest's
//! files. Built offline from a synthetic snapshot; no SDK repository is read.
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use xtask::c_sdk_pin::verify_vendored;

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::Dir;

fn git_hash(kind: &str, bytes: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin", "-t", kind])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A valid snapshot under `dir`; returns its revision.
fn snapshot(dir: &Path) -> String {
    let source = dir.join("source");
    fs::create_dir_all(source.join("src")).unwrap();
    fs::write(source.join("README.md"), "sdk\n").unwrap();
    fs::write(source.join("src/client.c"), "int x;\n").unwrap();
    let inventory = xtask::git_tree::inventory(&source).unwrap();
    let commit = format!(
        "tree {}\nauthor A <a@example.org> 0 +0000\ncommitter A <a@example.org> 0 +0000\n\nsnapshot\n",
        inventory.tree
    );
    let revision = git_hash("commit", commit.as_bytes());
    fs::write(dir.join("upstream.commit"), &commit).unwrap();
    let manifest = serde_json::json!({
        "schema": 1,
        "repository": "https://github.com/sophia-org/sophia-desktop-sdk-c",
        "revision": revision,
        "files": inventory.files,
    });
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    revision
}

#[test]
fn a_snapshot_at_the_supplied_revision_verifies() {
    let dir = Dir::new("sdk-vendored-good");
    let revision = snapshot(&dir.0);
    let verified = verify_vendored(&dir.0, &revision).unwrap();
    assert_eq!(verified.revision, revision);
    assert_eq!(
        verified.manifest_sha256,
        fixture::sha256(&fs::read(dir.0.join("manifest.json")).unwrap())
    );
}

#[test]
fn a_different_supplied_revision_is_refused() {
    let dir = Dir::new("sdk-vendored-rev");
    let revision = snapshot(&dir.0);
    let other = "841563d614ed8540472f0edfa7f4cddaafe3fdde";
    assert_ne!(revision, other);
    let error = verify_vendored(&dir.0, other).unwrap_err();
    assert!(error.contains("not the supplied"), "{error}");
    for bad in ["", "841563d", "HEAD"] {
        let error = verify_vendored(&dir.0, bad).unwrap_err();
        assert!(error.contains("40 lowercase hex"), "{error}");
    }
}

#[test]
fn a_tampered_vendored_file_is_refused() {
    let dir = Dir::new("sdk-vendored-tamper");
    let revision = snapshot(&dir.0);
    fs::write(dir.0.join("source/src/client.c"), "int y;\n").unwrap();
    let error = verify_vendored(&dir.0, &revision).unwrap_err();
    assert!(
        error.contains("differs from its manifest: src/client.c"),
        "{error}"
    );
}

#[test]
fn an_extra_or_missing_vendored_file_is_refused() {
    let dir = Dir::new("sdk-vendored-extra");
    let revision = snapshot(&dir.0);
    fs::write(dir.0.join("source/src/extra.c"), "int z;\n").unwrap();
    let error = verify_vendored(&dir.0, &revision).unwrap_err();
    assert!(
        error.contains("differs from its manifest: src/extra.c"),
        "{error}"
    );
    let dir = Dir::new("sdk-vendored-missing");
    let revision = snapshot(&dir.0);
    fs::remove_file(dir.0.join("source/README.md")).unwrap();
    let error = verify_vendored(&dir.0, &revision).unwrap_err();
    assert!(
        error.contains("differs from its manifest: README.md"),
        "{error}"
    );
}

#[test]
fn a_missing_manifest_or_unbound_commit_is_refused() {
    let dir = Dir::new("sdk-vendored-nomanifest");
    let revision = snapshot(&dir.0);
    fs::remove_file(dir.0.join("manifest.json")).unwrap();
    let error = verify_vendored(&dir.0, &revision).unwrap_err();
    assert!(error.contains("no regular"), "{error}");
    // A manifest naming the supplied revision whose upstream.commit does not
    // hash to it.
    let dir = Dir::new("sdk-vendored-commit");
    let revision = snapshot(&dir.0);
    let commit = fs::read_to_string(dir.0.join("upstream.commit")).unwrap();
    fs::write(
        dir.0.join("upstream.commit"),
        commit.replace("snapshot", "other"),
    )
    .unwrap();
    let error = verify_vendored(&dir.0, &revision).unwrap_err();
    assert!(
        error.contains("does not identify upstream.commit"),
        "{error}"
    );
}

#[test]
fn installed_snapshot_verifier_has_a_versioned_verdict_and_refuses_tamper() {
    let dir = Dir::new("sdk-vendored-cli");
    let revision = snapshot(&dir.0);
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("verify-c-sdk")
            .arg(&dir.0)
            .arg(format!("--revision={revision}"))
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success(), "{output:?}");
    let record = String::from_utf8(output.stdout).unwrap();
    assert_eq!(record.lines().count(), 1);
    assert!(record.starts_with(&format!(
        "c_sdk_verification schema=1 status=pass revision={revision} manifest_sha256="
    )));
    fs::write(dir.0.join("source/src/client.c"), "changed").unwrap();
    assert!(!run().status.success());
}
