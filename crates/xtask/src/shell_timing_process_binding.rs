//! Resolve the supervised host and peer processes beneath an owned
//! wrapper pid, from fixture or live `/proc` roots.
//!
//! Traversal follows only the owned wrapper's descendants, through
//! `proc/<pid>/task/*/children`, cross-checked against each child's
//! stat parent pid. It never scans the whole proc root, so an unrelated
//! process running the same executable elsewhere is not a candidate.
//! A candidate matches only when the device and inode of
//! `proc/<pid>/exe` equal the prepared file's device and inode; the
//! link text is retained as evidence but is never the match key, and
//! the command name is never consulted. Zero matching host or peer
//! processes is `Pending`, the normal state while the tree starts;
//! more than one match is an ambiguity error.
//!
//! The binding retains the whole ancestor chain from wrapper to peer,
//! each node pinned to `(pid, start_time_ticks)`. `recheck` re-reads
//! every node's identity and parent link, the host and peer executable
//! identities, and the time-namespace proof, so a reused pid, a broken
//! parent link or an ended ancestor cannot keep a stale binding alive.
//!
//! Clock comparability is part of the proof: the `ns/time` inode of
//! pid 1 (the reference), the host, the peer and the capturer must be
//! equal and nonzero, and the capturer's `timens_offsets` monotonic
//! offset must be exactly zero. Pid 1's namespace alone would not
//! prove this under pid namespaces, so all four are read. The
//! namespace reads for the host and peer sit between their stat reads,
//! so the proof is bound to the same start-time bracket as the
//! identities it covers. This module makes no credential claim about
//! any pid; peer credentials are the runner's separate check.
use crate::shell_timing_environment_capture::{CaptureRoots, ProcessIdentity};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

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

fn monotonic_usec() -> Result<u64, String> {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let seconds = u64::try_from(now.tv_sec).map_err(|_| "monotonic clock before boot")?;
    let nanos = u64::try_from(now.tv_nsec).map_err(|_| "monotonic clock nanoseconds")?;
    seconds
        .checked_mul(1_000_000)
        .and_then(|usec| usec.checked_add(nanos / 1000))
        .ok_or_else(|| "monotonic clock overflow".into())
}

/// Proof that reference, host, peer and capturer share one time
/// namespace with a zero monotonic offset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TimeNamespaceProof {
    pub reference: NamespaceEvidence,
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
    pub peer: ProcessIdentity,
    /// Wrapper first, peer last; host appears exactly once.
    pub chain: Vec<BoundNode>,
    pub host_executable: ExecutableEvidence,
    pub peer_executable: ExecutableEvidence,
    pub time: TimeNamespaceProof,
}

/// Which level is still missing during startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingStage {
    Host,
    Peer,
}

