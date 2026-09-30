//! Evidence: recognized records in a stage log, and the KMS and owner
//! readback sets they form.
use std::collections::BTreeMap;

use sophia_conformance::record::after_marker;

use super::unsigned;

// ---- log records ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Peer,
    Kms,
    Owner,
    PeerLoss,
    ReadbackProof,
    Supervisor,
    Authority(u8),
    /// Session's own lifecycle records, by schema.
    Session(u8),
}

const MARKERS: [(&str, Kind); 12] = [
    ("sophia_output_proof schema=1 ", Kind::Peer),
    ("sophia_output_kms_readback schema=1 ", Kind::Kms),
    ("sophia_output_owner_readback schema=1 ", Kind::Owner),
    ("sophia_output_peer_loss_proof schema=1 ", Kind::PeerLoss),
    (
        "sophia_output_readback_proof schema=1 ",
        Kind::ReadbackProof,
    ),
    ("sophia_live_output_supervisor schema=1 ", Kind::Supervisor),
    ("sophia_live_output_authority schema=1 ", Kind::Authority(1)),
    ("sophia_live_output_authority schema=2 ", Kind::Authority(2)),
    ("sophia_live_output_authority schema=3 ", Kind::Authority(3)),
    ("sophia_live_session schema=7 ", Kind::Session(7)),
    ("sophia_live_session schema=18 ", Kind::Session(18)),
    ("sophia_live_session schema=19 ", Kind::Session(19)),
];

/// One recognized record: its line, kind and fields. `free_text` marks a
/// record whose trailing text is not `key=value` (an unquoted error); fields
/// before it are kept, and exact-shape records refuse it.
#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub(super) line: usize,
    pub(super) kind: Kind,
    pub(super) fields: BTreeMap<String, String>,
    pub(super) free_text: bool,
}

impl Entry {
    pub(super) fn get(&self, key: &str) -> Result<&str, String> {
        self.fields
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("line {}: missing {key}", self.line))
    }

    pub(super) fn is(&self, key: &str, value: &str) -> bool {
        self.fields.get(key).is_some_and(|v| v == value)
    }

    pub(super) fn u64(&self, key: &str) -> Result<u64, String> {
        unsigned(self.get(key)?, &format!("line {} {key}", self.line))
    }

    pub(super) fn truth(&self, key: &str) -> Result<bool, String> {
        match self.get(key)? {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(format!(
                "line {}: {key} is not a boolean: {other:?}",
                self.line
            )),
        }
    }

    fn one_of(&self, key: &str, allowed: &[&str]) -> Result<String, String> {
        let value = self.get(key)?;
        if !allowed.contains(&value) {
            return Err(format!(
                "line {}: {key}={value:?} is not one of {allowed:?}",
                self.line
            ));
        }
        Ok(value.to_owned())
    }

    /// Exactly these field names, in any order, and no free text.
    pub(super) fn shaped(&self, keys: &[&str]) -> bool {
        !self.free_text
            && self.fields.len() == keys.len()
            && keys.iter().all(|k| self.fields.contains_key(*k))
    }

    pub(super) fn require_shape(&self, keys: &[&str]) -> Result<(), String> {
        if !self.shaped(keys) {
            return Err(format!(
                "line {}: fields {:?} are not exactly {keys:?}",
                self.line,
                self.fields.keys().collect::<Vec<_>>()
            ));
        }
        Ok(())
    }
}

