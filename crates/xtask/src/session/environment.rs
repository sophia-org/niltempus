// Provenance: moved from Sophia
// crates/sophia-cli/src/commands/session_prepare/environment.rs at
// a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at
// the pin de776c68) (recipe-only: Sophia keeps the TTY, bus and verbose-trace entries)
// (Sophia rule 13).
use super::{BTreeMap, Result, Write, append};

// Unlike the ordinary fallback reader, trace defaults preserve an explicitly
// empty value: the Bash contract uses ${NAME-default}, not ${NAME:-default}.
fn trace(name: &str, default: &str) -> Result<String> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(error) => Err(format!("{name}: {error}").into()),
    }
}

pub(super) fn run(options: &BTreeMap<String, String>, extra: &[String]) -> Result<()> {
    let proof = |name: &str| extra.iter().any(|arg| arg == name);
    let firefox = [
        "--firefox-m10-proof",
        "--firefox-m10-rendering-proof",
        "--firefox-m10-dialog-proof",
        "--firefox-m10-primary-proof",
        "--firefox-m10-selection-proof",
        "--firefox-m10-lifecycle-proof",
    ]
    .iter()
    .any(|name| proof(name));
    // Recipe entries only; Sophia's retained prepare-environment owns the
    // TTY, bus and verbose-trace entries.
    let mut values = Vec::new();
    if firefox {
        let mut slice = "promotion";
        for name in ["selection", "primary", "dialog", "rendering", "lifecycle"] {
            if proof(&format!("--firefox-m10-{name}-proof")) {
                slice = name;
                break;
            }
        }
        values.push(format!(
            "SOPHIA_FIREFOX_M10_KITTY_PROBE_DIR={}",
            super::required(options, "firefox-probe")?
        ));
        values.push(format!("SOPHIA_FIREFOX_M10_PROOF_SLICE={slice}"));
        append(
            &mut values,
            &[
                "GDK_BACKEND=x11",
                "GTK_USE_PORTAL=0",
                "MOZ_ENABLE_WAYLAND=0",
                "MOZ_FORCE_DISABLE_E10S=1",
                "MOZ_USE_XINPUT2=1",
            ],
        );
    }
    if proof("--firefox-m10-rendering-proof") {
        values.push(format!(
            "SOPHIA_NATIVE_COMPOSITION_PIXEL_TRACE={}",
            trace("SOPHIA_NATIVE_COMPOSITION_PIXEL_TRACE", "final-regions")?
        ));
        values.push(format!(
            "SOPHIA_X11_PIXEL_TRACE={}",
            trace("SOPHIA_X11_PIXEL_TRACE", "1")?
        ));
    }
    let mut output = std::io::stdout().lock();
    output.write_all(b"sophia_desktop_recipe_environment schema=1 status=prepared\0")?;
    for value in values {
        output.write_all(value.as_bytes())?;
        output.write_all(b"\0")?;
    }
    Ok(())
}
