//! `quickshell-probe`: the moved Quickshell X11 trace probes.
use std::path::PathBuf;

use quickshell_probe::Renderer;
use quickshell_probe::runner::{PROOF_DEADLINE, Probe, run};

const USAGE: &str = "usage: quickshell-probe --renderer=software|gpu --quickshell=/ABS --config=/ABS --out=/ABS/NEW [--render-node=/dev/dri/renderDN]
  software: QT_QUICK_BACKEND=software, no render device; must run device-hidden
  gpu: an explicit render node (never discovered); needs its own execution grant";

fn main() -> std::process::ExitCode {
    match parse(&std::env::args().skip(1).collect::<Vec<_>>()).and_then(|probe| run(&probe)) {
        Ok(line) => {
            println!("{line}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("quickshell-probe: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn parse(arguments: &[String]) -> Result<Probe, String> {
    let mut renderer = None;
    let mut quickshell = None;
    let mut config = None;
    let mut out = None;
    let mut render_node = None;
    for argument in arguments {
        let (name, value) = argument
            .split_once('=')
            .ok_or_else(|| format!("unknown argument {argument:?}\n{USAGE}"))?;
        let slot = match name {
            "--renderer" => {
                renderer = Some(Renderer::parse(value)?);
                continue;
            }
            "--quickshell" => &mut quickshell,
            "--config" => &mut config,
            "--out" => &mut out,
            "--render-node" => &mut render_node,
            _ => return Err(format!("unknown argument {argument:?}\n{USAGE}")),
        };
        if slot.replace(PathBuf::from(value)).is_some() {
            return Err(format!("{name} given twice"));
        }
    }
    let (Some(renderer), Some(quickshell), Some(config), Some(out)) =
        (renderer, quickshell, config, out)
    else {
        return Err(USAGE.to_owned());
    };
    if renderer == Renderer::Software && !device_hidden() {
        return Err(
            "the software probe runs device-hidden: /dev/dri must be absent or empty".to_owned(),
        );
    }
    Ok(Probe {
        renderer,
        quickshell,
        config,
        render_node,
        out,
        deadline: PROOF_DEADLINE,
    })
}

fn device_hidden() -> bool {
    match std::fs::read_dir("/dev/dri") {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}
