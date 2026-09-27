// Provenance: moved from Sophia crates/xtask/src/bemenu_artifact.rs at
// 9fcaec782ce4fe9978568c0466ee17a78b3d4571 (Sophia rule 13). The SDK pin is
// now pins/c-desktop-sdk rather than Sophia's vendored snapshot.
//! Prepare an immutable `bemenu-sophia` artifact from one signed revision for
//! the live launcher gate (crates/live-tests/tests/bemenu_files.rs).
//!
//! Signer AUTHORIZATION happens only here: `git verify-commit` must pass and
//! report status G. The live gate later re-checks content-to-revision BINDING
//! (commit object hash, tree, binary SHA-256, SDK pin) and never consults a
//! keyring. The source checkout is only read: the build runs in a fresh
//! scratch tree extracted from `git archive` of the signed commit, and that
//! tree must hash to exactly the commit's tree before anything is compiled.
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, WaitIdStatus, kill_process_group};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::{hex, read, sha256};

const USAGE: &str =
    "usage: cargo xtask prepare-bemenu-artifact <source-repo> <signed-commit> <new-output-dir>";
const GIT_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const BUILD_TIMEOUT: Duration = Duration::from_secs(900);
const OUTPUT_CAP: u64 = 1 << 20;
const PIPE_GRACE: Duration = Duration::from_secs(2);
const GROUP_GRACE: Duration = Duration::from_secs(2);
const BUILD_LOG_CAP: u64 = 4 << 20;
const BINARY: &str = "bemenu-sophia";
const MANIFEST: &str = "bemenu-artifact.manifest";
const COMMIT_OBJECT: &str = "source.commit";

pub(crate) struct RemoveOnDrop(pub(crate) PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `repo` is this repository (for pins/); `args` are the three explicit inputs.
pub fn run(repo: &Path, args: &[String]) -> Result<Vec<String>, String> {
    let [source, commit, output] = args else {
        return Err(USAGE.into());
    };
    let (source, output) = inputs(source, commit, output)?;
    let SignedTree {
        scratch,
        _scratch,
        tree_dir,
        raw,
        tree,
        signer,
    } = signed_tree(&source, commit, "bemenu")?;

    // SDK pin: the Bemenu snapshot is the same audited SDK revision Sophia
    // pinned at the integration revision (pins/c-desktop-sdk).
    let bemenu_sdk = tree_dir.join("vendor/sophia-desktop-sdk");
    let sdk = crate::c_sdk_pin::verify(&bemenu_sdk, repo)
        .map_err(|e| format!("Bemenu SDK snapshot: {e}"))?;
    let sdk_manifest = read(&bemenu_sdk.join("manifest.json"))?;
    if sdk_manifest != read(&repo.join(crate::pins::SDK_MANIFEST))? {
        return Err("SDK manifest differs between Bemenu and the pin".into());
    }

    // Low-priority, two-job build inside the scratch tree only.
    let log = scratch.join("build.log");
    let log_file = File::create(&log).map_err(|e| e.to_string())?;
    // CFLAGS/CPPFLAGS/LDFLAGS from the caller would replace the recipe's
    // `?=` warning set, so they are removed; EXTRA_WARNINGS makes it fatal.
    let child = Command::new("nice")
        .args(["-n", "19", "make", "-j2", "EXTRA_WARNINGS=-Werror"])
        .arg(format!("GIT_SHA1={commit}"))
        .arg(BINARY)
        .current_dir(&tree_dir)
        .process_group(0)
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .env_remove("MAKELEVEL")
        .env_remove("CFLAGS")
        .env_remove("CPPFLAGS")
        .env_remove("LDFLAGS")
        .env_remove("EXTRA_WARNINGS")
        .env("GIT_DIR", scratch.join("no-git"))
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|e| e.to_string())?)
        .stderr(log_file)
        .spawn()
        .map_err(|e| format!("make: {e}"))?;
    let built = wait_logged(child, &log, BUILD_TIMEOUT, &format!("make {BINARY}"));
    if let Err(error) = built {
        return Err(format!("{error}\n{}", tail(&log)));
    }
    let binary = tree_dir.join(BINARY);
    if !std::fs::symlink_metadata(&binary).is_ok_and(|m| m.is_file()) {
        return Err(format!("build produced no regular {BINARY}"));
    }

    // Immutable output: created last, removed again if any step fails.
    std::fs::create_dir(&output).map_err(|e| format!("{}: {e}", output.display()))?;
    let written = write_output(&output, &binary, &raw, |binary_sha256| {
        [
            "schema=1".to_owned(),
            format!("binary={BINARY}"),
            format!("binary_sha256={binary_sha256}"),
            format!("source_commit={commit}"),
            format!("source_tree={tree}"),
            "signature_status=G".to_owned(),
            format!("signer_fingerprint={signer}"),
            format!("sdk_revision={sdk}"),
            format!("sdk_manifest_sha256={}", sha256(&sdk_manifest)),
        ]
        .join("\n")
            + "\n"
    });
    match written {
        Ok(binary_sha256) => Ok(vec![format!(
            "bemenu_artifact status=prepared commit={commit} binary_sha256={binary_sha256} signer={signer} sdk={sdk} dir={}",
            output.display()
        )]),
        Err(error) => {
            let _ = set_mode(&output, 0o700);
            let _ = std::fs::remove_dir_all(&output);
            Err(error)
        }
    }
}

