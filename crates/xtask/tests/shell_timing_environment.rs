//! Environment capture checks over supplied fixture text. No devices, no
//! `/proc` or `/sys` reads.
use xtask::shell_timing_environment::*;

const HOST: u32 = 4100;
const PEER: u32 = 4200;

fn expect() -> Expect {
    Expect {
        output: 3,
        card: "card1".into(),
        connector: "DP-2".into(),
        width: 2560,
        height: 1440,
        refresh_millihz: 120000,
        scale_milli: 1000,
        vrr: Vrr::Off,
        host_pid: HOST,
        peer_pid: PEER,
        capture: Interval {
            start_usec: 1_000,
            end_usec: 9_000,
        },
        measurement: Interval {
            start_usec: 2_000,
            end_usec: 8_000,
        },
    }
}

/// A `/proc/<pid>/stat` line with 52 fields; fields 19, 40 and 41 set.
fn stat(pid: u32, comm: &str, nice: i64, rt: u64, policy: u64) -> String {
    let mut fields: Vec<String> = (3..=52).map(|_| "0".to_string()).collect();
    fields[0] = "S".into();
    fields[19 - 3] = nice.to_string();
    fields[40 - 3] = rt.to_string();
    fields[41 - 3] = policy.to_string();
    format!("{pid} ({comm}) {}\n", fields.join(" "))
}

fn status(pid: u32, cpus: &str) -> String {
    format!(
        "Name:\tx\nUmask:\t0022\nState:\tS (sleeping)\nTgid:\t{pid}\nPid:\t{pid}\nCpus_allowed:\tffff\nCpus_allowed_list:\t{cpus}\n"
    )
}

const OUTPUT: &str = "sophia_shell_environment_output schema=1 pid=4100 output=3 card=card1 connector=DP-2 width=2560 height=1440 refresh_millihz=120000 timing_refresh_millihz=120014 scale_milli=1000 vrr=off monotonic_usec=1500";
const RENDERER: &str = "sophia_shell_environment_renderer schema=1 pid=4100 output=3 card=card1 driver=amdgpu vendor=Mesa renderer=AMD%20Radeon%20RX%207900%20XTX%20%28radeonsi%2C%20navi31%29 version=4.6%20%28Core%20Profile%29%20Mesa%2025.2.4 monotonic_usec=1600";

fn host_log() -> String {
    format!(
        "sophia_shell_native_binding schema=1 output=3 mode_refresh_millihz=120000\n{OUTPUT}\n{RENDERER}\nunrelated line\n"
    )
}

fn source(kind: SourceKind, origin: &str, text: &str) -> SourceText {
    SourceText {
        kind,
        origin: origin.into(),
        captured_usec: 1_200,
        text: text.into(),
    }
}

fn capture() -> Capture {
    let mut version = source(
        SourceKind::ProcVersion,
        "/proc/version",
        "Linux version 6.18.55_1 (voidlinux@void) (gcc 14.2) #1 SMP PREEMPT_DYNAMIC\n",
    );
    version.captured_usec = 1_300;
    let mut sources = vec![
        source(
            SourceKind::Osrelease,
            "/proc/sys/kernel/osrelease",
            "6.18.55_1\n",
        ),
        version,
        source(
            SourceKind::CpuOnline,
            "/sys/devices/system/cpu/online",
            "0-3\n",
        ),
        source(
            SourceKind::ProcStat { pid: HOST },
            "/proc/4100/stat",
            &stat(HOST, "sophia-host", 0, 0, 0),
        ),
        source(
            SourceKind::ProcStatus { pid: HOST },
            "/proc/4100/status",
            &status(HOST, "0-3"),
        ),
        source(
            SourceKind::ProcStat { pid: PEER },
            "/proc/4200/stat",
            &stat(PEER, "timing peer", 5, 0, 3),
        ),
        source(
            SourceKind::ProcStatus { pid: PEER },
            "/proc/4200/status",
            &status(PEER, "2-3"),
        ),
        source(
            SourceKind::DrmDriver {
                card: "card1".into(),
            },
            "/sys/class/drm/card1/device/driver",
            "amdgpu\n",
        ),
        source(SourceKind::HostLog, "host.log", &host_log()),
        source(
            SourceKind::Note,
            "operator",
            "no other sessions; fans steady",
        ),
    ];
    for cpu in 0..4 {
        sources.push(source(
            SourceKind::Governor { cpu },
            &format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"),
            "performance\n",
        ));
    }
    Capture { sources }
}

