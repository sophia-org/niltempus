# E1: descriptor host retained coverage

This page maps what `shell_descriptor_conformance_host` covered, mode by mode.
The host is `crates/sophia-runtime/examples/shell_descriptor_conformance_host.rs`,
built as a `sophia-conformance` example. Each assertion is marked as one of
three things:
- **generic**: Sophia keeps it, in root's
  `crates/sophia-conformance/tests/shell_descriptor_modes.rs` at `9bc6bb7`;
- **product**: Narthex coverage, which belongs outside Sophia;
- **unpeered**: no independent peer covers it.

Sources:
- the pin, `de776c68afdf9a133818f86917893c3362dc9fb7`;
- root's `9bc6bb7` ("Exercise descriptor host modes with a generic protocol
  peer"), a descendant of the pin.

This page records a mapping. It adds no code and no test here.

## The 9P-only reading

The host is **IPC-only** at the pin and at `9bc6bb7`:
- It binds `ShellSessionTransport` and hands the client
  `SOPHIA_SHELL_SOCKET`.
- It calls `accept_and_negotiate` and exchanges `IpcMessageKind` frames.

Sophia's own file-wire spec says the same thing at the pin:
- `docs/sophia-shell-files.md` ("Legacy descriptor paths (`reference.rs`,
  `tabs.rs` and the descriptor launcher flow) keep sending frames until that
  profile moves to files").
- The t255 purge inventory lists the shell socket transport and the
  `ipc::shell_*` codecs.

Under the 9P-only direction, this host is a **seam, not something to wrap**.
This repository adds no IPC harness around it. Product coverage of the
descriptor profile (Narthex) can move here only after root gives the legacy
descriptor profile a file export (`descriptors`, `tabs`, `shortcuts`, the
reference and reservation flows) and a host that serves it over 9P.

## Mode map

### `--proof` (the default)

**What it covers.**
- Descriptor presentation of two surfaces, one exact activation, and
  withdrawal.
- The protected launch: bubblewrap `MetadataShell` domain, peer authorised from
  protection evidence.
- The verdict line:
  `sophia_shell_descriptor_corpus schema=1 status=complete protected=true descriptors=2 activations=1 withdrawn=true surface_ids_disclosed=0 coordinates_disclosed=0 icons_disclosed=0`.

**Generic (kept in Sophia).** `descriptor_presentation_activation_and_withdrawal`
asserts that exact line. Its peer, `shell_descriptor_contract_peer`, is a
scripted peer on the public Rust codecs. Those are the same codecs the host
uses, so this is an owner/codec check, not an independent one.

**Independent generic peer.** The C `sophia_shell_v1_client`
(`vendor/c-desktop-sdk/source/src/tests/sophia_shell_v1_client.c`) runs this
mode in Sophia's `tools/check_shell_protocol.sh:75-80`. The encoder is
independent, but the run is IPC (it uses the SDK's socket half, which is in
the purge inventory). It is retired with product IPC, not moved.

**Product (Narthex).** `check_shell_protocol.sh:123-124` runs the host against
a Narthex built from the sibling checkout. Narthex's own
`tools/check_narthex.sh` runs the host from `SOPHIA_ROOT`. Both are IPC.
External target: the 9P descriptor host seam above.

### `--serve`

**What it covers.** The tab protocol proof comes first. Two persistent
generations run, including a superseded transfer. There is one exact
activation. A stale presentation epoch is rejected. The line is
`sophia_tab_protocol_proof status=complete supersession=true activation=true stale_epoch_rejected=true`.
Next comes the reference corpus: 256 entries, paging, dismissal and no
disclosed actions (`sophia_reference_corpus status=complete entries=256 paging=true dismissal=true actions_disclosed=0`).
Last, the `--proof` descriptor lifecycle runs.

**Generic (kept in Sophia).**
- `persistent_tabs_reference_and_descriptor_lifecycles` asserts all three
  lines exactly.