/// Refuse ambiguous inputs before anything is read: an exact 40-hex commit,
/// an existing source and a new output under an existing parent.
pub(crate) fn inputs(
    source: &str,
    commit: &str,
    output: &str,
) -> Result<(PathBuf, PathBuf), String> {
    if !hex(commit, 40) {
        return Err(format!(
            "signed commit must be 40 lowercase hex: {commit:?}"
        ));
    }
    let source = std::fs::canonicalize(source).map_err(|e| format!("{source}: {e}"))?;
    let output = absolute(Path::new(output))?;
    if output.symlink_metadata().is_ok() {
        return Err(format!("output already exists: {}", output.display()));
    }
    let parent = output.parent().ok_or("output has no parent directory")?;
    if !parent.is_dir() {
        return Err(format!(
            "output parent is not a directory: {}",
            parent.display()
        ));
    }
    Ok((source, output))
}

/// The signed commit's exact tree, extracted into a private scratch directory
/// that is removed when this value is dropped.
pub(crate) struct SignedTree {
    pub(crate) scratch: PathBuf,
    pub(crate) _scratch: RemoveOnDrop,
    pub(crate) tree_dir: PathBuf,
    pub(crate) raw: Vec<u8>,
    pub(crate) tree: String,
    pub(crate) signer: String,
}

/// Signer authorization (verify-commit, status G) read-only against the
/// source repository, then `git archive` of that commit whose extracted tree
/// must hash to exactly the commit's tree.
pub(crate) fn signed_tree(source: &Path, commit: &str, label: &str) -> Result<SignedTree, String> {
    let signer = authorize(source, commit)?;
    archive_tree(source, commit, label, signer)
}

/// `signed_tree` with its scratch in a new, randomly named, private (0700)
/// directory under `parent` (a caller's private build directory) instead of
/// the system temporary directory.
pub(crate) fn signed_tree_under(
    parent: &Path,
    source: &Path,
    commit: &str,
    label: &str,
) -> Result<SignedTree, String> {
    let signer = authorize(source, commit)?;
    archive_tree_under(parent, source, commit, label, signer)
}

/// A new private (0700) directory `parent/<label>-<128 random bits>`,
/// created exclusively: never a predictable or pre-existing path.
pub(crate) fn private_scratch(parent: &Path, label: &str) -> Result<PathBuf, String> {
    let mut random = [0u8; 16];
    rustix::rand::getrandom(&mut random, rustix::rand::GetRandomFlags::empty())
        .map_err(|e| format!("getrandom: {e}"))?;
    let suffix = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let scratch = parent.join(format!("{label}-{suffix}"));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&scratch)
        .map_err(|e| format!("{}: {e}", scratch.display()))?;
    Ok(scratch)
}

/// Signer authorization, read-only against the source repository: the
/// commit resolves to exactly itself, `git verify-commit` passes with status
/// G, and the raw object carries a gpgsig header. Returns the signer.
pub(crate) fn authorize(source: &Path, commit: &str) -> Result<String, String> {
    let resolved = text(git(
        source,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{commit}^{{commit}}"),
        ],
    )?)?;
    if resolved.trim() != commit {
        return Err(format!("{commit} does not name that exact commit"));
    }
    git(source, &["verify-commit", commit])
        .map_err(|e| format!("signature authorization failed for {commit}: {e}"))?;
    let status = text(git(source, &["log", "-1", "--format=%G?%n%GF", commit])?)?;
    let mut status = status.lines();
    let (signature, signer) = (status.next().unwrap_or(""), status.next().unwrap_or(""));
    if signature != "G" || signer.is_empty() || !signer.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "commit {commit} is not a good signature: status={signature:?} signer={signer:?}"
        ));
    }
    let raw = git(source, &["cat-file", "commit", commit])?;
    if !raw
        .split(|b| *b == b'\n')
        .any(|line| line.starts_with(b"gpgsig "))
    {
        return Err("signed commit object has no gpgsig header".into());
    }
    Ok(signer.to_owned())
}

