use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::PathBuf};
use xtask::shell_timing_native::{Artifact, Request, Source, split_capture, validate};

struct Fixture {
    root: PathBuf,
    request: Request,
}
impl Fixture {
    fn new(name: &str, wire: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("native-timing-{}-{name}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let mut this = Self {
            root: root.clone(),
            request: Request {
                sophia_source: Source {
                    path: root.clone(),
                    commit: "a".repeat(40),
                },
                integration_source: Source {
                    path: root,
                    commit: "b".repeat(40),
                },
                artifacts: BTreeMap::new(),
                tty: "/dev/tty4".into(),
                display: 94,
                input_seat: "seat0".into(),
                runtime_ms: 60_000,
                wire: wire.into(),
                output: 1,
                frames: 3,
                mode_millihz: 60_000,
                instrumentation: "none".into(),
            },
        };
        // These fixtures are hashed only. None may be executed by validation.
        for key in [
            "sophia",
            "peer",
            "wm",
            "factotum",
            "pam-helper",
            "preflight",
            "runner",
        ] {
            this.put(key, b"must not execute", true);
        }
        this.put("core", b"schema 2\n", false);
        this.put(
            "peer-config",
            format!(r#"{{"wire":"{wire}","output":1,"frames":3,"height":24,"hold_msec":60000}}"#)
                .as_bytes(),
            false,
        );
        let transport = if wire == "files" {
            "9p2000.L"
        } else {
            "comparison-ipc"
        };
        let profile = format!(
            r#"schema 1
session {{
    shell-component "timing" "bar" {{ executable "{}"; config "{}"; transport "{transport}"; gpu "denied"; }}
    startup
}}
"#,
            this.request.artifacts["peer"].path.display(),
            this.request.artifacts["peer-config"].path.display()
        );
        this.put("profile", profile.as_bytes(), false);
        this
    }
    fn put(&mut self, key: &str, bytes: &[u8], executable: bool) {
        let path = self.root.join(key);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(
            &path,
            fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
        )
        .unwrap();
        self.request.artifacts.insert(
            key.into(),
            Artifact {
                path,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            },
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn capture_preserves_failed_client_records_and_refuses_interleaved_or_partial_writes() {
    let client = "{\"schema\":1,\"event\":\"failed\",\"error\":\"timeout\"}\n";
    let native = "sophia_shell_native_completion schema=1\n";
    let (actual, host) = split_capture(&format!(
        "unrelated host record\n{client}2026 INFO target: {native}"
    ))
    .unwrap();
    assert_eq!(actual, client);
    assert_eq!(host, native);
    assert!(xtask::shell_timing::collect(&actual, 1).is_err());
    for invalid in [
        format!("{client}{native}").trim_end().to_owned(),
        format!("{{\"schema\":1HOST_INTERLEAVED\n{native}"),
        client.into(),
        native.into(),
    ] {
        assert!(split_capture(&invalid).is_err());
    }
}

#[test]
fn both_wires_bind_exact_workload_without_executing_artifacts() {
    for wire in ["files", "ipc"] {
        let f = Fixture::new(wire, wire);
        let args = validate(&f.request).unwrap();
        assert_eq!(args.len(), 13);
        assert_eq!(
            args[0],
            f.request.artifacts["sophia"].path.to_str().unwrap()
        );
        assert!(args.contains(&"--max-runtime-ms=60000".into()));
        assert!(args.contains(&"--input-seat=seat0".into()));
        for (key, flag) in [
            ("factotum", "factotum-agent"),
            ("pam-helper", "factotum-pam-helper"),
        ] {
            assert!(args.contains(&format!(
                "--{flag}={}",
                f.request.artifacts[key].path.display()
            )));
        }
    }
}

#[test]
fn changed_files_and_workload_cannot_pass_a_prepared_identity() {
    let mut f = Fixture::new("changed", "files");
    fs::write(&f.request.artifacts["peer"].path, b"changed").unwrap();
    assert!(
        validate(&f.request)
            .unwrap_err()
            .contains("artifact changed")
    );
    f.put("peer", b"must not execute", true);
    f.request.frames = 4;
    assert!(validate(&f.request).is_err());
    f.request.frames = 3;
    f.request.wire = "ipc".into();
    assert!(validate(&f.request).is_err());
    f.request.wire = "files".into();
    f.request.runtime_ms = 70_000;
    assert!(validate(&f.request).is_err());
    f.request.runtime_ms = 60_000;
    f.request.input_seat = "seat0 --extra".into();
    assert!(validate(&f.request).is_err());
    f.request.input_seat = "seat0".into();
    let helper = f.request.artifacts.remove("pam-helper").unwrap();
    assert!(validate(&f.request).is_err());
    f.request.artifacts.insert("pam-helper".into(), helper);
    fs::set_permissions(
        &f.request.artifacts["factotum"].path,
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(validate(&f.request).is_err());
}

#[test]
fn profile_indirection_extra_clients_and_startup_are_refused() {
    let mut f = Fixture::new("profiles", "files");
    let profile = fs::read_to_string(&f.request.artifacts["profile"].path).unwrap();
    for bad in [
        profile.clone() + "include \"/tmp/elsewhere\"\n",
        profile.replace("startup", "startup \"/bin/true\""),
        profile.replace("startup", "startup {};"),
        profile.replace("startup", "shell-component \"other\" \"bar\" {}; startup"),
        profile.replace("gpu \"denied\"", "gpu \"direct\""),
        profile.replace("transport \"9p2000.L\"", "transport \"comparison-ipc\""),
        profile.replace("startup", "include \"/tmp/elsewhere\"; startup"),
    ] {
        f.put("profile", bad.as_bytes(), false);
        assert!(validate(&f.request).is_err(), "accepted {bad}");
    }
}

#[test]
fn plan_cli_rejects_tampering_before_any_run_directory_exists() {
    let f = Fixture::new("cli", "files");
    let path = f.root.join("plan.json");
    fs::write(&path, b"{}").unwrap();
    let out = f.root.join("not-created");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["shell-timing", "run-native"])
        .arg(&path)
        .arg("0".repeat(64))
        .arg(&out)
        .env_remove("SOPHIA_SHELL_TIMING_ARM")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("plan digest differs"));
    assert!(!out.exists());
}

#[test]
fn physical_launch_requires_arm_and_actual_console_before_source_or_process_work() {
    let f = Fixture::new("unarmed", "files");
    let argv = validate(&f.request).unwrap();
    let bytes =
        serde_json::to_vec(&serde_json::json!({"schema":1,"request":f.request,"argv":argv}))
            .unwrap();
    let path = f.root.join("plan.json");
    fs::write(&path, &bytes).unwrap();
    let out = f.root.join("not-created");
    for armed in [false, true] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"));
        command
            .args(["shell-timing", "run-native"])
            .arg(&path)
            .arg(format!("{:x}", Sha256::digest(&bytes)))
            .arg(&out)
            .stdin(std::process::Stdio::null())
            .env_remove("SOPHIA_SHELL_TIMING_ARM");
        if armed {
            command.env("SOPHIA_SHELL_TIMING_ARM", "1");
        }
        let result = command.output().unwrap();
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains(if armed {
                "run from the prepared console"
            } else {
                "SOPHIA_SHELL_TIMING_ARM=1"
            }),
            "{error}"
        );
        assert!(!out.exists());
    }
}
