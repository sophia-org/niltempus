//! Serve one Quickshell run on the public frontend and collect its trace.
//!
//! Custody: the client runs in its own process group under xtask's
//! `ProcessGroup` (WNOWAIT status, TERM, grace, KILL, reap last). Its stdout
//! and stderr go to capped files in the evidence directory, polled while it
//! runs, so a pipe held by a descendant cannot outlive the proof. The
//! frontend runs on its own thread with the nonblocking accept, explicit
//! worker polling and shutdown, and a bounded teardown.
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::Read;
use std::num::NonZeroUsize;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use sophia_protocol::{NamespaceId, TransactionId};
use sophia_x_authority::{
    X11CoreTraceObserver, X11DispatchObservation, X11ObservedRequestStage, X11SetupSocketError,
    XClientOutput, XServerFrontend, XServerFrontendConfig, XServerFrontendRouteBroker,
};
use xtask::bemenu_artifact::ProcessGroup;

use crate::{ClientEnd, Observed, ObservedError, Renderer, TraceRecord, stop_is_intentional};

/// The old probes' proof window (basic_smokes.rs at the pin): a Qt cold start.
pub const PROOF_DEADLINE: Duration = Duration::from_secs(20);
/// Requests the trace may hold before it counts as overflowed. The old
/// channel held 4096 observations and dropped the rest silently.
const TRACE_CAPACITY: usize = 65_536;
const CLIENT_LOG_CAP: u64 = 4 << 20;
const TEARDOWN_LIMIT: Duration = Duration::from_secs(10);
const DISPLAYS: std::ops::Range<u32> = 7_900..8_900;
const SOCKET_DIR: &str = "/tmp/.X11-unix";

pub struct Probe {
    pub renderer: Renderer,
    /// The Quickshell executable, absolute.
    pub quickshell: PathBuf,
    /// The QML passed as `--path`, absolute.
    pub config: PathBuf,
    /// GPU only: the explicit render node. Never discovered.
    pub render_node: Option<PathBuf>,
    /// A new evidence directory, created here with mode 0700.
    pub out: PathBuf,
    pub deadline: Duration,
}

/// Run the probe. `Ok` is the accepted report line and `Err` a refusal or a
/// harness failure; either way the evidence directory holds what was seen.
pub fn run(probe: &Probe) -> Result<String, String> {
    for (what, path) in [("quickshell", &probe.quickshell), ("config", &probe.config)] {
        if !path.is_absolute() || !path.is_file() {
            return Err(format!(
                "{what} must be an absolute file: {}",
                path.display()
            ));
        }
    }
    let gpu = match (probe.renderer, &probe.render_node) {
        (Renderer::Gpu, Some(node)) => Some(crate::runner::gpu::Device::open(node)?),
        (Renderer::Gpu, None) => {
            return Err("the GPU probe needs an explicit --render-node".to_owned());
        }
        (Renderer::Software, Some(_)) => {
            return Err("the software probe takes no render node".to_owned());
        }
        (Renderer::Software, None) => None,
    };
    if !probe.out.is_absolute() {
        return Err("--out must be absolute".to_owned());
    }
    DirBuilder::new()
        .mode(0o700)
        .create(&probe.out)
        .map_err(|e| format!("{}: {e}", probe.out.display()))?;
    let identity = format!(
        "schema=1\nrenderer={}\nquickshell={}\nquickshell_sha256={}\nconfig={}\nconfig_sha256={}\nrender_node={}\n",
        probe.renderer.name(),
        probe.quickshell.display(),
        file_sha256(&probe.quickshell)?,
        probe.config.display(),
        file_sha256(&probe.config)?,
        probe
            .render_node
            .as_deref()
            .map_or_else(|| "none".to_owned(), |node| node.display().to_string()),
    );
    write_new(&probe.out.join("identity.txt"), identity.as_bytes())?;

    let result = serve(probe, gpu);
    let verdict = match &result {
        Ok(line) | Err(line) => line.clone(),
    };
    write_new(
        &probe.out.join("verdict.txt"),
        format!("{verdict}\n").as_bytes(),
    )?;
    result
}

