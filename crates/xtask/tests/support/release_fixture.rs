//! Fixture inputs for the desktop packager: a prepared WM pair whose commit
//! objects really hash to their commits, stub executables that answer the
//! packaged policy verifier, and a stand-in for the staged pinned Sophia
//! tree's retained session files. Nothing here builds or runs a product.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use xtask::package_desktop::{Assembly, Binaries, SOPHIA_RETAINED};
use xtask::wm_pair::VerifiedPair;

pub struct Dir(pub PathBuf);
impl Dir {
    pub fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "desktop-release-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = Command::new("chmod")
            .arg("-R")
            .arg("u+w")
            .arg(&self.0)
            .status();
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn repo() -> PathBuf {
    fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
}

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

pub fn script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/usr/bin/env bash\nset -euo pipefail\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A raw signed-looking commit object and its real Git identity.
pub fn commit_object(label: &str) -> (Vec<u8>, String) {
    let raw = format!(
        "tree {}\nauthor A U Thor <a@example.com> 0 +0000\n\
         committer A U Thor <a@example.com> 0 +0000\n\
         gpgsig -----BEGIN PGP SIGNATURE-----\n fixture\n -----END PGP SIGNATURE-----\n\n{label}\n",
        "1".repeat(40)
    )
    .into_bytes();
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin", "-t", "commit"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(&raw).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    (
        raw,
        String::from_utf8(output.stdout).unwrap().trim().to_owned(),
    )
}

/// Identities of a fixture pair: (hagia commit, narthex commit, hagia sha,
/// narthex sha).
pub struct PairIds {
    pub commits: [String; 2],
    pub digests: [String; 2],
    pub profile: String,
}

/// The fixture's reviewed installation-config and stdlib digests.
pub const NIM_CONFIG_SHA256: &str =
    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
pub const NIM_STDLIB_SHA256: &str =
    "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

/// A reviewed dependency manifest for a fixture half (no packages; the
/// toolchain is not probed by the pair verifier, which only cross-checks the
/// installation-config and stdlib identities).
pub fn reviewed_deps(product: &str, commit: &str) -> String {
    let tree = |role: &str, path: &str, digest: &str| {
        xtask::records::Record::of("tree")
            .with("role", role)
            .with("path", path)
            .with("files", "1")
            .with("inventory_sha256", digest)
    };
    xtask::nim_deps::Manifest {
        status: "reviewed".into(),
        product: product.into(),
        source_commit: commit.into(),
        source_tree: "1".repeat(40),
        store: PathBuf::from("/nonexistent-fixture-store"),
        toolchain: vec![
            tree("nim-config", "/usr/lib/nim/config", NIM_CONFIG_SHA256),
            tree("nim-lib", "/usr/lib/nim/lib", NIM_STDLIB_SHA256),
        ],
        packages: Vec::new(),
    }
    .render()
    .unwrap()
}

/// A prepared-pair directory in `prepare-wm-pair`'s layout (schema 2).
/// The SDK revision the fixture pair's Hagia vendors.
pub const HAGIA_C_SDK_REV: &str = "841563d614ed8540472f0edfa7f4cddaafe3fdde";

