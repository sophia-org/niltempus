//! A neighbouring bar on its own ShellComponentTransport in the SAME
//! ContentEpochRegistry as the Bemenu launcher: the production Rust shell
//! client (`sophia_shell_client::ShellConnection`, from the SDK Sophia vendors
//! at the pinned revision; `connect_files` on the file wire, `connect` with
//! `ipc-compat` on the socket wire) negotiates revision 6 with a content grant
//! on the launcher's wire and is then serviced in lockstep with the launcher. It uploads nothing; its role is to prove that Bemenu's
//! whole lifecycle (opening, rasters, input, activation, close, retirement,
//! reopen, stop) leaves a neighbour's connection and exact content usage in
//! the shared registry unchanged. Protection evidence for this in-process
//! peer is supplied (Bubblewrap/MetadataShell, own pid), as in Sophia's own
//! file-wire client fixtures; only the launcher is a protected child.
use sophia_protocol::*;
use sophia_runtime::*;
use sophia_shell_client::{ShellClientOptions, ShellConnection};
use std::path::Path;
use std::time::{Duration, Instant};

pub const BAR_EPOCH: u64 = 61;
const HANDSHAKE: Duration = Duration::from_secs(5);

pub struct Bar {
    transport: ShellComponentTransport,
    client: ShellConnection,
    pub grant: ContentGrant,
}

impl Bar {
    /// Bind, reserve the bar's grant in the shared registry, and negotiate
    /// the real client (its blocking connect runs on its own thread while
    /// this one drives the transport).
    pub fn connect(root: &Path, registry: &mut ContentEpochRegistry, wire: crate::fixture::Wire) -> Self {
        let mut transport = ShellComponentTransport::bind_for_supervised_uid(
            root.join("bar-export"),
            rustix::process::geteuid().as_raw(),
        )
        .unwrap();
        transport
            .authorize_protected_peer(&ProtectionDomainEvidence {
                backend: ProtectionBackendKind::Bubblewrap,
                supervisor_pid: std::process::id(),
                peer_pid: std::process::id(),
                roles: [ProtectionDomainRole::MetadataShell].into_iter().collect(),
            })
            .unwrap();
        let grant = ContentGrant {
            connection_epoch: BAR_EPOCH,
            content_grant_epoch: 1,
        };
        transport
            .reserve_content(registry, ContentLimits::prototype(grant))
            .unwrap();
        let path = transport.socket_path().to_path_buf();
        let options = ShellClientOptions {
            minimum_revision: 5,
            maximum_revision: 6,
            required_capabilities: SOPHIA_SHELL_CAPABILITY_DESCRIPTOR_SWITCHER
                | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE,
            handshake_timeout: HANDSHAKE,
        };
        let policy = ShellContentAdmissionPolicy::Granted {
            discrete_input: false,
        };
        let client = match wire {
            crate::fixture::Wire::Files => {
                let client =
                    std::thread::spawn(move || ShellConnection::connect_files(&path, options));
                transport.begin_file_negotiation(registry, BAR_EPOCH, HANDSHAKE, policy)
                    .unwrap();
                client
            }
            crate::fixture::Wire::Ipc => {
                let client = std::thread::spawn(move || ShellConnection::connect(&path, options));
                transport.begin_negotiation(registry, BAR_EPOCH, HANDSHAKE, policy)
                    .unwrap();
                client
            }
        };
        let deadline = Instant::now() + HANDSHAKE;
        let mut negotiated = false;
        while !client.is_finished() {
            if negotiated {
                transport.poll_io(registry).unwrap();
            } else if transport.poll_negotiation(registry, 65536).unwrap().is_some() {
                negotiated = true;
            }
            assert!(Instant::now() < deadline, "neighbour bar handshake stalled");
            std::thread::sleep(Duration::from_micros(100));
        }
        let client = client
            .join()
            .unwrap()
            .unwrap_or_else(|e| panic!("neighbour bar connect_files: {e:?}"));
        assert!(negotiated, "neighbour bar transport completed negotiation");
        assert_eq!(client.connection_epoch(), BAR_EPOCH);
        Self {
            transport,
            client,
            grant,
        }
    }

    /// One lockstep round for the neighbour: transport, then client.
    pub fn tick(&mut self, registry: &mut ContentEpochRegistry) -> Result<(), String> {
        self.transport
            .poll_io(registry)
            .map_err(|e| format!("neighbour bar transport: {e:?}"))?;
        self.client
            .poll_io()
            .map_err(|e| format!("neighbour bar client: {e:?}"))
    }

    /// The bar's exact content usage in the shared registry.
    pub fn usage(&self, registry: &ContentEpochRegistry) -> Option<ContentMemoryUsage> {
        self.transport.content_usage(registry)
    }

    /// Disconnect the bar's epoch through the transport's own path.
    pub fn close(mut self, registry: &mut ContentEpochRegistry) {
        self.transport.disconnect(registry).unwrap();
        drop(self.client);
    }
}
