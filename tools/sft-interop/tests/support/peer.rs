use super::super::*;
use sophia_9p_client::pipeline::{Pipeline, PipelineLimits, Reply};
use sophia_shell_client::{ShellClientOptions, ShellConnection};
use std::os::unix::net::UnixStream;

pub fn start(
    socket: PathBuf,
    directory: PathBuf,
    stop: Arc<AtomicBool>,
    revision: u16,
) -> std::thread::JoinHandle<Result<usize, String>> {
    std::thread::spawn(move || {
        let timeout = Duration::from_secs(10);
        let mut pipeline = Pipeline::over(
            UnixStream::connect(socket).unwrap(),
            PipelineLimits::default(),
            timeout,
        )
        .unwrap();
        let (tag, afid) = pipeline.auth(&[], &[]).unwrap();
        assert!(matches!(
            pipeline.wait(tag, Instant::now() + timeout).unwrap(),
            Reply::Auth(_)
        ));
        let (tag, root) = pipeline.attach_with_afid(afid, &[], &[]).unwrap();
        assert!(matches!(
            pipeline.wait(tag, Instant::now() + timeout).unwrap(),
            Reply::Attach(_)
        ));
        let tag = pipeline.clunk(afid).unwrap();
        pipeline.wait(tag, Instant::now() + timeout).unwrap();
        let connection = ShellConnection::from_attached(
            pipeline,
            root,
            ShellClientOptions {
                minimum_revision: revision,
                maximum_revision: revision,
                required_capabilities: if revision == 13 {
                    sophia_protocol::shell_files::SHELL_FRAME_CAPABILITY
                } else {
                    0
                } | sophia_shell_client::IMAGE_CAPABILITY_OWNED_RING
                    | SOPHIA_SHELL_CAPABILITY_INDICATOR_ACTIVATION
                    | SOPHIA_SHELL_CAPABILITY_VIEW_INDICATORS
                    | SOPHIA_SHELL_CAPABILITY_CONTENT_DISCRETE_INPUT
                    | SOPHIA_SHELL_CAPABILITY_CONTENT_SURFACE
                    | SOPHIA_SHELL_CAPABILITY_DESCRIPTOR_SWITCHER,
                handshake_timeout: timeout,
            },
        )
        .map_err(|e| e.to_string())?;
        let (config, theme) = lom::config::parse_shell_config(include_str!("shell.kdl"))
            .map_err(|error| error.to_string())?;
        let renderer = lom::render::RendererWorker::start_cpu()?;
        let mut service =
            lom::service::ShellService::new(connection, config, theme, 24, renderer, directory)?;
        let mut presented = 0;
        while presented < 4 && !stop.load(Ordering::Relaxed) {
            presented += service.step()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(presented)
    })
}
