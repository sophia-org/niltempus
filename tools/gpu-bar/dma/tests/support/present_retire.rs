//! W2 prototype: `present` against a scripted session, both paths.
use super::*;
use std::collections::VecDeque;

const GRANT: ContentGrant = ContentGrant {
    connection_epoch: 1,
    content_grant_epoch: 1,
};
const OUTPUT: ContentOutputId = ContentOutputId {
    id: 1,
    generation: 1,
};

type Sent = (u64, Vec<ShellContentRecord>, Option<ContentCandidateRetire>);

/// Replies are queued up front, as if each arrived in one read.
#[derive(Default)]
struct Script {
    replies: VecDeque<(TransactionId, ShellContentRecord)>,
    sent: Vec<Sent>,
}

impl Dialogue for Script {
    fn send(&mut self, id: u64, records: &[ShellContentRecord]) -> Result<()> {
        self.sent.push((id, records.to_vec(), None));
        Ok(())
    }
    fn send_retiring(
        &mut self,
        id: u64,
        records: &[ShellContentRecord],
        retire: ContentCandidateRetire,
    ) -> Result<()> {
        self.sent.push((id, records.to_vec(), Some(retire)));
        Ok(())
    }
    fn receive_tx(&mut self) -> Result<(TransactionId, ShellContentRecord)> {
        self.replies.pop_front().ok_or("script exhausted".into())
    }
}

impl Script {
    fn reply(&mut self, tx: u64, record: ShellContentRecord) -> &mut Self {
        self.replies
            .push_back((TransactionId::from_raw(tx), record));
        self
    }
    /// Permit, Prepared, then the terminal outcome `kind`.
    fn frame(&mut self, generation: u64, kind: u16) -> &mut Self {
        self.reply(
            5,
            ShellContentRecord::FramePermit(ContentFramePermit {
                grant: GRANT,
                output: OUTPUT,
                demand_id: generation,
                permit_id: generation,
                state: 1,
                reason: 0,
                ttl_ms: 100,
                max_candidate_bytes: 4096,
            }),
        )
        .reply(6, outcome(generation, 1))
        .reply(6, outcome(generation, kind))
    }
    fn released(&mut self, tx: u64, grant: ContentGrant, resource: ContentResourceId) -> &mut Self {
        self.reply(
            tx,
            ShellContentRecord::ResourceReleased(ContentResourceReleased {
                grant,
                resource,
                reason: 0,
            }),
        )
    }
    fn ids(&self) -> Vec<u64> {
        self.sent.iter().map(|(id, _, _)| *id).collect()
    }
}

fn outcome(generation: u64, kind: u16) -> ShellContentRecord {
    ShellContentRecord::CandidateOutcome(ContentCandidateOutcome {
        grant: GRANT,
        candidate_generation: generation,
        output: OUTPUT,
        kind,
        reason: 0,
        presentation_epoch: u64::from(kind == 2),
        work_area_generation: 0,
        wm_commit_generation: 0,
    })
}

fn allocation() -> ContentAllocationResult {
    ContentAllocationResult {
        grant: GRANT,
        allocation_request_id: 1,
        status: 1,
        reason: 0,
        output: OUTPUT,
        allocation: ContentAllocationId {
            id: 1,
            generation: 1,
        },
        parent: ContentAllocationId::default(),
        scale_generation: 1,
        logical: ContentLogicalRect::default(),
        pixel: ContentPixelRect::default(),
        scale_numerator: 1,
        scale_denominator: 1,
        allowed_reservation_extent: 0,
        margins: ContentMargins::default(),
        acknowledged_anchor: ContentPixelRect::default(),
    }
}

fn run_frame(
    script: &mut Script,
    generation: u64,
    retire: Option<&mut RetireOnPresent>,
) -> Result<()> {
    run_counted(script, generation, retire, &mut RetireCounters::new())
}

fn run_counted(
    script: &mut Script,
    generation: u64,
    retire: Option<&mut RetireOnPresent>,
    counters: &mut RetireCounters,
) -> Result<()> {
    present(
        script,
        GRANT,
        &allocation(),
        OUTPUT,
        1,
        generation,
        retire,
        counters,
    )
}

fn counts(sent: u64, received: u64) -> RetireCounters {
    RetireCounters {
        retire_counter_schema: 1,
        explicit_retires_sent: sent,
        predecessor_releases_received: received,
    }
}

/// Both paths count one validated release per retired predecessor; only the
/// explicit path sends retires. A refused release and the first frame count
/// nothing.
#[test]
fn counters_follow_validated_exchanges_not_frames() {
    let mut script = Script::default();
    script.frame(1, 2);
    for generation in 2..=3 {
        script
            .frame(generation, 2)
            .released(7, GRANT, resource_for_frame(generation - 1));
    }
    let mut explicit = RetireCounters::new();
    for generation in 1..=3 {
        run_counted(&mut script, generation, None, &mut explicit).unwrap();
    }
    assert_eq!(explicit, counts(2, 2));

    let mut script = Script::default();
    script.frame(1, 2);
    for generation in 2..=3 {
        script
            .frame(generation, 2)
            .released(7, GRANT, resource_for_frame(generation - 1));
    }
    let mut tracker = RetireOnPresent::new(GRANT);
    let mut named = RetireCounters::new();
    for generation in 1..=3 {
        run_counted(&mut script, generation, Some(&mut tracker), &mut named).unwrap();
    }
    assert_eq!(named, counts(0, 2));

    let mut script = Script::default();
    script.frame(2, 2).released(7, GRANT, resource_for_frame(5));
    let mut refused = RetireCounters::new();
    assert!(run_counted(&mut script, 2, None, &mut refused).is_err());
    assert_eq!(refused, counts(1, 0));
    let mut script = Script::default();
    script.frame(2, 2).released(7, GRANT, resource_for_frame(5));
    let mut tracker = RetireOnPresent::new(GRANT);
    let mut refused = RetireCounters::new();
    assert!(run_counted(&mut script, 2, Some(&mut tracker), &mut refused).is_err());
    assert_eq!(refused, counts(0, 0));
}

