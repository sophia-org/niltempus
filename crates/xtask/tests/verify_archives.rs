//! `cargo xtask verify-archives`: a missing or empty family is reported absent
//! and never fails; every other listing, entry or metadata error fails with
//! its path; a run is a real directory (symlinks, dangling or not, are
//! refused, never followed); a run that no longer verifies stops the gate.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use xtask::verify_archives::{runs, verify};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Temp(PathBuf);
impl Temp {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "verify-archives-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::process::Command::new("chmod")
            .args(["-R", "u+rwx"])
            .arg(&self.0)
            .status();
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn running_as_root() -> bool {
    rustix::process::geteuid().is_root()
}

#[test]
fn missing_or_empty_families_are_reported_not_failed() {
    let root = Temp::new("absent");
    fs::create_dir(root.0.join("mirror-group-runs")).unwrap();
    assert_eq!(
        verify(&repo(), &root.0, false).unwrap(),
        "archives: (absent: hagia-native-runs mirror-group-runs)"
    );
}

#[test]
fn a_family_path_that_is_a_regular_file_is_refused() {
    let root = Temp::new("enotdir");
    let family = root.0.join("mirror-group-runs");
    fs::write(&family, "not a directory\n").unwrap();
    let error = verify(&repo(), &root.0, false).unwrap_err();
    assert!(error.contains(&family.display().to_string()), "{error}");
}

#[test]
fn an_unreadable_family_root_is_refused() {
    if running_as_root() {
        eprintln!("skipped: root bypasses directory permissions, so EACCES cannot be created");
        return;
    }
    let root = Temp::new("eacces");
    let family = root.0.join("hagia-native-runs");
    fs::create_dir(&family).unwrap();
    fs::create_dir(family.join("0001")).unwrap();
    fs::set_permissions(&family, fs::Permissions::from_mode(0o000)).unwrap();
    let error = verify(&repo(), &root.0, false).unwrap_err();
    assert!(error.contains(&family.display().to_string()), "{error}");
    assert!(error.contains("Permission denied"), "{error}");
}

#[test]
fn an_unreadable_run_directory_fails_verification() {
    if running_as_root() {
        eprintln!("skipped: root bypasses directory permissions, so EACCES cannot be created");
        return;
    }
    let root = Temp::new("run-eacces");
    let run = root.0.join("mirror-group-runs/0001");
    fs::create_dir_all(&run).unwrap();
    fs::set_permissions(&run, fs::Permissions::from_mode(0o000)).unwrap();
    let error = verify(&repo(), &root.0, false).unwrap_err();
    assert!(error.contains("no longer verifies"), "{error}");
    assert!(error.contains(&run.display().to_string()), "{error}");
}

#[test]
fn symlinked_dangling_and_non_directory_entries_are_refused() {
    let root = Temp::new("entries");
    let family = root.0.join("mirror-group-runs");
    fs::create_dir(&family).unwrap();
    let elsewhere = root.0.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();

    std::os::unix::fs::symlink(&elsewhere, family.join("0001")).unwrap();
    let error = runs(&family).unwrap_err();
    assert!(error.contains("is a symlink"), "{error}");
    fs::remove_file(family.join("0001")).unwrap();

    std::os::unix::fs::symlink(root.0.join("missing"), family.join("0002")).unwrap();
    let error = runs(&family).unwrap_err();
    assert!(error.contains("is a symlink"), "{error}");
    fs::remove_file(family.join("0002")).unwrap();

    fs::write(family.join("stray"), "x\n").unwrap();
    let error = runs(&family).unwrap_err();
    assert!(error.contains("is not a directory"), "{error}");
    fs::remove_file(family.join("stray")).unwrap();

    fs::create_dir(family.join("0003")).unwrap();
    assert_eq!(runs(&family).unwrap(), Some(vec![family.join("0003")]));
    assert_eq!(runs(&root.0.join("missing-family")).unwrap(), None);
}

#[test]
fn a_run_that_no_longer_verifies_stops_with_its_path() {
    let root = Temp::new("broken");
    let run = root.0.join("mirror-group-runs/0001");
    fs::create_dir_all(&run).unwrap();
    fs::write(run.join("SHA256SUMS"), "0  missing\n").unwrap();
    for legacy in [false, true] {
        let error = verify(&repo(), &root.0, legacy).unwrap_err();
        assert!(error.contains("no longer verifies"), "{error}");
        assert!(error.contains(&run.display().to_string()), "{error}");
    }
}