- `tab_acknowledgements_must_name_the_exact_event_and_transaction` sends seven
  malformed acks and requires the host's exact refusal: `ack-epoch`,
  `ack-activation`, `ack-transaction`, the three `stale-` variants, and
  `stale-accepted`. These tighten the pin. At the pin the host checked only the
  transaction and the disposition of the live ack, and only the disposition of
  the stale one (the `9bc6bb7` host diff).

**Unpeered: flag.** No independent (non-Rust-codec) generic peer runs `--serve`.
The C `sophia_shell_v1_client` runs only `--proof`. Before, the only
independent encoder of the tab and reference frames here was Narthex's Nim.
Root's peer shares the host's codecs, so a codec defect that is symmetric on
both sides would pass. The golden-frame corpora and `shell_tabs` and
`shell_reference` codec tests (`check_shell_protocol.sh:68`, `:72`) fix the byte
format. They do not drive this lifecycle.

**Product (Narthex).**
- `check_shell_protocol.sh:125-126` runs `--serve` against Narthex.
- `check_shell_protocol.sh:114-119` runs Narthex's `tshell_v1`, `tshell_tabs`
  and `tshell_launcher` Nim tests with `SOPHIA_ROOT` (Narthex-side codec tests
  against Sophia's golden frames).

All of it is IPC. External target: the 9P descriptor host seam. The physical
side of Narthex already has external coverage here, in the Hagia policy gate
(switcher and Narthex restart, `docs/hagia-gates.md`).

### `--bar-proof`

**What it covers.**
- The client gets `SOPHIA_SHELL_BAR_THICKNESS=28` and claims a bottom strip.
- Engine's coordinator admits it. The work area changes only when the bundle
  commits, and withdrawal restores it.
- The verdict line:
  `sophia_shell_reservation_corpus schema=1 status=complete protected=true edge=bottom thickness=28 reserved_height=1412 withdrawn=true`.

**Generic (kept in Sophia).** `reservation_changes_work_area_only_at_commit_and_withdraws`
asserts that exact line, with the same Rust-codec peer.

**Unpeered: flag.** No independent generic peer runs `--bar-proof`. The C
client does not implement the bar mode. Narthex was the only independent
implementation that claimed a strip through this host. The component bar's
file-wire reservation role (bit 1 in `sophia-shell-files.md`) is a different
profile. No test found at the pin drives "work area changes only at commit"
through it with an independent client. Root should confirm or name one.

**Product (Narthex).** `check_shell_protocol.sh:127-132` runs `--bar-proof`
against Narthex. It is IPC. External target: the 9P descriptor host seam.

## Launcher host (context)

`check_shell_protocol.sh:93` and `:134` also run
`shell_launcher_conformance_host`: against the C launcher client and against
Narthex. That is outside E1's three modes and is listed here only because the
same product script drives it. It too is IPC.

## Summary

| Mode | Generic (Sophia, `9bc6bb7`) | Independent generic peer | Product (Narthex) |
| --- | --- | --- | --- |
| `--proof` | `descriptor_presentation_activation_and_withdrawal` | C `sophia_shell_v1_client` (IPC, retires with product IPC) | seam: 9P descriptor host |
| `--serve` | `persistent_tabs_reference_and_descriptor_lifecycles`, `tab_acknowledgements_must_name_the_exact_event_and_transaction` | **none: flag** | seam: 9P descriptor host |
| `--bar-proof` | `reservation_changes_work_area_only_at_commit_and_withdraws` | **none: flag** | seam: 9P descriptor host |

Nothing in E1 moves to this repository until the seam exists. When it does:
- the Narthex runs of the three modes and the `tshell_*` tests move here as
  product gates against a prepared Narthex artifact;
- root's generic tests stay in Sophia;
- `check_shell_protocol.sh`'s Narthex section is deleted with the IPC host.
