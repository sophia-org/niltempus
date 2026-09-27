//! Offline gate entry for the attended Lom gates' verifiers: the GPU/content
//! proof and native-gate verifier mutations, the runner-ordering controls, and
//! the lom_workload and native_launcher Python unit tests, all run by
//! tools/check_lom_gpu_content_proof_verifiers.sh (moved from Sophia's
//! `cargo xtask check` list at 9fcaec782). Nothing here uses a device, a VT or
//! a real product binary.
use std::path::Path;
use std::process::Command;

#[test]
fn lom_gpu_content_and_workload_verifier_self_tests_pass() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("timeout")
        .args(["-s", "KILL", "600", "bash"])
        .arg(repo.join("tools/check_lom_gpu_content_proof_verifiers.sh"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("lom_gpu_content_verifiers schema=1 status=pass"),
        "{stdout}"
    );
}
