//! Campaign environment monitor: connects the supervised process
//! binding, the proc/sys capturer and the credentialled environment
//! receiver inside the runner's per-turn observation callback.
//!
//! The monitor owns no runner or provider decisions. It binds the
//! receiver before the wrapper starts, resolves the host and peer
//! beneath the owned wrapper pid, and accepts environment records
//! only in batches bracketed by binding rechecks. Accepted batches,
//! start and end captures (kept separate: their source kinds overlap,
//! so they are never concatenated into one capture), and every
//! retained receipt are serialized as evidence for the importer's
//! provider. Nothing here reads a session log; the only environment
//! records are datagrams whose kernel credentials and live pidfd
//! named the bound host at receive time.
//!
//! A host that emits and exits before the first bound drain leaves a
//! dead pidfd in the queue; the batch is refused and the outcome is
//! failure or ended, never a reconstructed success.
use crate::{
    shell_timing_environment::{SourceKind, SourceText},
    shell_timing_environment_capture::{Capturer, ProcessIdentity, ProcessRecheck},
    shell_timing_environment_socket::{EnvironmentReceipt, EnvironmentReceiver},
    shell_timing_process_binding::{Binder, BindingRecheck, ProcessBinding, ResolveOutcome},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Most accepted nonempty batches retained for one arm.
pub const MAX_BATCHES: usize = 64;

/// Serializable mirror of the environment module's `Interval`
/// (serialize-only there).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntervalRecord {
    pub start_usec: u64,
    pub end_usec: u64,
}

fn monotonic_usec() -> Result<u64, String> {
    let clock = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    u64::try_from(i128::from(clock.tv_sec) * 1_000_000 + i128::from(clock.tv_nsec) / 1_000)
        .map_err(|_| "environment monitor clock overflow".into())
}

/// Static monitor configuration. The campaign `Expect` is built by
/// the importer from the manifest and the binding, never here.
#[derive(Clone, Debug)]
pub struct MonitorConfig {
    pub host_executable: PathBuf,
    pub host_argv: Vec<String>,
    pub supervisor_executable: PathBuf,
    pub peer_executable: PathBuf,
    pub card: String,
    /// Inside the runner's private stage directory.
    pub environment_socket: PathBuf,
}

/// What one observation turn found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitorTurn {
    Pending,
    Bound,
    Ended,
}

/// Serializable mirror of `SourceKind` (the original is
/// serialize-only in its owning module).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKindRecord {
    Osrelease,
    ProcVersion,
    CpuOnline,
    Governor { cpu: u32 },
    ProcStat { pid: u32 },
    ProcStatus { pid: u32 },
    DrmDriver { card: String },
    HostLog,
    Note,
}

impl From<&SourceKind> for SourceKindRecord {
    fn from(kind: &SourceKind) -> Self {
        match kind {
            SourceKind::Osrelease => Self::Osrelease,
            SourceKind::ProcVersion => Self::ProcVersion,
            SourceKind::CpuOnline => Self::CpuOnline,
            SourceKind::Governor { cpu } => Self::Governor { cpu: *cpu },
            SourceKind::ProcStat { pid } => Self::ProcStat { pid: *pid },
            SourceKind::ProcStatus { pid } => Self::ProcStatus { pid: *pid },
            SourceKind::DrmDriver { card } => Self::DrmDriver { card: card.clone() },
            SourceKind::HostLog => Self::HostLog,
            SourceKind::Note => Self::Note,
        }
    }
}

/// One captured source, retained verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub kind: SourceKindRecord,
    pub origin: String,
    pub captured_usec: u64,
    pub text: String,
}

impl From<&SourceText> for SourceRecord {
    fn from(source: &SourceText) -> Self {
        Self {
            kind: SourceKindRecord::from(&source.kind),
            origin: source.origin.clone(),
            captured_usec: source.captured_usec,
            text: source.text.clone(),
        }
    }
}

