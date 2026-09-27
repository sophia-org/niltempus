<!-- Provenance: moved from Sophia docs/validation.md, sections "Installed Native Candidate" and the installed parts of "Keyboard Independence on Hardware", at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). Rewritten for this repository's package, install and verification commands. The day-to-day runbook is docs/operations.md. -->
# Installed candidate

An installed candidate is an immutable desktop release below `/opt/sophia`:
Sophia, the Hagia/Narthex pair, this repository's session wrappers, recipe
tool and host checker, and the retained Sophia session primitives. The
[operations runbook](operations.md) covers the host boundary, session entries,
logs, stop, recovery, fallback login and rollback. This page is the release
and verification path.

## Build and install a release

```sh
cargo xtask prepare-wm-pair --hagia /ABS/hagia <commit> --narthex /ABS/narthex <commit> /ABS/wm-pair \
    --build-dir=/ABS/private-build \
    --hagia-nim-deps=/ABS/hagia.nim-deps --hagia-nim-deps-sha256=<reviewed sha256> \
    --narthex-nim-deps=/ABS/narthex.nim-deps --narthex-nim-deps-sha256=<reviewed sha256> \
    --hagia-c-sdk-rev=<the C SDK revision Hagia vendors>
cargo xtask package-desktop --sophia-root=/ABS/sophia --sophia-rev=<pinned rev> \
    --wm-pair=/ABS/wm-pair --wm-pair-commits=<hagia>,<narthex> \
    --wm-pair-sha256=<hagia>,<narthex> --wm-pair-profile-sha256=<default.kdl> \
    --wm-pair-c-sdk-rev=<the same C SDK revision> \
    --build-dir=/ABS/private-build --out=/ABS/release
tools/install_live_session.sh /ABS/release
```

`--hagia-c-sdk-rev` has no default and is never read from Hagia's tree: the
operator or director names it. Hagia's staged signed tree must vendor exactly
that SDK revision (the manifest names it and `upstream.commit` hashes to it)
and its vendored `source/` must match the manifest file for file, with none
extra or missing. The pair (schema 3) records `hagia_c_sdk_revision`,
carries the vendored manifest as `hagia-c-sdk.manifest.json`, and binds it by
`hagia_c_sdk_manifest_sha256`; verification re-checks both. Hagia vendors
the snapshot at `vendor/sophia-desktop-sdk` (`wm_pair::HAGIA_C_SDK_VENDOR`).
`package-desktop` requires `--wm-pair-c-sdk-rev`, with no default, and
refuses before staging anything unless it equals the verified pair's
`hagia_c_sdk_revision`.

Packaging requires this repository clean with a signed HEAD, the pinned signed
Sophia checkout, the full provisioning marker and the operator's expected
pair commits and digests. Every source is staged as its exact signed tree and
built in the private build directory; the release (schema 7, `SHA256SUMS`)
records exact digests and Git identities. Schema 7 requires
`hagia_c_sdk_revision` and `hagia_c_sdk_manifest_sha256` and seals Hagia's
vendored SDK manifest as `share/sophia-policy/hagia/c-sdk.manifest.json`.
`package-desktop` cross-checks both fields against the verified pair and the
sealed file, and the packaged policy verifier (`tools/verify_packaged_policy.sh`,
run by packaging, installation, activation and rollback) re-checks them. Both
refuse, with no defaults, a missing, repeated, malformed or mismatched field
or sealed file. With `hagia_included=false` the SDK fields and the sealed
manifest must be absent. A schema-6 release is refused as a candidate:
- Installation runs this repository's current verifier on the staged copy,
  as well as the candidate's bundled one.
- Activating a release that is not recorded as activated also runs the
  current verifier, before any link changes.

Activation history lives in `$PREFIX/activated-releases`. Each line binds a
release_id to the sha256 of its `manifest` and of its `SHA256SUMS`:
- Activation appends the line only after a successful switch.
- A recorded release keeps its own packaged verifier, so an installed
  schema-6 release stays a valid rollback target.
- A recorded ID whose manifest or `SHA256SUMS` no longer matches its entry
  is refused, by activation and by rollback, before its bundled verifier
  runs.
- Links grant nothing, with one exception. An installation that has no
  ledger yet, because it predates this change, records its existing
  `current` and `previous` targets exactly once, on its first activation or
  rollback. After that, only ledger entries count.
- Rollback accepts only a recorded, unchanged previous release. An
  unrecorded one is a new candidate and must be activated.

Historical schema-6 releases are otherwise read only through the legacy path
of Sophia's Go verifier (root).

**Limitation.** These rules are enforced only by this repository's scripts
and by releases built from them. Activation always runs from this
repository's `tools/activate_live_session_release.sh`, but `sophia-rollback`
is the current release's own copy. If the current release is a schema-6-era
release, its rollback script predates the ledger, and it also predates the
schema-7 verifier. It keeps its old behaviour. Installation verifies the artifact
before an atomic `/opt/sophia/current` switch and keeps the former release as
`previous`. No package contains an X11 WM bridge, an embedded legacy WM or
bridge-specific configuration. Local installation does not require pushing or
fetching any repository; publication is separate.

Packaging and installing never switch or overwrite the user's own default
window manager (`$XDG_STATE_HOME/sophia/bin/hagia` and its reload workflow,
`tools/reload_policy_client.sh`, an exempt operator tool). The installed
session may still prefer that user-owned client; the packaged pair is the
fallback and the promotion profile.

Hagia and Narthex build only from reviewed Nim dependency manifests; the
compiler is a staged, verified copy of the reviewed installation. See
[operations](operations.md#bound-nim-dependencies). Host-toolchain identity is
recorded, not a fully reproducible closure.

## Offline regressions

```sh
cargo test --offline --locked -p xtask --test package_desktop
cargo test --offline --locked -p xtask --test wm_pair
cargo test --offline --locked -p xtask --test installed_selftests -- --include-ignored   # needs SOPHIA_TEST_TREE
cargo test --offline --locked -p xtask --test installed_xtask
```

`installed_selftests` runs the moved self-contained regressions:
`check_installed_session_type.sh`, `check_hagia_profile_selection.sh`,
`check_rehearse_wm_9p.sh` and `check_live_session_install.sh` (schema and digest
validation, retired bridge fields refused, base and Hagia activation,
rollback, removal of only Sophia-owned stale entries, foreign desktop entries
preserved). `installed_xtask` runs the packaged recipe tool with its build
checkout hidden. The installed native verifiers are pinned by
`check_installed_native_verifiers.sh` and its sub-checks in
`physical_selftests`.

## Installed evidence

```sh
sophia-status
sophia-verify-login-cycle
sophia-verify-truecolor-runs 1
sophia-verify-xterm-runs 1
sophia-verify-watchdog
sophia-verify-emergency
sophia-verify-fallback
sophia-verify-hagia
sophia-verify-native-chrome
sophia-verify-firefox-runs
```

The Firefox, TrueColor, xterm, watchdog, emergency-recovery, runtime-identity
and login-cycle recorders all identify the Hagia native session. Their
verifiers consume checksummed archives and fail closed on an unexpected
revision, binary identity, result, protocol fault or teardown residue.

Keyboard independence is accepted from an ordinary installed session, verified
with `tools/verify_keyboard_independence_session.sh`; see
[physical runners](physical-runners.md#keyboard-independence).

Use `sophia-stop` or the independent recovery entry to leave a failed session;
neither depends on the policy process continuing to answer.
