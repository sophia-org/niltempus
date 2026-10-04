//! A client's vendored C SDK snapshot, bound to an explicitly supplied
//! revision (Hagia's, which the Nix release step checks): the manifest names it, upstream.commit
//! hashes to it with source/'s tree, and source/ is exactly the manifest's
//! files. Built offline from a synthetic snapshot; no SDK repository is read.
use std::fs;
use std::path::Path;

use xtask::c_sdk_pin::verify_vendored;

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::Dir;

/// A valid snapshot under `dir`; returns its revision.
fn snapshot(dir: &Path) -> String {
    fixture::c_sdk_snapshot(dir)
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
