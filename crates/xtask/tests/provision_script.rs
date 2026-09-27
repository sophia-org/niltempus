//! tools/provision.sh up to its first cargo call, offline: a stub `cargo` on
//! PATH records its arguments and exits, so nothing is fetched or built. A
//! fresh checkout has no (git-ignored) .provision directory; provisioning
//! must create it before its first write there.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy(repo: &Path, root: &Path, name: &str) {
    let target = root.join(name);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::copy(repo.join(name), target).unwrap();
}

#[test]
fn a_fresh_checkout_without_provision_dir_reaches_cargo_offline() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch =
        Scratch(std::env::temp_dir().join(format!("provision-script-{}", std::process::id())));
    let root = scratch.0.join("checkout");
    for name in ["tools/provision.sh", "pins/sophia.toml", "Cargo.lock"] {
        copy(&repo, &root, name);
    }
    assert!(!root.join(".provision").exists());
    let bin = scratch.0.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let calls = scratch.0.join("cargo-calls");
    let stub = bin.join("cargo");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 42\n",
            calls.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).unwrap();
    let home = scratch.0.join("cargo-home");
    let seed = scratch.0.join("empty-seed");
    fs::create_dir_all(&seed).unwrap();
    let output = Command::new("sh")
        .arg(root.join("tools/provision.sh"))
        .arg("--cargo-home")
        .arg(&home)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("SOPHIA_PROVISION_SEED_REGISTRY", &seed)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The stub's refusal is the only failure: every path preflight passed,
    // .provision was created and written, and cargo was reached exactly once.
    assert!(!output.status.success(), "{stderr}");
    assert!(!stderr.contains("Directory nonexistent"), "{stderr}");
    assert!(root.join(".provision").is_dir());
    assert!(root.join(".provision/provision.log").is_file());
    assert!(!root.join(".provision/accepted").exists());
    let calls = fs::read_to_string(&calls).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        ["fetch --locked"],
        "{calls}"
    );
}

#[test]
fn a_cargo_home_inside_the_checkout_is_refused_before_any_cargo_call() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = Scratch(
        std::env::temp_dir().join(format!("provision-script-inside-{}", std::process::id())),
    );
    let root = scratch.0.join("checkout");
    for name in ["tools/provision.sh", "pins/sophia.toml", "Cargo.lock"] {
        copy(&repo, &root, name);
    }
    let bin = scratch.0.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let calls = scratch.0.join("cargo-calls");
    let stub = bin.join("cargo");
    fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
            calls.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new("sh")
        .arg(root.join("tools/provision.sh"))
        .arg("--cargo-home")
        .arg(root.join(".provision/cargo-home"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside this repository"));
    assert!(!calls.exists(), "cargo was called");
}