fn serve(probe: &Probe, gpu: Option<gpu::Device>) -> Result<String, String> {
    let namespace = NamespaceId::from_raw(match probe.renderer {
        Renderer::Gpu => 63,
        Renderer::Software => 65,
    });
    let (display, socket, frontend) = bind(namespace, gpu.as_ref())?;
    let _socket = RemoveOnDrop(socket);

    let overflowed = Arc::new(AtomicBool::new(false));
    let stopping = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::sync_channel::<TraceRecord>(TRACE_CAPACITY);
    let observer = observer(sender, overflowed.clone(), stopping.clone());

    let stop = Arc::new(AtomicBool::new(false));
    let connections = Arc::new(AtomicU64::new(0));
    let (done_sender, done) = mpsc::channel();
    {
        let stop = stop.clone();
        let connections = connections.clone();
        let mut frontend = frontend;
        std::thread::spawn(move || {
            let result = (|| {
                let broker = XServerFrontendRouteBroker::new(
                    NonZeroUsize::new(16).expect("nonzero route capacity"),
                );
                while !stop.load(Ordering::Acquire) {
                    if frontend
                        .try_serve_next_concurrently_routed_traced(&broker, observer.clone())?
                    {
                        connections.fetch_add(1, Ordering::AcqRel);
                    }
                    frontend.poll_client_workers()?;
                    std::thread::sleep(Duration::from_millis(2));
                }
                frontend.shutdown_all_client_workers()?;
                frontend.wait_for_clients()
            })();
            let _ = done_sender.send(result.map_err(|e| e.to_string()));
        });
    }

    let stdout_path = probe.out.join("client.stdout");
    let stderr_path = probe.out.join("client.stderr");
    let mut command = Command::new(&probe.quickshell);
    command
        .arg("--path")
        .arg(&probe.config)
        .env("DISPLAY", &display)
        .env("GDK_BACKEND", "x11")
        .env("GTK_USE_PORTAL", "0")
        .env("MOZ_ENABLE_WAYLAND", "0")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null())
        .stdout(create_log(&stdout_path)?)
        .stderr(create_log(&stderr_path)?)
        .process_group(0);
    if probe.renderer == Renderer::Software {
        command.env("QT_QUICK_BACKEND", "software");
    }
    let spawned = command.spawn();
    let mut observed = Observed::default();
    let child = match spawned {
        Ok(child) => ProcessGroup::new(child),
        Err(error) => {
            stop.store(true, Ordering::Release);
            return Err(format!(
                "could not start {}: {error}",
                probe.quickshell.display()
            ));
        }
    };

    let deadline = Instant::now() + probe.deadline;
    let end = loop {
        drain(&receiver, &mut observed);
        if observed.transactions > 0 {
            break ClientEnd::StoppedAfterTransaction;
        }
        match child.status() {
            Ok(Some(status)) => {
                break ClientEnd::Exited {
                    code: status.exit_status(),
                    signal: status.terminating_signal(),
                };
            }
            Ok(None) => {}
            Err(error) => {
                observed.server_error = Some(format!("client status: {error}"));
                break ClientEnd::StoppedAtDeadline;
            }
        }
        if [&stdout_path, &stderr_path]
            .iter()
            .any(|path| !within_cap(path))
        {
            observed.client_log_exceeded = true;
            break ClientEnd::StoppedAtDeadline;
        }
        if let Ok(result) = done.try_recv() {
            observed.server_error = Some(match result {
                Ok(()) => "frontend stopped before the client".to_owned(),
                Err(error) => error,
            });
            break ClientEnd::StoppedAtDeadline;
        }
        if Instant::now() >= deadline {
            break ClientEnd::StoppedAtDeadline;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Only the probe's own stops are intentional. After a natural exit the
    // flag stays clear, so a late departure is not excused.
    if stop_is_intentional(end) {
        stopping.store(true, Ordering::Release);
    }
    drop(child);

    stop.store(true, Ordering::Release);
    if observed.server_error.is_none() {
        match done.recv_timeout(TEARDOWN_LIMIT) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => observed.server_error = Some(error),
            Err(_) => {
                observed.server_error =
                    Some(format!("frontend teardown exceeded {TEARDOWN_LIMIT:?}"));
            }
        }
    }
    drain(&receiver, &mut observed);
    observed.connections = connections.load(Ordering::Acquire);
    observed.overflowed = overflowed.load(Ordering::Acquire);
    for path in [&stdout_path, &stderr_path] {
        if !within_cap(path) {
            observed.client_log_exceeded = true;
        }
    }
    crate::evaluate(probe.renderer, &observed, end).map(|line| format!("{line} display={display}"))
}

