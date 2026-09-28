// Provenance: moved from Sophia crates/sophia-cli/tests/launcher_safety.rs
// (lines 271-377) at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy;
// source unchanged at the pin de776c68) (Sophia rule 13), with the installed
// stack it inspects. Changes: the Hagia profile test asserts the retired
// per-client profile locations are gone (SOPHIA_DESKTOP_PROFILE_MODE, no
// alias); the recipe half it checked now lives in this repository's adapter;
// the installed session must hand Sophia the adapter, the host checker and
// the recipe tool as absolute release paths.
//! Static safety properties of the installed launchers and installer.
const ADAPTER: &str = include_str!("../../../tools/session/run_desktop_session.sh");
const INSTALLED_SESSION: &str = include_str!("../../../tools/installed/sophia-session");
const INSTALLED_HAGIA: &str = include_str!("../../../tools/installed/sophia-hagia-session");
const INSTALLED_HAGIA_PROMOTION: &str =
    include_str!("../../../tools/installed/sophia-hagia-promotion-session");
const INSTALLED_RECOVERY: &str = include_str!("../../../tools/installed/sophia-recovery-proof");
const INSTALLER: &str = include_str!("../../../tools/install_live_session.sh");
const ACTIVATOR: &str = include_str!("../../../tools/activate_live_session_release.sh");

#[test]
fn installed_session_uses_only_versioned_release_artifacts() {
    assert!(INSTALLED_SESSION.contains("SOPHIA_BUILD_SESSION=false"));
    assert!(INSTALLED_SESSION.contains("SOPHIA_MANAGE_KEYD=false"));
    assert!(INSTALLED_SESSION.contains("$RELEASE_DIR/target/release/sophia"));
    assert!(!INSTALLED_SESSION.contains("cargo "));
    assert!(!INSTALLED_SESSION.contains("sudo "));
    assert!(!INSTALLED_SESSION.contains("SOPHIA_NATIVE_WM_BIN"));
    assert!(!INSTALLED_SESSION.contains("sophia-wm-demo"));
    // Every path Sophia's launcher contract requires is absolute and sealed
    // in the release.
    for line in [
        "export SOPHIA_ROOT=\"$RELEASE_DIR\"",
        "export SOPHIA_SESSION_PREFLIGHT=\"$RELEASE_DIR/target/release/active-session-preflight\"",
        "export SOPHIA_INTEGRATION_XTASK=\"$RELEASE_DIR/target/release/sophia-integration-xtask\"",
        "adapter=\"$RELEASE_DIR/tools/session/run_desktop_session.sh\"",
    ] {
        assert!(INSTALLED_SESSION.contains(line), "{line}");
    }
    // Sophia's supervisor sees the opaque label, never the product profile.
    assert!(INSTALLED_SESSION.contains("_supervise --profile=\"$session_label\" -- \"$adapter\""));
    assert!(!INSTALLED_SESSION.contains("--profile=\"$SOPHIA_TTY_PROFILE\""));
}

#[test]
fn installed_hagia_separates_personal_and_packaged_promotion_profiles() {
    assert!(INSTALLED_HAGIA.contains("$config_home/sophia/desktop.kdl"));
    assert!(INSTALLED_HAGIA.contains("/etc/sophia/desktop.kdl"));
    assert!(!INSTALLED_HAGIA.contains("config.kdl"));
    assert!(INSTALLED_HAGIA.contains("packaged-fallback"));
    assert!(INSTALLED_HAGIA.contains("export SOPHIA_DESKTOP_PROFILE_MODE=\"$profile_mode\""));
    assert!(INSTALLED_HAGIA_PROMOTION.contains("packaged-promotion"));
    assert!(INSTALLED_HAGIA_PROMOTION.contains("unset SOPHIA_DESKTOP_PROFILE"));
    assert!(ADAPTER.contains("session-recipe prepare-arguments"));
    assert!(ADAPTER.contains("Ctrl+Alt+Delete to log out"));
}

#[test]
fn installed_watchdog_is_fixed_and_opt_in() {
    assert!(INSTALLED_RECOVERY.contains("SOPHIA_SESSION_WATCHDOG_SECONDS=45"));
    assert!(INSTALLED_RECOVERY.contains("$RELEASE_DIR/bin/sophia-hagia-session"));
    assert!(!INSTALLED_SESSION.contains("export SOPHIA_SESSION_WATCHDOG_SECONDS="));
}

