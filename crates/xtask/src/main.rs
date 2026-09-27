//! Development commands for the desktop integration repository.
use std::path::{Path, PathBuf};

const USAGE: &str = "usage:
  xtask prepare-bemenu-artifact SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
  xtask check-pins
  xtask audit-pins ABSOLUTE-SOPHIA-REPO";

fn main() -> std::process::ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match run(&arguments) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[String]) -> Result<Vec<String>, String> {
    let repo = workspace_root()?;
    match arguments.first().map(String::as_str) {
        Some("prepare-bemenu-artifact") => xtask::bemenu_artifact::run(&repo, &arguments[1..]),
        Some("check-pins") if arguments.len() == 1 => xtask::pins::check(&repo),
        Some("audit-pins") => match &arguments[1..] {
            [sophia] => xtask::pins::audit(&repo, Path::new(sophia)),
            _ => Err(USAGE.into()),
        },
        _ => Err(USAGE.into()),
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::canonicalize(&root).map_err(|e| format!("{}: {e}", root.display()))
}
