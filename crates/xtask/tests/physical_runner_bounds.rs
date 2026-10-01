//! Static controls: no physical runner builds anything, writes into a source
//! checkout's target, or uses a shared Nim cache; every one that needs
//! binaries takes them from `xtask prepare-physical-inputs` through
//! tools/lib/physical_inputs.sh, whose bounds are fixed here; every archive
//! site verifies the prepared inputs first. Each forbidden pattern is also
//! shown to fire on a planted line, so no check is vacuous.
use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Scripts that may still build until their own change lands, each for a
/// stated reason; the list may only shrink. None of them is a converted
/// physical runner.
const PENDING: [(&str, &str); 1] = [(
    "tools/provision.sh",
    "provisioning's offline --locked self-check of this repository's own xtask",
)];

/// Scripts explicitly exempt by ruling (fixed; not a waiting list).
const EXEMPT: [(&str, &str); 1] = [(
    "tools/reload_policy_client.sh",
    "operator tool (director ruling): the operator's own default-WM reload workflow, \
     not a gate; its conversion belongs with the niltempus prepare/reload-Hagia path",
)];

/// A forbidden construct: its name and whether a (comment-free) line has it.
struct Rule {
    name: &'static str,
    matches: fn(&str) -> bool,
}

fn words(line: &str) -> Vec<&str> {
    line.split(|c: char| c.is_whitespace() || c == '(' || c == ';' || c == '&' || c == '|')
        .filter(|w| !w.is_empty())
        .collect()
}

fn command_pair(line: &str, first: &str, seconds: &[&str]) -> bool {
    let words = words(line);
    words
        .windows(2)
        .any(|w| w[0] == first && seconds.contains(&w[1]))
}

const SOURCE_VARIABLES: [&str; 12] = [
    "SOPHIA_SOURCE",
    "sophia_source",
    "SOPHIA_ROOT",
    "sophia_root",
    "HAGIA_ROOT",
    "hagia_root",
    "NARTHEX_ROOT",
    "narthex_root",
    "ROOT_DIR",
    "root",
    "repo",
    "HAGIA_SOURCE",
];

const RULES: [Rule; 8] = [
    Rule {
        name: "cargo build/run",
        matches: |l| command_pair(l, "cargo", &["build", "run", "b", "r"]),
    },
    Rule {
        name: "cargo xtask (an alias that builds this checkout)",
        matches: |l| {
            let words = words(l);
            words
                .iter()
                .position(|w| *w == "cargo")
                .is_some_and(|at| words[at..].contains(&"xtask"))
        },
    },
    Rule {
        name: "nim compile",
        matches: |l| command_pair(l, "nim", &["c", "compile", "cpp", "js"]),
    },
    Rule {
        name: "nimble",
        matches: |l| words(l).contains(&"nimble"),
    },
    Rule {
        name: "a --target-dir",
        matches: |l| l.contains("--target-dir"),
    },
    Rule {
        name: "a source checkout's target directory",
        matches: |l| {
            SOURCE_VARIABLES.iter().any(|v| {
                l.contains(&format!("${v}/target")) || l.contains(&format!("${{{v}}}/target"))
            })
        },
    },
    Rule {
        name: "a Nim cache",
        matches: |l| l.contains("nimcache"),
    },
    Rule {
        name: "Sophia's cargo-running atomic_scanout_preflight.sh",
        matches: |l| l.contains("/atomic_scanout_preflight.sh"),
    },
];

/// The line without a trailing `#` comment (outside quotes), or None for a
/// comment line.
fn code(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return None;
    }
    let mut quoted = None;
    for (i, c) in line.char_indices() {
        match (c, quoted) {
            ('\'' | '"', None) => quoted = Some(c),
            (q, Some(open)) if q == open => quoted = None,
            ('#', None) if i > 0 && line[..i].ends_with(char::is_whitespace) => {
                return Some(&line[..i]);
            }
            _ => {}
        }
    }
    Some(line)
}

fn violations(text: &str) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let Some(code) = code(line) else { continue };
        for rule in &RULES {
            if (rule.matches)(code) {
                found.push((index + 1, rule.name));
            }
        }
    }
    found
}