#[test]
fn installer_preserves_a_rollback_pointer_before_activation() {
    let verify = ACTIVATOR.find("sha256sum -c SHA256SUMS").unwrap();
    let preserve = ACTIVATOR
        .find("mv -Tf \"$previous_temp\" \"$PREFIX/previous\"")
        .unwrap();
    let activate = ACTIVATOR
        .find("mv -Tf \"$current_temp\" \"$PREFIX/current\"")
        .unwrap();
    assert!(verify < preserve);
    assert!(preserve < activate);
    // A never-activated release must also pass this repository's current
    // verifier before any link changes, and the activation is recorded only
    // after the switch.
    let current = ACTIVATOR
        .find("\"$ROOT_DIR/tools/verify_packaged_policy.sh\" \"$release\"")
        .unwrap();
    // Activation history is read (and a recorded ID's contents checked)
    // before the bundled verifier runs, and recorded only after the switch.
    let history = ACTIVATOR
        .find("activation_ledger_status \"$release_id\" \"$release\"")
        .unwrap();
    let ledger = ACTIVATOR.find("activation_ledger_record").unwrap();
    assert!(ACTIVATOR.find("activation_ledger_bootstrap").unwrap() < history);
    assert!(history < verify);
    assert!(verify < current && current < preserve);
    assert!(activate < ledger);
    // Rollback reaches a target only through a recorded, unchanged entry.
    let rollback = include_str!("../../../tools/rollback_live_session.sh");
    let recorded = rollback
        .find("activation_ledger_status \"$target_id\" \"$target\"")
        .unwrap();
    assert!(recorded < rollback.find("sha256sum -c SHA256SUMS").unwrap());
    let installer_current = INSTALLER
        .find("\"$ROOT_DIR/tools/verify_packaged_policy.sh\" \"$staging\"")
        .unwrap();
    assert!(installer_current < INSTALLER.find("mv \"$staging\" \"$target\"").unwrap());
    assert!(INSTALLER.contains("sha256sum -c SHA256SUMS"));
    assert!(INSTALLER.contains("activate_live_session_release.sh"));
}

#[test]
fn installer_verifies_root_owned_staging_before_immutable_promotion() {
    let copy = INSTALLER.find("cp -a \"$artifact\" \"$staging\"").unwrap();
    let ownership = INSTALLER.find("chown -R 0:0 -- \"$staging\"").unwrap();
    let staged_ledger = ownership
        + INSTALLER[ownership..]
            .find("sha256sum -c SHA256SUMS")
            .unwrap();
    let verify = INSTALLER
        .find("\"$staging/tools/verify_packaged_policy.sh\" \"$staging\"")
        .unwrap();
    let promote = INSTALLER.find("mv \"$staging\" \"$target\"").unwrap();

    assert!(copy < ownership);
    assert!(ownership < staged_ledger);
    assert!(staged_ledger < verify);
    assert!(verify < promote);
    assert!(!INSTALLER.contains("\"$artifact/tools/verify_packaged_policy.sh\""));
    // The artifact is always explicit; nothing is packaged from a checkout.
    assert!(!INSTALLER.contains("install_current_live_session"));
}

/// The policy client is owner-only, and the release remains the fallback.
///
/// This path names the process that will be handed the WM socket, so the
/// checks around it are the point rather than decoration. A binary someone
/// else can write is a binary someone else can make the window manager.
#[test]
fn a_user_policy_client_is_refused_unless_its_owner_alone_can_write_it() {
    let override_block = INSTALLED_SESSION
        .split_once("sophia_user_wm=")
        .expect("installed session launcher never looks for a user policy client")
        .1;
    let export = override_block
        .find("export SOPHIA_HAGIA_BIN")
        .expect("the developer override never exports the policy client");
    let decision = &override_block[..export];

    assert!(
        decision.contains("-O \"$sophia_user_wm\""),
        "a policy client owned by another user must be refused"
    );
    assert!(
        decision.contains("*w*w*"),
        "a group- or world-writable policy client must be refused"
    );
    assert!(
        decision.contains("exit 1"),
        "an unsafe policy client must stop the session rather than fall back to \
         the release, which would silently run something else"
    );
    assert!(
        decision.contains("$RELEASE_DIR/target/release/hagia"),
        "the release binary must remain the default when no override exists"
    );

    // The user copy is chosen only when it is there, so an install that never
    // placed one still starts on the packaged binary.
    let opt_in = decision
        .find("elif [[ -x \"$sophia_user_wm\" ]]")
        .expect("the user policy client is not chosen by the file existing");
    assert!(
        decision[..opt_in].contains("-n \"${SOPHIA_HAGIA_BIN:-}\""),
        "an explicit SOPHIA_HAGIA_BIN must still win, which is how gates point \
         at their fixtures"
    );
}
