//! Verdicts: the per-stage rules over a log's recognized records.
use std::collections::BTreeSet;

use super::custody::{Layout, StageRecord};
use super::evidence::{Entry, Kind, Snapshot, entries, snapshots};
use super::{STAGES, STARTUP_TRANSACTION};

// ---- stage verification ----

fn exactly_one<X>(items: impl IntoIterator<Item = X>, what: &str) -> Result<X, String> {
    let mut items = items.into_iter();
    match (items.next(), items.next()) {
        (Some(one), None) => Ok(one),
        (None, _) => Err(format!("expected exactly one {what}, found none")),
        (Some(_), Some(_)) => Err(format!("expected exactly one {what}, found more")),
    }
}

/// The peer's records, exactly shaped per event.
fn peer_shape(event: &str) -> Option<&'static [&'static str]> {
    Some(match event {
        "start" => &["a_topology_epoch", "a_heads", "b"],
        "negotiated" => &["epoch", "capabilities"],
        "topology" => &["topology_epoch", "qid", "heads", "groups", "match"],
        "ready" => &["topology_epoch"],
        "submit" => &["layout", "txn", "intent", "base_topology_epoch"],
        "submitted" => &["ticket"],
        "outcome" => &["txn", "kind", "reason", "topology_epoch"],
        "reject-candidate" => &["head", "absent_mode"],
        "awaiting-termination" => &["txn"],
        "pass" => &[],
        "fail" => &["exit", "reason"],
        "unexpected" => &["kind"],
        "submit-refused" => &["errno"],
        "session" => &["state", "remote_errno"],
        _ => return None,
    })
}

struct Stage<'e> {
    entries: &'e [Entry],
    peer: Vec<&'e Entry>,
    sets: Vec<Snapshot>,
    epoch: u64,
    connection: u64,
    ready_line: usize,
}

impl<'e> Stage<'e> {
    fn of(&self, kind: Kind, status: &str) -> Vec<&'e Entry> {
        self.entries
            .iter()
            .filter(|e| e.kind == kind && e.is("status", status))
            .collect()
    }

    fn authority(&self, schema: u8, status: &str) -> Vec<&'e Entry> {
        self.of(Kind::Authority(schema), status)
    }

    fn for_transaction(&self, schema: u8, status: &str, transaction: u64) -> Vec<&'e Entry> {
        let transaction = transaction.to_string();
        self.authority(schema, status)
            .into_iter()
            .filter(|e| e.is("transaction", &transaction))
            .collect()
    }

    fn events(&self, event: &str) -> Vec<&'e Entry> {
        self.peer
            .iter()
            .copied()
            .filter(|e| e.is("event", event))
            .collect()
    }

    /// Publications the peer read after it became ready.
    fn published(&self) -> Vec<&'e Entry> {
        self.events("topology")
            .into_iter()
            .filter(|t| t.line > self.ready_line)
            .collect()
    }

    fn set(&self, stage: &str, transaction: u64) -> Result<&Snapshot, String> {
        exactly_one(
            self.sets
                .iter()
                .filter(|s| s.stage == stage && s.transaction == transaction),
            &format!("{stage} readback set for transaction {transaction}"),
        )
    }
}

/// Records whose presence alone fails any stage, including every
/// interrupted-proof and unproven-restoration marker.
fn refuse_failures(entries: &[Entry]) -> Result<(), String> {
    for entry in entries {
        let status = entry.fields.get("status").map(String::as_str);
        let failed = match entry.kind {
            Kind::PeerLoss | Kind::ReadbackProof | Kind::Supervisor => status == Some("failed"),
            Kind::Authority(2) => matches!(
                status,
                Some("apply_rejected" | "resource_preparation_rejected" | "rollback_started")
            ),
            Kind::Authority(3) => matches!(
                status,
                Some("proof_rollback_triggered" | "quiescence_timed_out")
            ),
            _ => false,
        };
        if failed
            || entry
                .fields
                .values()
                .any(|v| v == "unproven" || v == "session_runtime_deadline")
        {
            return Err(format!("line {}: failure record", entry.line));
        }
    }
    Ok(())
}

