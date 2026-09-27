//! The Quickshell X11 trace probe, outside Sophia (Sophia rule 13).
//!
//! Moved from Sophia's `x-authority-quickshell-smoke` and
//! `x-authority-quickshell-software-smoke` at the pin. The old probes ran on
//! Sophia's private one-shot traced server and a shared external-probe
//! harness; this one serves the client on `sophia-x-authority`'s public
//! nonblocking frontend and judges the trace here. What the old code
//! actually asserted, what its prose claimed, and what changed are in
//! docs/quickshell.md. In short, this probe keeps every old acceptance
//! condition and removes four weaknesses the root review named: it examines
//! every X error rather than the first, compares stage tokens exactly,
//! refuses trace overflow instead of dropping evidence, and refuses a client
//! that never connected.
pub mod runner;

use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Which of the two moved probes is running.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Renderer {
    /// `x-authority-quickshell-smoke`: a measured render device and a pixmap
    /// allocator on one explicit render node (Qt's GL path).
    Gpu,
    /// `x-authority-quickshell-software-smoke`: `QT_QUICK_BACKEND=software`,
    /// no render device and no allocator. Runs device-hidden.
    Software,
}

impl Renderer {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "gpu" => Ok(Self::Gpu),
            "software" => Ok(Self::Software),
            _ => Err(format!("renderer must be gpu or software, not {value:?}")),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Software => "software",
        }
    }

    /// The exact request-stage tokens the trace must contain. Moved from the
    /// old stage table (external_probe.rs:347 at the pin): the GPU label
    /// required RENDER:QueryPictFormats and the software label had no entry.
    pub const fn required_stages(self) -> &'static [&'static str] {
        match self {
            Self::Gpu => &["RENDER:QueryPictFormats"],
            Self::Software => &[],
        }
    }
}

/// One X error the server sent the client, as the public trace reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedError {
    /// `XErrorCode`'s variant name, e.g. `BadWindow`.
    pub code: String,
    pub major: u8,
    pub minor: u16,
    pub resource: u32,
    pub sequence: u16,
}

impl ObservedError {
    pub fn token(&self) -> String {
        format!(
            "{}:major={}:minor={}:resource={:#x}:sequence={}",
            self.code, self.major, self.minor, self.resource, self.sequence
        )
    }
}

/// Whether an error is one of the explicitly allowed shapes. Nothing else is.
///
/// Both shapes are carried from the old code, not widened:
/// - `BadWindow` on resource 0 from GetWindowAttributes (3) or GetGeometry
///   (14) with minor 0: the old observer dropped these for every label before
///   they reached the verdict (external_probe.rs:98-104 at the pin).
/// - `BadWindow` from PresentSelectInput (major 138, minor 3), GPU only: the
///   old `probe_tolerates_client_error` (external_probe.rs:548). Qt recreates
///   its window and Mesa's DRI3 loader then unselects Present events on the
///   destroyed drawable; BadWindow is the correct answer. Sophia keeps that
///   answer under test in
///   `present_selection_after_destroy_reports_bad_window_and_keeps_serving`.
pub fn allowed(renderer: Renderer, error: &ObservedError) -> bool {
    let bad_window = error.code == "BadWindow";
    let null_window_query =
        bad_window && error.resource == 0 && error.minor == 0 && matches!(error.major, 3 | 14);
    let present_unselect_after_destroy =
        renderer == Renderer::Gpu && bad_window && error.major == 138 && error.minor == 3;
    null_window_query || present_unselect_after_destroy
}

/// How the client's run ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientEnd {
    /// Still running at the proof deadline, then stopped. Allowed: a shell
    /// does not exit (old `allow_proof_kill_without_transactions`).
    StoppedAtDeadline,
    /// Stopped early because the authority produced a transaction, as the old
    /// proof loop did once its minimum of one transaction was met.
    StoppedAfterTransaction,
    /// Exited by itself with this status.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

impl ClientEnd {
    fn describe(self) -> String {
        match self {
            Self::StoppedAtDeadline => "stopped_at_deadline".to_owned(),
            Self::StoppedAfterTransaction => "stopped_after_transaction".to_owned(),
            Self::Exited {
                code: Some(code), ..
            } => format!("exited:{code}"),
            Self::Exited {
                signal: Some(signal),
                ..
            } => format!("signalled:{signal}"),
            Self::Exited { .. } => "exited:unknown".to_owned(),
        }
    }
}