fn observer(
    sender: SyncSender<TraceRecord>,
    overflowed: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
) -> Arc<X11CoreTraceObserver> {
    Arc::new(move |trace: X11DispatchObservation| {
        let errors = trace
            .result
            .outputs
            .iter()
            .filter_map(|output| match output {
                XClientOutput::Error(error) => Some(ObservedError {
                    code: format!("{:?}", error.code),
                    major: error.major_code,
                    minor: error.minor_code,
                    resource: error.resource_id,
                    sequence: error.sequence,
                }),
                _ => None,
            })
            .collect();
        let record = TraceRecord {
            stage: (trace.request_stage != X11ObservedRequestStage::Other)
                .then(|| trace.request_stage.evidence_name().to_owned()),
            major: trace.major_opcode,
            minor: trace.minor_opcode,
            failure: trace.failure.map(|failure| format!("{failure:?}")),
            errors,
            transactions: trace
                .result
                .response
                .as_ref()
                .map_or(0, |response| response.transactions.len() as u64),
            after_stop_initiated: stopping.load(Ordering::Acquire),
        };
        match sender.try_send(record) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => overflowed.store(true, Ordering::Release),
        }
        Ok::<Option<TransactionId>, X11SetupSocketError>(None)
    })
}

fn drain(receiver: &mpsc::Receiver<TraceRecord>, observed: &mut Observed) {
    while let Ok(trace) = receiver.try_recv() {
        observed.record(trace);
    }
}