/// The capture with one source's text replaced.
fn with(kind: SourceKind, text: &str) -> Capture {
    let mut capture = capture();
    let source = capture.sources.iter_mut().find(|s| s.kind == kind).unwrap();
    source.text = text.into();
    capture
}

fn without(kind: SourceKind) -> Capture {
    let mut capture = capture();
    capture.sources.retain(|s| s.kind != kind);
    capture
}

fn log_with(old: &str, new: &str) -> Capture {
    let log = host_log();
    assert!(log.contains(old), "{old}");
    with(SourceKind::HostLog, &log.replacen(old, new, 1))
}

#[test]
fn a_complete_capture_is_environment_complete_with_provenance() {
    let env = environment_record(&capture(), &expect()).unwrap();
    assert_eq!(env.verdict, "environment_complete");
    assert!(env.scope.contains("authenticity"));
    assert_eq!(env.kernel.release.value, "6.18.55_1");
    assert!(
        env.kernel
            .version
            .value
            .starts_with("Linux version 6.18.55_1 ")
    );
    assert_eq!(env.cpus_online.value.ranges, [(0, 3)]);
    assert_eq!(env.governors.len(), 4);
    assert_eq!(
        env.governors[2].source.origin,
        "/sys/devices/system/cpu/cpu2/cpufreq/scaling_governor"
    );
    let peer = &env.processes[1];
    assert_eq!((peer.role, peer.pid), ("peer", PEER));
    let scheduling = &peer.scheduling.value;
    assert_eq!(scheduling.scope, "process_leader_thread");
    assert_eq!(
        (scheduling.comm.as_str(), scheduling.nice, scheduling.policy),
        ("timing peer", 5, "batch")
    );
    assert_eq!(peer.affinity.value.ranges, [(2, 3)]);
    assert_eq!(peer.affinity.source.origin, "/proc/4200/status");
    assert_eq!(env.drm_driver.value, "amdgpu");
    let renderer = &env.renderer.value;
    assert_eq!(
        renderer.renderer,
        "AMD Radeon RX 7900 XTX (radeonsi, navi31)"
    );
    assert_eq!(renderer.version, "4.6 (Core Profile) Mesa 25.2.4");
    assert_eq!(env.renderer.source.origin, "host.log");
    assert_eq!(env.output.value.vrr, Vrr::Off);
    assert_eq!(env.notes.len(), 1);
    let json = serde_json::to_value(&env).unwrap();
    assert_eq!(json["output"]["source"]["kind"], "host_log");
    assert_eq!(json["governors"][0]["source"]["cpu"], 0);
    assert_eq!(json["expect"]["vrr"], "off");
    let kernel = &json["kernel"];
    assert_eq!(kernel["release"]["source"]["kind"], "osrelease");
    assert_eq!(
        kernel["release"]["source"]["origin"],
        "/proc/sys/kernel/osrelease"
    );
    assert_eq!(kernel["release"]["source"]["captured_usec"], 1_200);
    assert_eq!(kernel["version"]["source"]["kind"], "proc_version");
    assert_eq!(kernel["version"]["source"]["origin"], "/proc/version");
    assert_eq!(kernel["version"]["source"]["captured_usec"], 1_300);
}

