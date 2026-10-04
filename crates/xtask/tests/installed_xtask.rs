//! The recipe tool a release seals (target/release/sophia-integration-xtask)
//! is the REAL xtask binary, and it must run `session-recipe` with its build
//! checkout and build directory gone. Bubblewrap hides both (tmpfs over them)
//! while the copied binary stays visible; a repository-bound command in the
//! same sandbox fails, proving the checkout really is absent.
//!
//! Requires /usr/bin/bwrap (unprivileged user namespaces); the test fails
//! rather than skips without it.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BWRAP: &str = "/usr/bin/bwrap";

#[path = "support/release_fixture.rs"]
mod fixture;

struct Release(PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn installed_binary() -> (Release, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "installed-xtask-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(dir.join("target/release")).unwrap();
    let binary = dir.join("target/release/sophia-integration-xtask");
    fs::copy(env!("CARGO_BIN_EXE_xtask"), &binary).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    (Release(dir), binary)
}

/// Run `binary args` with the checkout and the original build output hidden.
fn hidden(binary: &Path, args: &[&str]) -> Output {
    assert!(Path::new(BWRAP).is_file(), "{BWRAP} is required");
    let checkout = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    let build = fs::canonicalize(Path::new(env!("CARGO_BIN_EXE_xtask")))
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(!binary.starts_with(&checkout) && !binary.starts_with(&build));
    let mut command = Command::new(BWRAP);
    command
        .args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"])
        .arg("--tmpfs")
        .arg(&checkout);
    if !build.starts_with(&checkout) {
        command.arg("--tmpfs").arg(build);
    }
    command
        .args(["--die-with-parent", "--"])
        .arg("/usr/bin/env")
        .args(["-i", "PATH=/usr/bin:/bin"])
        .arg(binary)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn the_installed_recipe_tool_runs_with_its_checkout_hidden() {
    let (_release, binary) = installed_binary();

    // Control: the checkout really is absent inside the sandbox.
    let control = hidden(&binary, &["check-pins"]);
    assert!(!control.status.success(), "{control:?}");

    let output = hidden(
        &binary,
        &[
            "session-recipe",
            "prepare-environment",
            "--firefox-probe=/probe",
            "--",
            "--firefox-m10-proof",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let fields = output
        .stdout
        .split(|b| *b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        fields[0],
        "sophia_desktop_recipe_environment schema=1 status=prepared"
    );
    assert!(fields.contains(&"SOPHIA_FIREFOX_M10_KITTY_PROBE_DIR=/probe".to_owned()));

    // Discovery for the managed profile, too.
    let output = hidden(
        &binary,
        &[
            "session-recipe",
            "prepare-inputs",
            "--profile=hagia",
            "--root=/nonexistent-release",
            "--",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert!(
        output
            .stdout
            .starts_with(b"sophia_session_inputs schema=1 status=prepared\0")
    );
}

#[test]
fn installed_product_builder_dispatch_is_independent_of_its_checkout() {
    let (_release, binary) = installed_binary();
    let output = hidden(&binary, &["prepare-product-artifact"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("usage: cargo xtask prepare-product-artifact"),
        "{error}"
    );
    assert!(!error.contains("No such file or directory"), "{error}");
}