/// Everything the run observed, in the form the verdict is computed from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Observed {
    /// Dispatched requests, across every connection the client opened.
    pub requests: u64,
    pub connections: u64,
    /// Exact `X11ObservedRequestStage::evidence_name` tokens, `Other` excluded.
    pub stages: BTreeSet<String>,
    /// Every X error, in trace order. None is dropped before the verdict.
    pub errors: Vec<ObservedError>,
    /// Dispatch failures, as `name:major=M:minor=N`. The old probe counted
    /// any failure as an error; only [`excused_failure`] is left out.
    pub failures: Vec<String>,
    pub transactions: u64,
    /// The trace channel was full at least once, so evidence may be missing.
    pub overflowed: bool,
    /// The frontend itself failed (accept, worker, or teardown).
    pub server_error: Option<String>,
    /// The client wrote more than the log cap to stdout or stderr.
    pub client_log_exceeded: bool,
}

/// One dispatched request, reduced to the facts the verdict uses.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TraceRecord {
    /// The exact `evidence_name`, or `None` for `Other`.
    pub stage: Option<String>,
    pub major: u8,
    pub minor: u16,
    /// `X11ObservedDispatchFailure`'s variant name, if the dispatch failed.
    pub failure: Option<String>,
    pub errors: Vec<ObservedError>,
    pub transactions: u64,
    /// The observation was made after the probe initiated an intentional
    /// stop of the client. This says when it was observed, not what caused it.
    pub after_stop_initiated: bool,
}

/// Whether the probe stopped the client on purpose. A client that exited by
/// itself was not stopped; departures after that are not excused.
pub fn stop_is_intentional(end: ClientEnd) -> bool {
    !matches!(end, ClientEnd::Exited { .. })
}

/// The one failure left out of the verdict: `ClientDeparted` observed after
/// the probe initiated an intentional stop (director's ruling). Before any
/// stop, after a natural exit, or of any other kind, a failure is refused.
pub fn excused_failure(failure: &str, after_stop_initiated: bool) -> bool {
    after_stop_initiated && failure == "ClientDeparted"
}

impl Observed {
    /// Fold one request into the observation. X errors are kept whatever
    /// their timing; the verdict checks every one.
    pub fn record(&mut self, trace: TraceRecord) {
        self.requests += 1;
        if let Some(stage) = trace.stage {
            self.stages.insert(stage);
        }
        if let Some(failure) = trace.failure
            && !excused_failure(&failure, trace.after_stop_initiated)
        {
            self.failures.push(format!(
                "{failure}:major={}:minor={}",
                trace.major, trace.minor
            ));
        }
        self.errors.extend(trace.errors);
        self.transactions += trace.transactions;
    }
}

/// The verdict. `Ok` carries the accepted report line; `Err` names every
/// reason for refusal, not only the first.
pub fn evaluate(renderer: Renderer, observed: &Observed, end: ClientEnd) -> Result<String, String> {
    let mut refusals = Vec::new();
    if observed.overflowed {
        refusals.push("trace_overflow".to_owned());
    }
    if let Some(error) = &observed.server_error {
        refusals.push(format!("server_error={error}"));
    }
    if observed.client_log_exceeded {
        refusals.push("client_log_exceeded".to_owned());
    }
    if observed.connections == 0 || observed.requests == 0 {
        refusals.push("client_never_connected".to_owned());
    }
    for failure in &observed.failures {
        refusals.push(format!("dispatch_failure={failure}"));
    }
    for error in observed
        .errors
        .iter()
        .filter(|error| !allowed(renderer, error))
    {
        refusals.push(format!("x_error={}", error.token()));
    }
    for stage in renderer.required_stages() {
        if !observed.stages.contains(*stage) {
            refusals.push(format!("missing_stage={stage}"));
        }
    }
    if let ClientEnd::Exited { code, .. } = end
        && code != Some(0)
    {
        refusals.push(format!("client_failed={}", end.describe()));
    }
    let allowed_errors = observed
        .errors
        .iter()
        .filter(|error| allowed(renderer, error))
        .count();
    let mut line = format!(
        "quickshell_probe schema=1 renderer={} end={} connections={} requests={} transactions={} allowed_errors={} stages={}",
        renderer.name(),
        end.describe(),
        observed.connections,
        observed.requests,
        observed.transactions,
        allowed_errors,
        if observed.stages.is_empty() {
            "none".to_owned()
        } else {
            observed
                .stages
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        },
    );
    if refusals.is_empty() {
        line.push_str(" status=accepted");
        Ok(line)
    } else {
        let _ = write!(line, " status=refused reasons={}", refusals.join(";"));
        Err(line)
    }
}