#[test]
fn every_required_source_must_be_present() {
    for kind in [
        SourceKind::Osrelease,
        SourceKind::ProcVersion,
        SourceKind::CpuOnline,
        SourceKind::Governor { cpu: 3 },
        SourceKind::ProcStat { pid: HOST },
        SourceKind::ProcStatus { pid: HOST },
        SourceKind::ProcStat { pid: PEER },
        SourceKind::ProcStatus { pid: PEER },
        SourceKind::DrmDriver {
            card: "card1".into(),
        },
        SourceKind::HostLog,
    ] {
        assert!(
            environment_record(&without(kind.clone()), &expect()).is_err(),
            "{kind:?}"
        );
    }
    // Notes are optional.
    assert!(environment_record(&without(SourceKind::Note), &expect()).is_ok());
}

#[test]
fn host_records_must_state_every_output_and_renderer_field() {
    for record in [OUTPUT, RENDERER] {
        assert!(environment_record(&log_with(record, ""), &expect()).is_err());
        let keys: Vec<&str> = record.split(' ').skip(1).collect();
        for key in keys {
            let missing = record.replacen(&format!(" {key}"), "", 1);
            assert!(
                environment_record(&log_with(record, &missing), &expect()).is_err(),
                "{key}"
            );
            let repeated = format!("{record} {key}");
            assert!(environment_record(&log_with(record, &repeated), &expect()).is_err());
        }
        let extra = format!("{record} requested_vrr=off");
        assert!(environment_record(&log_with(record, &extra), &expect()).is_err());
        let twice = format!("{record}\n{record}");
        assert!(environment_record(&log_with(record, &twice), &expect()).is_err());
    }
    // Field order is free.
    let reordered = OUTPUT.replacen("schema=1 pid=4100", "pid=4100 schema=1", 1);
    assert!(environment_record(&log_with(OUTPUT, &reordered), &expect()).is_ok());
    let unknown = "sophia_shell_environment_vrr schema=1 vrr=off\n";
    assert!(environment_record(&log_with("unrelated line\n", unknown), &expect()).is_err());
}

#[test]
fn vrr_and_mode_are_never_taken_from_config_or_notes() {
    // A note stating the mode and VRR state cannot stand in for the record.
    let mut capture = log_with(OUTPUT, "");
    capture.sources.push(SourceText {
        kind: SourceKind::Note,
        origin: "niltempus.kdl".into(),
        captured_usec: 1_200,
        text: OUTPUT.into(),
    });
    assert!(environment_record(&capture, &expect()).is_err());
    // Only the observed on/off value is accepted.
    for vrr in ["vrr=requested", "vrr=auto", "vrr=", "vrr=Off"] {
        assert!(environment_record(&log_with("vrr=off", vrr), &expect()).is_err());
    }
}

#[test]
fn every_output_and_renderer_fact_must_match_the_campaign() {
    for (old, new) in [
        (
            "pid=4100 output=3 card=card1 connector",
            "pid=4200 output=3 card=card1 connector",
        ),
        (
            "output=3 card=card1 connector",
            "output=4 card=card1 connector",
        ),
        ("card=card1 connector", "card=card0 connector"),
        ("connector=DP-2", "connector=HDMI-A-2"),
        ("width=2560", "width=1920"),
        ("height=1440", "height=1080"),
        (
            "refresh_millihz=120000 timing",
            "refresh_millihz=60000 timing",
        ),
        ("scale_milli=1000", "scale_milli=1250"),
        ("vrr=off", "vrr=on"),
        ("monotonic_usec=1500", "monotonic_usec=9001"),
        // The peer cannot supply graphics identity.
        (
            "pid=4100 output=3 card=card1 driver",
            "pid=4200 output=3 card=card1 driver",
        ),
        ("output=3 card=card1 driver", "output=4 card=card1 driver"),
        ("card=card1 driver", "card=card0 driver"),
        ("driver=amdgpu", "driver=i915"),
        ("monotonic_usec=1600", "monotonic_usec=999"),
    ] {
        assert!(
            environment_record(&log_with(old, new), &expect()).is_err(),
            "{new}"
        );
    }
    let mut on = expect();
    on.vrr = Vrr::On;
    assert!(environment_record(&capture(), &on).is_err());
    let mut fractional = expect();
    fractional.scale_milli = 1500;
    assert!(environment_record(&capture(), &fractional).is_err());
}

