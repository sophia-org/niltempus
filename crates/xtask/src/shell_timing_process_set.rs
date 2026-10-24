//! Read-only binding for a fixed-time measurement with several shell clients.
use crate::shell_timing_process_binding::{Binder, Executables, ProcessBinding, ResolveOutcome};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::PathBuf,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub wrapper_pid: u32,
    pub wrapper_start_time_ticks: u64,
    pub host: PathBuf,
    pub host_argv: Vec<String>,
    pub supervisor: PathBuf,
    pub peers: BTreeMap<String, PathBuf>,
}

/// None means startup has not produced the entire set yet. Each binding keeps
/// the existing executable, ancestry and time-namespace proof unchanged.
pub fn resolve(
    request: &Request,
    binder: &Binder,
) -> Result<Option<BTreeMap<String, ProcessBinding>>, String> {
    if request.peers.is_empty()
        || request.peers.len() > 8
        || !request.host.is_absolute()
        || !request.supervisor.is_absolute()
        || request.host_argv.first().map(PathBuf::from).as_ref() != Some(&request.host)
        || request.peers.iter().any(|(name, path)| {
            name.is_empty()
                || name == "session"
                || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                || !path.is_absolute()
        })
    {
        return Err("invalid process-set request".into());
    }
    let mut bindings: BTreeMap<String, ProcessBinding> = BTreeMap::new();
    let mut pids = BTreeSet::new();
    for (name, peer) in &request.peers {
        let outcome = binder.resolve(
            request.wrapper_pid,
            Executables {
                host: &request.host,
                host_argv: &request.host_argv,
                supervisor: &request.supervisor,
                peer,
            },
        )?;
        let ResolveOutcome::Bound(binding) = outcome else {
            return Ok(None);
        };
        if binding.wrapper.start_time_ticks != request.wrapper_start_time_ticks
            || !pids.insert(binding.peer.pid)
            || bindings.values().any(|previous| {
                previous.host != binding.host || previous.wrapper != binding.wrapper
            })
        {
            return Err("process-set identity changed or peer was counted twice".into());
        }
        bindings.insert(name.clone(), *binding);
    }
    Ok(Some(bindings))
}

pub fn run(arguments: &[String]) -> Result<Vec<String>, String> {
    let [path] = arguments else {
        return Err("usage: xtask shell-timing bind-processes REQUEST_JSON".into());
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65_536 {
        return Err("process-set request exceeds 64 KiB".into());
    }
    let request = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let bindings = resolve(&request, &Binder::system())?;
    Ok(vec![
        serde_json::to_string(&bindings).map_err(|e| e.to_string())?,
    ])
}

/// Retain the existing credential/pidfd-validated output records unchanged.
/// The parent owns process lifetime; this bound also ends an orphan collector.
pub fn receive_environment(arguments: &[String]) -> Result<Vec<String>, String> {
    use crate::shell_timing_environment_socket::EnvironmentReceiver;
    use std::{
        io::Write,
        time::{Duration, Instant},
    };
    let [path, duration] = arguments else {
        return Err("usage: xtask shell-timing receive-environment SOCKET_PATH DURATION_MS".into());
    };
    let millis: u64 = duration
        .parse()
        .map_err(|_| "invalid environment duration")?;
    if !(1..=180_000).contains(&millis) {
        return Err("environment duration must be 1..=180000 ms".into());
    }
    let mut receiver = EnvironmentReceiver::bind(std::path::Path::new(path))?;
    let until = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < until {
        while Instant::now() < until {
            let Some(receipt) = receiver.receive()? else {
                break;
            };
            println!(
                "{}",
                serde_json::to_string(&receipt).map_err(|e| e.to_string())?
            );
            std::io::stdout().flush().map_err(|e| e.to_string())?;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(Vec::new())
}