/// `key=value` tokens separated by single spaces; a value is bare or a Rust
/// debug-quoted string. The first token that is not `key=` ends the fields.
fn tokens(rest: &str, line: usize) -> Result<(BTreeMap<String, String>, bool), String> {
    let bytes = rest.as_bytes();
    let mut fields = BTreeMap::new();
    let mut at = 0;
    while at < bytes.len() {
        let key_end = rest[at..]
            .bytes()
            .position(|b| !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
            .map_or(bytes.len(), |p| at + p);
        if key_end == at || bytes.get(key_end) != Some(&b'=') {
            return Ok((fields, true));
        }
        let key = rest[at..key_end].to_owned();
        at = key_end + 1;
        let mut value = String::new();
        if bytes.get(at) == Some(&b'"') {
            at += 1;
            let mut chars = rest[at..].char_indices();
            let mut closed = false;
            while let Some((offset, c)) = chars.next() {
                match c {
                    '"' => {
                        at += offset + 1;
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next().map(|(_, e)| e) {
                        Some('"') => value.push('"'),
                        Some('\\') => value.push('\\'),
                        Some('\'') => value.push('\''),
                        Some('n') => value.push('\n'),
                        Some('r') => value.push('\r'),
                        Some('t') => value.push('\t'),
                        Some('0') => value.push('\0'),
                        _ => return Err(format!("line {line}: unsupported escape in {key}")),
                    },
                    c => value.push(c),
                }
            }
            if !closed {
                return Err(format!("line {line}: unterminated quoted {key}"));
            }
        } else {
            let end = rest[at..].find(' ').map_or(bytes.len(), |p| at + p);
            value.push_str(&rest[at..end]);
            at = end;
        }
        if fields.insert(key.clone(), value).is_some() {
            return Err(format!("line {line}: repeated field {key}"));
        }
        match bytes.get(at) {
            None => break,
            Some(b' ') => at += 1,
            Some(_) => return Err(format!("line {line}: text after quoted {key}")),
        }
    }
    Ok((fields, false))
}

/// Every recognized record, in line order. A line carrying two markers, or
/// a recognized record with an escape character, is refused.
pub(super) fn entries(text: &str) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let found = MARKERS
            .iter()
            .filter(|(marker, _)| raw.contains(marker))
            .collect::<Vec<_>>();
        let (marker, kind) = match found.as_slice() {
            [] => continue,
            [one] => **one,
            _ => return Err(format!("line {line}: more than one record marker")),
        };
        let rest = after_marker(raw, marker).ok_or("marker vanished")?;
        if rest.contains(marker) {
            return Err(format!("line {line}: repeated record marker"));
        }
        if rest.contains('\u{1b}') {
            return Err(format!("line {line}: escape characters in a record"));
        }
        let (fields, free_text) = tokens(rest, line)?;
        out.push(Entry {
            line,
            kind,
            fields,
            free_text,
        });
    }
    Ok(out)
}

// ---- readback sets ----

#[derive(Clone, Debug, PartialEq, Eq)]
struct KmsRow {
    head: u64,
    card: u64,
    connector: u64,
    crtc: u64,
    plane: u64,
    mode: String,
    properties: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnerHead {
    head: u64,
    enabled: bool,
    output: u64,
    native: (u64, u64, u64),
    scale: u64,
    refresh_millihz: u64,
    transform: String,
    mapping: String,
    vrr: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnerOutput {
    output: u64,
    logical: (u64, u64),
    scale: u64,
}

/// Display state compared across sets: never stage, time or attribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Display {
    kms: Vec<KmsRow>,
    heads: Vec<OwnerHead>,
    outputs: Vec<OwnerOutput>,
}

#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub(super) stage: String,
    pub(super) connection_epoch: u64,
    pub(super) transaction: u64,
    pub(super) base_topology_epoch: u64,
    /// The later of the two footers: the set is complete at this line.
    pub(super) line: usize,
    pub(super) display: Display,
}

impl Snapshot {
    pub(super) fn selections(&self) -> Vec<(u64, u64, u64, u64, u64)> {
        self.display
            .kms
            .iter()
            .map(|r| (r.head, r.card, r.connector, r.crtc, r.plane))
            .collect()
    }

    pub(super) fn enablement(&self) -> Vec<(u64, bool)> {
        self.display
            .heads
            .iter()
            .map(|h| (h.head, h.enabled))
            .collect()
    }

    pub(super) fn modes(&self) -> Vec<(u64, &str)> {
        self.display
            .kms
            .iter()
            .map(|r| (r.head, r.mode.as_str()))
            .collect()
    }
}

