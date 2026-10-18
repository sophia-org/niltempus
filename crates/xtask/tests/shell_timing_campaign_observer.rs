//! Device-free failure-path controls. Nothing calls a successful prepare,
//! reads live proc/sys state, starts a worker or launches a native session.
use std::sync::atomic::{AtomicUsize, Ordering};
use xtask::shell_timing_campaign::Condition;
use xtask::shell_timing_campaign_observer::{LiveCampaignObserver, load_lifetime_ms};
use xtask::shell_timing_native::{CampaignObserver, Request};

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "campaign-observer-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn json(&self, file: &str) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(self.0.join(file)).unwrap()).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn disabled_request() -> Request {
    serde_json::from_value(serde_json::json!({
        "sophia_source":{"path":"/fixture/sophia","commit":"unused"},
        "integration_source":{"path":"/fixture/integration","commit":"unused"},
        "artifacts":{}, "tty":"/dev/tty99", "display":99, "input_seat":"fixture",
        "runtime_ms":60000,"wire":"files","output":3,"frames":1100,
        "mode_millihz":120000,"instrumentation":"none","environment_readback":false,
    }))
    .unwrap()
}

#[test]
fn load_lifetime_leaves_bounded_setup_and_restoration_room() {
    assert_eq!(load_lifetime_ms(60_000).unwrap(), 180_000);
    assert_eq!(load_lifetime_ms(480_000).unwrap(), 600_000);
    assert!(load_lifetime_ms(480_001).is_err());
    assert!(load_lifetime_ms(u64::MAX).is_err());
}

#[test]
fn rejected_prepare_retains_failure_without_starting_any_owner() {
    for condition in [Condition::Idle, Condition::Load] {
        let scratch = Scratch::new();
        let mut observer = LiveCampaignObserver::new(condition, "card0".into());
        assert_eq!(
            observer
                .prepare(&disabled_request(), &[], &scratch.0)
                .unwrap_err(),
            "campaign requires environment observation"
        );
        assert!(
            observer
                .finish(&scratch.0)
                .unwrap_err()
                .contains("environment monitor was not prepared")
        );
        assert_eq!(
            scratch.json("environment.json")["error"],
            "environment monitor was not prepared"
        );
        assert_eq!(
            scratch.json("environment-receipts.json"),
            serde_json::json!([])
        );
        assert!(!scratch.0.join("environment.sock").exists());
        assert!(!scratch.0.join("load.json").exists());
        assert_eq!(
            observer.finish(&scratch.0).unwrap_err(),
            "campaign observer was already finished"
        );
    }
}

#[test]
fn finalization_uses_reserved_names_and_never_overwrites_existing_evidence() {
    let scratch = Scratch::new();
    for name in [
        "plan.json",
        "result.json",
        "session.log",
        "environment-receipts.json",
    ] {
        std::fs::write(scratch.0.join(name), "sentinel").unwrap();
    }
    let mut observer = LiveCampaignObserver::new(Condition::Idle, "card0".into());
    assert!(observer.finish(&scratch.0).is_err());
    // The receipt collision does not prevent retention of the other diagnostic.
    assert_eq!(
        scratch.json("environment.json")["error"],
        "environment monitor was not prepared"
    );
    for name in [
        "plan.json",
        "result.json",
        "session.log",
        "environment-receipts.json",
    ] {
        assert_eq!(
            std::fs::read_to_string(scratch.0.join(name)).unwrap(),
            "sentinel"
        );
    }
}

#[test]
fn campaign_commands_refuse_missing_or_relative_inputs_without_claiming() {
    use xtask::shell_timing_campaign_run::run;
    assert!(run(&[]).unwrap_err().starts_with("usage:"));
    assert_eq!(
        run(&[
            "run-campaign-arm".into(),
            "relative".into(),
            "0".repeat(64),
            "arm-0".into()
        ])
        .unwrap_err(),
        "campaign path must be absolute without parent traversal"
    );
    let scratch = Scratch::new();
    assert!(
        run(&[
            "run-campaign-arm".into(),
            scratch.0.to_str().unwrap().into(),
            "0".repeat(64),
            "arm-0".into()
        ])
        .is_err()
    );
    assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 0);
}
