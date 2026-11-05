use super::super::*;

const OUTPUT: ContentOutputId = ContentOutputId {
    id: 1,
    generation: 1,
};
const WIDTH: u32 = 1920;

#[derive(Default)]
pub struct Host {
    published: bool,
    demands: usize,
    pub shown: Vec<Vec<(ContentResourceId, Vec<u8>)>>,
    displayed: Option<ContentRenderBundle>,
}

impl Host {
    fn indicators(&self, transport: &mut ShellTransportConnection<'_>, generation: u64) {
        transport
            .publish_indicators(
                TransactionId::from_raw(100 + generation),
                &ShellIndicatorSnapshot {
                    connection_epoch: transport.connection_epoch(),
                    generation,
                    active_output: Some(OutputId::from_raw(1)),
                    statuses: vec![ShellOutputStatus {
                        output: OutputId::from_raw(1),
                        focus_bits: 1,
                        layout: "fixture".into(),
                    }],
                    indicators: vec![ShellIndicator {
                        output: OutputId::from_raw(1),
                        indicator: 1,
                        action: 1,
                        slot: 0,
                        state_bits: if generation.is_multiple_of(2) { 0 } else { 1 },
                        label: "workspace".into(),
                    }],
                },
            )
            .unwrap();
    }

    pub fn turn(&mut self, transport: &mut ShellTransportConnection<'_>, now: u64) {
        if !self.published {
            transport
                .publish_content_output_facts(
                    TransactionId::from_raw(90),
                    1,
                    vec![ContentOutputFactsEntry {
                        output: OUTPUT,
                        local_width: WIDTH,
                        local_height: 1080,
                        scale_numerator: 1,
                        scale_denominator: 1,
                        scale_generation: 1,
                    }],
                )
                .unwrap();
            self.indicators(transport, 1);
            self.published = true;
        }
        transport.service_content_resources(now).unwrap();
        transport
            .service_content_allocation_requests(&[], now)
            .unwrap();
        while let Some((_, request)) = transport.next_content_allocation_request() {
            assert_eq!(
                (
                    request.operation,
                    request.role,
                    request.edge,
                    request.desired_width,
                    request.desired_height
                ),
                (1, 1, 1, WIDTH, 24)
            );
            transport
                .grant_content_allocation(
                    request.allocation_request_id,
                    ContentAllocationSnapshot {
                        native_opening: None,
                        output: OUTPUT,
                        allocation: ContentAllocationId {
                            id: 1,
                            generation: 1,
                        },
                        scale_generation: 1,
                        scale_numerator: 1,
                        scale_denominator: 1,
                        role: 1,
                        edge: 1,
                        margins: ContentMargins::default(),
                        logical: ContentLogicalRect {
                            x: 0,
                            y: 0,
                            width: WIDTH,
                            height: 24,
                        },
                        pixel: ContentPixelRect {
                            x: 0,
                            y: 0,
                            width: WIDTH,
                            height: 24,
                        },
                        parent: ContentAllocationId::default(),
                        anchor_parent_rect: ContentPixelRect::default(),
                        allowed_reservation_extent: 24,
                    },
                    &[],
                )
                .unwrap();
        }
        let allocations = transport.content_allocation_snapshots();
        transport
            .service_content_demands(&[OUTPUT], &allocations)
            .unwrap();
        while let Some((transaction, demand)) = transport.next_content_demand() {
            self.demands += 1;
            transport
                .grant_content_demand(transaction, demand.output, self.demands as u64, now)
                .unwrap();
        }
        let contexts = [ContentCandidateContext {
            output: OUTPUT,
            facts_generation: 1,
            interaction_generation: 1,
            allocations: &allocations,
        }];
        transport
            .service_content_candidates(&contexts, now)
            .unwrap();
        if let Some((output, generation)) = transport.next_content_submission() {
            let bundle = transport
                .begin_content_submission(output, generation, now)
                .unwrap();
            assert_eq!(generation, self.shown.len() as u64 + 1);
            let pixels = bundle
                .placements
                .iter()
                .map(|surface| {
                    let lease = bundle.resource(surface.resource).unwrap();
                    let bytes = lease
                        .cpu_bytes()
                        .expect("CPU producer must not import GPU storage");
                    assert!(!bytes.is_empty());
                    (surface.resource, bytes.to_vec())
                })
                .collect();
            transport
                .content_prepared(bundle.grant, output, generation, 1, 1, now)
                .unwrap();
            // Supplied display completion; no GL or scanout claim.
            transport
                .content_presented_at(bundle.grant, output, generation, generation, 1, 1, now)
                .unwrap();
            self.displayed = Some(bundle);
            self.shown.push(pixels);
            if generation < 4 {
                self.indicators(transport, generation + 1);
            }
        }
        transport.poll_io().unwrap();
    }

    pub fn assert_frames(&self, revision: u16) {
        assert_eq!(
            self.shown.iter().map(Vec::len).collect::<Vec<_>>(),
            [1, 3, 3, 3]
        );
        assert_eq!(
            self.demands,
            if revision == 13 { 2 } else { 4 },
            "initial full/region establishment use permits; subsequent frames use SFT"
        );
        for pair in self.shown[1..].windows(2) {
            assert_ne!(
                pair[0][0].0, pair[1][0].0,
                "changed workspace has a new resource"
            );
            assert_ne!(
                pair[0][0].1, pair[1][0].1,
                "changed workspace state changes actual Vello pixels"
            );
            assert_eq!(
                pair[0][1..],
                pair[1][1..],
                "other regions retain the exact resource and bytes"
            );
        }
    }
}
