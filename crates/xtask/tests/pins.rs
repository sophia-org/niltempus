//! Every pin names one Sophia revision; each way of drifting is refused.
use std::path::Path;
use xtask::pins::{
    SOPHIA_REV, SOPHIA_URL, check, check_lock, check_manifest, check_pin_file, parse_contracts,
};

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn text(name: &str) -> String {
    std::fs::read_to_string(repo().join(name)).unwrap()
}

const OTHER_REV: &str = "0123456789abcdef0123456789abcdef01234567";

fn lock(runtime: &str, protocol: &str, extra: &str) -> String {
    format!(
        "version = 4\n\n[[package]]\nname = \"live-tests\"\nversion = \"0.1.0\"\n\n\
         [[package]]\nname = \"sophia-protocol\"\nversion = \"0.1.0\"\nsource = \"{protocol}\"\n\n\
         [[package]]\nname = \"sophia-runtime\"\nversion = \"0.1.0\"\nsource = \"{runtime}\"\n{extra}"
    )
}

fn pinned_source() -> String {
    format!("git+{SOPHIA_URL}?rev={SOPHIA_REV}#{SOPHIA_REV}")
}

#[test]
fn the_committed_pins_agree() {
    check_pin_file(&text("pins/sophia.toml")).unwrap();
    check_manifest("Cargo.toml", &text("Cargo.toml"), true).unwrap();
    for name in [
        "crates/xtask/Cargo.toml",
        "crates/live-tests/Cargo.toml",
        ".cargo/config.toml",
    ] {
        check_manifest(name, &text(name), false).unwrap();
    }
    let source = pinned_source();
    check_lock(&lock(&source, &source, "")).unwrap();
    // Cargo.lock is committed after provisioning; the whole check then holds.
    if repo().join("Cargo.lock").exists() {
        check(repo()).unwrap();
    }
}

#[test]
fn pin_file_url_or_revision_drift_is_refused() {
    let good = text("pins/sophia.toml");
    for bad in [
        good.replace(SOPHIA_REV, OTHER_REV),
        good.replace(SOPHIA_URL, "https://github.com/example/sophia.git"),
        good.replace("rev = ", "branch = "),
        format!("{good}rev = \"{SOPHIA_REV}\"\n"),
        good.lines()
            .filter(|l| !l.starts_with("rev"))
            .collect::<Vec<_>>()
            .join("\n"),
    ] {
        assert!(check_pin_file(&bad).is_err(), "{bad}");
    }
}

#[test]
fn manifest_revision_or_redirection_drift_is_refused() {
    let good = text("Cargo.toml");
    let pinned = format!("rev = \"{SOPHIA_REV}\"");
    let bad_rev = good.replacen(&pinned, &format!("rev = \"{OTHER_REV}\""), 1);
    let bad_url = good.replacen(
        "https://github.com/sophia-org/sophia.git",
        "file:///elsewhere/sophia",
        1,
    );
    let branch = good.replacen(&pinned, "branch = \"master\"", 1);
    let path = good.replacen(
        &format!("sophia-runtime = {{ git = \"{SOPHIA_URL}\", {pinned} }}"),
        "sophia-runtime = { path = \"../sophia/crates/sophia-runtime\" }",
        1,
    );
    let missing = good
        .lines()
        .filter(|l| !l.starts_with("sophia-protocol"))
        .collect::<Vec<_>>()
        .join("\n");
    let patch = format!(
        "{good}\n[patch.\"{SOPHIA_URL}\"]\nsophia-runtime = {{ path = \"../sophia/crates/sophia-runtime\" }}\n"
    );
    let other_git = format!("{good}\nother = {{ git = \"https://example.org/x.git\" }}\n");
    let duplicate = format!(
        "{good}\nsophia-runtime = {{ git = \"{SOPHIA_URL}\", {pinned} }}\n"
    );
    for (what, bad) in [
        ("rev", bad_rev),
        ("url", bad_url),
        ("branch", branch),
        ("path", path),
        ("missing crate", missing),
        ("patch", patch),
        ("other git", other_git),
        ("duplicate", duplicate),
    ] {
        assert_ne!(bad, good, "{what} fixture did not change the manifest");
        assert!(
            check_manifest("Cargo.toml", &bad, true).is_err(),
            "{what}: {bad}"
        );
    }
    // A member cannot name Sophia itself; it inherits the root pin.
    let member = "[dev-dependencies]\nsophia-runtime = { git = \"https://github.com/sophia-org/sophia.git\", rev = \"9fcaec782ce4fe9978568c0466ee17a78b3d4571\" }\n";
    assert!(check_manifest("member", member, false).is_err());
    let absolute = "[dependencies]\nhelper = { path = \"/home/user/helper\" }\n";
    assert!(check_manifest("member", absolute, false).is_err());
    let config = "[source.crates-io]\nreplace-with = \"local\"\n";
    assert!(check_manifest(".cargo/config.toml", config, false).is_err());
    let paths = "paths = [\"../sophia\"]\n";
    assert!(check_manifest(".cargo/config.toml", paths, false).is_err());
}

#[test]
fn lock_revision_or_source_drift_is_refused() {
    let good = pinned_source();
    let query = format!("git+{SOPHIA_URL}?rev={OTHER_REV}#{SOPHIA_REV}");
    let fragment = format!("git+{SOPHIA_URL}?rev={SOPHIA_REV}#{OTHER_REV}");
    let url = format!("git+file:///elsewhere/sophia?rev={SOPHIA_REV}#{SOPHIA_REV}");
    let branch = format!("git+{SOPHIA_URL}?branch=master#{SOPHIA_REV}");
    let registry = "registry+https://github.com/rust-lang/crates.io-index".to_owned();
    for (what, bad) in [
        ("query rev", lock(&query, &good, "")),
        ("fragment rev", lock(&good, &fragment, "")),
        ("url", lock(&url, &good, "")),
        ("branch", lock(&branch, &good, "")),
        ("registry", lock(&registry, &good, "")),
        (
            "path package",
            lock(
                &good,
                &good,
                "\n[[package]]\nname = \"sophia-9p\"\nversion = \"0.1.0\"\n",
            ),
        ),
        (
            "other git",
            lock(
                &good,
                &good,
                "\n[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nsource = \"git+https://example.org/x.git#0123\"\n",
            ),
        ),
        (
            "missing runtime",
            lock(&good, &good, "").replace("name = \"sophia-runtime\"", "name = \"x\""),
        ),
    ] {
        assert!(check_lock(&bad).is_err(), "{what}: {bad}");
    }
}

#[test]
fn contract_bindings_cannot_be_dropped_or_rebound() {
    let good = text("pins/contracts.sha256");
    assert_eq!(parse_contracts(&good).unwrap().len(), 18);
    let dropped = good.lines().skip(1).collect::<Vec<_>>().join("\n");
    let rebound = good.replacen(
        "protocol/sophia-shell-v1.kdl",
        "protocol/sophia-shell-files-v1.kdl",
        1,
    );
    let malformed = good.replacen(' ', "  ", 1);
    let escaping = good.replacen("docs/sophia-wm-files.md", "../sophia-wm-files.md", 1);
    for bad in [dropped, rebound, malformed, escaping] {
        assert!(parse_contracts(&bad).is_err(), "{bad}");
    }
}
