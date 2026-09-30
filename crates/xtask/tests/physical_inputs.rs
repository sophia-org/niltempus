//! `prepare-physical-inputs` refuses ambiguous or escaping inputs before
//! anything is staged or built, and `verify` binds a prepared output to its
//! expected manifest sha256 and re-derives every identity. The builders it
//! shares refuse a missing or misplaced dependency manifest before staging.
//! Nothing here builds, stages a signed tree or runs a gate.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use xtask::physical_inputs::{
    ENV_KEYS, header_for_tests, profile_for_tests, run_with, seal, verify, write_env_for_tests,
};

#[path = "support/release_fixture.rs"]
mod fixture;
use fixture::{Dir, NIM_CONFIG_SHA256, NIM_STDLIB_SHA256, repo, reviewed_deps, sha256};

fn args(values: &[String]) -> Vec<String> {
    values.to_vec()
}

struct Base {
    dir: Dir,
}

impl Base {
    fn new(tag: &str) -> Self {
        let dir = Dir::new(tag);
        fs::create_dir(dir.0.join("sophia")).unwrap();
        fs::create_dir(dir.0.join("build")).unwrap();
        fs::set_permissions(dir.0.join("build"), fs::Permissions::from_mode(0o700)).unwrap();
        Self { dir }
    }

    fn path(&self, name: &str) -> String {
        self.dir.0.join(name).display().to_string()
    }

    fn args(&self, extra: &[&str]) -> Vec<String> {
        let mut values = vec![
            format!("--sophia-root={}", self.path("sophia")),
            format!("--build-dir={}", self.path("build")),
            format!("--out={}", self.path("out")),
            "--sophia-features=native-session".to_owned(),
        ];
        values.extend(extra.iter().map(|v| (*v).to_owned()));
        values
    }

    fn refused(&self, values: Vec<String>, expected: &str) {
        let error = run_with(&repo(), &args(&values), None).unwrap_err();
        assert!(error.contains(expected), "{values:?}: {error}");
    }
}

