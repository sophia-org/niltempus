//! The `cargo xtask` alias (`run --package xtask --`) must resolve to the
//! xtask binary. The package ships a second binary (active-session-preflight),
//! so without `default-run` the alias fails as ambiguous; this pins both
//! halves: the alias as written and the manifest's default.
use std::path::Path;

#[test]
fn the_cargo_xtask_alias_resolves_to_the_xtask_binary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config = std::fs::read_to_string(root.join(".cargo/config.toml")).unwrap();
    assert!(
        config
            .lines()
            .any(|line| line.trim() == r#"xtask = "run --package xtask --""#),
        "{config}"
    );
    let manifest = std::fs::read_to_string(root.join("crates/xtask/Cargo.toml")).unwrap();
    let package = manifest.split("\n[").next().unwrap();
    assert!(
        package
            .lines()
            .any(|line| line.trim() == r#"default-run = "xtask""#),
        "{manifest}"
    );
    // Both binaries exist, which is exactly why the default is required.
    assert!(Path::new(env!("CARGO_BIN_EXE_xtask")).is_file());
    assert!(Path::new(env!("CARGO_BIN_EXE_active-session-preflight")).is_file());
}
