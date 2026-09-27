//! Development commands for the desktop integration repository.
use std::path::{Path, PathBuf};

const USAGE: &str = "usage:
  xtask prepare-bemenu-artifact SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
  xtask prepare-product-artifact lom|provlita|hagia SOURCE-REPO SIGNED-COMMIT NEW-OUTPUT-DIR
  xtask prepare-wm-pair --hagia REPO COMMIT --narthex REPO COMMIT NEW-OUTPUT-DIR
  xtask package-desktop --sophia-root=/ABS --sophia-rev=SHA --wm-pair=/ABS --wm-pair-commits=H,N --wm-pair-sha256=H,N --wm-pair-profile-sha256=SHA --build-dir=/ABS --out=/ABS/NEW
  xtask session-recipe prepare-arguments|prepare-inputs|stage-proofs|prepare-environment --name=value ... -- [session arguments]
  xtask check-pins
  xtask check-provision
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
    // Commands an installed release runs come first and never look up this
    // repository: the packaged binary must work with its build checkout gone
    // (tests/installed_xtask.rs runs it with the checkout hidden).
    if arguments.first().map(String::as_str) == Some("session-recipe") {
        xtask::session::run(&arguments[1..]).map_err(|e| e.to_string())?;
        return Ok(Vec::new());
    }
    let repo = workspace_root()?;
    match arguments.first().map(String::as_str) {
        Some("prepare-bemenu-artifact") => xtask::bemenu_artifact::run(&repo, &arguments[1..]),
        Some("prepare-product-artifact") => xtask::product_artifact::run(&arguments[1..]),
        Some("prepare-wm-pair") => xtask::wm_pair::run(&arguments[1..]),
        Some("package-desktop") => xtask::package_desktop::run(&repo, &arguments[1..]),
        // Moved from Sophia crates/xtask/src/main.rs:53-70 at 9fcaec782.
        Some("dock") => match &arguments[1..] {
            [command, paths @ ..] if command == "profile" => Ok(xtask::dock::profile(paths)?
                .lines()
                .map(str::to_owned)
                .collect()),
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
        Some("check-provision") if arguments.len() == 1 => xtask::pins::check_provision(&repo),
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
