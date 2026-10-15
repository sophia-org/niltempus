//! Real protected clients and production content owners in one shared registry.
//! Geometry, catalog and renderer completion are supplied by this test host.
use sophia_backend_live::LiveRenderDeviceIdentitySnapshot;
use sophia_config::*;
use sophia_protocol::*;
use sophia_runtime::*;
use sophia_session::shell_component_connections::ComponentConnectionKey;
use sophia_session::shell_component_launch::ShellComponentLaunch;
use std::path::Path;
use std::time::Duration;

pub const OUTPUT: ContentOutputId = ContentOutputId {
    id: 1,
    generation: 1,
};

pub struct Peer {
    pub role: ShellComponentRole,
    pub grant: ContentGrant,
    pub transport: ShellComponentTransport,
    process: ProcessSupervisor,
    pub connected: bool,
    pub frames: usize,
    pub held: Option<ContentRenderBundle>,
    catalog: ShellPersistentCatalog,
    permit: u64,
    allocation: u64,
    transaction: u64,
    stopped: bool,
}

impl Peer {
    pub fn launch(
        selection: ShellComponentConfig,
        identity: &LiveRenderDeviceIdentitySnapshot,
        directory: &Path,
        registry: &mut ContentEpochRegistry,
        epoch: u64,
    ) -> Self {
        Self::launch_inventory(selection, identity, directory, registry, epoch, true)
    }

    pub fn launch_baseline(
        selection: ShellComponentConfig,
        identity: &LiveRenderDeviceIdentitySnapshot,
        directory: &Path,
        registry: &mut ContentEpochRegistry,
        epoch: u64,
    ) -> Self {
        Self::launch_inventory(selection, identity, directory, registry, epoch, false)
    }