/// Bind the frontend on the first free display. `bind_exclusive` refuses an
/// existing path atomically, so a live server's socket is never displaced.
/// A client tries the abstract socket first; the gate runs with a private
/// network namespace, so no other server's abstract socket is reachable.
fn bind(
    namespace: NamespaceId,
    gpu: Option<&gpu::Device>,
) -> Result<(String, PathBuf, XServerFrontend), String> {
    match DirBuilder::new().create(SOCKET_DIR) {
        Ok(()) => std::fs::set_permissions(SOCKET_DIR, std::fs::Permissions::from_mode(0o1777))
            .map_err(|e| format!("{SOCKET_DIR}: {e}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("{SOCKET_DIR}: {error}")),
    }
    let mut last = String::new();
    for number in DISPLAYS {
        let socket = PathBuf::from(format!("{SOCKET_DIR}/X{number}"));
        if socket.exists() {
            continue;
        }
        let mut config =
            XServerFrontendConfig::new(&socket, namespace).map_err(|e| e.to_string())?;
        if let Some(device) = gpu {
            config = config
                .with_render_device_provider(device.provider())
                .with_pixmap_allocator(device.allocator());
        }
        match XServerFrontend::bind_exclusive(config) {
            Ok(frontend) => return Ok((format!(":{number}"), socket, frontend)),
            Err(error) => last = error.to_string(),
        }
    }
    Err(format!("no free display in {DISPLAYS:?}: {last}"))
}

fn within_cap(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.len() <= CLIENT_LOG_CAP)
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn create_log(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    create_log(path)?
        .write_all(bytes)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn file_sha256(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub mod gpu {
    //! The GPU variant's device: one explicit render node, measured once
    //! before the frontend exists, as the old probe's
    //! `ExternalProbeRenderDeviceProvider::measured` and
    //! `ExternalProbePixmapAllocator` did (render_device_provider.rs and
    //! basic_smokes.rs at the pin), through Sophia's public backend.
    use std::fs::File;
    use std::os::fd::{AsRawFd as _, OwnedFd};
    use std::path::Path;
    use std::sync::Arc;

    use sophia_x_authority::{
        XServerFrontendAllocatedPixmap, XServerFrontendDmaBufImportFormat,
        XServerFrontendPixmapAllocation, XServerFrontendPixmapAllocationError,
        XServerFrontendPixmapAllocator, XServerFrontendRenderDeviceError,
        XServerFrontendRenderDeviceProvider,
    };

    pub struct Device {
        provider: Arc<Provider>,
        allocator: Arc<Allocator>,
    }

    impl Device {
        pub fn open(node: &Path) -> Result<Self, String> {
            let name = node.to_str().unwrap_or_default();
            if !name
                .strip_prefix("/dev/dri/renderD")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err(format!(
                    "--render-node must be /dev/dri/renderDN, not {}",
                    node.display()
                ));
            }
            let open = || {
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(node)
                    .map_err(|e| format!("{}: {e}", node.display()))
            };
            let device = open()?;
            // A node that cannot be measured is advertised as nothing, as the
            // old probe and the session do; the required stage then says what
            // was lost.
            let import_formats = device
                .try_clone()
                .map_err(|_| sophia_backend_live::LiveDmaBufCapabilityError::DeviceUnavailable)
                .and_then(sophia_backend_live::query_dma_buf_import_formats)
                .map(|formats| {
                    formats
                        .into_iter()
                        .map(|row| XServerFrontendDmaBufImportFormat {
                            format: row.format,
                            modifiers: row.modifiers,
                        })
                        .collect()
                })
                .unwrap_or_else(|error| {
                    eprintln!(
                        "quickshell_probe schema=1 status=degraded reason=dma_buf_import_capabilities_unavailable error={error:?}"
                    );
                    Vec::new()
                });
            Ok(Self {
                provider: Arc::new(Provider {
                    device,
                    import_formats,
                }),
                allocator: Arc::new(Allocator { device: open()? }),
            })
        }

        pub fn provider(&self) -> Arc<dyn XServerFrontendRenderDeviceProvider> {
            self.provider.clone()
        }

        pub fn allocator(&self) -> Arc<dyn XServerFrontendPixmapAllocator> {
            self.allocator.clone()
        }
    }

    struct Provider {
        device: File,
        import_formats: Vec<XServerFrontendDmaBufImportFormat>,
    }

    impl XServerFrontendRenderDeviceProvider for Provider {
        fn dma_buf_import_formats(&self) -> Vec<XServerFrontendDmaBufImportFormat> {
            self.import_formats.clone()
        }

        fn open_render_device_fd(&self) -> Result<OwnedFd, XServerFrontendRenderDeviceError> {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/proc/self/fd/{}", self.device.as_raw_fd()))
                .map(OwnedFd::from)
                .map_err(|_| XServerFrontendRenderDeviceError::OpenFailed)
        }
    }

    struct Allocator {
        device: File,
    }

    impl XServerFrontendPixmapAllocator for Allocator {
        fn allocate_pixmap_buffer(
            &self,
            request: XServerFrontendPixmapAllocation,
        ) -> Result<XServerFrontendAllocatedPixmap, XServerFrontendPixmapAllocationError> {
            use XServerFrontendPixmapAllocationError as Error;
            use sophia_backend_live::LiveSharedBufferError as Live;
            let allocation = sophia_backend_live::allocate_shared_buffer(
                &self.device,
                request.handle,
                request.size,
                request.depth,
            )
            .map_err(|error| match error {
                Live::UnsupportedTarget => Error::UnsupportedTarget,
                Live::DeviceRejected | Live::ExportFailed => Error::AllocationFailed,
            })?;
            Ok(XServerFrontendAllocatedPixmap {
                descriptor: allocation.descriptor,
                plane_fds: allocation.plane_fds,
            })
        }
    }
}