/// `git archive` of `commit` into a private scratch directory whose tree
/// must hash to exactly the commit's tree (tracked files only: nothing
/// ignored or untracked in the checkout can enter). This proves content,
/// not authorization; callers authorize the signer first (`authorize`).
pub(crate) fn archive_tree(
    source: &Path,
    commit: &str,
    label: &str,
    signer: String,
) -> Result<SignedTree, String> {
    archive_tree_under(&std::env::temp_dir(), source, commit, label, signer)
}

/// `archive_tree` with its scratch under `parent` (`private_scratch`).
pub(crate) fn archive_tree_under(
    parent: &Path,
    source: &Path,
    commit: &str,
    label: &str,
    signer: String,
) -> Result<SignedTree, String> {
    let raw = git(source, &["cat-file", "commit", commit])?;
    let tree = commit_tree(&raw)?;

    // Isolated build input: exactly the signed tree, nothing from the checkout.
    let scratch = private_scratch(parent, &format!("sophia-{label}-artifact"))?;
    let _scratch = RemoveOnDrop(scratch.clone());
    let archive = scratch.join("source.tar");
    let tree_dir = scratch.join("source");
    std::fs::create_dir(&tree_dir).map_err(|e| e.to_string())?;
    git(
        source,
        &["archive", "--format=tar", "-o", path_str(&archive)?, commit],
    )?;
    bounded(
        Command::new("tar")
            .arg("-x")
            .arg("--no-same-owner")
            .arg("-f")
            .arg(&archive)
            .arg("-C")
            .arg(&tree_dir),
        GIT_TIMEOUT,
        "tar -x",
    )?;
    std::fs::remove_file(&archive).map_err(|e| e.to_string())?;
    let inventory = crate::git_tree::inventory(&tree_dir)?;
    crate::git_tree::verify_commit(&raw, commit, &inventory.tree)
        .map_err(|e| format!("archived {label} tree is not the signed commit {commit}: {e}"))?;

    Ok(SignedTree {
        scratch,
        _scratch,
        tree_dir,
        raw,
        tree,
        signer,
    })
}

fn write_output(
    output: &Path,
    binary: &Path,
    raw: &[u8],
    manifest: impl FnOnce(&str) -> String,
) -> Result<String, String> {
    let copy = output.join(BINARY);
    std::fs::copy(binary, &copy).map_err(|e| format!("copy {BINARY}: {e}"))?;
    let digest = sha256(&read(&copy)?);
    std::fs::write(output.join(COMMIT_OBJECT), raw).map_err(|e| e.to_string())?;
    std::fs::write(output.join(MANIFEST), manifest(&digest)).map_err(|e| e.to_string())?;
    set_mode(&copy, 0o555)?;
    set_mode(&output.join(COMMIT_OBJECT), 0o444)?;
    set_mode(&output.join(MANIFEST), 0o444)?;
    set_mode(output, 0o555)?;
    Ok(digest)
}

pub(crate) fn commit_tree(raw: &[u8]) -> Result<String, String> {
    let first = raw.split(|b| *b == b'\n').next().unwrap_or_default();
    let tree = std::str::from_utf8(first)
        .ok()
        .and_then(|line| line.strip_prefix("tree "))
        .filter(|tree| hex(tree, 40))
        .ok_or("commit object has no leading tree line")?;
    Ok(tree.to_owned())
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    bounded(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0"),
        GIT_TIMEOUT,
        &format!("git {}", args.first().copied().unwrap_or("")),
    )
}

