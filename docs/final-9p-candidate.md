# Qualified 9P desktop candidate

Release `niltempus-583dcced3b89e9319866` was built by the Go installer from
signed niltempus `4290d67c5427c1f08aaa8ebfd301e1ea67d795c1`. It has Plan 3,
an external schema-7 manifest, one login entry and explicit 9P transports for
Hagia, Lom and Bemenu. It is prepared in private state, not installed.

## Source and binary binding

| Component | Source commit | Packaged SHA-256 |
| --- | --- | --- |
| Sophia | `2d69924a9cac3ed1164089c7d1fae23b46d19d71` | `5d58c93dd68c485ccf32c9efb0239373d88f3d46b5f472102aa62061c969fb44` |
| Hagia | `69f427abb0565d10c04dab302915396048252dcf` | `24cf71ac50d942f657689462e7e6dd54b8b91711c8b52e7c8beee663cb647506` |
| Lom | `0d1ff046edd41de02fdf0948cdb0ff74d35a97e7` | `b065562ed67df6b74d6a681f35587a788f5dbc1bce01f34ff4599e25e56ab929` |
| Bemenu | `8e0d56d3ca292edf056822eedcbf2985a1cd0083` | `4b05c92632319677a05aaefc2b4e467472a72bbdd88ed058fdde91de12c3b45f` |

Hagia and Bemenu vendor C SDK `8decca1d73699d6750c9228ecbf6e27f589d965d`.
Lom uses Rust SDK `0da10428ad2ef85ff9f1c35c11238fbfebe81d04`.
Narthex remains packaged at `50b9014d96f675f515b5e092c071427fb8e34423`;
this profile does not select it as the bar or launcher.

Hagia's reviewed dependency manifest digest is
`fabc46a9c97091f6738d0ffe1cb04a07d783e61cd1db742667a533f71263e20e`;
Narthex's is
`35a5c578fdd556304f9f4e914e6dc45aa92a0b1564d3720f11c9fc48522eb2bf`.
The Go build exercised the nested sandbox with staged Nim config and stdlib.
These identities do not claim a fully reproducible host C toolchain.

## Results

- Sophia's combined offline gate: 5,858 workspace tests passed, zero failed;
  SDK checks, clippy, layout, generic protocol and retained verifier gates passed.
- Niltempus: 207 offline tests passed, zero failed; clippy, formatting, pinned
  self-tests and legacy archive verification passed. Go test and vet passed.
- Full Go build: WM pair, package, external verification, Lom, strict Bemenu
  compilation and `policy=validated` desktop-profile preflight passed.
- The exact packaged Hagia passed all 24 protected owner/pairing phases against
  Sophia `2d69924a9` plus its recorded Hagia-owned test overlay.
- The exact packaged Bemenu passed the production 9P launcher fixture:
  two openings, one edit, one activation, isolated font directories, an
  unchanged neighbour and retirement of a held lease.
- The exact packaged Lom passed the protected 9P content-conformance host:
  allocation, eight diagnostic bytes, accepted candidate, retained lease and
  release. This is a CPU/content-path check, not GPU or native presentation.
- The real Go CLI passed install, repeat install, second install, rollback and
  status in private mounts. It preserved the personal WM's bytes, metadata,
  mode, timestamp and inode, left one login entry, and wrote the activation ledger.

Logs live under the operator's development-evidence directories:
`final-9p/`, `final-9p/niltempus/`, and
`hagia-sdk-pairing/run5-final-release/`. Failed attempts remain alongside them.
The first final pairing attempt refused a stale fixture hash before running
Hagia; the binding was corrected without changing a product or assertion.

The packaged Bemenu differs from the separate helper-built artifact because
the Go build supplies both GIT_SHA1 and GIT_TAG. Its live fixture used a copy
of the verified sealed release, independently hashed and bound to the signed
source and SDK manifest. `bind-packaged-bemenu.py` and `PROVENANCE.json` record
that adapter; it does not claim the helper produced that binary.

## Installation boundary

No live session, installed release, selected personal WM or user profile was
changed during qualification. Physical display, GPU execution, hardware
latency and attended acceptance remain unclaimed. The descriptor host's
independent C serve/bar peer gaps remain recorded in E1-descriptor-hosts.md.

The operator's existing personal Hagia at source `5af36ac7` does not implement
`config check-environment-contract`; an isolated probe refused it. The new
launcher therefore cannot use that binary. Installation deliberately does not
replace an existing personal WM. The operator must explicitly select the
qualified Hagia before using this candidate at the next login; changing the
running WM is a separate action.
