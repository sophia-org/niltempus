//! Paired comparison of prepared shell timing arms against the accepted
//! budgets. Gates are cited from Sophia, never restated as new thresholds:
//! docs/sophia-shell-files.md Budgets and cost accounting scope, and the
//! lvlbjomq sampling rules. One condition is one refresh rate, so no report
//! here is a B9 verdict.
use crate::shell_timing::{Distribution, ImageSource, Measures, WindowedLatency};
use crate::shell_timing_cost::{Bounds, Report};
use serde::Serialize;
use std::collections::BTreeSet;

const PANEL_ACCEPT: &str = "docs/sophia-shell-files.md:1221";
const BULK_ACCEPT: &str = "docs/sophia-shell-files.md:1222";
const PRESENT: &str = "docs/sophia-shell-files.md:1223";
const PERMIT: &str = "docs/sophia-shell-files.md:1224";
const COST: &str = "docs/sophia-shell-files.md:1225,1231-1247; lvlbjomq:912-918";
const FAILURES: &str = "docs/sophia-shell-files.md:1222,1226";
/// Half the 250 ms permit TTL named by the permit budget.
const PERMIT_P99_BELOW_USEC: u64 = 125_000;
const PAIRS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Workload {
    Panel,
    Bulk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    Inconclusive,
}

/// The three arms of a pair: A is the IPC byte baseline, B files bytes and
/// C files sealed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Role {
    A,
    B,
    C,
}

impl Role {
    fn of(wire: &str, source: ImageSource) -> Result<Self, String> {
        match (wire, source) {
            ("ipc", ImageSource::Bytes) => Ok(Self::A),
            ("files", ImageSource::Bytes) => Ok(Self::B),
            ("files", ImageSource::Isb) => Ok(Self::C),
            _ => Err("arm wire and source name no comparison role".into()),
        }
    }
}

pub struct LatencyArm {
    pub label: String,
    pub wire: String,
    pub source: ImageSource,
    pub workload: Workload,
    pub latency: WindowedLatency,
}

pub struct CostArm {
    pub label: String,
    pub wire: String,
    pub source: ImageSource,
    pub workload: Workload,
    pub report: Report,
}

/// Points carry equal lower and upper values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Value {
    pub lower: u64,
    pub upper: u64,
}