#[test]
fn renderer_strings_use_canonical_percent_encoding() {
    assert_eq!(percent_decode("a%20b~").unwrap(), "a b~");
    assert_eq!(percent_decode("%C3%A9").unwrap(), "é");
    for bad in ["a%2", "a%2g", "a%2a", "%41", "a b", "%FF", "%", "a+b"] {
        assert!(percent_decode(bad).is_err(), "{bad}");
    }
    assert!(environment_record(&log_with("version=4.6%20", "version=4.6 "), &expect()).is_err());
    assert!(environment_record(&log_with("vendor=Mesa", "vendor=%4Desa"), &expect()).is_err());
}

#[test]
fn process_scheduling_is_read_past_the_last_paren_for_host_and_peer() {
    let tricky = stat(PEER, "a) 5 (b", -3, 0, 0);
    let parsed = with(SourceKind::ProcStat { pid: PEER }, &tricky);
    let env = environment_record(&parsed, &expect()).unwrap();
    let scheduling = &env.processes[1].scheduling.value;
    assert_eq!((scheduling.comm.as_str(), scheduling.nice), ("a) 5 (b", -3));
    for pid in [HOST, PEER] {
        for (text, what) in [
            (stat(pid, "x", 0, 0, 4), "unknown policy"),
            (stat(pid, "x", 20, 0, 0), "nice out of range"),
            (stat(pid + 1, "x", 0, 0, 0), "another pid"),
            (stat(pid, "", 0, 0, 0), "empty comm"),
            (format!("{pid} (x) S 1 2 3\n"), "truncated"),
            (
                format!("{}\n{}", stat(pid, "x", 0, 0, 0), stat(pid, "x", 0, 0, 0)),
                "two lines",
            ),
        ] {
            let bad = with(SourceKind::ProcStat { pid }, &text);
            assert!(environment_record(&bad, &expect()).is_err(), "{pid} {what}");
        }
        for text in [
            status(pid + 1, "0-3"),
            status(pid, "0-3").replace("Cpus_allowed_list:\t0-3\n", ""),
            status(pid, "0-3") + "Cpus_allowed_list:\t0-3\n",
            status(pid, "3-0"),
        ] {
            let bad = with(SourceKind::ProcStatus { pid }, &text);
            assert!(environment_record(&bad, &expect()).is_err(), "{text}");
        }
    }
    let rt = with(
        SourceKind::ProcStat { pid: HOST },
        &stat(HOST, "h", 0, 10, 1),
    );
    let env = environment_record(&rt, &expect()).unwrap();
    let host = &env.processes[0].scheduling.value;
    assert_eq!((host.policy, host.rt_priority), ("fifo", 10));
    // A process source for an undeclared pid is refused.
    let mut extra = capture();
    extra.sources.push(source(
        SourceKind::ProcStat { pid: 9 },
        "/proc/9/stat",
        &stat(9, "x", 0, 0, 0),
    ));
    assert!(environment_record(&extra, &expect()).is_err());
}