fn records(sources: &[SourceText]) -> Vec<SourceRecord> {
    sources.iter().map(SourceRecord::from).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRecord {
    pub pid: u32,
    pub start_time_ticks: u64,
}

impl From<ProcessIdentity> for IdentityRecord {
    fn from(identity: ProcessIdentity) -> Self {
        Self {
            pid: identity.pid,
            start_time_ticks: identity.start_time_ticks,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutableRecord {
    pub prepared_path: String,
    pub observed_link: String,
    pub dev: u64,
    pub ino: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceRecord {
    pub pid: u32,
    pub identity: IdentityRecord,
    pub time_namespace_inode: u64,
    pub origin: String,
    pub observed_target: String,
    pub captured_usec: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRecord {
    pub reference: InitialTimeNamespaceRecord,
    pub host: NamespaceRecord,
    pub peer: NamespaceRecord,
    pub capturer: NamespaceRecord,
    pub capturer_timens_offsets: String,
    pub capturer_timens_offsets_origin: String,
    pub monotonic_offset_seconds: i64,
    pub monotonic_offset_nanoseconds: i64,
}

/// Linux's initial time namespace identity, not a proc read of PID 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialTimeNamespaceRecord {
    pub inode: u64,
}

/// Serializable mirror of `ProcessBinding` (the original is
/// serialize-only in its owning module). Deserialization is parsing,
/// never authentication; the provider re-derives authority from the
/// supervised run, not from these bytes alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingRecord {
    pub wrapper: IdentityRecord,
    pub host: IdentityRecord,
    pub supervisor: IdentityRecord,
    pub peer: IdentityRecord,
    pub chain: Vec<ChainRecord>,
    pub host_executable: ExecutableRecord,
    pub host_argv: Vec<String>,
    pub supervisor_executable: ExecutableRecord,
    pub peer_executable: ExecutableRecord,
    pub unexamined: Vec<ChainRecord>,
    pub uniqueness: String,
    pub time: TimeRecord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainRecord {
    pub identity: IdentityRecord,
    pub parent_pid: u32,
}

impl From<&ProcessBinding> for BindingRecord {
    fn from(binding: &ProcessBinding) -> Self {
        fn executable(
            evidence: &crate::shell_timing_process_binding::ExecutableEvidence,
        ) -> ExecutableRecord {
            ExecutableRecord {
                prepared_path: evidence.prepared_path.display().to_string(),
                observed_link: evidence.observed_link.display().to_string(),
                dev: evidence.identity.dev,
                ino: evidence.identity.ino,
            }
        }
        fn namespace(
            evidence: &crate::shell_timing_process_binding::NamespaceEvidence,
        ) -> NamespaceRecord {
            NamespaceRecord {
                pid: evidence.pid,
                identity: evidence.identity.into(),
                time_namespace_inode: evidence.time_namespace_inode,
                origin: evidence.origin.clone(),
                observed_target: evidence.observed_target.clone(),
                captured_usec: evidence.captured_usec,
            }
        }
        Self {
            wrapper: binding.wrapper.into(),
            host: binding.host.into(),
            supervisor: binding.supervisor.into(),
            peer: binding.peer.into(),
            chain: binding
                .chain
                .iter()
                .map(|node| ChainRecord {
                    identity: node.identity.into(),
                    parent_pid: node.parent_pid,
                })
                .collect(),
            host_executable: executable(&binding.host_executable),
            host_argv: binding.host_argv.clone(),
            supervisor_executable: executable(&binding.supervisor_executable),
            peer_executable: executable(&binding.peer_executable),
            unexamined: binding
                .unexamined
                .iter()
                .map(|node| ChainRecord {
                    identity: node.identity.into(),
                    parent_pid: node.parent_pid,
                })
                .collect(),
            uniqueness: binding.uniqueness.to_owned(),
            time: TimeRecord {
                reference: InitialTimeNamespaceRecord {
                    inode: binding.time.reference.inode,
                },
                host: namespace(&binding.time.host),
                peer: namespace(&binding.time.peer),
                capturer: namespace(&binding.time.capturer),
                capturer_timens_offsets: binding.time.capturer_timens_offsets.clone(),
                capturer_timens_offsets_origin: binding.time.capturer_timens_offsets_origin.clone(),
                monotonic_offset_seconds: binding.time.monotonic_offset.0,
                monotonic_offset_nanoseconds: binding.time.monotonic_offset.1,
            },
        }
    }
}

/// Start-of-run capture: static facts plus both process captures.
/// Kept separate from the end capture in the serialized evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartEvidence {
    /// Bracket around binding recheck and measurement capture.
    pub window: IntervalRecord,
    pub sources: Vec<SourceRecord>,
    /// Bracketing second stat reads, host then peer.
    pub closing_stats: Vec<SourceRecord>,
    pub host: IdentityRecord,
    pub peer: IdentityRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecheckRecord {
    Alive(IdentityRecord),
    Exited,
    Error(String),
}

/// End-of-run capture, taken even after failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndEvidence {
    /// Bracket around the end captures; `None` when a clock read
    /// failed (see `MonitorEvidence::finish_error`) - no timestamp
    /// is manufactured in that case.
    pub window: Option<IntervalRecord>,
    /// Static capture result, or the error that prevented it.
    pub static_sources: Result<Vec<SourceRecord>, String>,
    pub host_recheck: Option<RecheckRecord>,
    pub peer_recheck: Option<RecheckRecord>,
}

/// One accepted batch of receipts, bracketed by binding rechecks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchEvidence {
    /// From before the opening recheck to after the successful
    /// closing recheck. Every accepted receipt's `received_usec`
    /// lies inside it.
    pub interval: IntervalRecord,
    pub host_start_time_ticks: u64,
    pub peer_start_time_ticks: u64,
    /// Index of this batch's first entry in `receipts`.
    pub first_receipt: usize,
    pub receipt_count: usize,
}

/// The last successful full binding recheck bracket. Constant-size:
/// overwritten on every successful turn, empty or not. Its opening
/// timestamp is the provider's proven-alive lower bound: a normal
/// run ends its host and peer before `finish`, so later exit is
/// expected and is never asserted to have been survived past the
/// observed-exit turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenAlive {
    pub interval: IntervalRecord,
    pub host_start_time_ticks: u64,
    pub peer_start_time_ticks: u64,
}

/// Terminal classification. `NeverBound`, `NoReceipts`,
/// `EndedBeforeAccept` and `Failed` cannot qualify an arm's
/// environment. `EndedAfterEvidence` is the normal lifecycle: the
/// processes ended after accepted, bracketed evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorOutcome {
    NeverBound,
    Bound,
    NoReceipts,
    EndedBeforeAccept,
    EndedAfterEvidence,
    Failed,
}

/// Everything the monitor retains for one arm.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MonitorEvidence {
    pub schema: u8,
    pub outcome: MonitorOutcome,
    /// First hard failure or end reason, when any.
    pub reason: Option<String>,
    /// Initialization-to-finish capture window; `None` when the
    /// finish clock read failed (see `finish_error`).
    pub capture_window: Option<IntervalRecord>,
    /// Set when a finish-time clock read failed; the outcome is
    /// then `Failed` and no window timestamps are reported.
    pub finish_error: Option<String>,
    pub card: String,
    pub environment_socket: String,
    pub binding: Option<BindingRecord>,
    /// Kernel real uid/gid of the bound host, from its status.
    pub host_uid: Option<u32>,
    pub host_gid: Option<u32>,
    /// Last successful full binding recheck bracket, when any.
    pub last_proven_alive: Option<ProvenAlive>,
    pub start: Option<StartEvidence>,
    pub end: EndEvidence,
    /// Nonempty accepted batches only, at most `MAX_BATCHES`.
    pub batches: Vec<BatchEvidence>,
    pub receipts: Vec<EnvironmentReceipt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    AwaitingBinding,
    Bound,
    Ended,
    Failed,
}

