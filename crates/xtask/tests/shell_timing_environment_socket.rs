//! Real local sockets and a bounded child, no native Session or graphics.
use std::{
    fs,
    io::Read,
    os::{
        fd::AsFd,
        unix::{
            fs::DirBuilderExt,
            net::{UnixDatagram, UnixStream},
        },
    },
    path::PathBuf,
    process::{Child, Command},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use xtask::shell_timing_environment_socket::{EnvironmentReceiver, MAX_RECORD_BYTES, MAX_RECORDS};

fn send_record(sender: &UnixDatagram, path: &std::path::Path, bytes: &[u8]) {
    let pidfd = rustix::process::pidfd_open(
        rustix::process::getpid(),
        rustix::process::PidfdFlags::empty(),
    )
    .unwrap();
    send_with_fd(sender, path, bytes, &pidfd);
}

fn send_with_fd(
    sender: &UnixDatagram,
    path: &std::path::Path,
    bytes: &[u8],
    fd: &std::os::fd::OwnedFd,
) {
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    sender.connect(path).unwrap();
    let rights = [fd.as_fd()];
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    assert!(control.push(SendAncillaryMessage::ScmRights(&rights)));
    sendmsg(
        sender,
        &[std::io::IoSlice::new(bytes)],
        &mut control,
        SendFlags::NOSIGNAL,
    )
    .unwrap();
}

static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "t338-env-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn socket(&self) -> PathBuf {
        self.0.join("environment.sock")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn credential_child() {
    let Ok(path) = std::env::var("T338_ENVIRONMENT_TEST_SOCKET") else {
        return;
    };
    let record = std::env::var("T338_ENVIRONMENT_TEST_RECORD").unwrap();
    send_record(
        &UnixDatagram::unbound().unwrap(),
        std::path::Path::new(&path),
        record.as_bytes(),
    );
    // Keep the sender alive until the parent has validated the queued handle.
    let release = std::path::PathBuf::from(std::env::var("T338_ENVIRONMENT_TEST_RELEASE").unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !release.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn sender_credentials_cannot_be_replaced_by_a_pid_printed_in_the_record() {
    let f = Fixture::new();
    let mut receiver = EnvironmentReceiver::bind(&f.socket()).unwrap();
    let forged = format!(
        "sophia_shell_environment_output pid={} schema=1",
        std::process::id()
    );
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "credential_child", "--test-threads=1"])
            .env("T338_ENVIRONMENT_TEST_SOCKET", receiver.path())
            .env("T338_ENVIRONMENT_TEST_RECORD", &forged)
            .env("T338_ENVIRONMENT_TEST_RELEASE", f.0.join("release"))
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(5);
    let receipt = loop {
        if let Some(receipt) = receiver.receive().unwrap() {
            break receipt;
        }
        assert!(Instant::now() < end, "child never sent environment record");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(receipt.pid, child.0.id());
    assert_eq!(receipt.record, forged);
    assert!(receipt.received_usec > 0);
    assert!(
        receipt
            .require_host(std::process::id(), receipt.uid, receipt.gid)
            .is_err()
    );
    receipt
        .require_host(
            child.0.id(),
            rustix::process::getuid().as_raw(),
            rustix::process::getgid().as_raw(),
        )
        .unwrap();
    assert!(
        receipt
            .require_host(child.0.id(), receipt.uid ^ 1, receipt.gid)
            .is_err()
    );
    assert!(
        receipt
            .require_host(child.0.id(), receipt.uid, receipt.gid ^ 1)
            .is_err()
    );
    fs::write(f.0.join("release"), b"done").unwrap();
    while child.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn empty_poll_and_exact_record_round_trip_do_not_guess_credentials() {
    let f = Fixture::new();
    let mut receiver = EnvironmentReceiver::bind(&f.socket()).unwrap();
    assert!(receiver.receive().unwrap().is_none());
    let sender = UnixDatagram::unbound().unwrap();
    let record = b"sophia_shell_environment_renderer schema=1 vendor=Mesa%20Project";
    send_record(&sender, receiver.path(), record);
    let receipt = receiver.receive().unwrap().unwrap();
    assert_eq!(receipt.record.as_bytes(), record);
    receipt
        .require_host(
            std::process::id(),
            rustix::process::getuid().as_raw(),
            rustix::process::getgid().as_raw(),
        )
        .unwrap();
    assert!(receiver.receive().unwrap().is_none());
    let path = receiver.path().to_owned();
    assert!(EnvironmentReceiver::bind(&path).is_err());
    drop(receiver);
    assert!(!path.exists());
}

#[test]
fn malformed_and_oversized_datagrams_poison_the_collector() {
    for bytes in [
        vec![],
        vec![0xff],
        b"sophia_shell_environment_output\nforged".to_vec(),
        vec![b'x'; MAX_RECORD_BYTES + 1],
    ] {
        let f = Fixture::new();
        let mut receiver = EnvironmentReceiver::bind(&f.socket()).unwrap();
        let sender = UnixDatagram::unbound().unwrap();
        send_record(&sender, receiver.path(), &bytes);
        assert!(receiver.receive().is_err());
        send_record(
            &sender,
            receiver.path(),
            b"sophia_shell_environment_output schema=1",
        );
        assert!(receiver.receive().is_err(), "bad input cannot be skipped");
    }
}

#[test]
fn ancillary_rights_close_even_when_control_is_truncated() {
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    for count in [1, 5, 8] {
        let f = Fixture::new();
        let mut receiver = EnvironmentReceiver::bind(&f.socket()).unwrap();
        let sender = UnixDatagram::unbound().unwrap();
        sender.connect(receiver.path()).unwrap();
        let (mut reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let rights = vec![writer.as_fd(); count];
        let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(8))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        assert!(control.push(SendAncillaryMessage::ScmRights(&rights)));
        sendmsg(
            &sender,
            &[std::io::IoSlice::new(
                b"sophia_shell_environment_output schema=1",
            )],
            &mut control,
            SendFlags::NOSIGNAL,
        )
        .unwrap();
        drop(rights);
        drop(writer);
        assert!(receiver.receive().is_err());
        // EOF proves no received duplicate of the write endpoint remains open.
        assert_eq!(reader.read(&mut [0]).unwrap(), 0);
    }
}

#[test]
fn the_record_count_is_bounded_and_failure_is_sticky() {
    let f = Fixture::new();
    let mut receiver = EnvironmentReceiver::bind(&f.socket()).unwrap();
    let sender = UnixDatagram::unbound().unwrap();
    for _ in 0..MAX_RECORDS {
        send_record(
            &sender,
            receiver.path(),
            b"sophia_shell_environment_output schema=1",
        );
        assert!(receiver.receive().unwrap().is_some());
    }
    send_record(
        &sender,
        receiver.path(),
        b"sophia_shell_environment_output schema=1",
    );
    assert!(receiver.receive().is_err());
    assert!(receiver.receive().is_err());
}

#[test]
fn bare_records_and_queued_records_from_exited_senders_are_refused() {
    let f = Fixture::new();
    let mut receiver = EnvironmentReceiver::bind(&f.socket()).unwrap();
    UnixDatagram::unbound()
        .unwrap()
        .send_to(b"sophia_shell_environment_output schema=1", receiver.path())
        .unwrap();
    assert!(
        receiver.receive().is_err(),
        "a live process handle is required"
    );

    let f = Fixture::new();
    let mut receiver = EnvironmentReceiver::bind(&f.socket()).unwrap();
    fs::write(f.0.join("release"), b"exit after send").unwrap();
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "credential_child", "--test-threads=1"])
            .env("T338_ENVIRONMENT_TEST_SOCKET", receiver.path())
            .env(
                "T338_ENVIRONMENT_TEST_RECORD",
                "sophia_shell_environment_output schema=1",
            )
            .env("T338_ENVIRONMENT_TEST_RELEASE", f.0.join("release"))
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(5);
    while child.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        receiver.receive().is_err(),
        "queued sender must still be alive"
    );
}

#[test]
fn another_process_pidfd_cannot_replace_the_senders_own_handle() {
    let f = Fixture::new();
    let mut receiver = EnvironmentReceiver::bind(&f.socket()).unwrap();
    let pidfd = rustix::process::pidfd_open(
        rustix::process::getppid().unwrap(),
        rustix::process::PidfdFlags::empty(),
    )
    .unwrap();
    send_with_fd(
        &UnixDatagram::unbound().unwrap(),
        receiver.path(),
        b"sophia_shell_environment_output schema=1",
        &pidfd,
    );
    assert_eq!(
        receiver.receive().unwrap_err(),
        "environment pidfd differs from kernel sender credentials"
    );
}