#[test]
fn cpu_lists_are_canonical_and_bounded() {
    assert_eq!(cpu_list("0-3,8,10-11").unwrap().count, 7);
    assert_eq!(cpu_list("8191").unwrap().ranges, [(8191, 8191)]);
    // A single cpu is written alone, never as a one-cpu range.
    assert_eq!(cpu_list("1,3-4").unwrap().ranges, [(1, 1), (3, 4)]);
    assert!(cpu_list("1-1,3-4").is_err());
    assert!(cpu_list("1,3-3").is_err());
    for bad in [
        "",
        "0-3,2-5",
        "0-3,4-5",
        "4,2",
        "3-1",
        "0-8192",
        "0-4294967295",
        "01",
        "0,,1",
        "0-",
        "-3",
        "+1",
        "0-3 ",
        "a",
    ] {
        assert!(cpu_list(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn governors_cover_exactly_the_online_cpus() {
    assert!(environment_record(&with(SourceKind::CpuOnline, "0-4\n"), &expect()).is_err());
    assert!(environment_record(&with(SourceKind::CpuOnline, "0-2\n"), &expect()).is_err());
    let env = environment_record(
        &with(SourceKind::Governor { cpu: 1 }, "powersave\n"),
        &expect(),
    )
    .unwrap();
    let governors: Vec<&str> = env
        .governors
        .iter()
        .map(|g| g.value.governor.as_str())
        .collect();
    assert_eq!(
        governors,
        ["performance", "powersave", "performance", "performance"]
    );
    for bad in ["", "per formance", "Performance", "a\nb"] {
        assert!(
            environment_record(&with(SourceKind::Governor { cpu: 0 }, bad), &expect()).is_err(),
            "{bad:?}"
        );
    }
}

#[test]
fn kernel_and_driver_must_agree() {
    let other = "Linux version 6.18.54_1 (voidlinux@void) #1\n";
    assert!(environment_record(&with(SourceKind::ProcVersion, other), &expect()).is_err());
    assert!(environment_record(&with(SourceKind::Osrelease, "6.18 55\n"), &expect()).is_err());
    let card = SourceKind::DrmDriver {
        card: "card1".into(),
    };
    assert!(environment_record(&with(card.clone(), "i915\n"), &expect()).is_err());
    assert!(environment_record(&with(card, "amd gpu\n"), &expect()).is_err());
    let mut extra = capture();
    extra.sources.push(source(
        SourceKind::DrmDriver {
            card: "card0".into(),
        },
        "/sys/class/drm/card0/device/driver",
        "i915\n",
    ));
    assert!(environment_record(&extra, &expect()).is_err());
}

#[test]
fn sources_are_unique_named_and_inside_a_window_that_brackets_the_run() {
    let mut duplicate = capture();
    duplicate.sources.push(source(
        SourceKind::Osrelease,
        "/proc/sys/kernel/osrelease",
        "6.18.55_1\n",
    ));
    assert!(environment_record(&duplicate, &expect()).is_err());
    let mut repeated_notes = capture();
    repeated_notes
        .sources
        .push(source(SourceKind::Note, "operator", "second note"));
    assert_eq!(
        environment_record(&repeated_notes, &expect())
            .unwrap()
            .notes
            .len(),
        2
    );
    for usec in [999, 9_001] {
        let mut late = capture();
        late.sources[0].captured_usec = usec;
        assert!(environment_record(&late, &expect()).is_err(), "{usec}");
    }
    let mut unnamed = capture();
    unnamed.sources[0].origin.clear();
    assert!(environment_record(&unnamed, &expect()).is_err());
    let mut huge = capture();
    huge.sources.last_mut().unwrap().text = "x".repeat((1 << 20) + 1);
    assert!(environment_record(&huge, &expect()).is_err());
    for (capture_window, measurement) in [
        ((2_500, 9_000), (2_000, 8_000)),
        ((1_000, 7_000), (2_000, 8_000)),
        ((1_000, 9_000), (8_000, 8_000)),
    ] {
        let mut e = expect();
        e.capture = Interval {
            start_usec: capture_window.0,
            end_usec: capture_window.1,
        };
        e.measurement = Interval {
            start_usec: measurement.0,
            end_usec: measurement.1,
        };
        assert!(environment_record(&capture(), &e).is_err());
    }
    let mut same = expect();
    same.peer_pid = HOST;
    assert!(environment_record(&capture(), &same).is_err());
}

#[test]
fn other_heads_are_validated_but_cannot_satisfy_the_campaign() {
    let other_output = OUTPUT
        .replace("output=3", "output=4")
        .replace("connector=DP-2", "connector=HDMI-A-1")
        .replace("refresh_millihz=120000", "refresh_millihz=60000")
        .replace("vrr=off", "vrr=on");
    let other_renderer = RENDERER.replace("output=3", "output=4");
    let heads = format!("{other_output}\n{other_renderer}\n");
    let both = log_with("unrelated line\n", &heads);
    let env = environment_record(&both, &expect()).unwrap();
    assert_eq!(env.output.value.connector, "DP-2");
    assert_eq!(env.output.value.output, 3);
    // Only the other head: nothing matches the campaign.
    let only_other = format!("{other_output}\n{other_renderer}\n");
    let capture = with(SourceKind::HostLog, &only_other);
    assert!(environment_record(&capture, &expect()).is_err());
    // Another head's records are still validated.
    let malformed = format!("{}\n", other_output.replace("vrr=on", "vrr=requested"));
    assert!(environment_record(&log_with("unrelated line\n", &malformed), &expect()).is_err());
    // A conflicting second record for the measured identity fails.
    let conflicting = format!("{OUTPUT}\n{}", OUTPUT.replace("width=2560", "width=1920"));
    assert!(environment_record(&log_with(OUTPUT, &conflicting), &expect()).is_err());
}

fn refusal(capture: &Capture, expect: &Expect) -> String {
    match environment_record(capture, expect) {
        Ok(_) => panic!("capture was accepted"),
        Err(error) => error,
    }
}

#[test]
fn refusals_name_their_reason() {
    let peer_renderer = log_with(
        "pid=4100 output=3 card=card1 driver",
        "pid=4200 output=3 card=card1 driver",
    );
    assert!(
        refusal(&peer_renderer, &expect())
            .contains("no sophia_shell_environment_renderer for the measured")
    );
    let mut on = expect();
    on.vrr = Vrr::On;
    assert_eq!(
        refusal(&capture(), &on),
        "output record vrr differs from the campaign"
    );
    let conflicting = format!("{OUTPUT}\n{}", OUTPUT.replace("width=2560", "width=1920"));
    assert!(refusal(&log_with(OUTPUT, &conflicting), &expect()).contains("more than one"));
    let driver = log_with("driver=amdgpu", "driver=i915");
    assert!(refusal(&driver, &expect()).contains("kernel driver"));
    let late = log_with("monotonic_usec=1600", "monotonic_usec=999");
    assert!(refusal(&late, &expect()).contains("capture window"));
    let missing = without(SourceKind::Governor { cpu: 3 });
    assert!(refusal(&missing, &expect()).contains("exactly the online cpus"));
    let policy = with(
        SourceKind::ProcStat { pid: PEER },
        &stat(PEER, "x", 0, 0, 4),
    );
    assert!(refusal(&policy, &expect()).contains("unknown scheduling policy"));
}

#[test]
fn timing_refresh_is_required_nonzero_and_never_compared() {
    // Reported beside the nominal rate.
    let env = environment_record(&capture(), &expect()).unwrap();
    assert_eq!(
        (
            env.output.value.refresh_millihz,
            env.output.value.timing_refresh_millihz
        ),
        (120_000, 120_014)
    );
    // A 119.98 Hz programmed timing with a nominal 120 Hz mode matches a
    // 120000 mHz campaign: only the nominal rate is compared.
    let near = log_with(
        "timing_refresh_millihz=120014",
        "timing_refresh_millihz=119980",
    );
    let env = environment_record(&near, &expect()).unwrap();
    assert_eq!(env.output.value.timing_refresh_millihz, 119_980);
    assert_eq!(env.output.value.refresh_millihz, 120_000);
    // A timing rate equal to the campaign's does not rescue a nominal mismatch.
    let mismatch = log_with(
        "refresh_millihz=120000 timing_refresh_millihz=120014",
        "refresh_millihz=119000 timing_refresh_millihz=120000",
    );
    assert_eq!(
        refusal(&mismatch, &expect()),
        "output record refresh_millihz differs from the campaign"
    );
    // Missing or zero is refused.
    let missing = log_with(" timing_refresh_millihz=120014", "");
    assert!(refusal(&missing, &expect()).contains("missing fields"));
    let zero = log_with("timing_refresh_millihz=120014", "timing_refresh_millihz=0");
    assert!(refusal(&zero, &expect()).contains("timing_refresh_millihz is zero"));
    let signed = log_with("timing_refresh_millihz=120014", "timing_refresh_millihz=-1");
    assert!(environment_record(&signed, &expect()).is_err());
}