/// Run in a private process group to completion within `limit`, capturing at
/// most OUTPUT_CAP per stream. Output collection is bounded too: a descendant
/// still holding a pipe after the direct child exits ends the run and its group.
pub fn bounded(command: &mut Command, limit: Duration, what: &str) -> Result<Vec<u8>, String> {
    let mut child = ProcessGroup(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("{what}: {e}"))?,
    );
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = match pipe {
                Some(pipe) => pipe.take(OUTPUT_CAP + 1).read_to_end(&mut bytes),
                None => Ok(0),
            };
            let _ = sender.send(result.map(|_| bytes));
        });
        receiver
    };
    let stdout = drain(
        child
            .0
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .0
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let status = wait(&child, limit).map_err(|error| format!("{what}: {error}"))?;
    let deadline = Instant::now() + PIPE_GRACE;
    let collect = |receiver: &mpsc::Receiver<std::io::Result<Vec<u8>>>| {
        receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    };
    let (Ok(stdout), Ok(stderr)) = (collect(&stdout), collect(&stderr)) else {
        return Err(format!("{what}: a descendant kept its output open"));
    };
    let (Ok(stdout), Ok(stderr)) = (stdout, stderr) else {
        return Err(format!("{what}: could not read command output"));
    };
    if stdout.len() as u64 > OUTPUT_CAP || stderr.len() as u64 > OUTPUT_CAP {
        return Err(format!("{what}: output exceeds {OUTPUT_CAP} bytes"));
    }
    if status.exit_status() != Some(0) {
        return Err(format!(
            "{what}: {status:?}: {}",
            String::from_utf8_lossy(&stderr)
        ));
    }
    Ok(stdout)
}

fn wait(child: &ProcessGroup, limit: Duration) -> Result<WaitIdStatus, String> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.status()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(format!("exceeded {limit:?}"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The leader stays waitable until the last group signal, pinning the PGID
/// against reuse. Cleanup also runs on successful exit and I/O errors. Only
/// this group is signalled: trusted build tools must not escape with setsid or
/// a different process group. Public so the Quickshell probe
/// (crates/quickshell-probe) reuses this custody rather than a copy.
pub struct ProcessGroup(Child);

impl ProcessGroup {
    /// Take custody of a child spawned with `process_group(0)`. The caller
    /// must not reap it elsewhere; dropping the value stops and reaps the group.
    pub fn new(child: Child) -> Self {
        Self(child)
    }

    pub fn pid(&self) -> Pid {
        Pid::from_raw(self.0.id() as i32).expect("spawned child has a process ID")
    }

    /// The leader's exit, observed without reaping it (WNOWAIT).
    pub fn status(&self) -> Result<Option<WaitIdStatus>, String> {
        match rustix::process::waitid(
            WaitId::Pid(self.pid()),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(status) => Ok(status),
            Err(rustix::io::Errno::INTR) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let group = self.pid();
        let _ = kill_process_group(group, Signal::TERM);
        // Existence probes include the unreaped leader, so they cannot prove
        // that descendants have exited. Keep the leader until after KILL.
        std::thread::sleep(GROUP_GRACE);
        let _ = kill_process_group(group, Signal::KILL);
        let _ = self.0.wait();
    }
}

/// Own a build child spawned with `process_group(0)`, bounding its runtime and
/// log. The caller must not reap it elsewhere. Cleanup precedes the final log
/// check, so successful exit cannot hide an overflow or a lingering writer.
pub fn wait_logged(child: Child, log: &Path, limit: Duration, what: &str) -> Result<(), String> {
    let child = ProcessGroup(child);
    let deadline = Instant::now() + limit;
    let status = loop {
        let size = std::fs::metadata(log)
            .map_err(|e| format!("{what} log: {e}"))?
            .len();
        if Instant::now() >= deadline || size > BUILD_LOG_CAP {
            return Err(format!(
                "{what} stopped (limit {limit:?}, log {size} bytes)"
            ));
        }
        if let Some(status) = child.status()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(child);
    let size = std::fs::metadata(log)
        .map_err(|e| format!("{what} log: {e}"))?
        .len();
    if size > BUILD_LOG_CAP {
        return Err(format!("{what} log exceeds {BUILD_LOG_CAP} bytes ({size})"));
    }
    if status.exit_status() != Some(0) {
        return Err(format!("{what}: {status:?}"));
    }
    Ok(())
}

pub(crate) fn tail(log: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(mut file) = File::open(log)
        && let Ok(size) = file.metadata().map(|m| m.len())
        && file
            .seek(SeekFrom::Start(size.saturating_sub(4096)))
            .is_ok()
    {
        let _ = file.take(4096).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path))
    }
}

pub(crate) fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("non-UTF-8 path {}", path.display()))
}

pub(crate) fn text(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("{}: {e}", path.display()))
}
