//! Fixture inputs for the desktop release layout: stub executables that
//! answer the packaged policy verifier, a vendored C SDK snapshot whose
//! upstream commit really hashes to its revision, and a stand-in for the
//! pinned Sophia tree's retained session files. Nothing here builds or runs
//! a product.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use xtask::release::{Assembly, Binaries, SOPHIA_RETAINED, WmPair};

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

fn git_hash(kind: &str, bytes: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin", "-t", kind])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A valid vendored C SDK snapshot under `dir` (manifest.json,
/// upstream.commit and source/); returns its revision.
pub fn c_sdk_snapshot(dir: &Path) -> String {
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

/// Hagia and narthex stubs that answer the packaged policy verifier
/// (`hagia config check`), Hagia's default profile and a vendored C SDK
/// snapshot, all under `dir`.
pub fn wm_pair(dir: &Path) -> WmPair {
    fs::create_dir(dir).unwrap();
    for name in ["hagia", "narthex"] {
        script(
            &dir.join(name),
            &format!("# fixture {name}\n[[ \"$1 $2\" == \"config check\" ]]"),
        );
    }
    fs::write(dir.join("default.kdl"), "schema 1\n").unwrap();
    let sdk = dir.join("c-sdk");
    fs::create_dir(&sdk).unwrap();
    let revision = c_sdk_snapshot(&sdk);
    let digest = |name: &str| sha256(&fs::read(dir.join(name)).unwrap());
    WmPair {
        hagia_sha256: digest("hagia"),
        narthex_sha256: digest("narthex"),
        profile_sha256: digest("default.kdl"),
        hagia_commit: "3".repeat(40),
        narthex_commit: "4".repeat(40),
        hagia_c_sdk_revision: revision,
        hagia_c_sdk_manifest_sha256: digest("c-sdk/manifest.json"),
        hagia_c_sdk_manifest: sdk.join("manifest.json"),
        hagia: dir.join("hagia"),
        narthex: dir.join("narthex"),
        profile: dir.join("default.kdl"),
    }
}

/// Stub binaries, a stand-in staged Sophia tree and a WM pair under
/// `root`; the release goes to `root/release`.
pub fn assembly(root: &Path) -> Assembly {
    let bins = root.join("bins");
    fs::create_dir(&bins).unwrap();
    script(
        &bins.join("sophia"),
        "# fixture sophia\n[[ \"$1 $2\" == \"config check\" ]]",
    );
    for name in [
        "xtask",
        "active-session-preflight",
        "sophia-factotum",
        "sophia-factotum-pam",
    ] {
        script(&bins.join(name), &format!("# fixture {name}\nexit 0"));
    }
    let tree = root.join("sophia-tree");
    for (path, _) in SOPHIA_RETAINED {
        let file = tree.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, format!("# retained Sophia file {path}\n")).unwrap();
    }
    Assembly {
        repo: repo(),
        sophia_tree: tree,
        sophia_rev: "de776c68afdf9a133818f86917893c3362dc9fb7".into(),
        sophia_version: "0.1.0".into(),
        integration_commit: "2".repeat(40),
        binaries: Binaries {
            sophia: bins.join("sophia"),
            xtask: bins.join("xtask"),
            preflight: bins.join("active-session-preflight"),
            factotum: bins.join("sophia-factotum"),
            pam_helper: bins.join("sophia-factotum-pam"),
        },
        pair: wm_pair(&root.join("wm-pair")),
        out: root.join("release"),
        built_at_utc: "2026-09-27T00:00:00Z".into(),
        verifier_interpreter: None,
        release_id: "niltempus-fixture".into(),
        extra_files: Vec::new(),
    }
}
