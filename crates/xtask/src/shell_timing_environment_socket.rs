//! Opt-in host environment delivery with Linux sender credentials. Log text
//! cannot supply these credentials. The runner must still bind the sender PID
//! and start time to its supervised host and retain these receipts with their
//! digest; this module alone neither authenticates a run nor qualifies an arm.
//! Each datagram also transfers exactly one TGID pidfd, opened by the sender
//! for itself without PIDFD_THREAD. Its fdinfo PID must match the credentials
//! and its final zero-time poll must show a live process. This rejects stale
//! queued credentials after numeric PID reuse. The handle is always closed
//! before return. Sender and receiver must use the same PID namespace view.
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{IoSliceMut, Read},
    os::fd::{AsRawFd, OwnedFd},
    os::unix::{fs::MetadataExt, net::UnixDatagram},
    path::{Path, PathBuf},
};

pub const MAX_RECORD_BYTES: usize = 65_536;
pub const MAX_RECORDS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentReceipt {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
    /// Receiver's CLOCK_MONOTONIC, not a timestamp supplied by the sender.
    pub received_usec: u64,
    pub record: String,
    /// Kernel fdinfo read from the sender's live TGID pidfd. This is retained
    /// evidence, not authority when deserialized from an arbitrary file.
    pub sender_fdinfo: String,
}

impl EnvironmentReceipt {
    /// The expected PID comes from supervision, never from `record`. Recheck
    /// that process's start time around receive to exclude reuse. UID alone is
    /// insufficient: untrusted peers normally run as the same desktop user.
    pub fn require_host(&self, pid: u32, uid: u32, gid: u32) -> Result<(), String> {
        if pid == 0 || self.pid != pid || self.uid != uid || self.gid != gid {
            return Err("environment sender is not the supervised host".into());
        }
        Ok(())
    }
}

pub struct EnvironmentReceiver {
    socket: UnixDatagram,
    path: PathBuf,
    identity: (u64, u64),
    received: usize,
    failure: Option<String>,
}

impl EnvironmentReceiver {
    /// Bind once beneath a private directory owned by the runner. Existing
    /// sockets are not removed or reused. This opens no graphics device.
    pub fn bind(path: &Path) -> Result<Self, String> {
        let parent = path.parent().ok_or("environment socket has no parent")?;
        let metadata = fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
        if !path.is_absolute()
            || !metadata.is_dir()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != rustix::process::getuid().as_raw()
        {
            return Err("environment socket requires an owned private directory".into());
        }
        let socket = UnixDatagram::bind(path).map_err(|e| e.to_string())?;
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let receiver = Self {
            socket,
            path: path.into(),
            identity: (metadata.dev(), metadata.ino()),
            received: 0,
            failure: None,
        };
        receiver
            .socket
            .set_nonblocking(true)
            .map_err(|e| e.to_string())?;
        rustix::net::sockopt::set_socket_passcred(&receiver.socket, true)
            .map_err(|e| e.to_string())?;
        Ok(receiver)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One bounded nonblocking receive. Malformed, oversized or ancillary-FD
    /// input permanently invalidates this collector; callers cannot discard a
    /// failed packet and then certify the remainder. Absence is `Ok(None)`.
    pub fn receive(&mut self) -> Result<Option<EnvironmentReceipt>, String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self.receive_one();
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }

    fn receive_one(&mut self) -> Result<Option<EnvironmentReceipt>, String> {
        let mut bytes = vec![0; MAX_RECORD_BYTES];
        // Exactly one self-pidfd is required in addition to kernel credentials.
        // A queued credential's numeric PID alone can outlive and be reused
        // after its sender exits. All installed rights are owned and closed.
        let mut space =
            [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmCredentials(1), ScmRights(4))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let message = loop {
            match recvmsg(
                &self.socket,
                &mut [IoSliceMut::new(&mut bytes)],
                &mut control,
                RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
            ) {
                Ok(message) => break message,
                Err(rustix::io::Errno::INTR) => continue,
                Err(rustix::io::Errno::AGAIN) => return Ok(None),
                Err(error) => return Err(format!("receive environment: {error}")),
            }
        };
        let mut credentials = None;
        let mut sender = None;
        let mut rights_count = 0;
        let mut invalid = false;
        for item in control.drain() {
            match item {
                RecvAncillaryMessage::ScmCredentials(value) => {
                    invalid |= credentials.replace(value).is_some();
                }
                RecvAncillaryMessage::ScmRights(rights) => {
                    for fd in rights {
                        rights_count += 1;
                        if sender.is_none() {
                            sender = Some(fd);
                        }
                    }
                }
                _ => invalid = true,
            }
        }
        if invalid
            || rights_count != 1
            || message
                .flags
                .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        {
            return Err(
                "environment datagram truncated or carried unexpected ancillary data".into(),
            );
        }
        let credentials = credentials.ok_or("environment datagram has no kernel credentials")?;
        self.received += 1;
        if self.received > MAX_RECORDS || message.bytes == 0 {
            return Err("environment record count or length is invalid".into());
        }
        bytes.truncate(message.bytes);
        let record = String::from_utf8(bytes).map_err(|_| "environment record is not UTF-8")?;
        if !record.starts_with("sophia_shell_environment_") || record.contains(['\r', '\n', '\0']) {
            return Err("environment datagram is not a single record".into());
        }
        let clock = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        let received_usec =
            u64::try_from(i128::from(clock.tv_sec) * 1_000_000 + i128::from(clock.tv_nsec) / 1_000)
                .map_err(|_| "environment receive clock overflow")?;
        let sender_fdinfo = check_sender(
            sender
                .as_ref()
                .ok_or("environment datagram has no sender pidfd")?,
            credentials.pid.as_raw_pid() as u32,
        )?;
        Ok(Some(EnvironmentReceipt {
            pid: credentials.pid.as_raw_pid() as u32,
            uid: credentials.uid.as_raw(),
            gid: credentials.gid.as_raw(),
            received_usec,
            record,
            sender_fdinfo,
        }))
    }
}

/// Validate the handle before dropping it. No signal is sent. The final poll
/// establishes liveness after fdinfo was read; the runner must still bracket
/// receive with its bound host's start-time checks.
fn check_sender(fd: &OwnedFd, pid: u32) -> Result<String, String> {
    let number = fd.as_raw_fd();
    if fs::read_link(format!("/proc/self/fd/{number}")).map_err(|e| e.to_string())?
        != Path::new("anon_inode:[pidfd]")
    {
        return Err("environment descriptor is not a pidfd".into());
    }
    let mut info = String::new();
    fs::File::open(format!("/proc/self/fdinfo/{number}"))
        .map_err(|e| e.to_string())?
        .take(4097)
        .read_to_string(&mut info)
        .map_err(|e| e.to_string())?;
    let ids: Vec<_> = info
        .lines()
        .filter_map(|line| line.strip_prefix("Pid:"))
        .collect();
    if info.len() > 4096
        || ids.len() != 1
        || pid == 0
        || ids[0].trim().parse::<u32>().ok() != Some(pid)
    {
        return Err("environment pidfd differs from kernel sender credentials".into());
    }
    let mut polls = [rustix::event::PollFd::new(fd, rustix::event::PollFlags::IN)];
    loop {
        match rustix::event::poll(&mut polls, Some(&rustix::event::Timespec::default())) {
            Ok(0) => return Ok(info),
            Ok(_) => return Err("environment sender pidfd is no longer live".into()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
}

impl Drop for EnvironmentReceiver {
    fn drop(&mut self) {
        // Never remove a replacement somebody installed at the same path.
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