#[test]
fn ambiguous_or_escaping_requests_are_refused_before_staging() {
    let base = Base::new("physical-request");
    let zero = "0".repeat(40);
    let digest = "a".repeat(64);
    let deps = base.path("deps");
    let hagia = [
        format!("--hagia={}", repo().display()),
        format!("--hagia-commit={zero}"),
        format!("--hagia-nim-deps={deps}"),
        format!("--hagia-nim-deps-sha256={digest}"),
    ];
    let with_hagia = |extra: &[&str]| {
        let mut values = base.args(extra);
        values.extend(hagia.iter().cloned());
        values
    };
    let mut no_features = base.args(&[]);
    no_features.retain(|v| !v.starts_with("--sophia-features"));
    base.refused(no_features, "--sophia-features is required");
    base.refused(
        base.args(&["--sophia-features=atomic-scanout-smoke-live"]),
        "is repeated",
    );
    let mut bad_feature = base.args(&[]);
    bad_feature[3] = "--sophia-features=atomic-scanout-smoke-live".into();
    base.refused(bad_feature, "must be one of");
    base.refused(
        base.args(&["--sophia-packages=sophia-wm-demo"]),
        "--sophia-packages must be one of",
    );
    let mut relative = base.args(&[]);
    relative[2] = "--out=out".into();
    base.refused(relative, "--out must be absolute");
    let mut spaced = base.args(&[]);
    spaced[2] = format!("--out={}", base.path("out dir"));
    base.refused(spaced, "--out may hold only");
    base.refused(base.args(&["--frobnicate=1"]), "unknown or empty option");
    for (profile, expected) in [
        ("--profile=hagia:../hagia/x.kdl", "escapes"),
        ("--profile=hagia:/etc/passwd", "escapes"),
        ("--profile=hagia:a//b", "escapes"),
        ("--profile=bemenu:config.kdl", "owner must be"),
        ("--profile=default.kdl", "OWNER:PATH"),
        ("--profile=hagia:examples/config/default.kdl", "not staged"),
    ] {
        base.refused(base.args(&[profile]), expected);
    }
    base.refused(
        with_hagia(&[
            "--profile=hagia:examples/config/default.kdl",
            "--profile=hagia:examples/config/default.kdl",
        ]),
        "repeated",
    );
    // A Nim half is all four options or none.
    let mut partial = base.args(&[]);
    partial.push(hagia[0].clone());
    partial.push(hagia[1].clone());
    base.refused(partial, "go together");
    let mut short = with_hagia(&[]);
    let at = short
        .iter()
        .position(|v| v.starts_with("--hagia-commit"))
        .unwrap();
    short[at] = "--hagia-commit=HEAD".into();
    base.refused(short, "40 lowercase hex");
    // Build and output directories: private, outside every source tree.
    let mut inside = base.args(&[]);
    inside[1] = format!("--build-dir={}/target-physical", repo().display());
    base.refused(inside, "outside every source tree");
    let mut out_inside = base.args(&[]);
    out_inside[2] = format!("--out={}/physical-out", base.path("sophia"));
    base.refused(out_inside, "outside every source tree");
    fs::set_permissions(base.dir.0.join("build"), fs::Permissions::from_mode(0o755)).unwrap();
    base.refused(base.args(&[]), "private (0700)");
    fs::set_permissions(base.dir.0.join("build"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(base.dir.0.join("out")).unwrap();
    base.refused(base.args(&[]), "--out already exists");
    fs::remove_dir(base.dir.0.join("out")).unwrap();
    // The full provisioning marker comes first among the custody checks: no
    // CARGO_HOME (or, in an unprovisioned checkout, no marker) is refused.
    let error = run_with(&repo(), &base.args(&[]), None).unwrap_err();
    assert!(
        error.contains("CARGO_HOME") || error.contains(".provision"),
        "{error}"
    );
    // verify needs the expected manifest digest.
    let error = run_with(
        &repo(),
        &args(&["verify".into(), format!("--out={}", base.path("x"))]),
        None,
    )
    .unwrap_err();
    assert!(error.contains("--manifest-sha256"), "{error}");
}

#[test]
fn profiles_resolve_only_to_regular_files_inside_their_source() {
    let dir = Dir::new("physical-profile");
    let root = dir.0.join("tree");
    fs::create_dir_all(root.join("examples/config")).unwrap();
    fs::write(root.join("examples/config/default.kdl"), "x\n").unwrap();
    std::os::unix::fs::symlink(root.join("examples"), root.join("linked")).unwrap();
    assert_eq!(
        profile_for_tests("hagia:examples/config/default.kdl", &root).unwrap(),
        root.join("examples/config/default.kdl")
    );
    assert_eq!(
        profile_for_tests("integration:examples/config/default.kdl", &root).unwrap(),
        root.join("examples/config/default.kdl")
    );
    for bad in [
        "hagia:linked/config/default.kdl",
        "hagia:examples/config",
        "hagia:examples/config/missing.kdl",
        "hagia:examples/../examples/config/default.kdl",
        "integration:linked/config/default.kdl",
        "integration:examples/../examples/config/default.kdl",
    ] {
        assert!(profile_for_tests(bad, &root).is_err(), "{bad}");
    }
}

/// A synthetic prepared output in the helper's layout.
fn prepared(root: &Path, products: &[(&str, &str)]) -> (std::path::PathBuf, String) {
    let out = root.join("inputs");
    for dir in ["bin", "profiles/sophia", "nim-deps", "sophia-tree/sub"] {
        fs::create_dir_all(out.join(dir)).unwrap();
    }
    fs::write(out.join("bin/sophia"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(out.join("bin/sophia"), fs::Permissions::from_mode(0o555)).unwrap();
    fs::write(out.join("profiles/sophia/core.kdl"), "core\n").unwrap();
    fs::write(out.join("sophia-tree/a.txt"), "a\n").unwrap();
    fs::write(out.join("sophia-tree/sub/b.sh"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(
        out.join("sophia-tree/sub/b.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let tree = xtask::git_tree::inventory(&out.join("sophia-tree"))
        .unwrap()
        .tree;
    let mut named = Vec::new();
    for (name, commit) in products {
        let text = reviewed_deps(name, commit);
        fs::write(out.join("nim-deps").join(format!("{name}.manifest")), &text).unwrap();
        fs::write(out.join("bin").join(name), "#!/bin/sh\n").unwrap();
        named.push((*name, *commit, sha256(text.as_bytes())));
    }
    let products = named
        .iter()
        .map(|(n, c, d)| (*n, *c, d.as_str(), NIM_CONFIG_SHA256, NIM_STDLIB_SHA256))
        .collect::<Vec<_>>();
    let header = header_for_tests(&"2".repeat(40), &tree, &products);
    write_env_for_tests(&out, &header).unwrap();
    let manifest = seal(&out, header).unwrap();
    fs::write(out.join("physical-inputs.manifest"), &manifest).unwrap();
    (out, sha256(manifest.as_bytes()))
}

#[test]
fn verify_binds_every_file_to_the_expected_manifest() {
    let dir = Dir::new("physical-verify");
    let (out, digest) = prepared(&dir.0, &[("hagia", &"3".repeat(40))]);
    let sealed = verify(&out, &digest).unwrap();
    assert_eq!(sealed.sha256, digest);

    // inputs.env: only known keys, absolute paths or hex, never sourced.
    let env = fs::read_to_string(out.join("inputs.env")).unwrap();
    for line in env.lines() {
        let (key, value) = line.split_once('=').unwrap();
        assert!(ENV_KEYS.contains(&key), "{key}");
        assert!(
            value.starts_with('/') || value.bytes().all(|b| b.is_ascii_hexdigit()),
            "{line}"
        );
    }
    assert!(env.contains(&format!("SOPHIA_HAGIA_BIN={}/bin/hagia\n", out.display())));
    assert!(env.contains(&format!("SOPHIA_ROOT={}/sophia-tree\n", out.display())));

    assert!(
        verify(&out, &"0".repeat(64))
            .unwrap_err()
            .contains("not the expected manifest")
    );
    assert!(verify(&out, "short").is_err());
    assert!(verify(Path::new("inputs"), &digest).is_err());
    let tampered = |path: &str, bytes: &[u8]| {
        let file = out.join(path);
        let before = fs::read(&file).unwrap();
        let mode = fs::metadata(&file).unwrap().permissions();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&file, bytes).unwrap();
        fs::set_permissions(&file, mode.clone()).unwrap();
        let error = verify(&out, &digest).unwrap_err();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&file, before).unwrap();
        fs::set_permissions(&file, mode).unwrap();
        verify(&out, &digest).unwrap();
        error
    };
    assert!(tampered("bin/sophia", b"#!/bin/sh\nexit 1\n").contains("not the ones"));
    assert!(tampered("sophia-tree/a.txt", b"b\n").contains("not the pinned Sophia tree"));
    assert!(tampered("nim-deps/hagia.manifest", b"other\n").contains("not the bound one"));
    // A mode change is a different file record.
    fs::set_permissions(out.join("bin/sophia"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(verify(&out, &digest).is_err());
    fs::set_permissions(out.join("bin/sophia"), fs::Permissions::from_mode(0o555)).unwrap();
    // An unlisted file, then a link.
    fs::write(out.join("profiles/sophia/extra.kdl"), "x\n").unwrap();
    assert!(verify(&out, &digest).is_err());
    fs::remove_file(out.join("profiles/sophia/extra.kdl")).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", out.join("profiles/sophia/link.kdl")).unwrap();
    assert!(verify(&out, &digest).is_err());
    fs::remove_file(out.join("profiles/sophia/link.kdl")).unwrap();
    verify(&out, &digest).unwrap();
    // A product record naming another installation config than the one its
    // reviewed dependency manifest records, even under a matching digest.
    let manifest_path = out.join("physical-inputs.manifest");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let forged = manifest.replace(
        &format!("nim_config_sha256={NIM_CONFIG_SHA256}"),
        &format!("nim_config_sha256={}", "f".repeat(64)),
    );
    assert_ne!(forged, manifest);
    fs::write(&manifest_path, &forged).unwrap();
    let error = verify(&out, &sha256(forged.as_bytes())).unwrap_err();
    assert!(
        error.contains("does not name its reviewed nim-config"),
        "{error}"
    );
    fs::write(&manifest_path, &manifest).unwrap();
    verify(&out, &digest).unwrap();
    // A relocated output no longer matches its own inputs.env.
    let moved = dir.0.join("moved");
    fs::rename(&out, &moved).unwrap();
    assert!(verify(&moved, &digest).unwrap_err().contains("inputs.env"));
}

#[test]
fn builders_refuse_a_missing_or_misplaced_dependency_manifest_before_staging() {
    let dir = Dir::new("builder-deps");
    let build = dir.0.join("build");
    fs::create_dir(&build).unwrap();
    fs::set_permissions(&build, fs::Permissions::from_mode(0o700)).unwrap();
    let root = repo().display().to_string();
    let zero = "0".repeat(40);
    let out = dir.0.join("out").display().to_string();
    let build_arg = format!("--build-dir={}", build.display());
    let product = |extra: &[&str]| {
        let mut values = vec!["hagia".to_owned(), root.clone(), zero.clone(), out.clone()];
        values.extend(extra.iter().map(|v| (*v).to_owned()));
        xtask::product_artifact::run(&values).unwrap_err()
    };
    assert!(product(&[]).contains("--build-dir is required"));
    assert!(product(&["--build-dir=build"]).contains("must be absolute"));
    let error = product(&[&build_arg]);
    assert!(
        error.contains("needs its reviewed dependency manifest"),
        "{error}"
    );
    let deps = format!("--nim-deps={}", dir.0.join("deps").display());
    let digest = format!("--nim-deps-sha256={}", "a".repeat(64));
    assert!(product(&[&build_arg, &deps]).contains("go together"));
    let lom = xtask::product_artifact::run(&[
        "lom".to_owned(),
        root.clone(),
        zero.clone(),
        out.clone(),
        build_arg.clone(),
        deps.clone(),
        digest.clone(),
    ])
    .unwrap_err();
    assert!(lom.contains("takes no --nim-deps"), "{lom}");
    fs::set_permissions(&build, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(product(&[&build_arg]).contains("private (0700)"));

    // The Hagia C SDK revision is required before any other option is
    // examined; supply it so the refusals below are the ones under test.
    let sdk_rev = format!("--hagia-c-sdk-rev={}", "8".repeat(40));
    let pair = xtask::wm_pair::run(&[
        "--hagia".to_owned(),
        root.clone(),
        zero.clone(),
        "--narthex".to_owned(),
        root.clone(),
        zero.clone(),
        out.clone(),
        sdk_rev.clone(),
    ])
    .unwrap_err();
    assert!(pair.contains("--build-dir is required"), "{pair}");
    fs::set_permissions(&build, fs::Permissions::from_mode(0o700)).unwrap();
    let pair = xtask::wm_pair::run(&[
        "--hagia".to_owned(),
        root.clone(),
        zero.clone(),
        "--narthex".to_owned(),
        root,
        zero,
        out,
        build_arg,
        format!("--hagia-nim-deps={}", dir.0.join("deps").display()),
        digest.replace("--nim", "--hagia-nim"),
        sdk_rev,
    ])
    .unwrap_err();
    assert!(
        pair.contains("--narthex-nim-deps and --narthex-nim-deps-sha256 are required"),
        "{pair}"
    );
}

#[test]
fn profile_probe_is_bound_and_required_by_the_package_selection() {
    let dir = Dir::new("physical-profile-probe");
    let (out, _) = prepared(&dir.0, &[]);
    let tree = xtask::git_tree::inventory(&out.join("sophia-tree"))
        .unwrap()
        .tree;
    let mut header = header_for_tests(&"2".repeat(40), &tree, &[]);
    let packages = header[2]
        .fields
        .iter_mut()
        .find(|(key, _)| key == "packages")
        .unwrap();
    packages.1 = "sophia-cli,sophia-conformance".into();
    write_env_for_tests(&out, &header).unwrap();
    let manifest = seal(&out, header.clone()).unwrap();
    fs::write(out.join("physical-inputs.manifest"), &manifest).unwrap();
    assert!(
        verify(&out, &sha256(manifest.as_bytes()))
            .unwrap_err()
            .contains("desktop_profile_probe")
    );

    let probe = out.join("bin/desktop_profile_probe");
    fs::write(&probe, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o555)).unwrap();
    let manifest = seal(&out, header).unwrap();
    fs::write(out.join("physical-inputs.manifest"), &manifest).unwrap();
    let digest = sha256(manifest.as_bytes());
    verify(&out, &digest).unwrap();
    assert!(
        fs::read_to_string(out.join("inputs.env"))
            .unwrap()
            .contains(&format!("SOPHIA_PROFILE_PROBE_BIN={}\n", probe.display()))
    );
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&probe, "#!/bin/sh\nexit 1\n").unwrap();
    assert!(verify(&out, &digest).is_err());
}
