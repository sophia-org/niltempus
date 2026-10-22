//! Resolve the supervised host, its Bubblewrap supervisor and the
//! protected peer beneath an owned wrapper pid, from fixture or live
//! `/proc` roots.
//!
//! Traversal follows only the owned wrapper's descendants, through
//! `proc/<pid>/task/*/children`, cross-checked against each child's
//! stat parent pid. It never scans the whole proc root, so an unrelated
//! process running the same executable elsewhere is not a candidate.
//! A candidate matches only when the device and inode of
//! `proc/<pid>/exe` equal the prepared file's device and inode; the
//! link text is retained as evidence but is never the match key, and
//! the command name is never consulted.
//!
//! Positions are explicit. Exactly one examined process below the
//! wrapper runs the host file. The peer must be the only child of a
//! process running the prepared supervisor file, and that supervisor
//! must be the host's direct child. Zero matches at a level are
//! `Pending`, the normal state while the tree starts; the caller's run
//! deadline bounds the retries. Host matches that all descend from one
//! of them are forks before exec and also `Pending`; the binder never
//! selects among them, so a permanent nested duplicate never binds.
//! Host matches with separate roots are an error, as are a peer match
//! outside a supervised position below the host and two supervised
//! peers.
//!
//! Some processes cannot be examined. A process that made itself
//! non-dumpable outside a user namespace the capturer owns (Factotum
//! and its PAM helper, for example) refuses `exe` and `ns/*` reads with
//! EACCES, while its stat and children stay readable. Such a process is
//! never a candidate and is listed as unexamined in the binding. It is
//! accepted only strictly below the host and outside the selected
//! supervisor's subtree; anywhere else the pass is `Pending`. So the
//! binding claims uniqueness only among examined processes
//! ([`UNIQUENESS_SCOPE`]), never across the whole tree.
//!
//! The binding retains the whole ancestor chain from wrapper to peer,
//! each node pinned to `(pid, start_time_ticks)`. `recheck` re-reads
//! every node's identity and parent link, the supervisor's direct
//! parent and sole child, the host, supervisor and peer executable
//! identities, and the time-namespace proof, so a reused pid, a broken
//! parent link or an ended ancestor cannot keep a stale binding alive.
//!
//! Clock comparability is part of the proof: the `ns/time` inodes of
//! the host, the peer and the capturer must all equal the kernel's
//! initial time namespace inode, and the capturer's `timens_offsets`
//! monotonic offset must be exactly zero. The initial namespace is
//! named by the kernel constant, not read from pid 1: an unprivileged
//! capturer cannot read init's namespace links, and under pid
//! namespaces pid 1 need not be init. The namespace reads for the host
//! and peer sit between their stat reads, so the proof is bound to the
//! same start-time bracket as the identities it covers. This module
//! makes no credential claim about any pid; peer credentials are the
//! runner's separate check.
use crate::shell_timing_environment_capture::{CaptureRoots, ProcessIdentity};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

mod time;
use time::proof_matches;

/// Bound on processes visited in one traversal.
pub const MAX_NODES: usize = 512;
/// Bound on task entries enumerated for one process.
pub const MAX_TASKS_PER_PROCESS: usize = 2048;
/// Bound on the child-pid union of one process; a larger union would
/// exceed the traversal bound anyway.
pub const MAX_CHILDREN_PER_PROCESS: usize = MAX_NODES;
/// Bound on wrapper-to-leaf depth in one traversal.
pub const MAX_DEPTH: usize = 32;
/// Bound on one stat, children or offsets read.
pub const MAX_READ_BYTES: u64 = 65_536;
/// Inode of the kernel's initial time namespace: `TIME_NS_INIT_INO` in
/// `include/uapi/linux/nsfs.h` (checked against the 6.18.55_1 headers).
pub const INITIAL_TIME_NAMESPACE_INODE: u64 = 0xEFFF_FFFA;
/// The only uniqueness a binding claims.
pub const UNIQUENESS_SCOPE: &str = "exactly one host below the wrapper and one supervised peer \
     below the host among examined processes; unexamined processes below the host were not \
     compared";