/// Per-turn campaign environment monitor. Construct (and thereby
/// bind the receiver) before the wrapper starts.
pub struct EnvironmentMonitor {
    config: MonitorConfig,
    binder: Binder,
    capturer: Capturer,
    receiver: EnvironmentReceiver,
    initialized_usec: u64,
    state: State,
    wrapper_pid: Option<u32>,
    binding: Option<ProcessBinding>,
    host_uid: Option<u32>,
    host_gid: Option<u32>,
    last_proven_alive: Option<ProvenAlive>,
    start: Option<StartEvidence>,
    batches: Vec<BatchEvidence>,
    receipts: Vec<EnvironmentReceipt>,
    reason: Option<String>,
    error_reported: bool,
}

impl EnvironmentMonitor {
    pub fn new(config: MonitorConfig, binder: Binder, capturer: Capturer) -> Result<Self, String> {
        let initialized_usec = monotonic_usec()?;
        let receiver = EnvironmentReceiver::bind(&config.environment_socket)?;
        Ok(Self {
            config,
            binder,
            capturer,
            receiver,
            initialized_usec,
            state: State::AwaitingBinding,
            wrapper_pid: None,
            binding: None,
            host_uid: None,
            host_gid: None,
            last_proven_alive: None,
            start: None,
            batches: Vec::new(),
            receipts: Vec::new(),
            reason: None,
            error_reported: false,
        })
    }