pub(super) fn verify_stage(plan: &StageRecord, a: &Layout, text: &str) -> Result<(), String> {
    let peer_stage = STAGES
        .iter()
        .find(|(name, _)| *name == plan.name)
        .map(|(_, peer)| *peer)
        .ok_or("unknown stage")?;
    let entries = entries(text)?;
    refuse_failures(&entries)?;
    let epoch = a.epoch;
    let peer = entries
        .iter()
        .filter(|e| e.kind == Kind::Peer)
        .collect::<Vec<_>>();
    let mut last_t = 0;
    for entry in &peer {
        let event = entry.get("event")?;
        let shape = peer_shape(event)
            .ok_or_else(|| format!("line {}: unknown peer event {event}", entry.line))?;
        let mut keys = vec!["stage", "event", "t"];
        keys.extend_from_slice(shape);
        entry.require_shape(&keys)?;
        if !entry.is("stage", peer_stage) {
            return Err(format!(
                "line {}: peer stage is not {peer_stage}",
                entry.line
            ));
        }
        let t = entry.u64("t")?;
        if t < last_t {
            return Err(format!("line {}: peer time went backwards", entry.line));
        }
        last_t = t;
        if matches!(event, "unexpected" | "submit-refused" | "session") {
            return Err(format!("line {}: peer reported {event}", entry.line));
        }
    }
    let first = *peer.first().ok_or("no peer records")?;
    let start = exactly_one(
        peer.iter().copied().filter(|e| e.is("event", "start")),
        "peer start",
    )?;
    if !std::ptr::eq(first, start) {
        return Err("the peer's first record is not start".into());
    }
    let wants_b = if plan.name == "reject" { "no" } else { "yes" };
    if start.u64("a_topology_epoch")? != epoch || !start.is("b", wants_b) {
        return Err("the peer start does not match the declared layouts".into());
    }
    let negotiated = exactly_one(
        peer.iter().copied().filter(|e| e.is("event", "negotiated")),
        "peer negotiated",
    )?;
    let connection = negotiated.u64("epoch")?;
    if negotiated.u64("capabilities")? & 2 == 0 {
        return Err("the peer was not granted configure".into());
    }
    let ready = exactly_one(
        peer.iter().copied().filter(|e| e.is("event", "ready")),
        "peer ready",
    )?;
    let stage = Stage {
        sets: snapshots(&entries)?,
        entries: &entries,
        peer,
        epoch,
        connection,
        ready_line: ready.line,
    };
    if ready.u64("topology_epoch")? != epoch {
        return Err("the peer became ready at another epoch".into());
    }
    let topologies = stage.events("topology");
    let baseline_publication = topologies
        .iter()
        .rfind(|t| t.line < ready.line)
        .ok_or("no baseline publication")?;
    if baseline_publication.u64("topology_epoch")? != epoch
        || !baseline_publication.is("match", "a")
    {
        return Err("the baseline publication is not layout a at the declared epoch".into());
    }
    let mut qids = BTreeSet::new();
    for topology in &topologies {
        let qid = topology.u64("qid")?;
        if qid == 0 || !qids.insert(qid) {
            return Err(format!("line {}: publication Qid reused", topology.line));
        }
    }
    let connected = exactly_one(stage.authority(1, "connected"), "connected record")?;
    let disconnected = exactly_one(stage.authority(1, "disconnected"), "disconnected record")?;
    if connected.u64("epoch")? != connection || disconnected.u64("epoch")? != connection {
        return Err("connected or disconnected epoch is not the peer's".into());
    }
    let startup = exactly_one(
        stage.for_transaction(3, "settled_locally", STARTUP_TRANSACTION),
        "startup settlement",
    )?;
    if !startup.is("outcome", "Committed")
        || startup.u64("topology_epoch")? != epoch
        || !startup.is("reason", "desktop profile startup")
    {
        return Err("startup did not commit the profile at the declared epoch".into());
    }
    let baseline = exactly_one(
        stage.sets.iter().filter(|s| s.stage == "baseline"),
        "baseline readback",
    )?;
    let peer_exit = exactly_one(
        stage.sets.iter().filter(|s| s.stage == "peer_exit"),
        "peer_exit readback",
    )?;
    startup_and_publications(&stage, plan, startup, baseline)?;
    if baseline.connection_epoch != 0
        || baseline.transaction != 0
        || baseline.base_topology_epoch != epoch
        || peer_exit.connection_epoch != 0
        || peer_exit.transaction != 0
    {
        return Err("baseline or peer_exit readback is attributed".into());
    }
    if peer_exit.display != baseline.display {
        return Err("the display after the peer exited differs from the baseline".into());
    }
    if stage
        .sets
        .iter()
        .any(|s| s.transaction != 0 && s.connection_epoch != connection)
    {
        return Err("a candidate readback names another connection".into());
    }
    let exited = exactly_one(stage.of(Kind::Supervisor, "exited"), "supervisor exit")?;
    exited.require_shape(&["status", "peer", "code", "signal"])?;
    exited.u64("peer")?;
    let paused = exactly_one(
        stage.of(Kind::Supervisor, "pause_requested"),
        "pause request",
    )?;
    paused.require_shape(&["status", "peer"])?;
    if paused.get("peer")? != exited.get("peer")? || paused.line < exited.line {
        return Err("the pause does not follow the supervised peer's exit".into());
    }
    session_lifecycle(&stage, plan, startup.line)?;
    match plan.name.as_str() {
        "validate" => validate(&stage, exited),
        "reject" => reject(&stage, exited),
        "commit-restore" => commit_restore(&stage, exited, peer_exit),
        _ => peer_death(&stage, exited, peer_exit, disconnected),
    }
}