/// Outcome of one resolve pass over the owned tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// No matching process yet at this level; poll again.
    Pending {
        stage: PendingStage,
        /// The bound host, when only the peer is still missing.
        host: Option<ProcessIdentity>,
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

#[derive(Clone, Copy, Debug)]
struct StatFacts {
    identity: ProcessIdentity,
    parent_pid: u32,
    state: char,
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

    /// Executable evidence when `pid` runs `expected`, or `None` when
    /// it provably runs something else (including a deleted target,
    /// which cannot prove identity).
    fn executable_match(
        &self,
        pid: u32,
        prepared: &Path,
        expected: &ExecutableIdentity,
    ) -> Result<Option<ExecutableEvidence>, String> {
        let exe = self.proc_path(&format!("{pid}/exe"));
        let observed = match fs::read_link(&exe) {
            Ok(target) => target,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", exe.display())),
        };
        if observed.to_string_lossy().ends_with(" (deleted)") {
            return Ok(None);
        }
        let meta = match fs::metadata(&exe) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", exe.display())),
        };
        let identity = ExecutableIdentity {
            dev: meta.dev(),
            ino: meta.ino(),
        };
        if identity != *expected {
            return Ok(None);
        }
        Ok(Some(ExecutableEvidence {
            prepared_path: prepared.to_path_buf(),
            observed_link: observed,
            identity,
        }))
    }

    fn namespace_evidence(&self, stat: &StatFacts) -> Result<NamespaceEvidence, String> {
        let pid = stat.identity.pid;
        let link = self.proc_path(&format!("{pid}/ns/time"));
        let target = fs::read_link(&link).map_err(|e| format!("{}: {e}", link.display()))?;
        let text = target.to_string_lossy().into_owned();
        let inode: u64 = text
            .strip_prefix("time:[")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|digits| digits.parse().ok())
            .ok_or_else(|| {
                format!(
                    "{}: unexpected time namespace target {text:?}",
                    link.display()
                )
            })?;
        if inode == 0 {
            return Err(format!(
                "{}: time namespace inode must be nonzero",
                link.display()
            ));
        }
        Ok(NamespaceEvidence {
            pid,
            identity: stat.identity,
            time_namespace_inode: inode,
            origin: link.display().to_string(),
            observed_target: text,
            captured_usec: monotonic_usec()?,
        })
    }

    /// Namespace proof for a selected host and peer. The host and peer
    /// stat reads bracketing the namespace reads must still name the
    /// bound identities, so the proof covers the same processes.
    fn time_proof(&self, host: &StatFacts, peer: &StatFacts) -> Result<TimeNamespaceProof, String> {
        let reference_pid = 1;
        let reference_stat = self.live_stat(reference_pid, "reference")?;
        let capturer_stat = self.live_stat(self.capturer_pid, "capturer")?;
        let reference = self.namespace_evidence(&reference_stat)?;
        let host_evidence = self.namespace_evidence(host)?;
        let peer_evidence = self.namespace_evidence(peer)?;
        let capturer_evidence = self.namespace_evidence(&capturer_stat)?;
        // Closing bracket: the namespace reads above must not have
        // raced a host or peer replacement.
        let host_close = self.live_stat(host.identity.pid, "host")?;
        let peer_close = self.live_stat(peer.identity.pid, "peer")?;
        if host_close.identity != host.identity || peer_close.identity != peer.identity {
            return Err("host or peer identity changed during namespace capture".into());
        }
        let (reference_inode, host_inode, peer_inode, capturer_inode) = (
            reference.time_namespace_inode,
            host_evidence.time_namespace_inode,
            peer_evidence.time_namespace_inode,
            capturer_evidence.time_namespace_inode,
        );
        if !(reference_inode == host_inode
            && host_inode == peer_inode
            && peer_inode == capturer_inode)
        {
            return Err(format!(
                "time namespace inodes differ: reference {reference_inode}, host {host_inode}, \
                 peer {peer_inode}, capturer {capturer_inode}"
            ));
        }
        let offsets_path = self.proc_path(&format!("{}/timens_offsets", self.capturer_pid));
        let offsets = read_bounded(&offsets_path, MAX_READ_BYTES)?;
        let mut monotonic: Option<(i64, i64)> = None;
        for line in offsets.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some("monotonic") {
                if monotonic.is_some() {
                    return Err("timens_offsets: duplicate monotonic line".into());
                }
                let seconds: i64 = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("timens_offsets: bad monotonic seconds")?;
                let nanos: i64 = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("timens_offsets: bad monotonic nanoseconds")?;
                if parts.next().is_some() {
                    return Err("timens_offsets: trailing fields on monotonic line".into());
                }
                monotonic = Some((seconds, nanos));
            }
        }
        let monotonic = monotonic.ok_or("timens_offsets: no monotonic line")?;
        if monotonic != (0, 0) {
            return Err(format!(
                "timens_offsets: monotonic offset must be exactly zero, got {} {}",
                monotonic.0, monotonic.1
            ));
        }
        Ok(TimeNamespaceProof {
            reference,
            host: host_evidence,
            peer: peer_evidence,
            capturer: capturer_evidence,
            capturer_timens_offsets_origin: offsets_path.display().to_string(),
            capturer_timens_offsets: offsets,
            monotonic_offset: monotonic,
        })
    }

    /// One resolve pass. See the module docs for the matching rules.
    pub fn resolve(
        &self,
        wrapper_pid: u32,
        host_executable: &Path,
        peer_executable: &Path,
    ) -> Result<ResolveOutcome, String> {
        if wrapper_pid == 0 || wrapper_pid == 1 {
            return Err("wrapper pid must be nonzero and not pid 1".into());
        }
        if host_executable == peer_executable {
            return Err("host and peer executables must be different files".into());
        }
        let host_expected = ExecutableIdentity::of_file(host_executable)?;
        let peer_expected = ExecutableIdentity::of_file(peer_executable)?;
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

        // Host: any descendant of the wrapper running the host file.
        let mut hosts: Vec<(StatFacts, ExecutableEvidence)> = Vec::new();
        for (&pid, (facts, _)) in &nodes {
            if pid == wrapper_pid {
                continue;
            }
            if let Some(evidence) = self.executable_match(pid, host_executable, &host_expected)? {
                hosts.push((*facts, evidence));
            }
        }
        let (host, host_evidence) = match hosts.len() {
            0 => {
                return Ok(ResolveOutcome::Pending {
                    stage: PendingStage::Host,
                    host: None,
                });
            }
            1 => hosts.pop().unwrap(),
            n => {
                return Err(format!(
                    "{n} descendants of wrapper {wrapper_pid} run the host executable; \
                     the host is ambiguous"
                ));
            }
        };

        // Peer: descendants of the host running the peer file.
        let mut peers: Vec<(StatFacts, ExecutableEvidence)> = Vec::new();
        for (&pid, (facts, _)) in &nodes {
            if pid == host.identity.pid || !is_descendant(&nodes, pid, host.identity.pid) {
                continue;
            }
            if let Some(evidence) = self.executable_match(pid, peer_executable, &peer_expected)? {
                peers.push((*facts, evidence));
            }
        }
        let (peer, peer_evidence) = match peers.len() {
            0 => {
                return Ok(ResolveOutcome::Pending {
                    stage: PendingStage::Peer,
                    host: Some(host.identity),
                });
            }
            1 => peers.pop().unwrap(),
            n => {
                return Err(format!(
                    "{n} descendants of host {} run the peer executable; \
                     the peer is ambiguous",
                    host.identity.pid
                ));
            }
        };

        // Retain the full chain wrapper -> ... -> host -> ... -> peer.
        // The peer's lineage already passes through the host.
        let mut chain: Vec<BoundNode> = Vec::new();
        for pid in lineage(&nodes, peer.identity.pid) {
            let (facts, _) = nodes[&pid];
            chain.push(BoundNode {
                identity: facts.identity,
                parent_pid: facts.parent_pid,
            });
        }
        let time = self.time_proof(&host, &peer)?;
        let binding = ProcessBinding {
            wrapper: wrapper.identity,
            host: host.identity,
            peer: peer.identity,
            chain,
            host_executable: host_evidence,
            peer_executable: peer_evidence,
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
                    return Err(format!(
                        "pid {} no longer lists pid {} as a child",
                        parent, node.identity.pid
                    ));
                }
            }
        }
        for (identity, evidence, prepared) in [
            (
                binding.host,
                &binding.host_executable,
                &binding.host_executable.prepared_path,
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

/// Semantic proof equality: every fact except per-read capture stamps.
fn proof_matches(a: &TimeNamespaceProof, b: &TimeNamespaceProof) -> bool {
    fn evidence_matches(a: &NamespaceEvidence, b: &NamespaceEvidence) -> bool {
        a.pid == b.pid
            && a.identity == b.identity
            && a.time_namespace_inode == b.time_namespace_inode
            && a.origin == b.origin
            && a.observed_target == b.observed_target
    }
    evidence_matches(&a.reference, &b.reference)
        && evidence_matches(&a.host, &b.host)
        && evidence_matches(&a.peer, &b.peer)
        && evidence_matches(&a.capturer, &b.capturer)
        && a.capturer_timens_offsets == b.capturer_timens_offsets
        && a.capturer_timens_offsets_origin == b.capturer_timens_offsets_origin
        && a.monotonic_offset == b.monotonic_offset
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
