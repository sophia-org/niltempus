#!/usr/bin/env bash
# Provenance: moved from Sophia tools/verify_mirror_group_physical_archive.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Sophia is explicit: its signed commits come from SOPHIA_SOURCE and its
# retained files from the pinned SOPHIA_ROOT (tools/lib/sophia_source.sh).
source "$ROOT_DIR/tools/lib/sophia_source.sh"
sophia_source="$(sophia_source_repo)" || exit 2
state_home="${XDG_STATE_HOME:-$HOME/.local/state}"
run_root="${SOPHIA_MIRROR_RUN_ROOT:-$state_home/sophia/promotion/mirror-group-runs}"
run="${1:-}"
if [[ -z "$run" ]]; then
    run="$(find "$run_root" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -n 1 || true)"
fi
[[ -n "$run" && -s "$run/SHA256SUMS" ]] || {
    echo "mirror-group archive is missing: ${run:-$run_root}" >&2
    exit 1
}
(
    cd "$run"
    sha256sum -c --status SHA256SUMS
) || {
    echo "mirror-group archive checksum verification failed: $run" >&2
    exit 1
}
[[ "$(sed -n 's/^record_kind=//p' "$run/manifest")" == mirror_group_physical ]] || {
    echo "mirror-group archive has the wrong record kind: $run" >&2
    exit 1
}
[[ "$(cat "$run/result.kdl")" == 'sophia_mirror_group_physical schema=1 status=passed' ]] || {
    echo "mirror-group archive is not passing: $run" >&2
    exit 1
}
for key in source_commit evidence_sha256 sophia_binary_sha256 profile_sha256; do
    [[ "$(grep -c "^${key}=" "$run/manifest")" == 1 ]] || {
        echo "mirror-group archive has invalid $key cardinality: $run" >&2
        exit 1
    }
done
source_commit="$(sed -n 's/^source_commit=//p' "$run/manifest")"
[[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] &&
    git -C "$sophia_source" cat-file -e "$source_commit^{commit}" || {
    echo "mirror-group archive has an invalid source commit: $run" >&2
    exit 1
}
git -C "$sophia_source" verify-commit "$source_commit" >/dev/null 2>&1 || {
    echo "mirror-group archive source commit does not have a valid signature: $run" >&2
    exit 1
}
evidence_sha256="$(sha256sum "$run/session.log" | awk '{ print $1 }')"
profile_sha256="$(sha256sum "$run/profile.kdl" | awk '{ print $1 }')"
[[ "$(sed -n 's/^evidence_sha256=//p' "$run/manifest")" == "$evidence_sha256" ]] || {
    echo "mirror-group evidence digest does not match its manifest: $run" >&2
    exit 1
}
[[ "$(sed -n 's/^profile_sha256=//p' "$run/manifest")" == "$profile_sha256" ]] || {
    echo "mirror-group profile digest does not match its manifest: $run" >&2
    exit 1
}
identity="$(grep -E '^sophia_mirror_group_gate schema=1 status=starting ' "$run/session.log")"
[[ "$(sed -n 's/.* source_commit=\([0-9a-f]\{40\}\) .*/\1/p' <<<"$identity")" == "$source_commit" ]] || {
    echo "mirror-group evidence and manifest name different source commits: $run" >&2
    exit 1
}
[[ "$(sed -n 's/.* sophia_sha256=\([0-9a-f]\{64\}\) .*/\1/p' <<<"$identity")" == \
    "$(sed -n 's/^sophia_binary_sha256=//p' "$run/manifest")" ]] || {
    echo "mirror-group evidence and manifest name different Sophia binaries: $run" >&2
    exit 1
}
[[ "$(sed -n 's/.* profile_sha256=\([0-9a-f]\{64\}\)$/\1/p' <<<"$identity")" == "$profile_sha256" ]] || {
    echo "mirror-group evidence and manifest name different profiles: $run" >&2
    exit 1
}
"$ROOT_DIR/tools/verify_mirror_group_physical.sh" "$run/session.log" >/dev/null

echo "mirror-group physical archive verified: run=$run commit=$source_commit"