/// Startup is a real physical transaction, completed before baseline capture.
/// Snapshot publication IDs belong to a separate Session counter; bind them
/// to the committed topology epoch and owner-thread ordering, never peer IDs.
fn startup_and_publications(
    stage: &Stage<'_>,
    plan: &StageRecord,
    startup: &Entry,
    baseline: &Snapshot,
) -> Result<(), String> {
    let applied = exactly_one(
        stage.for_transaction(2, "apply_started", STARTUP_TRANSACTION),
        "startup apply",
    )?;
    let presented = exactly_one(
        stage.for_transaction(2, "first_presented", STARTUP_TRANSACTION),
        "startup presentation",
    )?;
    let committed = exactly_one(
        stage.for_transaction(2, "committed", STARTUP_TRANSACTION),
        "startup commit",
    )?;
    if committed.u64("topology_epoch")? != stage.epoch
        || !(applied.line < presented.line
            && presented.line < startup.line
            && startup.line < committed.line
            && committed.line < baseline.line)
    {
        return Err("startup effects do not establish the declared baseline".into());
    }
    let publications = stage.authority(2, "committed_snapshot_published");
    let expected = if plan.name == "commit-restore" { 3 } else { 1 };
    if publications.len() != expected {
        return Err(format!(
            "expected {expected} committed snapshot publications"
        ));
    }
    let submits = stage.events("submit");
    // No other physical candidate may hide beside the expected transactions.
    for status in ["apply_started", "first_presented", "committed"] {
        let runtime_allowed = plan.name == "commit-restore"
            || (plan.name == "peer-death" && status == "apply_started");
        for record in stage.authority(2, status) {
            let txn = record.u64("transaction")?;
            if txn != STARTUP_TRANSACTION
                && (!runtime_allowed || !submits.iter().any(|s| s.is("txn", &txn.to_string())))
            {
                return Err(format!(
                    "unexpected physical effect: {status} transaction={txn}"
                ));
            }
        }
    }
    let mut last_id = 0;
    for (index, publication) in publications.iter().enumerate() {
        let id = publication.u64("transaction")?;
        let epoch = stage
            .epoch
            .checked_add(index as u64)
            .ok_or("epoch overflow")?;
        if id <= last_id
            || publication.u64("topology_epoch")? != epoch
            || !publication.truth("transport_published")?
        {
            return Err("snapshot publication identity, epoch or transport differs".into());
        }
        last_id = id;
        let (settled, committed) = if index == 0 {
            (startup, committed)
        } else {
            let txn = submits
                .get(index - 1)
                .ok_or("missing commit submit")?
                .u64("txn")?;
            (
                exactly_one(
                    stage.for_transaction(1, "settled", txn),
                    "commit settlement",
                )?,
                exactly_one(
                    stage.for_transaction(2, "committed", txn),
                    "candidate commit",
                )?,
            )
        };
        // Worker and peer stdout can overtake these owner records after the
        // command is enqueued. Only owner-thread order is a synchronization fact.
        if !(settled.line < publication.line && publication.line < committed.line)
            || settled.u64("topology_epoch")? != epoch
            || committed.u64("topology_epoch")? != epoch
            || !settled.is("outcome", "Committed")
        {
            return Err(
                "snapshot publication is not bracketed by its owner settlement and commit".into(),
            );
        }
    }
    Ok(())
}

