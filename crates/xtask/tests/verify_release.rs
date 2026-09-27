//! `xtask verify-release`: a real schema-7 release (assembled from fixture
//! inputs) passes; malformed and tampered releases are refused; nothing from
//! the candidate is executed.
use std::fs;
use std::path::{Path, PathBuf};

use xtask::package_desktop::{RELEASE_C_SDK_MANIFEST, assemble};
use xtask::release_verify::run;

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, HAGIA_C_SDK_REV, sha256};

fn release(tag: &str) -> (Dir, PathBuf) {
    let dir = Dir::new(tag);
    let assembly = fixture::assembly(&dir.0);
    assemble(&assembly).unwrap();
    let out = assembly.out.clone();
    (dir, out)
}

fn verify(out: &Path, rev: &str) -> Result<Vec<String>, String> {
    run(&[out.display().to_string(), format!("--c-sdk-rev={rev}")])
}

/// Rewrite SHA256SUMS over bin, share, target and tools, as packaging does.
fn reseal(out: &Path) {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                files.push(path.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    let mut files = Vec::new();
    for top in ["bin", "share", "target", "tools"] {
        walk(out, &out.join(top), &mut files);
    }
    files.sort();
    let sums = files
        .iter()
        .map(|f| format!("{}  {f}\n", sha256(&fs::read(out.join(f)).unwrap())))
        .collect::<String>();
    fs::write(out.join("SHA256SUMS"), sums).unwrap();
}

#[test]
fn a_sealed_schema_7_release_passes_with_one_record() {
    let (_dir, out) = release("verify-pass");
    let lines = verify(&out, HAGIA_C_SDK_REV).unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let digest = sha256(&fs::read(out.join(RELEASE_C_SDK_MANIFEST)).unwrap());
    assert!(
        lines[0].starts_with("release_verification schema=1 status=pass release_id="),
        "{lines:?}"
    );
    assert!(
        lines[0].contains(&format!(
            " c_sdk_revision={HAGIA_C_SDK_REV} c_sdk_manifest_sha256={digest} sealed_files="
        )),
        "{lines:?}"
    );
}

#[test]
fn arguments_are_exact() {
    let (_dir, out) = release("verify-args");
    let dir = out.display().to_string();
    for args in [
        vec![],
        vec![dir.clone()],
        vec![
            dir.clone(),
            format!("--c-sdk-rev={HAGIA_C_SDK_REV}"),
            "extra".into(),
        ],
        vec![format!("--c-sdk-rev={HAGIA_C_SDK_REV}"), dir.clone()],
        vec![dir.clone(), format!("--sdk-rev={HAGIA_C_SDK_REV}")],
    ] {
        assert!(run(&args).is_err(), "{args:?}");
    }
    for bad in ["", "841563d", "HEAD", &HAGIA_C_SDK_REV.to_uppercase()] {
        let error = verify(&out, bad).unwrap_err();
        assert!(error.contains("40 lowercase hex"), "{bad}: {error}");
    }
    let error = run(&["relative".into(), format!("--c-sdk-rev={HAGIA_C_SDK_REV}")]).unwrap_err();
    assert!(error.contains("absolute"), "{error}");
    let link = out.with_file_name("linked-release");
    std::os::unix::fs::symlink(&out, &link).unwrap();
    assert!(verify(&link, HAGIA_C_SDK_REV).is_err());
}

#[test]
fn the_wrong_or_absent_revision_is_refused() {
    let (_dir, out) = release("verify-rev");
    let error = verify(&out, &"0".repeat(40)).unwrap_err();
    assert!(error.contains("is not the supplied"), "{error}");
    let manifest = out.join("manifest");
    let text = fs::read_to_string(&manifest).unwrap();
    let without = text
        .lines()
        .filter(|l| !l.starts_with("hagia_c_sdk_revision="))
        .map(|l| format!("{l}\n"))
        .collect::<String>();
    fs::write(&manifest, without).unwrap();
    let error = verify(&out, HAGIA_C_SDK_REV).unwrap_err();
    assert!(error.contains("no hagia_c_sdk_revision"), "{error}");
    fs::write(&manifest, text.replace("schema=7", "schema=6")).unwrap();
    let error = verify(&out, HAGIA_C_SDK_REV).unwrap_err();
    assert!(error.contains("is not schema 7"), "{error}");
    fs::write(
        &manifest,
        text.replace("hagia_included=true", "hagia_included=false"),
    )
    .unwrap();
    assert!(verify(&out, HAGIA_C_SDK_REV).is_err());
    fs::write(&manifest, format!("{text}release_id=again\n")).unwrap();
    assert!(verify(&out, HAGIA_C_SDK_REV).is_err());
    fs::write(&manifest, &text).unwrap();
    verify(&out, HAGIA_C_SDK_REV).unwrap();
}

#[test]
fn a_changed_or_missing_sealed_sdk_manifest_is_refused() {
    let (_dir, out) = release("verify-sealed");
    let sealed = out.join(RELEASE_C_SDK_MANIFEST);
    let bytes = fs::read(&sealed).unwrap();
    // Changed without resealing: SHA256SUMS catches it.
    fs::write(&sealed, [bytes.as_slice(), b" "].concat()).unwrap();
    let error = verify(&out, HAGIA_C_SDK_REV).unwrap_err();
    assert!(error.contains("does not match SHA256SUMS"), "{error}");
    // Changed and resealed: the recorded digest catches it.
    reseal(&out);
    let error = verify(&out, HAGIA_C_SDK_REV).unwrap_err();
    assert!(error.contains("does not hash to"), "{error}");
    // Missing and resealed.
    fs::remove_file(&sealed).unwrap();
    reseal(&out);
    let error = verify(&out, HAGIA_C_SDK_REV).unwrap_err();
    assert!(error.contains("no regular"), "{error}");
}

/// One way to damage a sealed release in place.
type Damage = fn(&Path);

#[test]
fn sums_and_contents_must_agree_exactly() {
    let cases: [(&str, Damage); 7] = [
        ("changed binary", |out| {
            fs::write(out.join("target/release/sophia"), "changed").unwrap()
        }),
        ("unlisted file", |out| {
            fs::write(out.join("tools/extra.sh"), "x").unwrap()
        }),
        ("missing listed file", |out| {
            fs::remove_file(out.join("tools/verify_packaged_policy.sh")).unwrap()
        }),
        ("extra top-level entry", |out| {
            fs::write(out.join("extra"), "x").unwrap()
        }),
        ("symlink inside", |out| {
            std::os::unix::fs::symlink("/etc/passwd", out.join("share/linked")).unwrap()
        }),
        ("traversal in sums", |out| {
            let sums = fs::read_to_string(out.join("SHA256SUMS")).unwrap();
            fs::write(
                out.join("SHA256SUMS"),
                format!("{sums}{}  ../outside\n", "a".repeat(64)),
            )
            .unwrap()
        }),
        ("malformed sums", |out| {
            let sums = fs::read_to_string(out.join("SHA256SUMS")).unwrap();
            fs::write(out.join("SHA256SUMS"), format!("{sums}not a line\n")).unwrap()
        }),
    ];
    for (name, damage) in cases {
        let (_dir, out) = release("verify-sums");
        damage(&out);
        assert!(verify(&out, HAGIA_C_SDK_REV).is_err(), "accepted {name}");
    }
    let (_dir, out) = release("verify-nosums");
    fs::remove_file(out.join("SHA256SUMS")).unwrap();
    assert!(verify(&out, HAGIA_C_SDK_REV).is_err());
}

#[test]
fn nothing_from_the_candidate_is_executed() {
    let (dir, out) = release("verify-noexec");
    let marker = dir.0.join("executed");
    let script = format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display());
    for path in ["tools/verify_packaged_policy.sh", "bin/sophia-status"] {
        fs::write(out.join(path), &script).unwrap();
    }
    reseal(&out);
    verify(&out, HAGIA_C_SDK_REV).unwrap();
    assert!(!marker.exists(), "verification executed a candidate script");
}