const KMS_ROW: [&str; 12] = [
    "stage",
    "t",
    "connection_epoch",
    "base_topology_epoch",
    "transaction",
    "head",
    "card",
    "connector",
    "crtc",
    "plane",
    "mode",
    "properties",
];
const KMS_FOOTER: [&str; 7] = [
    "stage",
    "t",
    "connection_epoch",
    "base_topology_epoch",
    "transaction",
    "heads",
    "complete",
];
const OWNER_HEAD: [&str; 16] = [
    "stage",
    "t",
    "connection_epoch",
    "base_topology_epoch",
    "transaction",
    "head",
    "enabled",
    "output",
    "native_width",
    "native_height",
    "native_scale",
    "scale",
    "refresh_millihz",
    "transform",
    "mapping",
    "vrr",
];
const OWNER_OUTPUT: [&str; 9] = [
    "stage",
    "t",
    "connection_epoch",
    "base_topology_epoch",
    "transaction",
    "output",
    "logical_width",
    "logical_height",
    "scale",
];
const OWNER_FOOTER: [&str; 8] = [
    "stage",
    "t",
    "connection_epoch",
    "base_topology_epoch",
    "transaction",
    "heads",
    "outputs",
    "complete",
];
const READBACK_STAGES: [&str; 7] = [
    "baseline",
    "peer_exit",
    "before",
    "applied",
    "installed",
    "presented",
    "restored",
];
const REQUIRED_PROPERTIES: [&str; 11] = [
    "connector.CRTC_ID",
    "crtc.ACTIVE",
    "plane.CRTC_ID",
    "plane.SRC_X",
    "plane.SRC_Y",
    "plane.SRC_W",
    "plane.SRC_H",
    "plane.CRTC_X",
    "plane.CRTC_Y",
    "plane.CRTC_W",
    "plane.CRTC_H",
];
const OPTIONAL_PROPERTIES: [&str; 2] = ["crtc.VRR_ENABLED", "plane.rotation"];
const TRANSFORMS: [&str; 8] = [
    "Normal",
    "Rotate90",
    "Rotate180",
    "Rotate270",
    "Flipped",
    "Flipped90",
    "Flipped180",
    "Flipped270",
];

/// Stage, time, connection epoch, transaction and base topology epoch.
type SetKey = (String, u64, u64, u64, u64);

fn set_key(entry: &Entry) -> Result<SetKey, String> {
    let stage = entry.get("stage")?.to_owned();
    if !READBACK_STAGES.contains(&stage.as_str()) {
        return Err(format!(
            "line {}: unknown readback stage {stage:?}",
            entry.line
        ));
    }
    Ok((
        stage,
        entry.u64("t")?,
        entry.u64("connection_epoch")?,
        entry.u64("transaction")?,
        entry.u64("base_topology_epoch")?,
    ))
}

fn kms_row(entry: &Entry) -> Result<KmsRow, String> {
    let line = entry.line;
    let properties = entry.get("properties")?.to_owned();
    let mut seen = BTreeMap::new();
    for item in properties.split(',') {
        let (name, value) = item
            .split_once(':')
            .ok_or_else(|| format!("line {line}: malformed property {item:?}"))?;
        if !REQUIRED_PROPERTIES.contains(&name) && !OPTIONAL_PROPERTIES.contains(&name) {
            return Err(format!("line {line}: unexpected property {name}"));
        }
        if seen.insert(name, unsigned(value, name)?).is_some() {
            return Err(format!("line {line}: repeated property {name}"));
        }
    }
    if let Some(missing) = REQUIRED_PROPERTIES.iter().find(|p| !seen.contains_key(*p)) {
        return Err(format!("line {line}: missing property {missing}"));
    }
    let mode = entry.get("mode")?.to_owned();
    if seen["crtc.ACTIVE"] != 0 {
        let parts = mode.split(',').collect::<Vec<_>>();
        if parts.len() != 13 || parts.iter().any(|p| unsigned(p, "mode").is_err()) {
            return Err(format!("line {line}: mode is not a 13-value timing tuple"));
        }
    } else if mode != "disabled" {
        return Err(format!(
            "line {line}: an inactive CRTC must have mode=disabled"
        ));
    }
    Ok(KmsRow {
        head: entry.u64("head")?,
        card: entry.u64("card")?,
        connector: entry.u64("connector")?,
        crtc: entry.u64("crtc")?,
        plane: entry.u64("plane")?,
        mode,
        properties,
    })
}

fn owner_head(entry: &Entry) -> Result<OwnerHead, String> {
    Ok(OwnerHead {
        head: entry.u64("head")?,
        enabled: entry.truth("enabled")?,
        output: entry.u64("output")?,
        native: (
            entry.u64("native_width")?,
            entry.u64("native_height")?,
            entry.u64("native_scale")?,
        ),
        scale: entry.u64("scale")?,
        refresh_millihz: entry.u64("refresh_millihz")?,
        transform: entry.one_of("transform", &TRANSFORMS)?,
        mapping: entry.one_of("mapping", &["Fit", "Cover", "Exact"])?,
        vrr: entry.one_of("vrr", &["Disabled", "Automatic", "Always"])?,
    })
}

