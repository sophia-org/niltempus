# Component update release

Release `niltempus-4f498ff25c3a6d89c16f` is verified and selected for
`niltempus install`. Installer and packaging code are signed commit `9f42a6b`.
The installed CLI was updated to that clean build, SHA-256
`1b6fcb7bbf582620bf06c6468a5de04cb304be2f8c88b30a6f253b01fe7980c0`.
The selection binds manifest SHA-256
`d1616c2d8cc36328cb50c7a8856a197734a463cd196db187270fcda483228bd8`.

The product sources remain those of the preceding readiness release: Sophia
`2d69924a9`, Hagia `69f427ab`, Lom `53d3a921`, Bemenu `8e0d56d3` and Narthex
`50b9014d`. This does not take the subsequent SDK release or Sophia changes.

## Verification

The complete offline build, schema-7 checks and desktop policy preflight passed.
The actual release passed private installation, repeat installation, second
installation and rollback. The rollback fixture now changes a Git reference
spelling to produce its second identity while preserving the real installer
hash. Its managed WM update also carries matching selection metadata; the
original failed fixture log is retained.

The installed CLI then built and selected real Lom, Bemenu and Hagia in private
mounts. Every update preserved the other components' selection records. The C
SDK checks and Hagia policy preflight passed. Hagia used the reviewed dependency
manifest and staged toolchain through the packaged helper, including nested
Bubblewrap. No component was signalled in these build checks.

Pidfd replacement is covered separately by supervised fixture processes,
including a Bubblewrap boundary. This is process replacement evidence, not
physical rendering or live Session acceptance. No new product pairing or GPU
qualification is claimed for this release; its sources are unchanged.

## Preparation and use

Preparation copied the release into durable user state, selected it, updated
the CLI and completed the installer config with the explicit reviewed inputs.
The former CLI, config and selection are backed up. The current system release,
personal Hagia executable and metadata, and user desktop profile were preserved.
No installation, activation or live restart was performed.

Install, then log in once to adopt the stable component paths. Afterwards:

```sh
niltempus restart lom     # selected executable, no build
niltempus reload lom      # build and replace
niltempus restart bemenu
niltempus restart hagia
```

`reload` also accepts `bemenu` and `hagia`; `rollback-component NAME` restores
the previous component. See [component updates](component-updates.md).

Evidence and the preservation receipt are under
`development-evidence/component-updates/`, including `prepared-result.json`,
`real-component-builds.log` and `actual-release-install-2.log`.

## Namespace-init restart correction

The first operator restart timed out: Lom ran as namespace PID 1 without a
SIGTERM handler, so SIGTERM did not stop it. The original Go fixture installed
a handler and missed this case. Fix `187386a` checks namespace identity and
signal disposition, choosing SIGKILL only for init without a handler, and still
signals the exact revalidated pidfd once. A native fixture first proves the
ignored SIGTERM, then verifies replacement and neighbour survival. Go tests and
vet passed; the CLI was updated without changing the installed release.

One live retry succeeded: Lom PID 8823 became 14964 with the same selected
binary hash. Sophia 8745, Hagia 8752 and Bemenu 9007 remained running. Evidence:
`pid1-live-restart.log`. This confirms process replacement; no automated visual
acceptance is claimed. Graceful SIGTERM handling remains product-side work for
Lom, Hagia and Provlita; Bemenu already has a handler.