    /// Record a hard failure: the first call reports it as an error
    /// to the observer seam; later turns report `Ended` so the
    /// runner keeps collecting end evidence.
    fn fail(&mut self, message: String) -> Result<MonitorTurn, String> {
        self.state = State::Failed;
        self.reason.get_or_insert_with(|| message.clone());
        if self.error_reported {
            return Ok(MonitorTurn::Ended);
        }
        self.error_reported = true;
        Err(message)
    }

    fn ended(&mut self, message: String) -> MonitorTurn {
        self.state = State::Ended;
        self.reason.get_or_insert(message);
        MonitorTurn::Ended
    }

    /// Identity established by this monitor while its owners remain bound.
    /// Diagnostic observers may sample these exact processes; discovery by
    /// executable name or an inner namespace PID is not an equivalent binding.
    pub fn live_binding(&self) -> Option<&ProcessBinding> {
        matches!(self.state, State::Bound)
            .then_some(self.binding.as_ref())
            .flatten()
    }

    /// One observation turn for the owned wrapper pid.
    pub fn observe(&mut self, wrapper_pid: u32) -> Result<MonitorTurn, String> {
        if let Some(pinned) = self.wrapper_pid {
            if pinned != wrapper_pid && self.state != State::Failed {
                let message =
                    format!("wrapper pid changed during run: {pinned} replaced by {wrapper_pid}");
                self.state = State::Failed;
                self.reason.get_or_insert_with(|| message.clone());
                if !self.error_reported {
                    self.error_reported = true;
                    return Err(message);
                }
            }
        } else {
            self.wrapper_pid = Some(wrapper_pid);
        }
        match self.state {
            State::Ended | State::Failed => Ok(MonitorTurn::Ended),
            State::AwaitingBinding => match self.binder.resolve(
                wrapper_pid,
                crate::shell_timing_process_binding::Executables {
                    host: &self.config.host_executable,
                    host_argv: &self.config.host_argv,
                    supervisor: &self.config.supervisor_executable,
                    peer: &self.config.peer_executable,
                },
            ) {
                Err(error) => self.fail(error),
                Ok(ResolveOutcome::Pending { .. }) => Ok(MonitorTurn::Pending),
                Ok(ResolveOutcome::Bound(binding)) => {
                    let binding = *binding;
                    match self.initialize(&binding) {
                        Ok(()) => {
                            self.binding = Some(binding);
                            self.state = State::Bound;
                            self.batch()
                        }
                        Err(InitError::Ended(message)) => Ok(self.ended(message)),
                        Err(InitError::Failed(message)) => self.fail(message),
                    }
                }
            },
            State::Bound => self.batch(),
        }
    }

