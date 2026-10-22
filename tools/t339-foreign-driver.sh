#!/bin/sh
# External driver for the t339 two-process FOREIGN control.
# Owns all producer orchestration: the Sophia oracle only listens.
# Usage: t339-foreign-driver.sh <oracle-test-binary> <dma_peer-binary> <log>
set -u
if [ $# -ne 3 ]; then
    echo "usage: $0 <oracle-test-binary> <dma_peer-binary> <log>" >&2
    exit 2
fi
ORACLE=$1
PEER=$2
LOG=$3
for f in "$ORACLE" "$PEER"; do
    if [ ! -x "$f" ]; then
        echo "not an executable: $f" >&2
        exit 2
    fi
done
WORK=$(mktemp -d /tmp/sophia-foreign-driver.XXXXXX) || exit 1
SOCK=$WORK/oracle.sock
cleanup() {
    [ -n "${oracle_pid:-}" ] && kill "$oracle_pid" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT INT TERM HUP

SOPHIA_TEST_RENDER_NODE=/dev/dri/by-path/pci-0000:03:00.0-render \
SOPHIA_TEST_FOREIGN_SOCKET=$SOCK \
  timeout 120 "$ORACLE" --ignored --exact \
  vulkan_foreign_frames_capture_exactly_and_stay_independent --nocapture \
  > "$LOG" 2>&1 &
oracle_pid=$!
i=0
while [ ! -S "$SOCK" ]; do
    if ! kill -0 "$oracle_pid" 2>/dev/null; then
        echo "oracle exited before binding its socket" >> "$LOG"
        wait "$oracle_pid"
        echo "driver peer_status=not-run oracle_status=$?" >> "$LOG"
        exit 1
    fi
    i=$((i+1))
    if [ $i -ge 100 ]; then
        echo "oracle did not bind its socket within 10s" >> "$LOG"
        exit 1
    fi
    sleep 0.1
done
timeout 60 bwrap --ro-bind / / --dev /dev --tmpfs /dev/dri \
  --dev-bind /dev/dri/renderD128 /dev/dri/renderD128 \
  --unshare-all --die-with-parent --new-session -- \
  env SOPHIA_SHELL_GPU_MODE=direct SOPHIA_SHELL_GPU_GRANT_EPOCH=1 \
  SOPHIA_SHELL_GPU_RENDER_NODE=/dev/dri/renderD128 \
  SOPHIA_SHELL_GPU_DEVICE_MAJOR=226 SOPHIA_SHELL_GPU_DEVICE_MINOR=128 \
  "$PEER" "$SOCK" >> "$LOG" 2>&1
peer_status=$?
wait "$oracle_pid"
oracle_status=$?
oracle_pid=
echo "driver peer_status=$peer_status oracle_status=$oracle_status" >> "$LOG"
[ "$peer_status" -eq 0 ] && [ "$oracle_status" -eq 0 ]