/// The three prepared files a binding matches, by device and inode.
#[derive(Clone, Copy, Debug)]
pub struct Executables<'a> {
    pub host: &'a Path,
    /// The prepared host command line, argv0 included. Other processes
    /// running the host file (the input guard, launch checks, the metadata
    /// broker) carry other arguments and are not hosts.
    pub host_argv: &'a [String],
    /// The protection-domain supervisor (Bubblewrap) the host runs the
    /// peer under.
    pub supervisor: &'a Path,
    pub peer: &'a Path,
}

/// Device and inode identity of one executable file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExecutableIdentity {
    pub dev: u64,
    pub ino: u64,
}

impl ExecutableIdentity {
    /// Identity of a prepared file on disk.
    pub fn of_file(path: &Path) -> Result<Self, String> {
        let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }
}

/// How one process's executable matched its prepared file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExecutableEvidence {
    /// The prepared file the runner supplied.
    pub prepared_path: PathBuf,
    /// The raw `proc/<pid>/exe` link text, kept as evidence only.
    pub observed_link: PathBuf,
    pub identity: ExecutableIdentity,
}

/// One node of the retained ancestor chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BoundNode {
    pub identity: ProcessIdentity,
    /// Stat parent pid read at bind time; rechecked on every recheck.
    pub parent_pid: u32,
}

/// The kernel's initial time namespace, named by its fixed inode. No
/// process, stat read or capture time stands behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct InitialTimeNamespace {
    pub inode: u64,
}

/// Time-namespace evidence for one pid.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NamespaceEvidence {
    pub pid: u32,
    pub identity: ProcessIdentity,
    /// Inode parsed from the `time:[<inode>]` link target.
    pub time_namespace_inode: u64,
    /// The `proc/<pid>/ns/time` path the link was read from.
    pub origin: String,
    /// The raw link target text, retained as read provenance.
    pub observed_target: String,
    /// Direct `CLOCK_MONOTONIC` microseconds when the read completed.
    pub captured_usec: u64,
}

/// Proof that host, peer and capturer share the initial time namespace
/// with a zero monotonic offset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TimeNamespaceProof {
    pub reference: InitialTimeNamespace,
    pub host: NamespaceEvidence,
    pub peer: NamespaceEvidence,
    pub capturer: NamespaceEvidence,
    /// The capturer's raw `timens_offsets` text, retained verbatim.
    pub capturer_timens_offsets: String,
    /// The `proc/<capturer>/timens_offsets` path the text was read from.
    pub capturer_timens_offsets_origin: String,
    /// Parsed `(seconds, nanoseconds)` monotonic offset; always zero
    /// in a constructed proof.
    pub monotonic_offset: (i64, i64),
}

/// A completed binding: identities, chain and namespace proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProcessBinding {
    pub wrapper: ProcessIdentity,
    pub host: ProcessIdentity,
    /// The host's direct child whose only child is the peer.
    pub supervisor: ProcessIdentity,
    pub peer: ProcessIdentity,
    /// Wrapper first, then host, supervisor and peer last; each once.
    pub chain: Vec<BoundNode>,
    pub host_executable: ExecutableEvidence,
    /// The host's `proc/<pid>/cmdline`, which matched the prepared argv
    /// exactly.
    pub host_argv: Vec<String>,
    pub supervisor_executable: ExecutableEvidence,
    pub peer_executable: ExecutableEvidence,
    /// Processes below the host whose executable the kernel refused to
    /// show, by pid. Never candidates; listed so the binding states what
    /// it did not compare.
    pub unexamined: Vec<BoundNode>,
    /// Always [`UNIQUENESS_SCOPE`].
    pub uniqueness: &'static str,
    pub time: TimeNamespaceProof,
}

/// Which level is still missing during startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingStage {
    Host,
    Peer,
}

/// Why a level is not yet bindable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingReason {
    /// No examined process runs the file in its position yet.
    NoMatch,
    /// Several processes in one rooted tree run the host file with the
    /// prepared argv: forks not yet followed by exec.
    ForkBeforeExec,
    /// An unexamined process sits outside the host's subtree, so the
    /// host's uniqueness cannot be stated.
    UnexaminedOutsideHost,
    /// An unexamined process sits inside the selected supervisor's
    /// subtree.
    UnexaminedInSelectedSubtree,
}