/// Session's own start and ordinary bounded completion around the stage
/// (sophia-session run.rs `status=running` schema 7; owner_loop
/// completion.rs `status=bounded_complete`, schema 19 when no startup proof
/// is requested, as in native normal mode). Both name the argv's display
/// and the runner's controls: native presentation, physical input and an
/// external WM. Completion reports no native scanout in flight and no
/// cleanup pending (bools read after suspension and cleanup). Other declared
/// fields are allowed in any order. The peer child may print before the
/// running record; the startup settlement, every readback set and every
/// candidate record may not. Completion follows all peer, proof, supervisor
/// and readback records.
fn session_lifecycle(
    stage: &Stage<'_>,
    plan: &StageRecord,
    startup_line: usize,
) -> Result<(), String> {
    let displays = plan
        .argv
        .iter()
        .filter_map(|a| a.strip_prefix("--display="))
        .collect::<Vec<_>>();
    let [display] = displays.as_slice() else {
        return Err("the Session argv has no single --display".into());
    };
    let sessions = stage
        .entries
        .iter()
        .filter(|e| matches!(e.kind, Kind::Session(_)))
        .collect::<Vec<_>>();
    let running = exactly_one(
        sessions
            .iter()
            .copied()
            .filter(|e| e.is("status", "running")),
        "Session running record",
    )?;
    let complete = exactly_one(
        sessions
            .iter()
            .copied()
            .filter(|e| e.is("status", "bounded_complete")),
        "Session completion record",
    )?;
    if running.kind != Kind::Session(7) || complete.kind != Kind::Session(19) {
        return Err("Session start or completion is not the ordinary native schema".into());
    }
    for record in [running, complete] {
        if record.free_text
            || !record.is("display", display)
            || !record.is("native_presentation", "enabled")
            || !record.is("physical_input", "enabled")
            || !record.is("wm_policy", "external")
        {
            return Err(format!(
                "line {}: Session record does not match the argv display and controls",
                record.line
            ));
        }
    }
    complete.u64("elapsed_msec")?;
    if complete.truth("native_in_flight")?
        || complete.truth("native_cleanup_pending")?
        || complete.u64("authority_batches_dropped")? != 0
    {
        return Err("Session completed with native scanout in flight or cleanup pending".into());
    }
    let first_physical = stage
        .sets
        .iter()
        .map(|s| s.line)
        .chain([startup_line])
        .min()
        .unwrap_or(startup_line);
    let last_work = stage
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                Kind::Peer | Kind::PeerLoss | Kind::Supervisor | Kind::Kms | Kind::Owner
            )
        })
        .map(|e| e.line)
        .max()
        .unwrap_or(0);
    if running.line > first_physical || complete.line < last_work {
        return Err("Session start and completion do not bracket the stage's work".into());
    }
    Ok(())
}

fn peer_passed(stage: &Stage<'_>) -> Result<(), String> {
    let last = stage.peer.last().ok_or("no peer records")?;
    if !last.is("event", "pass") || !stage.events("fail").is_empty() {
        return Err("the peer did not end with pass".into());
    }
    exactly_one(stage.events("pass"), "peer pass")?;
    Ok(())
}

