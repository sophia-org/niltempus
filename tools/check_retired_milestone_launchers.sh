#!/usr/bin/env bash
# Provenance: moved from Sophia tools/check_retired_milestone_launchers.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The historical readers and their fixtures stay in Sophia (kept by ruling):
# they are read from the staged pinned tree.
tree="${SOPHIA_TEST_TREE:-}"
[[ "$tree" == /* && -d "$tree/tools" ]] || {
    echo "SOPHIA_TEST_TREE must name the absolute staged pinned tree." >&2
    exit 2
}
fixture="$(mktemp -d)"
trap 'rm -rf -- "$fixture"' EXIT
# Only shell builtins are available. With tracing enabled, even an ignored
# external-command attempt is visible and fails this test.
for name in two_xterm milestone3; do
    set +e
    PATH="$fixture" /bin/bash -x "$root/tools/live_session_${name}_hardware_proof.sh" \
        >"$fixture/stdout" 2>"$fixture/stderr"
    status=$?
    set -e
    [[ "$status" == 2 && ! -s "$fixture/stdout" ]]
    grep -Fq 'historical hardware gate is retired' "$fixture/stderr"
    grep -Fq "verify_live_session_${name}_evidence.sh" "$fixture/stderr"
    if grep -E '^\+ ' "$fixture/stderr" | grep -Ev '^\+ (printf |exit 2$)'; then
        echo "retired launcher attempted more than its retirement notice" >&2
        exit 1
    fi
done
# Historical readers must continue to validate their original evidence.
"$tree/tools/verify_live_session_two_xterm_evidence.sh" \
    "$tree/tools/fixtures/live_session_two_xterm_evidence_pass.log" >/dev/null
sed 's/namespace_profile=classic_shared/namespace_profile=confined/g' \
    "$tree/tools/fixtures/live_session_two_xterm_evidence_pass.log" >"$fixture/confined.log"
"$tree/tools/verify_live_session_milestone3_evidence.sh" \
    "$tree/tools/fixtures/live_session_two_xterm_evidence_pass.log" "$fixture/confined.log" >/dev/null
printf '%s\n' 'retired launcher and historical evidence checks passed'