/// Outcome of one resolve pass over the owned tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// Not bindable yet at this level; poll again.
    Pending {
        stage: PendingStage,
        /// The bound host, when only the peer level is still open.
        host: Option<ProcessIdentity>,
        reason: PendingReason,
    },
    Bound(Box<ProcessBinding>),
}

/// Outcome of rechecking a retained binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingRecheck {
    /// Every retained identity, link and proof still holds.
    Bound,
    /// A chain process is gone or in an ended state.
    Ended,
}

const HOST: usize = 0;
const SUPERVISOR: usize = 1;
const PEER: usize = 2;

#[derive(Clone, Copy, Debug)]
struct StatFacts {
    identity: ProcessIdentity,
    parent_pid: u32,
    state: char,
}

/// What one process's `exe` link showed.
enum Observed {
    File {
        link: PathBuf,
        identity: ExecutableIdentity,
    },
    /// Gone, or a deleted target.
    Absent,
    /// The kernel refused the read (EACCES).
    Opaque,
}

fn ended(state: char) -> bool {
    matches!(state, 'Z' | 'X' | 'x')
}

fn known_state(state: char) -> bool {
    matches!(
        state,
        'R' | 'S' | 'D' | 'T' | 't' | 'Z' | 'X' | 'x' | 'K' | 'W' | 'P' | 'I'
    )
}

fn read_bounded(path: &Path, limit: u64) -> Result<String, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{}: exceeds {limit} byte bound", path.display()));
    }
    String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", path.display()))
}

/// `argv` as `proc/<pid>/cmdline` stores it: every argument NUL-terminated.
fn cmdline_bytes(argv: &[String]) -> Result<Vec<u8>, String> {
    if argv.is_empty() || argv.iter().any(|arg| arg.contains('\0')) {
        return Err("host argv must be nonempty without NUL bytes".into());
    }
    Ok(argv.iter().flat_map(|arg| arg.bytes().chain([0])).collect())
}

/// The matches with no other match among their ancestors.
fn host_roots<'m, T>(
    nodes: &BTreeMap<u32, (StatFacts, usize)>,
    matches: &'m [(StatFacts, T)],
) -> Vec<&'m (StatFacts, T)> {
    matches
        .iter()
        .filter(|(facts, _)| {
            !matches
                .iter()
                .any(|(other, _)| is_descendant(nodes, facts.identity.pid, other.identity.pid))
        })
        .collect()
}