fn clean_exit(exited: &Entry) -> Result<(), String> {
    if !exited.is("code", "0") || !exited.is("signal", "none") {
        return Err("the peer did not exit 0".into());
    }
    Ok(())
}

fn submit<'e>(stage: &Stage<'e>) -> Result<(&'e Entry, u64), String> {
    let submit = exactly_one(stage.events("submit"), "peer submit")?;
    Ok((submit, submit.u64("txn")?))
}

/// Validate and reject apply nothing and publish nothing new.
fn no_effects(stage: &Stage<'_>) -> Result<(), String> {
    if stage.sets.iter().any(|s| s.transaction != 0)
        || ["apply_started", "committed", "first_presented"]
            .iter()
            .any(|status| {
                stage
                    .authority(2, status)
                    .iter()
                    .any(|entry| !entry.is("transaction", &STARTUP_TRANSACTION.to_string()))
            })
        || !stage.published().is_empty()
    {
        return Err("a stage without a physical effect applied or published one".into());
    }
    Ok(())
}

fn validate(stage: &Stage<'_>, exited: &Entry) -> Result<(), String> {
    let (submit, txn) = submit(stage)?;
    if !submit.is("layout", "b")
        || !submit.is("intent", "validate-only")
        || submit.u64("base_topology_epoch")? != stage.epoch
    {
        return Err(
            "the validate submit is not layout b validate-only at the declared epoch".into(),
        );
    }
    let outcome = exactly_one(stage.events("outcome"), "peer outcome")?;
    if outcome.u64("txn")? != txn
        || !outcome.is("kind", "validated")
        || outcome.u64("topology_epoch")? != stage.epoch
    {
        return Err("the peer outcome is not Validated at the declared epoch".into());
    }
    let settled = exactly_one(
        stage.for_transaction(1, "settled", txn),
        "Validated settlement",
    )?;
    if !settled.is("outcome", "Validated") || settled.u64("topology_epoch")? != stage.epoch {
        return Err("Session did not settle Validated at the declared epoch".into());
    }
    no_effects(stage)?;
    peer_passed(stage)?;
    clean_exit(exited)
}

fn reject(stage: &Stage<'_>, exited: &Entry) -> Result<(), String> {
    exactly_one(stage.events("reject-candidate"), "reject candidate")?;
    let (submit, txn) = submit(stage)?;
    if !submit.is("layout", "a-unknown-mode")
        || !submit.is("intent", "apply")
        || submit.u64("base_topology_epoch")? != stage.epoch
    {
        return Err("the reject submit is not the unknown-mode apply".into());
    }
    let outcome = exactly_one(stage.events("outcome"), "peer outcome")?;
    if outcome.u64("txn")? != txn
        || !outcome.is("kind", "rejected")
        || outcome.u64("reason")? != 7
        || outcome.u64("topology_epoch")? != stage.epoch
    {
        return Err("the peer outcome is not Rejected (invariant) at the declared epoch".into());
    }
    let rejected = exactly_one(
        stage.for_transaction(1, "rejected", txn),
        "admission rejection",
    )?;
    // Transport admission refuses the absent mode with a quoted reason and
    // never reaches the owner; the owner's own refusal logs error= and then
    // settles, which this stage does not accept.
    if !rejected.is("phase", "admission")
        || rejected.free_text
        || !rejected.fields.contains_key("reason")
    {
        return Err("the rejection was not the transport admission refusal".into());
    }
    if !stage.for_transaction(1, "settled", txn).is_empty() {
        return Err("a rejected candidate reached owner settlement".into());
    }
    no_effects(stage)?;
    peer_passed(stage)?;
    clean_exit(exited)
}

