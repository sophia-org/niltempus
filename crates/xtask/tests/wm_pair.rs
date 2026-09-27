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
    // The Hagia C SDK revision is an explicit input with no default: absent
    // or malformed, the run stops before any build (the build dir and Nim
    // dependency paths are never consulted).
    let complete = |rev: Option<&str>| {
        let mut values = vec![
            "--hagia".to_owned(),
            root.clone(),
            zero.clone(),
            "--narthex".to_owned(),
            root.clone(),
            zero.clone(),
            out.clone(),
            "--build-dir=/nonexistent/build".to_owned(),
            "--hagia-nim-deps=/nonexistent/h".to_owned(),
            format!("--hagia-nim-deps-sha256={}", "a".repeat(64)),
            "--narthex-nim-deps=/nonexistent/n".to_owned(),
            format!("--narthex-nim-deps-sha256={}", "b".repeat(64)),
        ];
        values.extend(rev.map(|rev| format!("--hagia-c-sdk-rev={rev}")));
        values
    };
    let error = run(&complete(None)).unwrap_err();
    assert!(
        error.contains("--hagia-c-sdk-rev=<40 hex> is required"),
        "{error}"
    );
    for bad in ["841563d", "HEAD", &"G".repeat(40)] {
        let error = run(&complete(Some(bad))).unwrap_err();
        assert!(
            error.contains("--hagia-c-sdk-rev must be 40 lowercase hex"),
            "{error}"
        );
    }
    // A well-formed revision gets past this check to the next one.
    let error = run(&complete(Some(fixture::HAGIA_C_SDK_REV))).unwrap_err();
    assert!(error.contains("--build-dir"), "{error}");
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
    let verified = verify(&pair, commits, digests, &ids.profile).unwrap();
    assert_eq!(verified.hagia, pair.join("hagia"));
    assert_eq!(verified.narthex_sha256, ids.digests[1]);
    assert_eq!(verified.hagia_c_sdk_revision, fixture::HAGIA_C_SDK_REV);

    assert!(
        verify(std::path::Path::new("pair"), commits, digests, &ids.profile)
            .unwrap_err()
            .contains("absolute")
    );
    // The default profile replaced together with its manifest hash: the
    // manifest agrees, the operator's expected digest does not.
    let replaced = b"schema 1\n// substituted\n";
    fs::write(pair.join("default.kdl"), replaced).unwrap();
    let manifest = fs::read_to_string(pair.join("wm-pair.manifest")).unwrap();
    fs::write(
        pair.join("wm-pair.manifest"),
        manifest.replace(&ids.profile, &fixture::sha256(replaced)),
    )
    .unwrap();
    let error = verify(&pair, commits, digests, &ids.profile).unwrap_err();
    assert!(
        error.contains("default.kdl SHA-256 is not the expected one"),
        "{error}"
    );
    // Accepted only when the operator names the new digest, which is the point.
    verify(&pair, commits, digests, &fixture::sha256(replaced)).unwrap();
    // A raw commit object that does not hash to the expected commit.
    fs::write(pair.join("narthex.commit"), b"tree 1111\n").unwrap();
    assert!(verify(&pair, commits, digests, &ids.profile).is_err());
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
        &ids.profile,
    )
    .unwrap_err();
    assert!(error.contains("no regular hagia"), "{error}");
}

#[test]
fn verify_binds_the_vendored_c_sdk_manifest_and_revision() {
    let dir = Dir::new("wm-pair-sdk");
    let pair = dir.0.join("pair");
    let ids = write_pair(&pair);
    let commits = [ids.commits[0].as_str(), ids.commits[1].as_str()];
    let digests = [ids.digests[0].as_str(), ids.digests[1].as_str()];
    verify(&pair, commits, digests, &ids.profile).unwrap();
    let sdk = pair.join("hagia-c-sdk.manifest.json");
    let manifest_path = pair.join("wm-pair.manifest");
    let original_sdk = fs::read_to_string(&sdk).unwrap();
    let original = fs::read_to_string(&manifest_path).unwrap();
    // The carried manifest changed: its digest no longer binds.
    fs::write(&sdk, original_sdk.replace("README.md", "README.txt")).unwrap();
    let error = verify(&pair, commits, digests, &ids.profile).unwrap_err();
    assert!(error.contains("hagia_c_sdk_manifest_sha256"), "{error}");
    // Missing entirely.
    fs::remove_file(&sdk).unwrap();
    let error = verify(&pair, commits, digests, &ids.profile).unwrap_err();
    assert!(
        error.contains("no regular hagia-c-sdk.manifest.json"),
        "{error}"
    );
    fs::write(&sdk, &original_sdk).unwrap();
    // The recorded revision is not the one the carried manifest names.
    let other = "0".repeat(40);
    fs::write(
        &manifest_path,
        original.replace(fixture::HAGIA_C_SDK_REV, &other),
    )
    .unwrap();
    let error = verify(&pair, commits, digests, &ids.profile).unwrap_err();
    assert!(error.contains("hagia_c_sdk_revision"), "{error}");
    // A schema-2 pair (no SDK binding) is refused.
    let schema2 = original
        .replace("schema=3", "schema=2")
        .lines()
        .filter(|line| !line.starts_with("hagia_c_sdk_"))
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    fs::write(&manifest_path, schema2).unwrap();
    assert!(verify(&pair, commits, digests, &ids.profile).is_err());
    fs::write(&manifest_path, &original).unwrap();
    verify(&pair, commits, digests, &ids.profile).unwrap();
}
