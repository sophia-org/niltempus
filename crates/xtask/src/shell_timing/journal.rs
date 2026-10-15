//! Validate the paced peer's complete journal before exporting any samples.
use super::{MAX_BYTES, MAX_RECORDS, Offer};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Start {
        frames: usize,
        wire: String,
    },
    Permit {
        generation: u64,
        demand_usec: u64,
        received_usec: u64,
    },
    Offer {
        connection_epoch: u64,
        content_grant_epoch: u64,
        output: u64,
        candidate_generation: u64,
        submitted_monotonic_usec: u64,
    },
    Terminal {
        connection_epoch: u64,
        content_grant_epoch: u64,
        output: u64,
        candidate_generation: u64,
        outcome: String,
        kind: u16,
        reason: u16,
        observed_monotonic_usec: u64,
    },
    End {
        offered: usize,
        completed: usize,
    },
}

#[derive(Deserialize)]
struct Record {
    schema: u64,
    #[serde(flatten)]
    event: Event,
}

fn event(line: &str) -> Result<Event, String> {
    let record: Record = serde_json::from_str(line).map_err(|e| e.to_string())?;
    if record.schema != 1 {
        return Err("missing or unsupported journal schema".into());
    }
    Ok(record.event)
}

pub fn collect(journal: &str, expected: usize) -> Result<Vec<Offer>, String> {
    if journal.len() > MAX_BYTES as usize
        || !(1..=MAX_RECORDS).contains(&expected)
        || !journal.ends_with('\n')
    {
        return Err("invalid journal size, count or truncated final line".into());
    }
    let mut lines = journal.lines();
    match event(lines.next().ok_or("missing start")?)? {
        Event::Start { frames, wire }
            if frames == expected && matches!(wire.as_str(), "files" | "ipc") => {}
        _ => return Err("wrong start count or wire".into()),
    }
    let mut offers = Vec::with_capacity(expected);
    let mut last = 0;
    let mut identity = None;
    for generation in 1..=expected as u64 {
        let received = match event(lines.next().ok_or("missing permit")?)? {
            Event::Permit {
                generation: g,
                demand_usec,
                received_usec,
            } if g == generation
                && demand_usec > 0
                && demand_usec >= last
                && received_usec >= demand_usec =>
            {
                received_usec
            }
            _ => return Err("missing, out-of-order or invalid permit".into()),
        };
        let offer = match event(lines.next().ok_or("missing offer")?)? {
            Event::Offer {
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
                submitted_monotonic_usec,
            } if candidate_generation == generation
                && submitted_monotonic_usec >= received
                && ![connection_epoch, content_grant_epoch, output].contains(&0) =>
            {
                Offer {
                    connection_epoch,
                    content_grant_epoch,
                    output,
                    candidate_generation,
                    submitted_monotonic_usec,
                    outcome: "presented".into(),
                }
            }
            _ => return Err("missing, out-of-order or invalid offer".into()),
        };
        let current = (
            offer.connection_epoch,
            offer.content_grant_epoch,
            offer.output,
        );
        if identity.replace(current).is_some_and(|old| old != current) {
            return Err("grant or output changed during workload".into());
        }
        match event(lines.next().ok_or("missing terminal")?)? {
            Event::Terminal {
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
                outcome,
                kind,
                reason,
                observed_monotonic_usec,
            } if (
                connection_epoch,
                content_grant_epoch,
                output,
                candidate_generation,
            ) == offer.key()
                && outcome == "presented"
                && kind == 2
                && reason == 0
                && observed_monotonic_usec >= offer.submitted_monotonic_usec =>
            {
                last = observed_monotonic_usec
            }
            _ => return Err("missing, misattributed or non-presented terminal".into()),
        }
        offers.push(offer);
    }
    match event(lines.next().ok_or("missing end")?)? {
        Event::End { offered, completed } if offered == expected && completed == expected => {}
        _ => return Err("wrong end counts".into()),
    }
    if lines.next().is_some() {
        return Err("records after end".into());
    }
    Ok(offers)
}