/// The applied set changes KMS timing; installation keeps selections and
/// enablement; the order is before, applied, installed.
fn physical_sequence<'s>(
    stage: &'s Stage<'_>,
    txn: u64,
    base: u64,
) -> Result<[&'s Snapshot; 3], String> {
    let before = stage.set("before", txn)?;
    let applied = stage.set("applied", txn)?;
    let installed = stage.set("installed", txn)?;
    if [before, applied, installed]
        .iter()
        .any(|s| s.base_topology_epoch != base)
    {
        return Err(format!(
            "readback for transaction {txn} names another base epoch"
        ));
    }
    if !(before.line < applied.line && applied.line < installed.line) {
        return Err(format!("readback for transaction {txn} is out of order"));
    }
    if applied.modes() == before.modes() {
        return Err(format!("transaction {txn} did not change KMS mode timing"));
    }
    if installed.selections() != before.selections()
        || installed.enablement() != before.enablement()
    {
        return Err(format!(
            "transaction {txn} changed head selections or enablement"
        ));
    }
    Ok([before, applied, installed])
}

fn commit_restore(stage: &Stage<'_>, exited: &Entry, peer_exit: &Snapshot) -> Result<(), String> {
    let submits = stage.events("submit");
    let outcomes = stage.events("outcome");
    let published = stage.published();
    if submits.len() != 2 || outcomes.len() != 2 || published.len() != 2 {
        return Err("commit-restore needs two submits, outcomes and publications".into());
    }
    let mut displays = Vec::new();
    for (index, layout) in ["b", "a"].into_iter().enumerate() {
        let base = stage.epoch + index as u64;
        let committed_epoch = base + 1;
        let submit = submits[index];
        let txn = submit.u64("txn")?;
        if !submit.is("layout", layout)
            || !submit.is("intent", "apply")
            || submit.u64("base_topology_epoch")? != base
        {
            return Err(format!(
                "commit-restore submit {} is not layout {layout}",
                index + 1
            ));
        }
        let (outcome, topology) = (outcomes[index], published[index]);
        if outcome.u64("txn")? != txn
            || !outcome.is("kind", "committed")
            || outcome.u64("topology_epoch")? != committed_epoch
            || topology.u64("topology_epoch")? != committed_epoch
            || !topology.is("match", layout)
            || !(submit.line < outcome.line && outcome.line < topology.line)
        {
            return Err(format!(
                "transaction {txn} did not commit and publish layout {layout}"
            ));
        }
        exactly_one(
            stage.for_transaction(2, "apply_started", txn),
            "apply start",
        )?;
        for (schema, status) in [(2, "committed"), (1, "settled")] {
            let record = exactly_one(stage.for_transaction(schema, status, txn), status)?;
            if record.u64("topology_epoch")? != committed_epoch
                || (status == "settled" && !record.is("outcome", "Committed"))
            {
                return Err(format!(
                    "transaction {txn} {status} names another epoch or outcome"
                ));
            }
        }
        let first_presented = exactly_one(
            stage.for_transaction(2, "first_presented", txn),
            "candidate first presentation",
        )?;
        let settlement = exactly_one(stage.for_transaction(1, "settled", txn), "settlement")?;
        if first_presented.line >= settlement.line {
            return Err("candidate settlement precedes first presentation".into());
        }
        let [before, _, installed] = physical_sequence(stage, txn, base)?;
        let presented = stage.set("presented", txn)?;
        if presented.line < installed.line || presented.base_topology_epoch != base {
            return Err(format!("transaction {txn} presented out of order"));
        }
        displays.push((before.display.clone(), presented.display.clone()));
    }
    if submits[0].u64("txn")? >= submits[1].u64("txn")? || outcomes[0].line > submits[1].line {
        return Err("commit-restore transactions are out of order".into());
    }
    if displays[1].1 != displays[0].0 {
        return Err("restoring layout a did not return to the pre-apply display".into());
    }
    if stage.sets.iter().any(|s| s.stage == "restored")
        || peer_exit.base_topology_epoch != stage.epoch + 2
    {
        return Err("commit-restore rolled back or ended at another epoch".into());
    }
    peer_passed(stage)?;
    clean_exit(exited)
}