pub fn write_pair(dir: &Path) -> PairIds {
    fs::create_dir(dir).unwrap();
    let mut commits = Vec::new();
    let mut digests = Vec::new();
    let mut trees = Vec::new();
    let mut deps = Vec::new();
    for name in ["hagia", "narthex"] {
        let (raw, commit) = commit_object(name);
        fs::write(dir.join(format!("{name}.commit")), &raw).unwrap();
        // The packaged policy verifier runs `hagia config check`.
        script(
            &dir.join(name),
            &format!("# fixture {name}\n[[ \"$1 $2\" == \"config check\" ]]"),
        );
        digests.push(sha256(&fs::read(dir.join(name)).unwrap()));
        let manifest = reviewed_deps(name, &commit);
        fs::write(dir.join(format!("{name}-nim-deps.manifest")), &manifest).unwrap();
        deps.push(sha256(manifest.as_bytes()));
        commits.push(commit);
        trees.push("1".repeat(40));
    }
    let profile = b"schema 1\n";
    fs::write(dir.join("default.kdl"), profile).unwrap();
    // Hagia's vendored C SDK manifest, carried into the pair.
    let sdk = format!(
        "{{\"schema\":1,\"repository\":\"https://github.com/sophia-org/sophia-desktop-sdk-c\",\"revision\":\"{HAGIA_C_SDK_REV}\",\"files\":{{\"README.md\":\"{}\"}}}}\n",
        "a".repeat(64)
    );
    fs::write(dir.join("hagia-c-sdk.manifest.json"), &sdk).unwrap();
    let manifest = [
        "schema=3".to_owned(),
        format!("hagia_source_commit={}", commits[0]),
        format!("hagia_source_tree={}", trees[0]),
        "hagia_signer_fingerprint=ABCDEF0123".to_owned(),
        format!("hagia_binary_sha256={}", digests[0]),
        format!("narthex_source_commit={}", commits[1]),
        format!("narthex_source_tree={}", trees[1]),
        "narthex_signer_fingerprint=ABCDEF0123".to_owned(),
        format!("narthex_binary_sha256={}", digests[1]),
        "default_profile=default.kdl".to_owned(),
        "default_profile_source=examples/config/default.kdl".to_owned(),
        format!("default_profile_sha256={}", sha256(profile)),
        format!("hagia_nim_deps_sha256={}", deps[0]),
        format!("narthex_nim_deps_sha256={}", deps[1]),
        "toolchain_identity=recorded-not-a-reproducible-closure".to_owned(),
    ]
    .into_iter()
    .chain(["hagia", "narthex"].into_iter().flat_map(|name| {
        [
            format!("{name}_nim_config_sha256={NIM_CONFIG_SHA256}"),
            format!("{name}_nim_config_read=config/nim.cfg:{}", "e".repeat(64)),
            format!("{name}_nim_stdlib_sha256={NIM_STDLIB_SHA256}"),
            format!("{name}_nim_command=/b/nim/bin/nim c -d:release src/{name}.nim"),
        ]
    }))
    .chain([
        format!("hagia_c_sdk_revision={HAGIA_C_SDK_REV}"),
        format!("hagia_c_sdk_manifest_sha256={}", sha256(sdk.as_bytes())),
    ])
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(dir.join("wm-pair.manifest"), manifest).unwrap();
    PairIds {
        commits: [commits[0].clone(), commits[1].clone()],
        digests: [digests[0].clone(), digests[1].clone()],
        profile: sha256(profile),
    }
}

pub fn verified_pair(dir: &Path, ids: &PairIds) -> VerifiedPair {
    xtask::wm_pair::verify(
        dir,
        [ids.commits[0].as_str(), ids.commits[1].as_str()],
        [ids.digests[0].as_str(), ids.digests[1].as_str()],
        &ids.profile,
    )
    .unwrap()
}

/// Stub binaries, a stand-in staged Sophia tree and a verified pair under
/// `root`; the release goes to `root/release`.
pub fn assembly(root: &Path) -> Assembly {
    let bins = root.join("bins");
    fs::create_dir(&bins).unwrap();
    script(
        &bins.join("sophia"),
        "# fixture sophia\n[[ \"$1 $2\" == \"config check\" ]]",
    );
    for name in ["sophia-wm-demo", "xtask", "active-session-preflight"] {
        script(&bins.join(name), &format!("# fixture {name}\nexit 0"));
    }
    let tree = root.join("sophia-tree");
    for (path, _) in SOPHIA_RETAINED {
        let file = tree.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, format!("# retained Sophia file {path}\n")).unwrap();
    }
    let ids = write_pair(&root.join("wm-pair"));
    Assembly {
        repo: repo(),
        sophia_tree: tree,
        sophia_rev: "de776c68afdf9a133818f86917893c3362dc9fb7".into(),
        sophia_version: "0.1.0".into(),
        integration_commit: "2".repeat(40),
        binaries: Binaries {
            sophia: bins.join("sophia"),
            sophia_wm_demo: bins.join("sophia-wm-demo"),
            xtask: bins.join("xtask"),
            preflight: bins.join("active-session-preflight"),
        },
        pair: verified_pair(&root.join("wm-pair"), &ids),
        out: root.join("release"),
        built_at_utc: "2026-09-27T00:00:00Z".into(),
    }
}
