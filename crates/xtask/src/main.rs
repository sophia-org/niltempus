//! Development commands for the desktop integration repository.
use std::path::{Path, PathBuf};

const USAGE: &str = "usage:
  xtask prepare-bemenu-artifact SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
  xtask prepare-product-artifact lom|provlita|hagia SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
  xtask check-pins
  xtask dock profile LOM LOM_CONFIG BEMENU PROVLITA DOCK_CONFIG
  xtask dock verify HOST_LOG
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
        Some("prepare-product-artifact") => xtask::product_artifact::run(&arguments[1..]),
        // Moved from Sophia crates/xtask/src/main.rs:53-70 at 9fcaec782.
        Some("dock") => match &arguments[1..] {
            [command, paths @ ..] if command == "profile" => {
                Ok(xtask::dock::profile(paths)?.lines().map(str::to_owned).collect())
            }
            [command, path] if command == "verify" => {
                use std::io::Read;
                let mut text = String::new();
                std::fs::File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(64 * 1024 * 1024 + 1)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
                Ok(vec![xtask::dock::verify(&text)?])
            }
            _ => Err(
                "usage: xtask dock profile LOM CONFIG BEMENU PROVLITA CONFIG | dock verify LOG"
                    .into(),
            ),
        },
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
