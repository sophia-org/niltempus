#!/usr/bin/env bash
# Provenance: moved from Sophia tools/check_live_session_milestone5_verifier.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The GTK evidence verifier stays in Sophia (kept by ruling): it is read from
# the staged pinned tree; its fixtures and the TTY-recovery verifier are here.
tree="${SOPHIA_TEST_TREE:-}"
[[ "$tree" == /* && -d "$tree/tools" ]] || {
    echo "SOPHIA_TEST_TREE must name the absolute staged pinned tree." >&2
    exit 2
}
verify="$tree/tools/verify_live_session_milestone5_gtk_evidence.sh"
classic="$root/tools/fixtures/live_session_milestone5_gtk_classic_pass.log"
confined="$root/tools/fixtures/live_session_milestone5_gtk_confined_pass.log"
bad="$root/tools/fixtures/live_session_milestone5_gtk_protocol_error.log"
"$verify" "$classic" "$confined"
if "$verify" "$classic" "$bad" >/dev/null 2>&1; then
    echo "Milestone 5 verifier accepted protocol-error evidence" >&2
    exit 1
fi
echo "Milestone 5 GTK verifier fixtures passed"
recovery_verify="$root/tools/verify_live_session_milestone5_tty_recovery.sh"
recovery_pass="$root/tools/fixtures/live_session_milestone5_tty_recovery_pass.log"
recovery_emergency="$root/tools/fixtures/live_session_milestone5_tty_recovery_emergency.log"
recovery_bad="$root/tools/fixtures/live_session_milestone5_tty_recovery_bad_kd.log"
"$recovery_verify" "$recovery_pass"
"$recovery_verify" "$recovery_emergency" true
if "$recovery_verify" "$recovery_bad" >/dev/null 2>&1; then
    echo "Milestone 5 recovery verifier accepted a changed KD mode" >&2
    exit 1
fi
echo "Milestone 5 TTY recovery verifier fixtures passed"
