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

#[test]
fn campaign_socket_address_limit_is_checked_without_creating_a_run() {
    use xtask::shell_timing_environment_socket::socket_path;
    let component_bytes = 107 - "/environment.sock".len() - 1;
    let fits = PathBuf::from(format!("/{}", "x".repeat(component_bytes)));
    assert!(socket_path(&fits).is_ok());
    let too_long = PathBuf::from(format!("/{}", "x".repeat(component_bytes + 1)));
    assert!(
        socket_path(&too_long)
            .unwrap_err()
            .contains("socket path is invalid")
    );
    let multibyte = PathBuf::from(format!("/{}", "é".repeat(component_bytes)));
    assert!(socket_path(&multibyte).is_err());
}

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

#[test]
fn ready_cli_publishes_after_binding_and_preserves_ancillary_refusal() {
    use std::{io::BufRead, process::Stdio, sync::mpsc};
    for valid in [true, false] {
        let fixture = Fixture::new();
        let mut child = ChildGuard(
            Command::new(env!("CARGO_BIN_EXE_xtask"))
                .args(["shell-timing", "receive-environment"])
                .arg(fixture.socket())
                .args(["300", "--ready"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            let mut ready = String::new();
            reader.read_line(&mut ready).unwrap();
            tx.send(ready).unwrap();
            let mut rest = String::new();
            reader.read_to_string(&mut rest).unwrap();
            rest
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            "{\"schema\":1,\"status\":\"ready\"}\n"
        );
        let sender = UnixDatagram::unbound().unwrap();
        let record = b"sophia_shell_environment_output schema=1";
        if valid {
            send_record(&sender, &fixture.socket(), record);
        } else {
            sender.send_to(record, fixture.socket()).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "receiver did not terminate");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.success(), valid);
        let rest = reader.join().unwrap();
        if valid {
            let receipt: xtask::shell_timing_environment_socket::EnvironmentReceipt =
                serde_json::from_str(&rest).unwrap();
            receipt
                .require_host(
                    std::process::id(),
                    rustix::process::getuid().as_raw(),
                    rustix::process::getgid().as_raw(),
                )
                .unwrap();
            assert_eq!(receipt.record.as_bytes(), record);
        } else {
            assert!(rest.is_empty());
        }
        assert!(!fixture.socket().exists());
    }
}

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
        // Parallel child-spawn tests may briefly inherit a copy between fork
        // and exec, before CLOEXEC closes it. Bound that transient explicitly;
        // a descriptor retained by this receiver never reaches EOF.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match reader.read(&mut [0]) {
                Ok(0) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "received descriptor remains open"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                other => panic!("unexpected data or read error: {other:?}"),
            }
        }
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
