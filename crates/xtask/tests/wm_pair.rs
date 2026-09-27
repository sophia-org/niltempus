//! `cargo xtask prepare-wm-pair` refuses ambiguous inputs before anything is
//! read or built; `verify` binds a prepared pair to the operator's commits and
//! digests (its refusals are exercised through package-desktop too).
use std::fs;
use xtask::wm_pair::{run, verify};

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, repo, write_pair};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn prepare_refuses_ambiguous_inputs_before_building() {
    let root = repo().display().to_string();
    let zero = "0".repeat(40);
    let dir = Dir::new("wm-pair-inputs");
    let out = dir.0.join("pair").display().to_string();
    assert!(run(&[]).unwrap_err().contains("usage"));
    assert!(
        run(&args(&[
            "--narthex",
            &root,
            &zero,
            "--hagia",
            &root,
            &zero,
            &out
        ]))
        .unwrap_err()
        .contains("usage")
    );
    assert!(
        run(&args(&[
            "--hagia",
            &root,
            "HEAD",
            "--narthex",
            &root,
            &zero,
            &out
        ]))
        .unwrap_err()
        .contains("40 lowercase hex")
    );
    assert!(
        run(&args(&[
            "--hagia",
            &root,
            &zero,
            "--narthex",
            &root,
            "main",
            &out
        ]))
        .unwrap_err()
        .contains("40 lowercase hex")
    );
    fs::create_dir(dir.0.join("pair")).unwrap();
    assert!(
        run(&args(&[
            "--hagia",
            &root,
            &zero,
            "--narthex",
            &root,
            &zero,
            &out
        ]))
        .unwrap_err()
        .contains("output already exists")
    );
}

#[test]
fn verify_binds_every_identity_to_the_files() {
    let dir = Dir::new("wm-pair-verify");
    let pair = dir.0.join("pair");
    let ids = write_pair(&pair);
    let commits = [ids.commits[0].as_str(), ids.commits[1].as_str()];
    let digests = [ids.digests[0].as_str(), ids.digests[1].as_str()];
    let verified = verify(&pair, commits, digests).unwrap();
    assert_eq!(verified.hagia, pair.join("hagia"));
    assert_eq!(verified.narthex_sha256, ids.digests[1]);

    assert!(
        verify(std::path::Path::new("pair"), commits, digests)
            .unwrap_err()
            .contains("absolute")
    );
    // A raw commit object that does not hash to the expected commit.
    fs::write(pair.join("narthex.commit"), b"tree 1111\n").unwrap();
    assert!(verify(&pair, commits, digests).is_err());
    // A symlinked binary is not a regular file.
    let dir = Dir::new("wm-pair-link");
    let pair = dir.0.join("pair");
    let ids = write_pair(&pair);
    fs::rename(pair.join("hagia"), dir.0.join("hagia")).unwrap();
    std::os::unix::fs::symlink(dir.0.join("hagia"), pair.join("hagia")).unwrap();
    let error = verify(
        &pair,
        [ids.commits[0].as_str(), ids.commits[1].as_str()],
        [ids.digests[0].as_str(), ids.digests[1].as_str()],
    )
    .unwrap_err();
    assert!(error.contains("no regular hagia"), "{error}");
}