fn scripts(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            scripts(&path, out);
        } else if meta.is_file() {
            let text = fs::read(&path).unwrap();
            let shell = path.extension().is_some_and(|e| e == "sh")
                || text.starts_with(b"#!/usr/bin/env bash")
                || text.starts_with(b"#!/bin/bash")
                || text.starts_with(b"#!/bin/sh");
            if shell {
                out.push(path);
            }
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(repo())
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn every_rule_fires_on_a_planted_line() {
    for (line, rule) in [
        (
            "exec cargo --quiet --offline --locked xtask direct-scanout-gate",
            "cargo xtask (an alias that builds this checkout)",
        ),
        (
            "(cd \"$SOPHIA_SOURCE\" && cargo build --release)",
            "cargo build/run",
        ),
        ("cargo run --offline -p sophia-cli", "cargo build/run"),
        ("    nim c -d:release src/hagia.nim", "nim compile"),
        ("(cd \"$hagia_root\" && nimble build)", "nimble"),
        ("x --target-dir \"$dir\"", "a --target-dir"),
        (
            "SOPHIA_BIN=\"$SOPHIA_SOURCE/target/release/sophia\"",
            "a source checkout's target directory",
        ),
        (
            "bin=\"${sophia_source}/target/debug/sophia\"",
            "a source checkout's target directory",
        ),
        ("cache=\"${TMPDIR:-/tmp}/hagia-nimcache-x\"", "a Nim cache"),
        (
            "\"$SOPHIA_ROOT/tools/atomic_scanout_preflight.sh\"",
            "Sophia's cargo-running atomic_scanout_preflight.sh",
        ),
    ] {
        let found = violations(line);
        assert!(
            found.iter().any(|(_, name)| *name == rule),
            "{line:?}: {found:?}"
        );
    }
    // Comments, installed release layouts and quoted mentions are not code.
    for line in [
        "# it is built with cargo build in Sophia",
        "sophia=\"$release/target/release/sophia\"",
        "ln -sfn \"$prefix/current/target/release/sophia\" \"$command_dir/sophia\"",
    ] {
        assert!(violations(line).is_empty(), "{line:?}");
    }
}

#[test]
fn no_runner_builds_or_reads_a_source_target() {
    let root = repo();
    let mut all = Vec::new();
    scripts(&root.join("tools"), &mut all);
    assert!(all.len() > 100, "scanned only {} scripts", all.len());
    let mut failures = Vec::new();
    let mut pending_seen = Vec::new();
    for path in &all {
        let name = relative(path);
        let text = fs::read_to_string(path).unwrap();
        let found = violations(&text);
        if PENDING.iter().chain(EXEMPT.iter()).any(|(p, _)| *p == name) {
            pending_seen.push(name);
            continue;
        }
        for (line, rule) in found {
            failures.push(format!("{name}:{line}: {rule}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    // Every pending entry still exists (the list only shrinks, deliberately).
    for (path, reason) in PENDING.iter().chain(EXEMPT.iter()) {
        assert!(
            pending_seen.iter().any(|p| p == path),
            "{path} ({reason}) is gone: drop it"
        );
    }
    // The two lists never overlap.
    for (path, _) in EXEMPT {
        assert!(
            !PENDING.iter().any(|(p, _)| *p == path),
            "{path} is both pending and exempt"
        );
    }
}

/// The converted runners: each prepares its inputs through the helper.
const PREPARING: [&str; 18] = [
    "tools/run_current_hagia_native_gate_tty4.sh",
    "tools/run_current_hagia_policy_gate_tty4.sh",
    "tools/run_frame_fed_output_gate_tty4.sh",
    "tools/run_keyboard_independence_gate_tty4.sh",
    "tools/run_mirror_group_gate_tty4.sh",
    "tools/run_mixed_output_gate_tty4.sh",
    "tools/run_output_topology_gate_tty4.sh",
    "tools/run_sophia_input_latency_tty3.sh",
    "tools/run_x11_live_session_stability.sh",
    "tools/live_session_milestone4_hardware_proof.sh",
    "tools/live_session_milestone5_gtk_hardware_proof.sh",
    "tools/native_egl_vkcube_mixed_smoke.sh",
    "tools/hagia_live_session_smoke.sh",
    "tools/hagia_client_lifecycle_fault_smoke.sh",
    "tools/hagia_owner_settlement_fault_smoke.sh",
    "tools/direct_scanout_gate.sh",
    "tools/lib/physical_inputs.sh",
    "tools/lib/physical_runner.sh",
];

#[test]
fn converted_runners_prepare_through_the_bounded_helper() {
    let root = repo();
    for path in &PREPARING[..16] {
        let text = fs::read_to_string(root.join(path)).unwrap();
        assert!(
            text.contains("physical_inputs_prepare "),
            "{path} prepares no inputs"
        );
        assert!(
            text.contains("physical_inputs_bound "),
            "{path} binds no integration commit"
        );
        assert!(
            text.contains("tools/lib/physical_runner.sh"),
            "{path} does not load the runner library"
        );
    }
    let library = fs::read_to_string(root.join(PREPARING[16])).unwrap();
    // The one invocation of the helper, and its bounds: a deadline, and the
    // caller's priority and jobs (no forced nice value or job count).
    for required in [
        "timeout -s KILL \"$PHYSICAL_INPUTS_DEADLINE\" \\\n        \"$SOPHIA_INTEGRATION_XTASK\" prepare-physical-inputs",
        "--sophia-root=\"$SOPHIA_SOURCE\" --build-dir=\"$build\" --out=\"$out\"",
        "prepare-physical-inputs verify",
        "--manifest-sha256=\"$sha\"",
        "--manifest-sha256=\"$SOPHIA_PHYSICAL_INPUTS_SHA256\"",
        "physical_inputs_verify_exported() {",
        "SOPHIA_GATE_BUILD_DIR must name an absolute private build directory (no default)",
    ] {
        assert!(
            library.contains(required),
            "physical_inputs.sh lacks {required:?}"
        );
    }
    assert_eq!(library.matches("prepare-physical-inputs \\").count(), 1);
    for line in library.lines().filter(|l| !l.trim_start().starts_with('#')) {
        assert!(
            !line.contains("nice ") && !line.contains("CARGO_BUILD_JOBS="),
            "physical_inputs.sh overrides the caller's priority or jobs: {line}"
        );
    }
    assert!(
        !library.contains("source \"$dir/inputs.env\"")
            && !library.contains(". \"$dir/inputs.env\"")
    );
    // The strict reader knows exactly the helper's keys.
    let keys = library
        .lines()
        .find_map(|l| l.strip_prefix("PHYSICAL_INPUTS_KEYS=\""))
        .and_then(|l| l.strip_suffix('"'))
        .unwrap()
        .split(' ')
        .collect::<Vec<_>>();
    assert_eq!(keys, xtask::physical_inputs::ENV_KEYS);
    let runner = fs::read_to_string(root.join(PREPARING[17])).unwrap();
    assert!(runner.contains(". \"$ROOT_DIR/tools/lib/physical_inputs.sh\""));
}

#[test]
fn every_archive_site_verifies_the_prepared_inputs_first() {
    let root = repo();
    let mut all = Vec::new();
    scripts(&root.join("tools"), &mut all);
    let mut sites = 0;
    for path in &all {
        let name = relative(path);
        if name.starts_with("tools/archive_") || name.contains("/check_") || name.contains("test_")
        {
            continue;
        }
        let text = fs::read_to_string(path).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            let Some(code) = code(line) else { continue };
            if !(code.contains("tools/archive_") && code.contains("_run.sh")) {
                continue;
            }
            sites += 1;
            let window = &lines[index.saturating_sub(8)..index];
            assert!(
                window
                    .iter()
                    .any(|l| l.contains("physical_inputs_verify_exported")),
                "{name}:{} archives without verifying the prepared inputs",
                index + 1
            );
        }
    }
    assert_eq!(sites, 7, "the archive sites changed; review this control");
}

#[test]
fn sophia_session_wrapper_never_builds() {
    let text = fs::read_to_string(repo().join("tools/session/run_desktop_session.sh")).unwrap();
    // The one call of Sophia's wrapper (the last mention, with its `--`).
    let call = text
        .rsplit_once("\"$SOPHIA_ROOT/tools/run_sophia_session.sh\" --")
        .unwrap()
        .0;
    assert!(
        call.rsplit("status=0")
            .next()
            .unwrap()
            .contains("SOPHIA_BUILD_SESSION=false"),
        "run_desktop_session.sh must hand Sophia's wrapper SOPHIA_BUILD_SESSION=false"
    );
}

/// The Rust side: only the bounded builders spawn a build. The direct-scanout
/// gate (a Rust runner) takes prepared inputs like the shell runners.
#[test]
fn only_the_bounded_builders_spawn_a_build() {
    const BUILDERS: [&str; 3] = [
        "crates/xtask/src/package_desktop.rs",
        "crates/xtask/src/product_artifact.rs",
        "crates/xtask/src/bemenu_artifact.rs",
    ];
    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let root = repo();
    let mut all = Vec::new();
    for krate in [
        "crates/xtask/src",
        "crates/desktop-comparison/src",
        "crates/quickshell-probe/src",
    ] {
        sources(&root.join(krate), &mut all);
    }
    let mut found = Vec::new();
    for path in &all {
        let name = relative(path);
        if BUILDERS.contains(&name.as_str()) {
            continue;
        }
        let compact = fs::read_to_string(path)
            .unwrap()
            .split_whitespace()
            .collect::<String>();
        for pattern in [
            "\"cargo\",\"build\"",
            "var_os(\"CARGO\")",
            "\"--target-dir\"",
            "\"nim\",\"c\"",
            "Command::new(\"nimble\")",
        ] {
            if compact.contains(pattern) {
                found.push(format!("{name}: {pattern}"));
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
    let gate = fs::read_to_string(root.join("crates/xtask/src/direct_scanout_gate.rs")).unwrap();
    assert!(
        gate.contains("crate::physical_inputs::verify(&sources.inputs, &sources.inputs_sha256)")
    );
}