    fn initialize(&mut self, binding: &ProcessBinding) -> Result<(), InitError> {
        let pre = monotonic_usec().map_err(InitError::Failed)?;
        match self.binder.recheck(binding) {
            Ok(BindingRecheck::Bound) => {}
            Ok(BindingRecheck::Ended) => {
                return Err(InitError::Ended(
                    "binding ended before start capture".into(),
                ));
            }
            Err(error) => return Err(InitError::Failed(error)),
        }
        // Capture failures (denial, malformed, static errors and
        // process errors alike) are Failed here: only an explicit
        // closing recheck below may classify a normal end.
        let measured = self
            .capturer
            .capture_measurement(&self.config.card, binding.host.pid, binding.peer.pid)
            .map_err(|error| InitError::Failed(format!("start capture failed: {error}")))?;
        if measured.host != binding.host || measured.peer != binding.peer {
            return Err(InitError::Failed(
                "start capture identities differ from the binding".into(),
            ));
        }
        let status = measured
            .sources
            .iter()
            .find(|source| {
                source.kind
                    == SourceKind::ProcStatus {
                        pid: binding.host.pid,
                    }
            })
            .ok_or_else(|| InitError::Failed("host status source missing".into()))?;
        let uid = parse_status_id(&status.text, "Uid").map_err(InitError::Failed)?;
        let gid = parse_status_id(&status.text, "Gid").map_err(InitError::Failed)?;
        // Closing full-chain recheck: the start evidence and the
        // proven-alive bracket are retained only if the whole chain
        // still binds after the captures.
        match self.binder.recheck(binding) {
            Ok(BindingRecheck::Bound) => {}
            Ok(BindingRecheck::Ended) => {
                return Err(InitError::Ended(
                    "binding ended before start evidence was retained".into(),
                ));
            }
            Err(error) => return Err(InitError::Failed(error)),
        }
        let post = monotonic_usec().map_err(InitError::Failed)?;
        self.last_proven_alive = Some(ProvenAlive {
            interval: IntervalRecord {
                start_usec: pre,
                end_usec: post,
            },
            host_start_time_ticks: binding.host.start_time_ticks,
            peer_start_time_ticks: binding.peer.start_time_ticks,
        });
        self.host_uid = Some(uid);
        self.host_gid = Some(gid);
        self.start = Some(StartEvidence {
            window: IntervalRecord {
                start_usec: pre,
                end_usec: post,
            },
            sources: records(&measured.sources),
            closing_stats: records(&measured.closing_stats),
            host: measured.host.into(),
            peer: measured.peer.into(),
        });
        Ok(())
    }

    /// One bracketed drain. Only a nonempty batch whose receipts all
    /// validate and whose closing recheck still binds is accepted.
    fn batch(&mut self) -> Result<MonitorTurn, String> {
        let binding = self.binding.clone().expect("batch requires a binding");
        let (uid, gid) = (self.host_uid.unwrap(), self.host_gid.unwrap());
        // The batch interval opens before the opening recheck.
        let pre = match monotonic_usec() {
            Ok(usec) => usec,
            Err(error) => return self.fail(error),
        };
        match self.binder.recheck(&binding) {
            Ok(BindingRecheck::Bound) => {}
            Ok(BindingRecheck::Ended) => {
                return Ok(self.ended("binding ended before batch".into()));
            }
            Err(error) => return self.fail(error),
        }
        let mut drained: Vec<EnvironmentReceipt> = Vec::new();
        loop {
            match self.receiver.receive() {
                Ok(Some(receipt)) => drained.push(receipt),
                Ok(None) => break,
                Err(error) => return self.fail(error),
            }
        }
        for receipt in &drained {
            if let Err(error) = receipt.require_host(binding.host.pid, uid, gid) {
                return self.fail(error);
            }
        }
        match self.binder.recheck(&binding) {
            Ok(BindingRecheck::Bound) => {}
            Ok(BindingRecheck::Ended) => {
                return Ok(self.ended("binding ended during batch".into()));
            }
            Err(error) => return self.fail(error),
        }
        // The interval closes after the successful closing recheck.
        let post = match monotonic_usec() {
            Ok(usec) => usec,
            Err(error) => return self.fail(error),
        };
        // Every successful turn, empty or not, refreshes the
        // constant-size proven-alive bracket.
        self.last_proven_alive = Some(ProvenAlive {
            interval: IntervalRecord {
                start_usec: pre,
                end_usec: post,
            },
            host_start_time_ticks: binding.host.start_time_ticks,
            peer_start_time_ticks: binding.peer.start_time_ticks,
        });
        if drained.is_empty() {
            return Ok(MonitorTurn::Bound);
        }
        if self.batches.len() >= MAX_BATCHES {
            return self.fail(format!("accepted batch count exceeds {MAX_BATCHES}"));
        }
        for receipt in &drained {
            if !(pre..=post).contains(&receipt.received_usec) {
                return self.fail("receipt timestamp lies outside its batch interval".into());
            }
        }
        self.batches.push(BatchEvidence {
            interval: IntervalRecord {
                start_usec: pre,
                end_usec: post,
            },
            host_start_time_ticks: binding.host.start_time_ticks,
            peer_start_time_ticks: binding.peer.start_time_ticks,
            first_receipt: self.receipts.len(),
            receipt_count: drained.len(),
        });
        self.receipts.extend(drained);
        Ok(MonitorTurn::Bound)
    }