fn owner_output(entry: &Entry) -> Result<OwnerOutput, String> {
    Ok(OwnerOutput {
        output: entry.u64("output")?,
        logical: (entry.u64("logical_width")?, entry.u64("logical_height")?),
        scale: entry.u64("scale")?,
    })
}

fn strictly_increasing(values: impl Iterator<Item = u64>) -> bool {
    let values = values.collect::<Vec<_>>();
    values.windows(2).all(|w| w[0] < w[1])
}

#[derive(Default)]
struct Rows {
    kms: Vec<KmsRow>,
    heads: Vec<OwnerHead>,
    outputs: Vec<OwnerOutput>,
    kms_footer: Option<(usize, u64)>,
    owner_footer: Option<(usize, u64, u64)>,
}

/// Group KMS and owner rows into sets keyed by stage, time, connection,
/// transaction and base epoch. Every set needs both footers, row counts
/// matching them, and rows in the Session's sorted order.
pub(super) fn snapshots(entries: &[Entry]) -> Result<Vec<Snapshot>, String> {
    let mut sets: BTreeMap<SetKey, Rows> = BTreeMap::new();
    for entry in entries {
        if !matches!(entry.kind, Kind::Kms | Kind::Owner) {
            continue;
        }
        let key = set_key(entry)?;
        let rows = sets.entry(key).or_default();
        let complete = || -> Result<(), String> {
            if entry.get("complete")? != "true" {
                return Err(format!("line {}: incomplete readback set", entry.line));
            }
            Ok(())
        };
        match entry.kind {
            Kind::Kms if entry.shaped(&KMS_ROW) => rows.kms.push(kms_row(entry)?),
            Kind::Kms if entry.shaped(&KMS_FOOTER) => {
                complete()?;
                if rows
                    .kms_footer
                    .replace((entry.line, entry.u64("heads")?))
                    .is_some()
                {
                    return Err(format!("line {}: repeated KMS set", entry.line));
                }
            }
            Kind::Kms => entry.require_shape(&KMS_ROW)?,
            _ if entry.shaped(&OWNER_HEAD) => rows.heads.push(owner_head(entry)?),
            _ if entry.shaped(&OWNER_OUTPUT) => rows.outputs.push(owner_output(entry)?),
            _ if entry.shaped(&OWNER_FOOTER) => {
                complete()?;
                let counts = (entry.u64("heads")?, entry.u64("outputs")?);
                if rows
                    .owner_footer
                    .replace((entry.line, counts.0, counts.1))
                    .is_some()
                {
                    return Err(format!("line {}: repeated owner set", entry.line));
                }
            }
            _ => entry.require_shape(&OWNER_HEAD)?,
        }
    }
    let mut out = Vec::new();
    for (key, rows) in sets {
        let describe = format!("{} set t={} transaction={}", key.0, key.1, key.3);
        let (kms_line, kms_count) = rows
            .kms_footer
            .ok_or_else(|| format!("{describe} has no KMS row footer"))?;
        let (owner_line, head_count, output_count) = rows
            .owner_footer
            .ok_or_else(|| format!("{describe} has no owner row footer"))?;
        if rows.kms.is_empty()
            || rows.outputs.is_empty()
            || rows.kms.len() as u64 != kms_count
            || rows.heads.len() as u64 != head_count
            || rows.outputs.len() as u64 != output_count
        {
            return Err(format!("{describe} rows do not match its footers"));
        }
        if !strictly_increasing(rows.kms.iter().map(|r| r.head))
            || !strictly_increasing(rows.heads.iter().map(|h| h.head))
            || !strictly_increasing(rows.outputs.iter().map(|o| o.output))
        {
            return Err(format!("{describe} rows are not in sorted order"));
        }
        out.push(Snapshot {
            stage: key.0,
            connection_epoch: key.2,
            transaction: key.3,
            base_topology_epoch: key.4,
            line: kms_line.max(owner_line),
            display: Display {
                kms: rows.kms,
                heads: rows.heads,
                outputs: rows.outputs,
            },
        });
    }
    out.sort_by_key(|s| s.line);
    Ok(out)
}
