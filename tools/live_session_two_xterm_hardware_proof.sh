#!/usr/bin/env bash
# Provenance: moved from Sophia tools/live_session_two_xterm_hardware_proof.sh at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13).
# Historical evidence remains readable; this launcher no longer owns hardware.
printf '%s\n' \
    'This historical hardware gate is retired.' \
    'For current native-session validation, see docs/validation.md (Native Session Integration).' \
    'Read retained evidence with tools/verify_live_session_two_xterm_evidence.sh.' >&2
exit 2
