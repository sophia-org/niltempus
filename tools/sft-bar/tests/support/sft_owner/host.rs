use super::*;

const OUTPUT: ContentOutputId = ContentOutputId {
    id: 1,
    generation: 1,
};
const WIDTH: u32 = 1920;

#[derive(Default)]
pub struct Host {
    published: bool,
    pub demands: usize,
    pub shown: Vec<Vec<(ContentResourceId, Vec<u8>)>>,
    displayed: Option<ContentRenderBundle>,
}

impl Host {
    pub fn turn(&mut self, transport: &mut ShellTransportConnection<'_>, now: u64, height: u32) {
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
                (1, 1, 1, WIDTH, height)
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
                            height,
                        },
                        pixel: ContentPixelRect {
                            x: 0,
                            y: 0,
                            width: WIDTH,
                            height,
                        },
                        parent: ContentAllocationId::default(),
                        anchor_parent_rect: ContentPixelRect::default(),
                        allowed_reservation_extent: height,
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
        }
        transport.poll_io().unwrap();
    }
}
