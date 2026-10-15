# Provlita's protected file-export join

`crates/live-tests/tests/provlita_files.rs` runs the real Provlita executable
against Sophia's production shell file export. The production supervisor
launches it in a Bubblewrap metadata-shell domain. The host admits the peer
using that supervisor's PID evidence and a persistent-catalog content profile.
This is supplied conformance admission, not Session's factotum launch path.

Provlita must negotiate revision 8 over 9P2000.L and then refuse startup because
it has no direct-GPU grant. The test repeats with a fresh connection and content
grant, and requires empty content accounting after each exit. No render node,
display socket or GPU grant is exposed. There is no CPU rendering substitute.

The gate is ignored by ordinary tests and requires explicit absolute binary
and configuration paths, each with an expected SHA-256. It checks both hashes
before and after execution. Source-to-binary provenance comes from the recorded
build; these hash checks do not reproduce it or prevent concurrent file changes.
Do not rebuild the executable while the gate runs.

After provisioning the private Cargo home, run under a device-hidden wrapper:

```sh
SOPHIA_PROVLITA_BINARY=/absolute/path/to/provlita \
SOPHIA_PROVLITA_SHA256=EXPECTED_BINARY_SHA256 \
SOPHIA_PROVLITA_CONFIG=/absolute/path/to/config.kdl \
SOPHIA_PROVLITA_CONFIG_SHA256=EXPECTED_CONFIG_SHA256 \
CARGO_HOME=/absolute/private/cargo-home \
cargo-slot niltempus bwrap --bind / / --dev /dev --tmpfs /tmp \
  --tmpfs /run/user --dir /run/user/1000 --chmod 0700 /run/user/1000 \
  --setenv XDG_RUNTIME_DIR /run/user/1000 \
  --unshare-net --unshare-pid --proc /proc \
  --unsetenv DISPLAY --unsetenv WAYLAND_DISPLAY \
  cargo test --offline --locked -p live-tests --test provlita_files \
  -- --ignored --nocapture
```

Use the operator's UID in the runtime-directory arguments. The private `/proc`
must match the private PID namespace for nested protected launches to work.

## October 15 evidence

The external test uses Sophia's pinned public crates at `316d969507cdeb6ef061ea2d9645758113c53db6`.
Provlita's clean, signed source is `da4a15adb9a6555441653a9dd9ec0e4d69f3d6af`.
The existing offline release build supplied:

- Binary: `~/.cache/cargo-slots/provlita/1/release/provlita`, SHA-256
  `1bfa663d7583014d049ad5695e8ea869df33d964dbc90a60672e7feb9568be6b`.
- Configuration: Provlita's `examples/minimal/config.kdl`, SHA-256
  `c46db4786c9021a9a65d3537b53c8761c65226ab089aacab47d6e46170e192ef`.

Both epochs, 61 and 62, pass. Replacing the host's persistent-catalog profile
with a native-launcher profile fails negotiation (`UnsupportedRevision`);
restoring the profile passes. The first fixture attempt reused content-grant
generation 1 and was correctly refused as stale on the second launch. The
fixture now advances both generations. Logs are retained under
`~/.local/state/sophia/development-evidence/t252-provlita-20261015/`:
`join.log`, `join-final.log`, `wrong-profile-negative.log` and `restored.log`.

This proves protected executable negotiation, the missing-GPU boundary and
reservation reclamation on this exact pair. It does not prove authenticated
Session startup, GPU rendering, upload, Presented, input, native retirement,
three-component coexistence or performance. It also does not qualify the newer
Sophia t330 repair or change the installed release. Those remain separate
t252 checks before dock rollout.

## Authenticated pair, October 15

The external Cargo pin now selects signed Sophia
`58a481ebc756b50fe97a890b968a6c9b70863496`, including t330. The C SDK manifest
and all sixteen contract bindings are refreshed from that tree; the two added
bindings cover namespace discovery and launch authentication. Pin audit,
provisioning, device-hidden workspace tests and strict workspace Clippy pass.
The supplied-admission Provlita gate also passes against the updated pin.
The desktop flake and installed release are separate selections and have not
been changed by this Cargo update.

Sophia's existing generic `supplied_component_client_authenticates_across_launches`
test now also passes with the actual Provlita executable above, installed Lom
and installed Bemenu. This uses the real Factotum owner and authenticated
protected launch path. Provlita and Lom negotiate and then refuse the missing
GPU grant; Bemenu remains alive until explicit revocation. Each runs twice with
fresh component epochs and launch registrations, and each process is reaped
before replacement. A Provlita launch provisioned for the bar role fails with
authentication refusal 13 before shell negotiation.

The packaged Narthex `56d513fa` fails authenticated startup: it predates the
authentication implementation. Signed Narthex
`11eb0332859ed0e0b9244dd3ddafcea70c2b1b6e`, built from a clean tree, passes
`supplied_descriptor_client_authenticates_and_recovers` on the same Sophia.
It negotiates descriptor revision 8, survives the idle observation, and recovers
after revocation at two fresh epochs. Its binary SHA-256 is
`699505779b729084de04f25fc8752b417cd40671048258cd9b35a5d89d43e536`.
Select this authenticated revision for the next fallback package; the old
installed package is not evidence of compatibility with the new server.

Logs and input hashes are under
`~/.local/state/sophia/development-evidence/t252-authenticated-20261015/`.
The original Narthex failure and wrong-role negative control are retained.
The new Narthex build needed an explicit read-only bind into the outer test
sandbox because that sandbox hides `/tmp`; the first attempt without that bind
ran no client. All successful authentication checks hide display devices and
sockets. Authentication and cleanup do not establish catalog actions, content,
GPU work, native presentation or B9 performance.
