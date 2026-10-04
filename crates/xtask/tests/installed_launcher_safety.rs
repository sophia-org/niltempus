// Provenance: moved from Sophia crates/sophia-cli/tests/launcher_safety.rs
// (lines 271-377) at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy;
// source unchanged at the pin de776c68) (Sophia rule 13), with the installed
// stack it inspects. Changes: the Hagia profile test asserts the retired
// per-client profile locations are gone (SOPHIA_DESKTOP_PROFILE_MODE, no
// alias); the recipe half it checked now lives in this repository's adapter;
// the installed session must hand Sophia the adapter, the host checker and
// the recipe tool as absolute release paths.
//! Static safety properties of the installed launchers and tools/desktop.
const ADAPTER: &str = include_str!("../../../tools/session/run_desktop_session.sh");
const INSTALLED_SESSION: &str = include_str!("../../../tools/installed/sophia-session");
const INSTALLED_HAGIA: &str = include_str!("../../../tools/installed/sophia-hagia-session");
const INSTALLED_HAGIA_PROMOTION: &str =
    include_str!("../../../tools/installed/sophia-hagia-promotion-session");
const INSTALLED_RECOVERY: &str = include_str!("../../../tools/installed/sophia-recovery-proof");
const DESKTOP: &str = include_str!("../../../tools/desktop");

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
        "export SOPHIA_FACTOTUM_AGENT=\"$RELEASE_DIR/target/release/sophia-factotum\"",
        "export SOPHIA_FACTOTUM_PAM_HELPER=\"$RELEASE_DIR/target/release/sophia-factotum-pam\"",
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
fn desktop_install_checks_the_root_owned_copy_before_switching() {
    let install = &DESKTOP[DESKTOP.find("cmd_install() {").unwrap()..];
    let at = |needle: &str| {
        install
            .find(needle)
            .unwrap_or_else(|| panic!("tools/desktop install lacks {needle:?}"))
    };
    let lock = at("    lock\n");
    let root = at("nix-store --add-root");
    let existing = at("same \"$out\" \"$prefix/releases/$id\" ||");
    let copy = at("as_root cp -r \"$out\" \"$stage\"");
    let ownership = at("as_root chown -R 0:0 \"$stage\"");
    let compare = at("same \"$out\" \"$stage\" ||");
    let promote = at("as_root mv -T \"$stage\" \"$prefix/releases/$id\"");
    let previous = at("switch previous \"$current\"");
    let current = at("switch current \"$id\"");
    assert!(lock < root && root < existing && existing < copy);
    assert!(copy < ownership && ownership < compare && compare < promote);
    assert!(promote < previous && previous < current);
    // The comparison covers contents and the executable set.
    assert!(DESKTOP.contains(
        "same() { { diff -r --no-dereference \"$1\" \"$2\" && diff <(executables \"$1\") <(executables \"$2\"); }"
    ));
    // Rollback and prune change the prefix only under the same lock.
    for command in ["cmd_rollback() {", "cmd_prune() {"] {
        let body = &DESKTOP[DESKTOP.find(command).unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        let locked = body.find("    lock\n").expect("command takes the lock");
        let first_change = ["switch ", "as_root rm"]
            .iter()
            .filter_map(|change| body.find(change))
            .min()
            .unwrap();
        assert!(
            locked < first_change,
            "{command} changes the prefix before locking"
        );
    }
    // A link is replaced only by renaming a complete new link over it.
    assert!(DESKTOP.contains("ln -sfn \"releases/$2\" \"$prefix/.$1.new\" && as_root mv -T"));
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