    fn launch_inventory(
        selection: ShellComponentConfig,
        identity: &LiveRenderDeviceIdentitySnapshot,
        directory: &Path,
        registry: &mut ContentEpochRegistry,
        epoch: u64,
        has_dock: bool,
    ) -> Self {
        let role = selection.role;
        let profile = match role {
            ShellComponentRole::Bar => ContentStoreProfile::Legacy,
            ShellComponentRole::Dock => ContentStoreProfile::PersistentCatalog,
            ShellComponentRole::ApplicationLauncher => ContentStoreProfile::NativeLauncher,
            _ => panic!("unexpected role"),
        };
        let grant = ContentGrant {
            connection_epoch: epoch,
            content_grant_epoch: epoch,
        };
        // Session's three-role envelopes at the pinned revision. The public
        // reconnect selector reduces only the replacing role's allowance.
        let mut nominal = ContentLimits::prototype(grant);
        if has_dock {
            nominal.max_staging_bytes = 4 * 1024 * 1024;
            nominal.max_resident_bytes = if role == ShellComponentRole::Bar {
                12
            } else {
                8
            } * 1024
                * 1024;
            nominal.max_retiring_bytes = 8 * 1024 * 1024;
        } else if role == ShellComponentRole::ApplicationLauncher {
            nominal.max_staging_bytes = 4 * 1024 * 1024;
            nominal.max_resident_bytes = 12 * 1024 * 1024;
            nominal.max_retiring_bytes = 8 * 1024 * 1024;
        }
        let limits = select_reconnect_limits(&nominal, registry.reconnect_budget(profile)).unwrap();
        std::fs::create_dir(directory).unwrap();
        let mut transport = ShellComponentTransport::bind_for_supervised_uid(
            directory.join("export"),
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
        transport
            .reserve_content_with_profile(registry, limits, profile)
            .unwrap();
        let device = (selection.gpu == ShellGpuMode::Direct).then(|| identity.clone());
        let launch =
            ShellComponentLaunch::new(selection, Some(24), device, Path::new("/usr/bin/bwrap"))
                .unwrap();
        let (mut spec, _) = launch
            .prepare(
                ComponentConnectionKey { slot: 0, grant },
                transport.socket_path(),
            )
            .unwrap();
        spec.protection_domain = Some(
            spec.protection_domain
                .take()
                .unwrap()
                .inherited_fds([1, 2])
                .unwrap(),
        );
        let mut process = ProcessSupervisor::new(SupervisedProcessKind::Shell, spec);
        process
            .apply(SupervisorCommand::StartProcess {
                process: SupervisedProcessKind::Shell,
                delay: Duration::ZERO,
            })
            .unwrap();
        transport
            .authorize_protected_peer(process.protection_evidence().unwrap())
            .unwrap();
        transport
            .begin_file_negotiation(
                registry,
                epoch,
                Duration::from_secs(10),
                ShellContentAdmissionPolicy::Granted {
                    discrete_input: true,
                },
            )
            .unwrap();
        let catalog = ShellPersistentCatalog {
            catalog: ShellApplicationCatalog {
                connection_epoch: epoch,
                generation: 1,
                entries: vec![ShellApplicationDescriptor {
                    slot: 1,
                    available: true,
                    label: "Terminal".into(),
                    keywords: "terminal".into(),
                }],
            },
            identities: [(1, "registered:terminal".into())].into_iter().collect(),
        };
        Self {
            role,
            grant,
            transport,
            process,
            connected: false,
            frames: 0,
            held: None,
            catalog,
            permit: 0,
            allocation: 0,
            transaction: 100,
            stopped: false,
        }
    }

    fn tx(&mut self) -> TransactionId {
        self.transaction += 1;
        TransactionId::from_raw(self.transaction)
    }

    pub fn tick(
        &mut self,
        registry: &mut ContentEpochRegistry,
        now: u64,
    ) -> Result<(), ShellTransportError> {
        assert_ne!(
            self.process.poll().unwrap(),
            Some(SupervisorEvent::ProcessExited),
            "{:?} exited",
            self.role
        );
        if !self.connected {
            if self.transport.poll_negotiation(registry, 65536)?.is_some() {
                self.connected = true;
                if self.role != ShellComponentRole::Bar {
                    let tx = self.tx();
                    self.transport
                        .publish_catalog(registry, tx, &self.catalog)?;
                }
                let tx = self.tx();
                self.transport.publish_content_output_facts(
                    registry,
                    tx,
                    1,
                    vec![ContentOutputFactsEntry {
                        output: OUTPUT,
                        local_width: 1280,
                        local_height: 720,
                        scale_numerator: 1,
                        scale_denominator: 1,
                        scale_generation: 1,
                    }],
                )?;
                if self.role == ShellComponentRole::ApplicationLauncher {
                    let tx = self.tx();
                    self.transport.publish_native_launcher_opening(
                        registry,
                        tx,
                        NativeLauncherOpening {
                            grant: self.grant,
                            opening: 1,
                            output: OUTPUT,
                            catalog_generation: 1,
                            state_revision: 1,
                        },
                    )?;
                }
            }
            return Ok(());
        }
        self.transport.service_owner_turn(registry)?;
        let allocations = self.transport.content_allocation_snapshots(registry);
        let context = ContentCandidateContext {
            output: OUTPUT,
            facts_generation: 1,
            interaction_generation: 1,
            allocations: &allocations,
        };
        if self.role == ShellComponentRole::ApplicationLauncher {
            let (opening, state_revision) = self.transport.native_launcher_state().unwrap();
            self.transport.service_native_launcher_content(
                registry,
                context,
                NativeLauncherCandidateContext {
                    opening,
                    state_revision,
                    catalog: &self.catalog.catalog,
                },
                now,
            )?;
        } else {
            self.transport.service_content_resources(registry, now)?;
            self.transport
                .service_content_allocation_requests(registry, &[], now)?;
            self.transport
                .service_content_demands(registry, &[OUTPUT], &allocations)?;
            if self.role == ShellComponentRole::Dock {
                self.transport.service_catalog_candidates(
                    registry,
                    &[context],
                    &self.catalog.catalog,
                    now,
                )?;
            } else {
                self.transport
                    .service_content_candidates(registry, &[context], now)?;
            }
        }
        while let Some((_, request)) = self.transport.next_content_allocation_request(registry) {
            assert_eq!(request.operation, 1);
            assert_eq!(request.output, OUTPUT);
            assert!(request.desired_width > 0 && request.desired_width <= 1280);
            assert!(request.desired_height > 0 && request.desired_height <= 720);
            let launcher = self.role == ShellComponentRole::ApplicationLauncher;
            assert_eq!(request.role, if launcher { 3 } else { 1 });
            if !launcher {
                assert_eq!(
                    request.edge,
                    if self.role == ShellComponentRole::Dock {
                        3
                    } else {
                        1
                    }
                );
                assert!(
                    request.desired_height
                        <= if self.role == ShellComponentRole::Dock {
                            64
                        } else {
                            24
                        }
                );
            }
            self.allocation += 1;
            let y = if self.role == ShellComponentRole::Dock {
                720 - request.desired_height
            } else {
                0
            };
            self.transport.grant_content_allocation(
                registry,
                request.allocation_request_id,
                ContentAllocationSnapshot {
                    native_opening: launcher.then_some(1),
                    output: OUTPUT,
                    allocation: ContentAllocationId {
                        id: self.allocation,
                        generation: 1,
                    },
                    scale_generation: 1,
                    scale_numerator: 1,
                    scale_denominator: 1,
                    role: request.role,
                    edge: request.edge,
                    margins: ContentMargins::default(),
                    logical: ContentLogicalRect {
                        x: 0,
                        y: y as i32,
                        width: request.desired_width,
                        height: request.desired_height,
                    },
                    pixel: ContentPixelRect {
                        x: 0,
                        y: y as i32,
                        width: request.desired_width,
                        height: request.desired_height,
                    },
                    parent: ContentAllocationId::default(),
                    anchor_parent_rect: ContentPixelRect::default(),
                    allowed_reservation_extent: if launcher {
                        0
                    } else if self.role == ShellComponentRole::Dock {
                        64
                    } else {
                        24
                    },
                },
                &[],
            )?;
        }
        while let Some((tx, demand)) = self.transport.next_content_demand(registry) {
            self.permit += 1;
            self.transport
                .grant_content_demand(registry, tx, demand.output, self.permit, now)?;
        }
        if let Some((_, generation)) = self.transport.next_content_submission(registry) {
            let bundle = match self.role {
                ShellComponentRole::Dock => self
                    .transport
                    .connection(registry)
                    .begin_catalog_submission(generation, context, &self.catalog.catalog, now)?,
                ShellComponentRole::ApplicationLauncher => {
                    let (opening, state_revision) = self.transport.native_launcher_state().unwrap();
                    self.transport.begin_native_launcher_submission(
                        registry,
                        generation,
                        context,
                        NativeLauncherCandidateContext {
                            opening,
                            state_revision,
                            catalog: &self.catalog.catalog,
                        },
                        now,
                    )?
                }
                _ => self
                    .transport
                    .begin_content_submission(registry, OUTPUT, generation, now)?,
            };
            assert!(!bundle.placements.is_empty());
            for placement in &bundle.placements {
                let resource = bundle.resource(placement.resource).unwrap();
                assert_eq!(
                    resource.bytes().len() as u64,
                    resource.description().total_bytes
                );
                assert!(resource.bytes().len() > 1024);
            }
            // Supplied completion lets real clients continue producing content.
            // This is deliberately not evidence of screen presentation.
            self.transport
                .content_prepared(registry, self.grant, OUTPUT, generation, 1, 1, now)?;
            self.transport
                .content_presented(registry, self.grant, OUTPUT, generation, generation, 1, 1)?;
            if self.role == ShellComponentRole::ApplicationLauncher {
                let tx = self.tx();
                self.transport.install_native_launcher_focus(registry, tx)?;
            }
            self.frames += 1;
            println!(
                "trio_content role={:?} epoch={} frame={} bytes=true presentation=supplied",
                self.role, self.grant.connection_epoch, self.frames
            );
            self.held = Some(bundle);
        }
        if self.role == ShellComponentRole::ApplicationLauncher {
            while let Some((_, _, matched)) =
                self.transport.poll_native_launcher_input_ack(registry)?
            {
                assert!(matched);
            }
        }
        Ok(())
    }

    pub fn edit(&mut self, registry: &mut ContentEpochRegistry, text: &str) {
        let focus = self.transport.native_launcher_focus().unwrap();
        let tx = self.tx();
        let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        let micros = now.tv_sec as u64 * 1_000_000 + now.tv_nsec as u64 / 1000;
        assert!(
            self.transport
                .issue_native_launcher_input(
                    registry,
                    focus,
                    tx,
                    NativeLauncherInputKind::Text,
                    text,
                    micros
                )
                .unwrap()
                .is_some()
        );
    }

    pub fn pause(&mut self, paused: bool) {
        let pid = rustix::process::Pid::from_raw(self.process.peer_id().unwrap() as i32).unwrap();
        rustix::process::kill_process(
            pid,
            if paused {
                rustix::process::Signal::STOP
            } else {
                rustix::process::Signal::CONT
            },
        )
        .unwrap();
        self.stopped = paused;
        if paused {
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            loop {
                let status =
                    std::fs::read_to_string(format!("/proc/{}/status", pid.as_raw_nonzero()))
                        .unwrap();
                if status.lines().any(|line| line.starts_with("State:\tT")) {
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "dock did not stop");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    pub fn fill_stalled_catalog(&mut self, registry: &ContentEpochRegistry) {
        assert!(self.stopped && self.role == ShellComponentRole::Dock);
        for generation in 2..=1024 {
            let tx = self.tx();
            let mut catalog = self.catalog.clone();
            catalog.catalog.generation = generation;
            match self.transport.publish_catalog(registry, tx, &catalog) {
                Ok(()) => {}
                Err(ShellTransportError::ActivationQueueSaturated) => {
                    println!(
                        "trio_isolation dock_catalog_backpressure=true publications={}",
                        generation - 2
                    );
                    return;
                }
                Err(error) => panic!("unexpected stalled publication error: {error:?}"),
            }
        }
        panic!("stopped dock did not reach bounded publication backpressure");
    }

    pub fn revoke(&mut self, registry: &mut ContentEpochRegistry) {
        if self.stopped {
            self.pause(false);
        }
        self.transport.disconnect(registry).unwrap();
        self.process.request_termination().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !self.process.poll_termination().unwrap() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        registry.collect();
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        if self.stopped {
            // Always resume a stopped test child before its supervisor tears it down.
            if let Some(pid) = self
                .process
                .peer_id()
                .and_then(|p| rustix::process::Pid::from_raw(p as i32))
            {
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::CONT);
            }
        }
    }
}
