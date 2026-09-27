//! `cargo xtask verify-archives`: absent families are reported and never fail;
//! a run that no longer verifies stops the gate with its path.
use std::fs;
use std::path::Path;
use xtask::verify_archives::verify;

fn repo() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn temp(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "verify-archives-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn absent_families_are_reported_not_failed() {
    let root = temp("absent");
    fs::create_dir(root.join("mirror-group-runs")).unwrap();
    assert_eq!(
        verify(&repo(), &root).unwrap(),
        "archives: (absent: hagia-native-runs mirror-group-runs)"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_run_that_no_longer_verifies_stops_with_its_path() {
    let root = temp("broken");
    let run = root.join("mirror-group-runs/0001");
    fs::create_dir_all(&run).unwrap();
    fs::write(run.join("SHA256SUMS"), "0  missing\n").unwrap();
    let error = verify(&repo(), &root).unwrap_err();
    assert!(error.contains("no longer verifies"), "{error}");
    assert!(error.contains(&run.display().to_string()), "{error}");
    fs::remove_dir_all(root).unwrap();
}