/// Pid, parent and start time of each candidate, for refusals.
fn candidates<T>(matches: &[(StatFacts, T)]) -> String {
    matches
        .iter()
        .map(|(facts, _)| {
            format!(
                "pid {} (ppid {}, start {})",
                facts.identity.pid, facts.parent_pid, facts.identity.start_time_ticks
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Stat facts binding needs: identity, parent pid and state. Fields are
/// counted from the last `)` because the command name may contain
/// spaces and parentheses. Field 3 is the state, field 4 the parent
/// pid and field 22 the start time.
fn stat_facts(text: &str, expected_pid: u32) -> Result<StatFacts, String> {
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.contains('\n') {
        return Err("stat: expected one line".into());
    }
    let open = line.find(" (").ok_or("stat: no command name")?;
    let close = line.rfind(") ").ok_or("stat: no command name")?;
    if close < open {
        return Err("stat: malformed command name".into());
    }
    let pid: u32 = line[..open]
        .parse()
        .map_err(|_| "stat: bad pid".to_string())?;
    if pid != expected_pid {
        return Err(format!(
            "stat: record is for pid {pid}, not requested pid {expected_pid}"
        ));
    }
    let fields: Vec<&str> = line[close + 2..].split(' ').collect();
    let state = fields
        .first()
        .copied()
        .filter(|state| state.len() == 1)
        .and_then(|state| state.chars().next())
        .filter(|&state| known_state(state))
        .ok_or("stat: unknown state field")?;
    let parent_pid: u32 = fields
        .get(1)
        .copied()
        .ok_or("stat: missing ppid field")?
        .parse()
        .map_err(|_| "stat: bad ppid".to_string())?;
    let start_time_ticks: u64 = fields
        .get(22 - 3)
        .copied()
        .ok_or("stat: missing start_time field")?
        .parse()
        .map_err(|_| "stat: bad start_time".to_string())?;
    Ok(StatFacts {
        identity: ProcessIdentity {
            pid,
            start_time_ticks,
        },
        parent_pid,
        state,
    })
}

/// A resolver over fixed roots. The capturer pid is explicit so tests
/// can name a fixture process as the reader; `Binder::system` uses the
/// calling process.
#[derive(Clone, Debug)]
pub struct Binder {
    proc_root: PathBuf,
    capturer_pid: u32,
}

impl Binder {
    pub fn new(roots: &CaptureRoots, capturer_pid: u32) -> Self {
        Self {
            proc_root: roots.proc_root.clone(),
            capturer_pid,
        }
    }

    pub fn system() -> Self {
        Self::new(&CaptureRoots::system(), std::process::id())
    }

    fn proc_path(&self, relative: &str) -> PathBuf {
        self.proc_root.join(relative)
    }

    fn stat(&self, pid: u32) -> Result<Option<StatFacts>, String> {
        let path = self.proc_path(&format!("{pid}/stat"));
        let text = match fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_READ_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                if bytes.len() as u64 > MAX_READ_BYTES {
                    return Err(format!(
                        "{}: exceeds {MAX_READ_BYTES} byte bound",
                        path.display()
                    ));
                }
                String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", path.display()))?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        stat_facts(&text, pid)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    fn live_stat(&self, pid: u32, role: &str) -> Result<StatFacts, String> {
        let facts = self
            .stat(pid)?
            .ok_or_else(|| format!("{role} pid {pid} is not present"))?;
        if ended(facts.state) {
            return Err(format!(
                "{role} pid {pid} is in ended state {}",
                facts.state
            ));
        }
        Ok(facts)
    }

    /// Child pids of one process, unioned over its tasks. A process
    /// whose task tree vanished mid-read has no children here; its own
    /// stat decides whether that is an exit or an error upstream.
    fn children_of(&self, pid: u32) -> Result<Vec<u32>, String> {
        let task_dir = self.proc_path(&format!("{pid}/task"));
        let entries = match fs::read_dir(&task_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", task_dir.display())),
        };
        // Stream the task entries: neither the entry list nor the
        // child union is collected past its bound. Entry order does
        // not matter because the union is sorted by construction.
        let mut children = BTreeSet::new();
        let mut task_count = 0usize;
        for entry in entries {
            task_count += 1;
            if task_count > MAX_TASKS_PER_PROCESS {
                return Err(format!(
                    "{}: task count exceeds {MAX_TASKS_PER_PROCESS}",
                    task_dir.display()
                ));
            }
            let entry = entry.map_err(|e| format!("{}: {e}", task_dir.display()))?;
            let path = entry.path().join("children");
            let text = match read_bounded(&path, MAX_READ_BYTES) {
                Ok(text) => text,
                Err(e) if e.contains("No such file") => continue,
                Err(e) => return Err(e),
            };
            for token in text.split_whitespace() {
                let child: u32 = token
                    .parse()
                    .map_err(|_| format!("{}: bad child pid {token:?}", path.display()))?;
                if child == 0 {
                    return Err(format!("{}: child pid must be nonzero", path.display()));
                }
                children.insert(child);
                if children.len() > MAX_CHILDREN_PER_PROCESS {
                    return Err(format!(
                        "{}: child count exceeds {MAX_CHILDREN_PER_PROCESS}",
                        task_dir.display()
                    ));
                }
            }
        }
        Ok(children.into_iter().collect())
    }

    /// What `proc/<pid>/exe` shows. Only EACCES makes a process opaque;
    /// any other unexpected error is an error.
    fn observe_executable(&self, pid: u32) -> Result<Observed, String> {
        let exe = self.proc_path(&format!("{pid}/exe"));
        let denied =
            |e: &io::Error| e.raw_os_error() == Some(rustix::io::Errno::ACCESS.raw_os_error());
        let link = match fs::read_link(&exe) {
            Ok(target) => target,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Observed::Absent),
            Err(e) if denied(&e) => return Ok(Observed::Opaque),
            Err(e) => return Err(format!("{}: {e}", exe.display())),
        };
        if link.to_string_lossy().ends_with(" (deleted)") {
            return Ok(Observed::Absent);
        }
        let meta = match fs::metadata(&exe) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Observed::Absent),
            Err(e) if denied(&e) => return Ok(Observed::Opaque),
            Err(e) => return Err(format!("{}: {e}", exe.display())),
        };
        Ok(Observed::File {
            link,
            identity: ExecutableIdentity {
                dev: meta.dev(),
                ino: meta.ino(),
            },
        })
    }

    /// Whether `pid`'s command line is exactly `expected` bytes, or `None`
    /// when the process is gone. The command line selects a role among
    /// processes already matched by executable identity; it authenticates
    /// nothing, and Sophia never rewrites its own arguments.
    fn cmdline_matches(&self, pid: u32, expected: &[u8]) -> Result<Option<bool>, String> {
        let path = self.proc_path(&format!("{pid}/cmdline"));
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut bytes = Vec::new();
        file.take(MAX_READ_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if bytes.len() as u64 > MAX_READ_BYTES {
            return Err(format!(
                "{}: exceeds {MAX_READ_BYTES} byte bound",
                path.display()
            ));
        }
        Ok(Some(bytes == expected))
    }

    /// Executable evidence when a bound `pid` still runs `expected`, or
    /// `None` when it provably runs something else (including a deleted
    /// target, which cannot prove identity). A bound process that turned
    /// opaque is an error.
    fn executable_match(
        &self,
        pid: u32,
        prepared: &Path,
        expected: &ExecutableIdentity,
    ) -> Result<Option<ExecutableEvidence>, String> {
        match self.observe_executable(pid)? {
            Observed::File { link, identity } if identity == *expected => {
                Ok(Some(ExecutableEvidence {
                    prepared_path: prepared.to_path_buf(),
                    observed_link: link,
                    identity,
                }))
            }
            Observed::File { .. } | Observed::Absent => Ok(None),
            Observed::Opaque => Err(format!("pid {pid}: executable is no longer readable")),
        }
    }

    /// One resolve pass. See the module docs for the matching rules.
    pub fn resolve(
        &self,
        wrapper_pid: u32,
        executables: Executables<'_>,
    ) -> Result<ResolveOutcome, String> {
        if wrapper_pid == 0 || wrapper_pid == 1 {
            return Err("wrapper pid must be nonzero and not pid 1".into());
        }
        let host_cmdline = cmdline_bytes(executables.host_argv)?;
        let files = [executables.host, executables.supervisor, executables.peer];
        let expected = [
            ExecutableIdentity::of_file(files[HOST])?,
            ExecutableIdentity::of_file(files[SUPERVISOR])?,
            ExecutableIdentity::of_file(files[PEER])?,
        ];
        if expected[HOST] == expected[SUPERVISOR]
            || expected[HOST] == expected[PEER]
            || expected[SUPERVISOR] == expected[PEER]
        {
            return Err("host, supervisor and peer executables must be different files".into());
        }
        let wrapper = self.live_stat(wrapper_pid, "wrapper")?;

        // Breadth-first walk of the owned tree only.
        let mut nodes: BTreeMap<u32, (StatFacts, usize)> = BTreeMap::new();
        let mut queue: VecDeque<u32> = VecDeque::new();
        nodes.insert(wrapper_pid, (wrapper, 0));
        queue.push_back(wrapper_pid);
        while let Some(pid) = queue.pop_front() {
            let depth = nodes[&pid].1;
            for child in self.children_of(pid)? {
                if let Some((existing, _)) = nodes.get(&child) {
                    if existing.parent_pid != pid {
                        return Err(format!(
                            "pid {child} appears under two parents: {} and {pid}",
                            existing.parent_pid
                        ));
                    }
                    continue;
                }
                let Some(facts) = self.stat(child)? else {
                    continue; // Ended between listing and read.
                };
                if facts.parent_pid != pid {
                    return Err(format!(
                        "pid {child} is listed as a child of {pid} but stat names parent {}",
                        facts.parent_pid
                    ));
                }
                if ended(facts.state) {
                    continue; // An ended process binds nothing below it.
                }
                if depth + 1 > MAX_DEPTH {
                    return Err(format!("descendant depth exceeds {MAX_DEPTH}"));
                }
                if nodes.len() + 1 > MAX_NODES {
                    return Err(format!("descendant count exceeds {MAX_NODES}"));
                }
                nodes.insert(child, (facts, depth + 1));
                queue.push_back(child);
            }
        }

        // Classify every examined node once; opaque nodes are only listed.
        let mut matches: [Vec<(StatFacts, ExecutableEvidence)>; 3] = Default::default();
        let mut opaque: Vec<u32> = Vec::new();
        for (&pid, (facts, _)) in &nodes {
            if pid == wrapper_pid {
                continue;
            }
            match self.observe_executable(pid)? {
                Observed::File { link, identity } => {
                    if let Some(role) = expected.iter().position(|e| *e == identity) {
                        // Another mode of the host file is not a host.
                        if role == HOST && self.cmdline_matches(pid, &host_cmdline)? != Some(true) {
                            continue;
                        }
                        matches[role].push((
                            *facts,
                            ExecutableEvidence {
                                prepared_path: files[role].to_path_buf(),
                                observed_link: link,
                                identity,
                            },
                        ));
                    }
                }
                Observed::Absent => {}
                Observed::Opaque => opaque.push(pid),
            }
        }
        let pending = |stage, host, reason| {
            Ok(ResolveOutcome::Pending {
                stage,
                host,
                reason,
            })
        };

        // Host: one examined descendant of the wrapper, or a fork pair.
        let (host, host_evidence) = match matches[HOST].as_slice() {
            [] => return pending(PendingStage::Host, None, PendingReason::NoMatch),
            [one] => one.clone(),
            // Several exact matches in one rooted tree are forks not yet
            // followed by exec. Never select among them; a permanent nested
            // duplicate stays Pending until the caller's deadline.
            many if host_roots(&nodes, many).len() == 1 => {
                return pending(PendingStage::Host, None, PendingReason::ForkBeforeExec);
            }
            many => {
                return Err(format!(
                    "{} examined descendants of wrapper {wrapper_pid} run the host executable \
                     with the prepared argv; the host is ambiguous: {}",
                    many.len(),
                    candidates(many)
                ));
            }
        };
        let host_pid = host.identity.pid;
        if opaque
            .iter()
            .any(|&pid| !is_descendant(&nodes, pid, host_pid))
        {
            return pending(
                PendingStage::Host,
                None,
                PendingReason::UnexaminedOutsideHost,
            );
        }

        // Peer: the only child of a supervisor that is the host's direct
        // child. Peer matches outside the host's subtree are not examined.
        let mut supervised = Vec::new();
        for (facts, evidence) in &matches[PEER] {
            let pid = facts.identity.pid;
            if !is_descendant(&nodes, pid, host_pid) {
                continue;
            }
            if opaque.contains(&facts.parent_pid) {
                // The would-be supervisor itself is unexamined.
                return pending(
                    PendingStage::Peer,
                    Some(host.identity),
                    PendingReason::UnexaminedInSelectedSubtree,
                );
            }
            let supervisor = matches[SUPERVISOR]
                .iter()
                .find(|(s, _)| s.identity.pid == facts.parent_pid && s.parent_pid == host_pid);
            match supervisor {
                Some(supervisor) if self.children_of(facts.parent_pid)? == [pid] => {
                    supervised.push((supervisor.clone(), (*facts, evidence.clone())));
                }
                _ => {
                    return Err(format!(
                        "pid {pid} runs the peer executable outside a supervised position \
                         below host {host_pid}"
                    ));
                }
            }
        }
        let ((supervisor, supervisor_evidence), (peer, peer_evidence)) = match supervised.len() {
            0 => {
                return pending(
                    PendingStage::Peer,
                    Some(host.identity),
                    PendingReason::NoMatch,
                );
            }
            1 => supervised.pop().unwrap(),
            n => {
                let peers: Vec<_> = supervised.iter().map(|(_, peer)| peer.clone()).collect();
                return Err(format!(
                    "{n} supervised peers below host {host_pid}; the peer is ambiguous: {}",
                    candidates(&peers)
                ));
            }
        };
        let supervisor_pid = supervisor.identity.pid;
        if opaque
            .iter()
            .any(|&pid| is_descendant(&nodes, pid, supervisor_pid))
        {
            return pending(
                PendingStage::Peer,
                Some(host.identity),
                PendingReason::UnexaminedInSelectedSubtree,
            );
        }

        // Retain the full chain wrapper -> ... -> host -> supervisor -> peer.
        let mut chain: Vec<BoundNode> = Vec::new();
        for pid in lineage(&nodes, peer.identity.pid) {
            let (facts, _) = nodes[&pid];
            chain.push(BoundNode {
                identity: facts.identity,
                parent_pid: facts.parent_pid,
            });
        }
        let tail: Vec<u32> = chain.iter().rev().take(3).map(|n| n.identity.pid).collect();
        if tail != [peer.identity.pid, supervisor_pid, host_pid] {
            return Err("retained chain does not end host, supervisor, peer".into());
        }
        let unexamined = opaque
            .iter()
            .map(|pid| {
                let (facts, _) = nodes[pid];
                BoundNode {
                    identity: facts.identity,
                    parent_pid: facts.parent_pid,
                }
            })
            .collect();
        let time = self.time_proof(&host, &peer)?;
        let binding = ProcessBinding {
            wrapper: wrapper.identity,
            host: host.identity,
            supervisor: supervisor.identity,
            peer: peer.identity,
            chain,
            host_executable: host_evidence,
            host_argv: executables.host_argv.to_vec(),
            supervisor_executable: supervisor_evidence,
            peer_executable: peer_evidence,
            unexamined,
            uniqueness: UNIQUENESS_SCOPE,
            time,
        };
        // Close the full retained chain before reporting success: an
        // ancestor that exited or was replaced during traversal must
        // not produce a nominally successful binding.
        match self.recheck(&binding)? {
            BindingRecheck::Bound => Ok(ResolveOutcome::Bound(Box::new(binding))),
            BindingRecheck::Ended => {
                Err("retained chain ended during resolve; binding refused".into())
            }
        }
    }

    /// Whether the process with this exact identity has exited: absent,
    /// in an ended state, or its pid now names another process. Under an
    /// exited parent a broken link is the reparenting of teardown; under a
    /// live one it stays an error.
    pub fn exited(&self, identity: ProcessIdentity) -> Result<bool, String> {
        Ok(self
            .stat(identity.pid)?
            .is_none_or(|facts| facts.identity != identity || ended(facts.state)))
    }

    /// Exact protected position: the supervisor is the host's direct
    /// child and the peer is its only child.
    fn exact_position(&self, binding: &ProcessBinding) -> Result<(), String> {
        let supervisor = self.live_stat(binding.supervisor.pid, "supervisor")?;
        if supervisor.identity != binding.supervisor || supervisor.parent_pid != binding.host.pid {
            return Err(format!(
                "supervisor pid {} is no longer the host's direct child",
                binding.supervisor.pid
            ));
        }
        let peer = self.live_stat(binding.peer.pid, "peer")?;
        if peer.identity != binding.peer
            || peer.parent_pid != binding.supervisor.pid
            || self.children_of(binding.supervisor.pid)? != [binding.peer.pid]
        {
            return Err(format!(
                "supervisor pid {} no longer has peer pid {} as its only child",
                binding.supervisor.pid, binding.peer.pid
            ));
        }
        Ok(())
    }

    /// Recheck every retained fact. Absence or an ended state anywhere
    /// in the chain is `Ended`; a changed identity, parent link,
    /// executable or namespace proof is an error.
    pub fn recheck(&self, binding: &ProcessBinding) -> Result<BindingRecheck, String> {
        for (index, node) in binding.chain.iter().enumerate() {
            let Some(facts) = self.stat(node.identity.pid)? else {
                return Ok(BindingRecheck::Ended);
            };
            if facts.identity != node.identity {
                return Err(format!(
                    "pid {} was reused: start_time {} replaced {}",
                    node.identity.pid,
                    facts.identity.start_time_ticks,
                    node.identity.start_time_ticks
                ));
            }
            if ended(facts.state) {
                return Ok(BindingRecheck::Ended);
            }
            if facts.parent_pid != node.parent_pid {
                if index > 0 && self.exited(binding.chain[index - 1].identity)? {
                    return Ok(BindingRecheck::Ended);
                }
                return Err(format!(
                    "pid {} parent changed: {} replaced {}",
                    node.identity.pid, facts.parent_pid, node.parent_pid
                ));
            }
            if index > 0 {
                let parent = binding.chain[index - 1].identity.pid;
                if facts.parent_pid != parent {
                    return Err(format!(
                        "pid {} is no longer below pid {parent} in the retained chain",
                        node.identity.pid
                    ));
                }
                if !self.children_of(parent)?.contains(&node.identity.pid) {
                    if self.exited(binding.chain[index - 1].identity)? {
                        return Ok(BindingRecheck::Ended);
                    }
                    return Err(format!(
                        "pid {} no longer lists pid {} as a child",
                        parent, node.identity.pid
                    ));
                }
            }
        }
        // The walk above read each process once; one that exited since is
        // the same teardown race, not a broken position.
        if let Err(error) = self.exact_position(binding) {
            if self.exited(binding.host)?
                || self.exited(binding.supervisor)?
                || self.exited(binding.peer)?
            {
                return Ok(BindingRecheck::Ended);
            }
            return Err(error);
        }
        match self.cmdline_matches(binding.host.pid, &cmdline_bytes(&binding.host_argv)?)? {
            Some(true) => {}
            Some(false) => {
                return Err(format!(
                    "host pid {} argv changed since binding",
                    binding.host.pid
                ));
            }
            None => return Ok(BindingRecheck::Ended),
        }
        for (identity, evidence, prepared) in [
            (
                binding.host,
                &binding.host_executable,
                &binding.host_executable.prepared_path,
            ),
            (
                binding.supervisor,
                &binding.supervisor_executable,
                &binding.supervisor_executable.prepared_path,
            ),
            (
                binding.peer,
                &binding.peer_executable,
                &binding.peer_executable.prepared_path,
            ),
        ] {
            let expected = ExecutableIdentity::of_file(prepared)?;
            if expected != evidence.identity {
                return Err(format!(
                    "prepared executable {} changed since binding",
                    prepared.display()
                ));
            }
            match self.executable_match(identity.pid, prepared, &expected)? {
                Some(current) if current == *evidence => {}
                Some(_) => {
                    return Err(format!(
                        "pid {} executable evidence changed since binding",
                        identity.pid
                    ));
                }
                None => {
                    return Err(format!(
                        "pid {} no longer runs its bound executable",
                        identity.pid
                    ));
                }
            }
        }
        // The namespace proof must still hold for the same processes.
        // Capture stamps differ per read and are not compared.
        let host = self.live_stat(binding.host.pid, "host")?;
        let peer = self.live_stat(binding.peer.pid, "peer")?;
        let proof = self.time_proof(&host, &peer)?;
        if !proof_matches(&proof, &binding.time) {
            return Err("time namespace proof changed since binding".into());
        }
        Ok(BindingRecheck::Bound)
    }
}

/// True when `ancestor` is on `pid`'s parent chain in `nodes`.
fn is_descendant(nodes: &BTreeMap<u32, (StatFacts, usize)>, pid: u32, ancestor: u32) -> bool {
    let mut current = pid;
    while let Some((facts, _)) = nodes.get(&current) {
        if facts.parent_pid == ancestor {
            return true;
        }
        if facts.parent_pid == current || facts.parent_pid == 0 {
            return false;
        }
        current = facts.parent_pid;
    }
    false
}

/// Pids from the tree root down to `pid`, following parent links.
fn lineage(nodes: &BTreeMap<u32, (StatFacts, usize)>, pid: u32) -> Vec<u32> {
    let mut path = vec![pid];
    let mut current = pid;
    while let Some((facts, _)) = nodes.get(&current) {
        if facts.parent_pid == 0 || !nodes.contains_key(&facts.parent_pid) {
            break;
        }
        current = facts.parent_pid;
        path.push(current);
    }
    path.reverse();
    path
}
