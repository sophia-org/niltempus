#!/usr/bin/env bash
set -euo pipefail
umask 077
harness="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sdk="$(realpath "${1:?SDK worktree}")"
out="$(realpath -m "${2:?new output directory}")"
[[ "$(git -C "$sdk" rev-parse HEAD)" == fd9330b64fb415dab8daacb769bcca835b546661 ]]
[[ -z "$(git -C "$sdk" status --porcelain)" ]]
mkdir "$out"
mkdir "$out/traces"
git -C "$sdk" apply --check "$harness/instrumentation.patch"
git -C "$sdk" apply "$harness/instrumentation.patch"
(
    cd "$sdk"
    SFT_TRACE_DIR="$out/traces" timeout 30m cargo-slot sdk-rs cargo test --locked \
        -p sophia-shell-client --test file_wire_scripted frames:: \
        -- --test-threads=1 >"$out/tests.log" 2>&1
)
shopt -s nullglob
traces=("$out"/traces/*.ndjson)
(( ${#traces[@]} > 0 ))
wc -l "${traces[@]}"