impl Value {
    fn point(value: u64) -> Self {
        Self {
            lower: value,
            upper: value,
        }
    }
    fn bounds(bounds: &Bounds) -> Self {
        Self {
            lower: bounds.lower,
            upper: bounds.upper,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct GateResult {
    pub gate: &'static str,
    pub citation: &'static str,
    pub workload: Workload,
    /// Names the measured scope, including bar measures inside bulk arms.
    pub measure: &'static str,
    pub arm: String,
    pub baseline_arm: Option<String>,
    pub candidate: Value,
    pub baseline: Option<Value>,
    pub limit: u64,
    pub outcome: Outcome,
}

#[derive(Debug, Serialize)]
pub struct SourceEntry {
    pub workload: Workload,
    pub measure: &'static str,
    pub statistic: &'static str,
    pub sealed_arm: String,
    pub bytes_arm: String,
    pub sealed: Value,
    pub bytes: Value,
    /// Sealed minus bytes, for each bound.
    pub difference_lower: i128,
    pub difference_upper: i128,
    pub outcome: &'static str,
}

fn label(arm: &str) -> Result<(), String> {
    if arm.is_empty() || arm.len() > 128 || arm.chars().any(char::is_control) {
        return Err("invalid arm label".into());
    }
    Ok(())
}

/// The bulk workload carries exactly one 4 MiB transfer probe per frame.
fn probe_bytes(workload: Workload) -> u64 {
    match workload {
        Workload::Panel => 0,
        Workload::Bulk => 4_194_304,
    }
}

/// Every upload's generation, purpose and exact shape, warmup included.
/// Epochs and timestamps may differ between arms; the payloads may not.
type UploadShape = (u64, &'static str, u32, u32, u64);

fn upload_shapes(arm: &LatencyArm) -> Vec<UploadShape> {
    arm.latency
        .journal_metrics
        .uploads
        .iter()
        .map(|accepted| {
            let upload = &accepted.upload;
            (
                upload.candidate_generation,
                accepted.purpose,
                upload.width,
                upload.height,
                upload.total_bytes,
            )
        })
        .collect()
}

/// What two cost arms of one workload and method must share.
fn cost_signature(arm: &CostArm) -> Result<(&'static str, u64, u64, u64), String> {
    let (_, normalized) = cost_role(arm)?;
    Ok((
        arm.report.instrumentation,
        normalized.frame_count,
        normalized.presentation_bytes,
        normalized.transfer_probe_bytes,
    ))
}

fn latency_role(arm: &LatencyArm) -> Result<Role, String> {
    label(&arm.label)?;
    let latency = &arm.latency;
    if latency.wire != arm.wire
        || latency.source != arm.source
        || latency.probe_bytes != probe_bytes(arm.workload)
        || latency.sample_scope != "declared_warmup_excluded"
    {
        return Err(format!(
            "{}: arm identity differs from its evidence",
            arm.label
        ));
    }
    Role::of(&arm.wire, arm.source)
}

fn cost_role(arm: &CostArm) -> Result<(Role, &crate::shell_timing_cost::Normalized), String> {
    label(&arm.label)?;
    let normalized = arm
        .report
        .normalized
        .as_ref()
        .ok_or_else(|| format!("{}: cost arm lacks byte normalization", arm.label))?;
    if normalized.workload == crate::shell_timing::WorkloadMode::Retained {
        return Err(format!(
            "{}: retained cost arms have no B9 comparison until designed",
            arm.label
        ));
    }
    if normalized.source != arm.source
        || Some(normalized.transfer_probe_bytes)
            != normalized
                .frame_count
                .checked_mul(probe_bytes(arm.workload))
        || !matches!(arm.report.instrumentation, "cpu-only" | "rust-global")
    {
        return Err(format!(
            "{}: arm identity differs from its evidence",
            arm.label
        ));
    }
    Ok((Role::of(&arm.wire, arm.source)?, normalized))
}

fn same_latency_workload(a: &LatencyArm, b: &LatencyArm) -> Result<(), String> {
    let (x, y) = (&a.latency, &b.latency);
    let counts = |m: &Measures<Vec<u64>>| {
        [
            m.candidate_to_native_presented.len(),
            m.demand_to_permit.len(),
            m.presentation_upload_to_accepted.len(),
            m.probe_upload_to_accepted.len(),
        ]
    };
    if a.workload != b.workload
        || x.mode_millihz != y.mode_millihz
        || x.window != y.window
        || x.probe_bytes != y.probe_bytes
        || counts(&x.measured_samples) != counts(&y.measured_samples)
        || upload_shapes(a) != upload_shapes(b)
        || x.journal_metrics.schema != 2
        || y.journal_metrics.schema != 2
    {
        return Err(format!(
            "{} and {}: latency workloads differ",
            a.label, b.label
        ));
    }
    Ok(())
}

fn same_cost_workload<'a>(
    a: &'a CostArm,
    b: &'a CostArm,
) -> Result<
    (
        &'a crate::shell_timing_cost::Normalized,
        &'a crate::shell_timing_cost::Normalized,
    ),
    String,
> {
    let (_, x) = cost_role(a)?;
    let (_, y) = cost_role(b)?;
    if a.workload != b.workload
        || a.report.instrumentation != b.report.instrumentation
        || x.frame_count != y.frame_count
        || x.presentation_bytes != y.presentation_bytes
        || x.transfer_probe_bytes != y.transfer_probe_bytes
    {
        return Err(format!(
            "{} and {}: cost workloads differ",
            a.label, b.label
        ));
    }
    Ok((x, y))
}

fn required(value: Option<Distribution>, arm: &str, measure: &str) -> Result<Distribution, String> {
    value.ok_or_else(|| format!("{arm}: missing {measure} measurements"))
}

fn outcome(pass: bool) -> Outcome {
    if pass { Outcome::Pass } else { Outcome::Fail }
}

/// Compare one files arm with its paired IPC byte baseline. Percentile points
/// are compared per pair; the budgets carry no uncertainty bounds.
pub fn compare_latency(
    candidate: &LatencyArm,
    baseline: &LatencyArm,
) -> Result<Vec<GateResult>, String> {
    if latency_role(baseline)? != Role::A || latency_role(candidate)? == Role::A {
        return Err("latency gates compare a files arm with the IPC byte baseline".into());
    }
    same_latency_workload(candidate, baseline)?;
    let (c, b) = (
        &candidate.latency.measured_distributions,
        &baseline.latency.measured_distributions,
    );
    let mut results = Vec::new();
    let mut add = |gate, citation, measure, cv: u64, bv: u64, limit: u64| {
        results.push(GateResult {
            gate,
            citation,
            workload: candidate.workload,
            measure,
            arm: candidate.label.clone(),
            baseline_arm: Some(baseline.label.clone()),
            candidate: Value::point(cv),
            baseline: Some(Value::point(bv)),
            limit,
            outcome: outcome(cv <= limit),
        });
    };
    let bar = candidate.workload == Workload::Bulk;
    for (gates, citation, measure, cd, bd) in [
        (
            ["panel_accept_p95", "panel_accept_p99"],
            PANEL_ACCEPT,
            if bar {
                "bulk_arm_bar_upload_to_accepted"
            } else {
                "bar_upload_to_accepted"
            },
            c.presentation_upload_to_accepted,
            b.presentation_upload_to_accepted,
        ),
        (
            ["present_p95", "present_p99"],
            PRESENT,
            if bar {
                "bulk_arm_candidate_to_native_presented"
            } else {
                "candidate_to_native_presented"
            },
            c.candidate_to_native_presented,
            b.candidate_to_native_presented,
        ),
    ] {
        let cd = required(cd, &candidate.label, measure)?;
        let bd = required(bd, &baseline.label, measure)?;
        add(
            gates[0],
            citation,
            measure,
            cd.p95_usec,
            bd.p95_usec,
            bd.p95_usec.saturating_add(1_000),
        );
        add(
            gates[1],
            citation,
            measure,
            cd.p99_usec,
            bd.p99_usec,
            bd.p99_usec.saturating_add(2_000),
        );
    }
    if bar {
        let measure = "probe_upload_to_accepted";
        let cd = required(c.probe_upload_to_accepted, &candidate.label, measure)?;
        let bd = required(b.probe_upload_to_accepted, &baseline.label, measure)?;
        // p95 within IPC + 10%: candidate * 100 <= baseline * 110.
        let limit = u64::try_from(u128::from(bd.p95_usec) * 110 / 100)
            .map_err(|_| "latency limit overflow")?;
        add(
            "bulk_accept_p95",
            BULK_ACCEPT,
            measure,
            cd.p95_usec,
            bd.p95_usec,
            limit,
        );
    }
    Ok(results)
}

/// Gates without a baseline, evaluated on every arm: the permit p99 bound and
/// the transfer bound, which warmup frames cannot escape.
pub fn absolute_gates(arm: &LatencyArm) -> Result<Vec<GateResult>, String> {
    latency_role(arm)?;
    let permits = required(
        arm.latency.measured_distributions.demand_to_permit,
        &arm.label,
        "demand_to_permit",
    )?;
    let reached = arm.latency.transfer_bound_reached.len() as u64;
    let gate = |gate, citation, measure, value: u64, limit, pass| GateResult {
        gate,
        citation,
        workload: arm.workload,
        measure,
        arm: arm.label.clone(),
        baseline_arm: None,
        candidate: Value::point(value),
        baseline: None,
        limit,
        outcome: outcome(pass),
    };
    Ok(vec![
        gate(
            "permit_p99",
            PERMIT,
            "demand_to_permit",
            permits.p99_usec,
            PERMIT_P99_BELOW_USEC,
            permits.p99_usec < PERMIT_P99_BELOW_USEC,
        ),
        gate(
            "transfer_bound",
            FAILURES,
            // A count of uploads at or beyond shell_timing::TRANSFER_BOUND_USEC; none may.
            "uploads_reaching_transfer_bound_including_warmup",
            reached,
            0,
            reached == 0,
        ),
    ])
}

/// Conservative +10%: pass only when the candidate upper bound is at most
/// 1.10 times the baseline lower bound, fail only when the candidate lower
/// bound exceeds 1.10 times the baseline upper bound.
pub fn cost_outcome(candidate: Value, baseline: Value) -> Outcome {
    let scaled = |value: u64| u128::from(value) * 110;
    if u128::from(candidate.upper) * 100 <= scaled(baseline.lower) {
        Outcome::Pass
    } else if u128::from(candidate.lower) * 100 > scaled(baseline.upper) {
        Outcome::Fail
    } else {
        Outcome::Inconclusive
    }
}

/// cpu-only arms gate CPU per MiB; rust-global arms gate allocations per
/// frame. The other measure of each arm is not gated here.
pub fn compare_cost(candidate: &CostArm, baseline: &CostArm) -> Result<GateResult, String> {
    let (role, _) = cost_role(candidate)?;
    if cost_role(baseline)?.0 != Role::A || role == Role::A {
        return Err("cost gates compare a files arm with the IPC byte baseline".into());
    }
    let (c, b) = same_cost_workload(candidate, baseline)?;
    let (gate, measure, cv, bv) = if candidate.report.instrumentation == "cpu-only" {
        (
            "cost_cpu_per_mib",
            "cpu_ns_per_uploaded_mib",
            Value::bounds(&c.cpu_ns_per_uploaded_mib),
            Value::bounds(&b.cpu_ns_per_uploaded_mib),
        )
    } else {
        let allocations = |n: &crate::shell_timing_cost::Normalized, arm: &str| {
            n.allocation_operations_per_frame
                .as_ref()
                .map(Value::bounds)
                .ok_or_else(|| format!("{arm}: rust-global arm lacks allocation counts"))
        };
        (
            "cost_allocations_per_frame",
            "allocation_operations_per_frame",
            allocations(c, &candidate.label)?,
            allocations(b, &baseline.label)?,
        )
    };
    Ok(GateResult {
        gate,
        citation: COST,
        workload: candidate.workload,
        measure,
        arm: candidate.label.clone(),
        baseline_arm: Some(baseline.label.clone()),
        candidate: cv,
        baseline: Some(bv),
        limit: u64::try_from(u128::from(bv.lower) * 110 / 100)
            .map_err(|_| "cost limit overflow")?,
        outcome: cost_outcome(cv, bv),
    })
}

fn entry(
    workload: Workload,
    measure: &'static str,
    statistic: &'static str,
    sealed: (&str, Value),
    bytes: (&str, Value),
) -> SourceEntry {
    SourceEntry {
        workload,
        measure,
        statistic,
        sealed_arm: sealed.0.into(),
        bytes_arm: bytes.0.into(),
        sealed: sealed.1,
        bytes: bytes.1,
        difference_lower: i128::from(sealed.1.lower) - i128::from(bytes.1.lower),
        difference_upper: i128::from(sealed.1.upper) - i128::from(bytes.1.upper),
        outcome: "reported_only",
    }
}

fn source_pair(sealed: Role, bytes: Role) -> Result<(), String> {
    if sealed != Role::C || bytes != Role::B {
        return Err("source comparisons need a files sealed and a files bytes arm".into());
    }
    Ok(())
}

/// Sealed against bytes on the files wire. No budget exists for this, so
/// every entry is reported only.
pub fn compare_source_latency(
    sealed: &LatencyArm,
    bytes: &LatencyArm,
) -> Result<Vec<SourceEntry>, String> {
    source_pair(latency_role(sealed)?, latency_role(bytes)?)?;
    same_latency_workload(sealed, bytes)?;
    let (s, b) = (
        &sealed.latency.measured_distributions,
        &bytes.latency.measured_distributions,
    );
    let mut entries = Vec::new();
    for (measure, sd, bd) in [
        (
            "candidate_to_native_presented",
            s.candidate_to_native_presented,
            b.candidate_to_native_presented,
        ),
        ("demand_to_permit", s.demand_to_permit, b.demand_to_permit),
        (
            "presentation_upload_to_accepted",
            s.presentation_upload_to_accepted,
            b.presentation_upload_to_accepted,
        ),
        (
            "probe_upload_to_accepted",
            s.probe_upload_to_accepted,
            b.probe_upload_to_accepted,
        ),
    ] {
        let (Some(sd), Some(bd)) = (sd, bd) else {
            if sd.is_some() != bd.is_some() {
                return Err(format!("{measure}: present in only one arm"));
            }
            continue;
        };
        for (suffix, sv, bv) in [
            ("p95", sd.p95_usec, bd.p95_usec),
            ("p99", sd.p99_usec, bd.p99_usec),
        ] {
            entries.push(entry(
                sealed.workload,
                measure,
                suffix,
                (&sealed.label, Value::point(sv)),
                (&bytes.label, Value::point(bv)),
            ));
        }
    }
    Ok(entries)
}

pub fn compare_source_cost(sealed: &CostArm, bytes: &CostArm) -> Result<SourceEntry, String> {
    source_pair(cost_role(sealed)?.0, cost_role(bytes)?.0)?;
    let (s, b) = same_cost_workload(sealed, bytes)?;
    let (measure, sv, bv) = if sealed.report.instrumentation == "cpu-only" {
        (
            "cpu_ns_per_uploaded_mib",
            Value::bounds(&s.cpu_ns_per_uploaded_mib),
            Value::bounds(&b.cpu_ns_per_uploaded_mib),
        )
    } else {
        let allocations = |n: &crate::shell_timing_cost::Normalized| {
            n.allocation_operations_per_frame
                .as_ref()
                .map(Value::bounds)
                .ok_or("rust-global arm lacks allocation counts")
        };
        (
            "allocation_operations_per_frame",
            allocations(s)?,
            allocations(b)?,
        )
    };
    Ok(entry(
        sealed.workload,
        measure,
        "bounds",
        (&sealed.label, sv),
        (&bytes.label, bv),
    ))
}

pub struct PairInput {
    pub index: usize,
    /// Execution order, exactly as run; rotation is the manifest's rule.
    pub order: Vec<String>,
    pub latency: Vec<LatencyArm>,
    pub cost: Vec<CostArm>,
}

#[derive(Debug, Serialize)]
pub struct PairReport {
    pub index: usize,
    pub order: Vec<String>,
    /// C against A, including A and C absolute gates. Decides the verdict.
    pub sealed_vs_ipc: Vec<GateResult>,
    pub sealed_vs_ipc_outcome: Outcome,
    /// B against A, including B absolute gates. Reported separately.
    pub bytes_vs_ipc: Vec<GateResult>,
    pub bytes_vs_ipc_outcome: Outcome,
    pub sealed_vs_bytes: Vec<SourceEntry>,
}

#[derive(Debug, Serialize)]
pub struct Pooled {
    pub role: Role,
    pub workload: Workload,
    pub distributions: Measures<Option<Distribution>>,
}

#[derive(Debug, Serialize)]
pub struct ConditionReport {
    pub schema: u8,
    pub scope: String,
    pub verdict_scope: &'static str,
    pub verdict: Outcome,
    pub bytes_vs_ipc_scope: &'static str,
    pub bytes_vs_ipc_outcome: Outcome,
    pub pairs: Vec<PairReport>,
    /// Reported only. A pooled distribution never changes a verdict.
    pub pooled: Vec<Pooled>,
}

fn fold(results: &[GateResult]) -> Outcome {
    if results.iter().any(|r| r.outcome == Outcome::Fail) {
        Outcome::Fail
    } else if results.iter().any(|r| r.outcome == Outcome::Inconclusive) {
        Outcome::Inconclusive
    } else {
        Outcome::Pass
    }
}

fn fold_outcomes(outcomes: impl Iterator<Item = Outcome>) -> Outcome {
    let outcomes: Vec<_> = outcomes.collect();
    if outcomes.contains(&Outcome::Fail) {
        Outcome::Fail
    } else if outcomes.contains(&Outcome::Inconclusive) {
        Outcome::Inconclusive
    } else {
        Outcome::Pass
    }
}

const INSTRUMENTATION: [&str; 2] = ["cpu-only", "rust-global"];
const WORKLOADS: [Workload; 2] = [Workload::Panel, Workload::Bulk];
const ROLES: [Role; 3] = [Role::A, Role::B, Role::C];

fn find_latency(pair: &PairInput, role: Role, workload: Workload) -> Result<&LatencyArm, String> {
    let mut found = pair
        .latency
        .iter()
        .filter(|arm| arm.workload == workload && latency_role(arm).is_ok_and(|r| r == role));
    match (found.next(), found.next()) {
        (Some(arm), None) => Ok(arm),
        _ => Err(format!(
            "pair {}: needs exactly one {role:?} {workload:?} latency arm",
            pair.index
        )),
    }
}

fn find_cost<'a>(
    pair: &'a PairInput,
    role: Role,
    workload: Workload,
    instrumentation: &str,
) -> Result<&'a CostArm, String> {
    let mut found = pair.cost.iter().filter(|arm| {
        arm.workload == workload
            && arm.report.instrumentation == instrumentation
            && cost_role(arm).is_ok_and(|(r, _)| r == role)
    });
    match (found.next(), found.next()) {
        (Some(arm), None) => Ok(arm),
        _ => Err(format!(
            "pair {}: needs exactly one {role:?} {workload:?} {instrumentation} cost arm",
            pair.index
        )),
    }
}

fn pair_report(pair: &PairInput, include_bytes: bool) -> Result<PairReport, String> {
    let expected = if include_bytes { 3 } else { 2 } * WORKLOADS.len();
    if pair.latency.len() != expected || pair.cost.len() != expected * INSTRUMENTATION.len() {
        return Err(format!("pair {}: incomplete or extra arms", pair.index));
    }
    for arm in &pair.latency {
        latency_role(arm)?;
    }
    for arm in &pair.cost {
        cost_role(arm)?;
    }
    let labels: BTreeSet<&str> = pair
        .latency
        .iter()
        .map(|a| a.label.as_str())
        .chain(pair.cost.iter().map(|a| a.label.as_str()))
        .collect();
    let order: BTreeSet<&str> = pair.order.iter().map(String::as_str).collect();
    if order.len() != pair.order.len() || order != labels {
        return Err(format!(
            "pair {}: order must name every arm exactly once",
            pair.index
        ));
    }
    let mut sealed_vs_ipc = Vec::new();
    let mut bytes_vs_ipc = Vec::new();
    let mut sealed_vs_bytes = Vec::new();
    for workload in WORKLOADS {
        let a = find_latency(pair, Role::A, workload)?;
        let c = find_latency(pair, Role::C, workload)?;
        sealed_vs_ipc.extend(compare_latency(c, a)?);
        sealed_vs_ipc.extend(absolute_gates(a)?);
        sealed_vs_ipc.extend(absolute_gates(c)?);
        if include_bytes {
            let b = find_latency(pair, Role::B, workload)?;
            bytes_vs_ipc.extend(compare_latency(b, a)?);
            bytes_vs_ipc.extend(absolute_gates(b)?);
            sealed_vs_bytes.extend(compare_source_latency(c, b)?);
        }
        for instrumentation in INSTRUMENTATION {
            let a = find_cost(pair, Role::A, workload, instrumentation)?;
            let c = find_cost(pair, Role::C, workload, instrumentation)?;
            sealed_vs_ipc.push(compare_cost(c, a)?);
            if include_bytes {
                let b = find_cost(pair, Role::B, workload, instrumentation)?;
                bytes_vs_ipc.push(compare_cost(b, a)?);
                sealed_vs_bytes.push(compare_source_cost(c, b)?);
            }
        }
    }
    Ok(PairReport {
        index: pair.index,
        order: pair.order.clone(),
        sealed_vs_ipc_outcome: fold(&sealed_vs_ipc),
        sealed_vs_ipc,
        bytes_vs_ipc_outcome: if include_bytes {
            fold(&bytes_vs_ipc)
        } else {
            Outcome::Inconclusive
        },
        bytes_vs_ipc,
        sealed_vs_bytes,
    })
}

/// Exactly three complete restarted pairs of one condition. Every pair must
/// pass; a failing or inconclusive pair decides the verdict whatever the
/// pooled data show.
pub fn condition_report(pairs: &[PairInput]) -> Result<ConditionReport, String> {
    condition_report_for_roles(pairs, true)
}

/// ISB-only campaign: the retired file-byte arm is absent, not a passing arm.
/// The same C-versus-A and absolute gates decide every restarted pair.
pub fn isb_condition_report(pairs: &[PairInput]) -> Result<ConditionReport, String> {
    condition_report_for_roles(pairs, false)
}

fn condition_report_for_roles(
    pairs: &[PairInput],
    include_bytes: bool,
) -> Result<ConditionReport, String> {
    if pairs.len() != PAIRS {
        return Err("a condition requires exactly three pairs".into());
    }
    let indexes: BTreeSet<usize> = pairs.iter().map(|p| p.index).collect();
    if indexes != (1..=PAIRS).collect() {
        return Err("pair indexes must be 1, 2 and 3".into());
    }
    let mut labels = BTreeSet::new();
    for pair in pairs {
        for arm in pair
            .latency
            .iter()
            .map(|a| &a.label)
            .chain(pair.cost.iter().map(|a| &a.label))
        {
            if !labels.insert(arm.as_str()) {
                return Err(format!("arm label {arm} is not unique"));
            }
        }
    }
    let first = &pairs[0].latency.first().ok_or("empty pair")?.latency;
    let (mode, window) = (first.mode_millihz, first.window);
    if pairs
        .iter()
        .flat_map(|p| &p.latency)
        .any(|a| a.latency.mode_millihz != mode || a.latency.window != window)
    {
        return Err("a condition has one refresh rate and one latency window".into());
    }
    // Each pair matches internally; all three must also run one workload.
    for workload in WORKLOADS {
        let latency = find_latency(&pairs[0], Role::A, workload)?;
        let costs = INSTRUMENTATION
            .iter()
            .map(|method| find_cost(&pairs[0], Role::A, workload, method))
            .collect::<Result<Vec<_>, _>>()?;
        for pair in &pairs[1..] {
            same_latency_workload(latency, find_latency(pair, Role::A, workload)?)?;
            for (method, first) in INSTRUMENTATION.iter().zip(&costs) {
                if cost_signature(first)?
                    != cost_signature(find_cost(pair, Role::A, workload, method)?)?
                {
                    return Err(format!(
                        "pair {}: {workload:?} {method} workload differs from pair 1",
                        pair.index
                    ));
                }
            }
        }
    }
    let reports = pairs
        .iter()
        .map(|pair| pair_report(pair, include_bytes))
        .collect::<Result<Vec<_>, _>>()?;
    let mut pooled = Vec::new();
    let roles: &[Role] = if include_bytes {
        &ROLES
    } else {
        &[Role::A, Role::C]
    };
    for &role in roles {
        for workload in WORKLOADS {
            let mut samples = Measures::<Vec<u64>>::default();
            for pair in pairs {
                let arm = &find_latency(pair, role, workload)?.latency.measured_samples;
                samples
                    .candidate_to_native_presented
                    .extend(&arm.candidate_to_native_presented);
                samples.demand_to_permit.extend(&arm.demand_to_permit);
                samples
                    .presentation_upload_to_accepted
                    .extend(&arm.presentation_upload_to_accepted);
                samples
                    .probe_upload_to_accepted
                    .extend(&arm.probe_upload_to_accepted);
            }
            pooled.push(Pooled {
                role,
                workload,
                distributions: Measures {
                    candidate_to_native_presented: Distribution::from_samples(
                        &samples.candidate_to_native_presented,
                    ),
                    demand_to_permit: Distribution::from_samples(&samples.demand_to_permit),
                    presentation_upload_to_accepted: Distribution::from_samples(
                        &samples.presentation_upload_to_accepted,
                    ),
                    probe_upload_to_accepted: Distribution::from_samples(
                        &samples.probe_upload_to_accepted,
                    ),
                },
            });
        }
    }
    Ok(ConditionReport {
        schema: if include_bytes { 1 } else { 2 },
        scope: format!("single condition at {mode} mHz; not a B9 verdict"),
        verdict_scope: "files sealed (C) against IPC bytes (A): t331 gates after t338",
        verdict: fold_outcomes(reports.iter().map(|r| r.sealed_vs_ipc_outcome)),
        bytes_vs_ipc_scope: if include_bytes {
            "files bytes (B) against IPC bytes (A): reported, not the condition verdict"
        } else {
            "not run: file-byte uploads are retired; no B-arm result is claimed"
        },
        bytes_vs_ipc_outcome: fold_outcomes(reports.iter().map(|r| r.bytes_vs_ipc_outcome)),
        pairs: reports,
        pooled,
    })
}