fn intent_for(generation: u64) -> ContentCandidateRetire {
    ContentCandidateRetire {
        retire_transaction: TransactionId::from_raw(7),
        resource: resource_for_frame(generation - 1),
    }
}

#[test]
fn explicit_path_still_retires_the_predecessor_separately() {
    let mut script = Script::default();
    script.frame(2, 2).released(7, GRANT, resource_for_frame(1));
    run_frame(&mut script, 2, None).unwrap();
    assert_eq!(script.ids(), [5, 6, 7]);
    assert!(script.sent.iter().all(|(_, _, intent)| intent.is_none()));
    assert!(matches!(
        script.sent[2].1[..],
        [ShellContentRecord::ResourceRetire(ContentResourceRetire { resource, .. })]
            if resource == resource_for_frame(1)
    ));
}

#[test]
fn first_frame_names_nothing() {
    let mut script = Script::default();
    script.frame(1, 2);
    let mut tracker = RetireOnPresent::new(GRANT);
    run_frame(&mut script, 1, Some(&mut tracker)).unwrap();
    assert_eq!(script.ids(), [5, 6]);
    assert!(script.sent[1].2.is_none());
    assert!(script.replies.is_empty());
}

#[test]
fn successor_names_its_predecessor_and_sends_no_retire() {
    let mut script = Script::default();
    script.frame(2, 2).released(7, GRANT, resource_for_frame(1));
    let mut tracker = RetireOnPresent::new(GRANT);
    run_frame(&mut script, 2, Some(&mut tracker)).unwrap();
    assert_eq!(script.ids(), [5, 6]);
    assert_eq!(script.sent[1].2, Some(intent_for(2)));
    assert!(!tracker.holds(resource_for_frame(1)));
    assert!(script.replies.is_empty());
}

#[test]
fn rejected_successor_keeps_its_predecessor() {
    let mut script = Script::default();
    script.frame(2, 3);
    let mut tracker = RetireOnPresent::new(GRANT);
    assert!(run_frame(&mut script, 2, Some(&mut tracker)).is_err());
    // Kept: no longer named, never retiring, and no retire was sent.
    assert!(!tracker.holds(resource_for_frame(1)));
    assert_eq!(script.ids(), [5, 6]);
}

#[test]
fn only_the_exact_release_frees_the_predecessor() {
    let old = resource_for_frame(1);
    let stale = ContentGrant {
        content_grant_epoch: 2,
        ..GRANT
    };
    for (tx, grant, resource) in [
        (8, GRANT, old),
        (
            7,
            GRANT,
            ContentResourceId {
                generation: 2,
                ..old
            },
        ),
        (7, stale, old),
        // Unrelated to the tracker, so only the peer's own check refuses it.
        (
            9,
            GRANT,
            ContentResourceId {
                id: 9,
                generation: 9,
            },
        ),
    ] {
        let mut script = Script::default();
        script.frame(2, 2).released(tx, grant, resource);
        let mut tracker = RetireOnPresent::new(GRANT);
        assert!(run_frame(&mut script, 2, Some(&mut tracker)).is_err());
        assert!(tracker.holds(old), "{tx} {grant:?} {resource:?}");
    }
    // A release that never comes is a timeout, not a retirement.
    let mut script = Script::default();
    script.frame(2, 2);
    let mut tracker = RetireOnPresent::new(GRANT);
    assert!(run_frame(&mut script, 2, Some(&mut tracker)).is_err());
    assert!(tracker.holds(old));
}

#[test]
fn final_frame_drains_every_predecessor() {
    let mut script = Script::default();
    script.frame(1, 2);
    for generation in 2..=3 {
        script
            .frame(generation, 2)
            .released(7, GRANT, resource_for_frame(generation - 1));
    }
    let mut tracker = RetireOnPresent::new(GRANT);
    for generation in 1..=3 {
        run_frame(&mut script, generation, Some(&mut tracker)).unwrap();
    }
    assert_eq!(script.ids(), [5, 6, 5, 6, 5, 6]);
    drained(&tracker, 3).unwrap();
    // The current resource was never named; a named one fails the drain.
    tracker.name(OUTPUT, 4, intent_for(4)).unwrap();
    assert!(drained(&tracker, 4).is_err());
}

#[test]
fn retire_on_present_is_an_explicit_boolean() {
    const RING: &str = r#"{"wire":"files","output":1,"frames":2120,"height":24,"hold_msec":60000,"source":"owned_ring","ring_slots":2}"#;
    assert!(!parse_config(RING).unwrap().retire_on_present);
    let on = RING.replace('}', r#","retire_on_present":true}"#);
    assert!(parse_config(&on).unwrap().retire_on_present);
    for value in ["null", "1", "\"true\""] {
        assert!(parse_config(&on.replace("true", value)).is_err());
    }
}
