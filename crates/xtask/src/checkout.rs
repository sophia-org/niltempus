//! Shared custody helpers for builds from explicit checkouts (physical
//! inputs, product artifacts and the Nim dependency tool): private build
//! directories, clean signed checkouts, the provisioned CARGO_HOME and a
//! bounded offline cargo build.
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::bemenu_artifact::{bounded, tail, text, wait_logged};
use crate::{pins, read, sha256};

const BUILD_TIMEOUT: Duration = Duration::from_secs(5400);
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Absolute path with `.`/`..` removed, without following links.
pub(crate) fn resolve_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    // Resolve the deepest existing ancestor so symlinked parents compare
    // against the real source trees.
    let mut existing = out.clone();
    let mut rest = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => rest.push(name.to_owned()),
            None => break,
        }
        existing.pop();
    }
    let mut resolved = std::fs::canonicalize(&existing).unwrap_or(existing);
    for name in rest.into_iter().rev() {
        resolved.push(name);
    }
    resolved
}

/// An existing directory owned by this user, not a link, private (0700).
pub(crate) fn private_dir(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("--build-dir {}: {e}", path.display()))?;
    if !meta.is_dir()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(format!(
            "--build-dir must be a private (0700) directory owned by this user: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn git_text(repo: &Path, args: &[&str]) -> Result<String, String> {
    text(bounded(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0"),
        GIT_TIMEOUT,
        &format!("git {}", args.first().copied().unwrap_or("")),
    )?)
}

pub(crate) fn clean_checkout(repo: &Path, what: &str) -> Result<(), String> {
    let status = git_text(repo, &["status", "--porcelain", "--untracked-files=normal"])
        .map_err(|e| format!("{what} {} is not a readable checkout: {e}", repo.display()))?;
    if !status.is_empty() {
        return Err(format!("{what} must be clean: {}", repo.display()));
    }
    Ok(())
}

/// The full provisioning marker check (pins::check_marker: canonical URL,
/// pinned revision, this checkout's Cargo.lock digest) plus the caller's
/// CARGO_HOME, which must be the provisioned one. Returns (home, lock digest).
pub(crate) fn provisioned(
    repo: &Path,
    cargo_home: Option<&str>,
) -> Result<(PathBuf, String), String> {
    let marker =
        String::from_utf8(read(&repo.join(pins::PROVISION_MARKER))?).map_err(|e| e.to_string())?;
    let lock = sha256(&read(&repo.join("Cargo.lock"))?);
    let home = cargo_home
        .filter(|home| home.starts_with('/'))
        .ok_or("CARGO_HOME must name the provisioned private CARGO_HOME")?;
    pins::check_marker(&marker, &lock, Some(home))?;
    Ok((PathBuf::from(home), lock))
}

/// Offline, locked release build at the caller's priority with
/// [`crate::product_artifact::build_jobs`] jobs, in a private process group and bounded by time and log size.
pub(crate) fn cargo(
    dir: &Path,
    target: &Path,
    cargo_home: Option<&Path>,
    packages: &[&str],
    log: &Path,
    what: &str,
) -> Result<(), String> {
    let log_file = File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let jobs = crate::product_artifact::build_jobs()?;
    let mut command = Command::new("cargo");
    command
        .args(["build", "--offline", "--locked", "--release"])
        .args(["--jobs", &jobs])
        .args(packages)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_BUILD_JOBS", &jobs)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|e| e.to_string())?)
        .stderr(log_file);
    if let Some(home) = cargo_home {
        command.env("CARGO_HOME", home);
    }
    let child = command.spawn().map_err(|e| format!("{what}: {e}"))?;
    wait_logged(child, log, BUILD_TIMEOUT, what).map_err(|e| format!("{e}\n{}", tail(log)))
}
