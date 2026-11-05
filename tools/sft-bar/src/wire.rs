//! Existing authenticated launch and owned-ring setup from gpu-bar-dma e730d3910.
use super::*;
pub(super) struct Wire {
    pub(super) client: ShellConnection,
}

/// Validate and adopt the exclusive launch channel, as Lom does.
fn adopt(fd: i32) -> Result<UnixStream> {
    if fd < 3 {
        return Err("invalid launch descriptor".into());
    }
    // SAFETY: probed only; nothing has opened a descriptor yet that could reuse it.
    let flags = rustix::io::fcntl_getfd(unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) })?;
    // SAFETY: the launch contract transfers this inherited fd exclusively to us.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    rustix::io::fcntl_setfd(&stream, flags | rustix::io::FdFlags::CLOEXEC)?;
    Ok(stream)
}

impl Wire {
    pub(super) fn connect(
        frame_transactions: bool,
    ) -> Result<(Self, sophia_9p_client::pipeline::Pipeline)> {
        if std::env::var_os("SOPHIA_SHELL_SOCKET").is_some() {
            return Err("ambiguous shell wire".into());
        }
        let socket = std::env::var_os("SOPHIA_SHELL_9P_SOCKET").ok_or("missing file endpoint")?;
        let fd = std::env::var_os("SOPHIA_LAUNCH_AUTH_FD")
            .ok_or("DMA source requires an authenticated file launch")?;
        let relay = adopt(fd.to_str().ok_or("non-UTF8 launch descriptor")?.parse()?)?;
        let mut auth = sophia_launch_client::Client::with_timeout(
            UnixStream::connect(&socket)?,
            relay,
            b"shell",
            b"bar",
            WAIT,
        )?;
        while !auth.ready() {
            auth.poll()?;
            if !auth.ready() {
                auth.wait_for_io(Duration::from_millis(10))?;
            }
        }
        let attached = auth.into_authenticated()?;
        let (revision, frame) = if frame_transactions {
            (
                13,
                sophia_shell_protocol::shell_files::SHELL_FRAME_CAPABILITY,
            )
        } else {
            (10, 0)
        };
        let options = ShellClientOptions {
            minimum_revision: revision,
            maximum_revision: revision,
            required_capabilities: SOPHIA_SHELL_CAPABILITY_DESCRIPTOR_SWITCHER
                | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE
                | SOPHIA_SHELL_CAPABILITY_CONTENT_DISCRETE_INPUT
                | IMAGE_CAPABILITY_OWNED_RING
                | frame,
            handshake_timeout: WAIT,
        };
        Ok((
            Self {
                client: ShellConnection::from_attached(attached.role, attached.root, options)?,
            },
            attached.relay,
        ))
    }

    pub(super) fn send(&mut self, id: u64, records: &[ShellContentRecord]) -> Result<()> {
        let transaction = TransactionId::from_raw(id);
        self.admit(|client| {
            if records.len() == 1 {
                client.enqueue_content(transaction, &records[0])
            } else {
                client.enqueue_content_group(transaction, records)
            }
        })
    }

    /// Admit one unit, retrying while the local queue is full.
    pub(super) fn admit(
        &mut self,
        mut enqueue: impl FnMut(&mut ShellConnection) -> std::result::Result<(), ShellClientError>,
    ) -> Result<()> {
        let deadline = Instant::now() + WAIT;
        loop {
            match enqueue(&mut self.client) {
                Ok(()) => break,
                Err(ShellClientError::QueueSaturated) if Instant::now() < deadline => {
                    self.client
                        .wait_for_io(deadline.saturating_duration_since(Instant::now()))?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        self.client.poll_io()?;
        Ok(())
    }

    pub(super) fn receive(&mut self) -> Result<ShellContentRecord> {
        self.receive_tx().map(|(_, record)| record)
    }

    pub(super) fn receive_tx(&mut self) -> Result<(TransactionId, ShellContentRecord)> {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(received) = self.client.poll_content()? {
                return Ok(received);
            }
            if Instant::now() >= deadline {
                return Err("response timeout".into());
            }
            self.client
                .wait_for_io(deadline.saturating_duration_since(Instant::now()))?;
        }
    }

    /// Image bounds come from the content limits, as Lom derives them. A
    /// ring registers two slots of exactly `capacity` bytes.
    pub(super) fn enable_image(
        &mut self,
        socket: &std::ffi::OsStr,
        limits: &ContentLimits,
        capacity: u64,
    ) -> Result<()> {
        let endpoint_dir = std::path::Path::new(socket)
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .ok_or("file endpoint has no directory")?;
        let origin_ms = clock_usec() / 1000;
        let image = ShellImageConfig {
            endpoint_dir: endpoint_dir.to_owned(),
            bounds: ProducerRoleBounds {
                max_resource_bytes: limits.max_resource_bytes,
                max_width: limits.max_width_px,
                max_height: limits.max_height_px,
                max_admitted_outputs: limits.max_outputs,
                transfer_slots: limits.max_open_transfers,
            },
            transfer_timeout_ms: limits.transfer_timeout_ms,
            startup_deadline_ms: origin_ms + WAIT.as_millis() as u64,
            formats_capacity: 4096,
            ring_slots: 2,
            ring_capacity: capacity,
            content_grant_epoch: limits.grant.content_grant_epoch,
        };
        self.client.enable_image(image, Instant::now(), origin_ms)?;
        Ok(())
    }

    pub(super) fn wait_ready(&mut self) -> Result<()> {
        let deadline = Instant::now() + WAIT;
        loop {
            self.client.poll_io()?;
            let adapter = self.client.image_adapter().ok_or("image adapter absent")?;
            match adapter.channel() {
                Some(ImageChannelState::Ready) => return Ok(()),
                Some(ImageChannelState::Closed) => return Err("image channel closed".into()),
                _ => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("image channel not ready".into());
            }
            self.client.wait_for_io(left)?;
        }
    }

    /// A ring copy is adoptable once the SDK reports Settled: Accepted status
    /// and its exact successful SlotReleased. Returns the release observation.
    pub(super) fn settle_ring(&mut self, offer: u64, resource: ContentResourceId) -> Result<u64> {
        let deadline = Instant::now() + WAIT;
        loop {
            self.client.poll_io()?;
            let info = self
                .client
                .image_ring_offer(offer)
                .ok_or("ring offer lost")?;
            if info.resource != resource.id || info.resource_generation != resource.generation {
                return Err("ring offer names another resource".into());
            }
            match info.state {
                RingOfferState::Settled if info.status == Some(ResourceStatusKind::Accepted) => {
                    let released_usec = clock_usec();
                    self.client
                        .image_forget(offer)
                        .map_err(|e| format!("image forget refused: {e:?}"))?;
                    return Ok(released_usec);
                }
                RingOfferState::Pending => {}
                state => {
                    return Err(
                        format!("ring offer ended {state:?} status {:?}", info.status).into(),
                    );
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!(
                    "ring offer did not settle: {:?} status {:?}",
                    info.state, info.status
                )
                .into());
            }
            self.client.wait_for_io(left)?;
        }
    }
}