    /// Finish the arm. Always captures end evidence, even after
    /// failure, and reports capture errors explicitly in the result.
    pub fn finish(mut self) -> MonitorEvidence {
        let pre = monotonic_usec();
        let static_sources = self
            .capturer
            .capture_static(&self.config.card)
            .map(|sources| records(&sources));
        let (host_recheck, peer_recheck) = match &self.binding {
            Some(binding) => (
                Some(recheck_record(&self.capturer, binding.host)),
                Some(recheck_record(&self.capturer, binding.peer)),
            ),
            None => (None, None),
        };
        let post = monotonic_usec();
        // A failed finish clock read manufactures no timestamps: the
        // windows are withheld, the error is retained, and the
        // outcome is Failed whatever the turns concluded.
        let (windows, finish_error) = match (pre, post) {
            (Ok(pre), Ok(post)) => (
                Some((
                    IntervalRecord {
                        start_usec: self.initialized_usec,
                        end_usec: post,
                    },
                    IntervalRecord {
                        start_usec: pre,
                        end_usec: post,
                    },
                )),
                None,
            ),
            (Err(error), _) | (_, Err(error)) => (None, Some(error)),
        };
        let outcome = if finish_error.is_some() {
            MonitorOutcome::Failed
        } else {
            match self.state {
                State::AwaitingBinding => MonitorOutcome::NeverBound,
                State::Failed => MonitorOutcome::Failed,
                State::Ended if self.receipts.is_empty() => MonitorOutcome::EndedBeforeAccept,
                State::Ended => MonitorOutcome::EndedAfterEvidence,
                State::Bound if self.receipts.is_empty() => MonitorOutcome::NoReceipts,
                State::Bound => MonitorOutcome::Bound,
            }
        };
        let mut reason = self.reason.take();
        if let Some(error) = &finish_error {
            reason.get_or_insert_with(|| format!("finish: {error}"));
        }
        MonitorEvidence {
            schema: 1,
            outcome,
            reason,
            capture_window: windows.map(|(capture_window, _)| capture_window),
            finish_error,
            card: self.config.card.clone(),
            environment_socket: self.config.environment_socket.display().to_string(),
            binding: self.binding.as_ref().map(BindingRecord::from),
            host_uid: self.host_uid,
            host_gid: self.host_gid,
            last_proven_alive: self.last_proven_alive,
            start: self.start.take(),
            end: EndEvidence {
                window: windows.map(|(_, end_window)| end_window),
                static_sources,
                host_recheck,
                peer_recheck,
            },
            batches: std::mem::take(&mut self.batches),
            receipts: std::mem::take(&mut self.receipts),
        }
    }
}

enum InitError {
    Ended(String),
    Failed(String),
}

fn recheck_record(capturer: &Capturer, identity: ProcessIdentity) -> RecheckRecord {
    match capturer.recheck(&identity) {
        Ok(ProcessRecheck::Alive(current)) => RecheckRecord::Alive(current.into()),
        Ok(ProcessRecheck::Exited) => RecheckRecord::Exited,
        Err(error) => RecheckRecord::Error(error),
    }
}

/// Real id (first column) from a four-value `Uid:`/`Gid:` status
/// line. All four values (real, effective, saved, filesystem) must
/// be present and numeric; the raw status text stays in the start
/// sources as evidence.
fn parse_status_id(status: &str, name: &str) -> Result<u32, String> {
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .ok_or_else(|| format!("status has no {name} line"))?;
    let values: Vec<&str> = line.split_whitespace().collect();
    if values.len() != 4 {
        return Err(format!(
            "status {name} line must carry four ids, found {}",
            values.len()
        ));
    }
    let parsed: Vec<u32> = values
        .iter()
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| format!("status {name} id {value:?} is not numeric"))
        })
        .collect::<Result<_, _>>()?;
    Ok(parsed[0])
}
