//! Untimed three-frame oracle; every unexpected wire record refuses the hold.
use super::{Config, Producer, Result, Source, Wire};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::inotify;
use serde::{Deserialize, Serialize};
use sophia_shell_protocol::{ContentGrant, ContentOutputId};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::{
    fs::{FileTypeExt, MetadataExt},
    net::UnixStream,
};
use std::path::Path;
use std::time::{Duration, Instant};

const CONNECT: Duration = Duration::from_secs(30);
const CAPTURE: Duration = Duration::from_secs(80);

pub(super) fn validate_config(config: &Config) -> Result<()> {
    if config.source != Source::DmaBuf
        || config.producer != Producer::Gpu
        || config.capture_profile
        || config.dma_reuse
        || config.frames != 3
        || config.height != 24
        || config.hold_msec != 0
    {
        return Err(
            "colour hold requires three fresh GPU DMA frames, height24, hold0, no profiling/reuse"
                .into(),
        );
    }
    Ok(())
}

/// Mirror the pinned SDK's external-loop contract, adding the hold/inotify fd.
fn wait(wire: &mut Wire, extra: &impl AsFd, events: PollFlags, deadline: Instant) -> Result<()> {
    wire.client.poll_io()?;
    if let Some((_, record)) = wire.client.poll_content()? {
        return Err(format!("unexpected content during diagnostic hold: {record:?}").into());
    }
    if Instant::now() >= deadline {
        return Err("diagnostic hold deadline".into());
    }
    let readiness = wire
        .client
        .image_readiness()?
        .ok_or("missing DMA readiness")?;
    if readiness.session.immediate {
        return Ok(());
    }
    let mut flags = PollFlags::empty();
    if readiness.session.readable {
        flags |= PollFlags::IN;
    }
    if readiness.session.writable {
        flags |= PollFlags::OUT;
    }
    let session = wire.client.as_fd();
    let mut fds = vec![PollFd::new(&session, flags), PollFd::new(extra, events)];
    if let Some(image) = readiness.image_fd.as_ref() {
        fds.push(PollFd::new(
            image,
            PollFlags::from_bits_retain(readiness.image_events.0),
        ));
    }
    let deadline = wire
        .client
        .wake_deadline()
        .map_or(deadline, |retry| retry.min(deadline));
    let timeout = Timespec::try_from(deadline.saturating_duration_since(Instant::now()))?;
    match poll(&mut fds, Some(&timeout)) {
        Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[derive(Serialize)]
struct Ready<'a> {
    schema: u8,
    status: &'static str,
    generation: u64,
    pattern: u8,
    connection_epoch: u64,
    content_grant_epoch: u64,
    output: u64,
    shell_output_generation: u64,
    presentation_epoch: u64,
    nonce: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    schema: u8,
    status: String,
    generation: u64,
    nonce: String,
}

fn ack(line: &[u8], generation: u64, nonce: &str) -> Result<()> {
    let value: Ack = serde_json::from_slice(line)?;
    if value.schema != 1
        || value.status != "ack"
        || value.generation != generation
        || value.nonce != nonce
    {
        return Err("stale or foreign diagnostic ACK".into());
    }
    Ok(())
}

pub(super) struct Control {
    socket: UnixStream,
    next: u64,
}

impl Control {
    pub(super) fn connect(shell: &std::ffi::OsStr, wire: &mut Wire) -> Result<Self> {
        let shell = Path::new(shell);
        let parent = shell.parent().ok_or("shell endpoint lacks parent")?;
        let endpoint = parent.join("hold.sock");
        if !shell.is_absolute() || endpoint.as_os_str().len() >= 108 || endpoint.try_exists()? {
            return Err("hold endpoint is not fresh or bounded".into());
        }
        let watch = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
        inotify::add_watch(
            &watch,
            parent,
            inotify::WatchFlags::CREATE | inotify::WatchFlags::MOVED_TO,
        )?;
        // Hint only. Parent independently finds Session's listener and derives hold.sock.
        super::canvas::diagnostic_line(format_args!(
            "gpu_colour_hold schema=1 status=endpoint_hint shell_socket={}",
            shell.display()
        ));
        let deadline = Instant::now() + CONNECT;
        loop {
            match std::fs::symlink_metadata(&endpoint) {
                Ok(metadata) => {
                    if !metadata.file_type().is_socket() || metadata.mode() & 0o777 != 0o600 {
                        return Err("invalid hold endpoint".into());
                    }
                    break;
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            wait(wire, &watch, PollFlags::IN, deadline)?;
            let mut events = [0; 4096];
            match rustix::io::read(&watch, &mut events) {
                Ok(_) | Err(rustix::io::Errno::AGAIN) => {}
                Err(error) => return Err(error.into()),
            }
        }
        // Parent listens before publishing by rename. Exactly one connection attempt.
        let socket = UnixStream::connect(endpoint)?;
        let credentials = rustix::net::sockopt::socket_peercred(&socket)?;
        if credentials.uid != rustix::process::geteuid() {
            return Err("hold controller UID differs".into());
        }
        socket.set_nonblocking(true)?;
        Ok(Self { socket, next: 1 })
    }

    fn send(&mut self, wire: &mut Wire, bytes: &[u8], deadline: Instant) -> Result<()> {
        // Records are small; partial writes still retain their exact suffix.
        let mut left = bytes;
        while !left.is_empty() {
            if Instant::now() >= deadline {
                return Err("hold write deadline".into());
            }
            match self.socket.write(left) {
                Ok(0) => return Err("hold controller closed".into()),
                Ok(count) => left = &left[count..],
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    wait(wire, &self.socket, PollFlags::OUT, deadline)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub(super) fn presented(
        &mut self,
        wire: &mut Wire,
        grant: ContentGrant,
        output: ContentOutputId,
        generation: u64,
        presentation_epoch: u64,
    ) -> Result<()> {
        if generation != self.next || generation > 3 || presentation_epoch == 0 {
            return Err("diagnostic generation is not next/presented".into());
        }
        let mut probe = [0; 1];
        match self.socket.read(&mut probe) {
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            _ => return Err("early ACK or closed controller".into()),
        }
        let mut random = [0; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let ready = Ready {
            schema: 1,
            status: "ready",
            generation,
            pattern: if generation == 2 { 2 } else { 1 },
            connection_epoch: grant.connection_epoch,
            content_grant_epoch: grant.content_grant_epoch,
            output: output.id,
            shell_output_generation: output.generation,
            presentation_epoch,
            nonce: &nonce,
        };
        let deadline = Instant::now() + CAPTURE;
        let mut bytes = serde_json::to_vec(&ready)?;
        bytes.push(b'\n');
        self.send(wire, &bytes, deadline)?;
        let mut line = Vec::new();
        loop {
            if Instant::now() >= deadline {
                return Err("capture ACK deadline".into());
            }
            // Service/drain before accepting ACK, including a simultaneous revocation.
            wire.client.poll_io()?;
            if let Some((_, record)) = wire.client.poll_content()? {
                return Err(format!("unexpected content during hold: {record:?}").into());
            }
            let mut bytes = [0; 512];
            match self.socket.read(&mut bytes) {
                Ok(0) => return Err("controller EOF before ACK".into()),
                Ok(count) => {
                    line.extend_from_slice(&bytes[..count]);
                    if line.len() > 512 {
                        return Err("ACK exceeds bound".into());
                    }
                    if let Some(end) = line.iter().position(|byte| *byte == b'\n') {
                        if end + 1 != line.len() {
                            return Err("extra ACK bytes".into());
                        }
                        ack(&line[..end], generation, &nonce)?;
                        self.next += 1;
                        return Ok(());
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    wait(wire, &self.socket, PollFlags::IN, deadline)?
                }
                Err(error) => return Err(error.into()),
            }
            if Instant::now() >= deadline {
                return Err("capture ACK deadline".into());
            }
        }
    }

    pub(super) fn finish(&mut self, wire: &mut Wire) -> Result<()> {
        if self.next != 4 {
            return Err("incomplete diagnostic sequence".into());
        }
        let deadline = Instant::now() + CONNECT;
        self.send(
            wire,
            b"{\"schema\":1,\"status\":\"done\",\"generation\":3}\n",
            deadline,
        )?;
        // Stay connected and service transport until the parent stops Session.
        // No fourth frame, timed sleep, synthetic resource release or restart request.
        loop {
            if let Err(error) = wait(wire, &self.socket, PollFlags::IN, deadline) {
                if matches!(
                    error.downcast_ref::<super::ShellClientError>(),
                    Some(super::ShellClientError::PeerClosed)
                ) {
                    return Ok(());
                }
                return Err(error);
            }
            let mut byte = [0];
            match self.socket.read(&mut byte) {
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                _ => return Err("controller closed or sent data before Session shutdown".into()),
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/support/hold.rs"]
mod tests;