fn peer_death(
    stage: &Stage<'_>,
    exited: &Entry,
    peer_exit: &Snapshot,
    disconnected: &Entry,
) -> Result<(), String> {
    let (submit, txn) = submit(stage)?;
    if !submit.is("layout", "b")
        || !submit.is("intent", "apply")
        || submit.u64("base_topology_epoch")? != stage.epoch
    {
        return Err("the peer-death submit is not layout b apply at the declared epoch".into());
    }
    let waiting = exactly_one(stage.events("awaiting-termination"), "awaiting-termination")?;
    if waiting.u64("txn")? != txn {
        return Err("awaiting-termination names another transaction".into());
    }
    if ["outcome", "pass", "fail"]
        .iter()
        .any(|e| !stage.events(e).is_empty())
        || !stage.published().is_empty()
    {
        return Err("the terminated peer reported an outcome, verdict or publication".into());
    }
    let requested = exactly_one(
        stage.of(Kind::PeerLoss, "termination_requested"),
        "termination request",
    )?;
    requested.require_shape(&[
        "status",
        "epoch",
        "transaction",
        "peer",
        "boundary",
        "deadline_ms",
    ])?;
    requested.u64("deadline_ms")?;
    if requested.u64("epoch")? != stage.connection
        || requested.u64("transaction")? != txn
        || !requested.is("boundary", "all_cards_applied")
    {
        return Err("the termination request names another epoch, transaction or boundary".into());
    }
    if exited.get("peer")? != requested.get("peer")?
        || !exited.is("code", "none")
        || !(exited.is("signal", "15") || exited.is("signal", "9"))
    {
        return Err("the requested peer did not exit by TERM or KILL".into());
    }
    exactly_one(
        stage.for_transaction(2, "apply_started", txn),
        "apply start",
    )?;
    if stage
        .for_transaction(2, "cancellation_observed", txn)
        .is_empty()
    {
        return Err("no cancellation was observed for the held transaction".into());
    }
    let rolled_back = exactly_one(stage.for_transaction(2, "rolled_back", txn), "rollback")?;
    let settled = exactly_one(
        stage.for_transaction(3, "settled_locally", txn),
        "local rollback settlement",
    )?;
    if !settled.is("outcome", "RolledBack") || settled.u64("topology_epoch")? != stage.epoch {
        return Err("the held transaction did not settle RolledBack at the declared epoch".into());
    }
    for (schema, status) in [(1, "settled"), (2, "committed"), (2, "first_presented")] {
        if !stage.for_transaction(schema, status, txn).is_empty() {
            return Err(format!("the held transaction reached {status}"));
        }
    }
    let [before, applied, _] = physical_sequence(stage, txn, stage.epoch)?;
    let restored = stage.set("restored", txn)?;
    if stage
        .sets
        .iter()
        .any(|s| s.stage == "presented" && s.transaction == txn)
    {
        return Err("the held transaction was presented".into());
    }
    if restored.base_topology_epoch != stage.epoch || restored.display != before.display {
        return Err("the restored display differs from the pre-apply display".into());
    }
    if peer_exit.base_topology_epoch != stage.epoch {
        return Err("peer death changed the published epoch".into());
    }
    let passed = exactly_one(stage.of(Kind::PeerLoss, "passed"), "peer-loss verdict")?;
    passed.require_shape(&[
        "status",
        "epoch",
        "transaction",
        "disconnected",
        "terminated",
        "restored",
    ])?;
    if passed.u64("epoch")? != stage.connection
        || passed.u64("transaction")? != txn
        || !passed.truth("disconnected")?
        || !passed.truth("terminated")?
        || !passed.truth("restored")?
    {
        return Err("the peer-loss verdict names another epoch or transaction".into());
    }
    // Exit and disconnect may come in either order. Both follow the
    // request; the verdict follows both, the rollback, its local settlement
    // and the restored readback.
    let after_request = [
        exited.line,
        disconnected.line,
        rolled_back.line,
        restored.line,
    ];
    let before_verdict = [
        exited.line,
        disconnected.line,
        rolled_back.line,
        restored.line,
        settled.line,
    ];
    if !(before.line < requested.line
        && applied.line < requested.line
        && after_request.iter().all(|l| *l > requested.line)
        && before_verdict.iter().all(|l| *l < passed.line))
    {
        return Err("peer-death records are out of order".into());
    }
    Ok(())
}
