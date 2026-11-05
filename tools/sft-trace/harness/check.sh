#!/usr/bin/env bash
set -euo pipefail
umask 077
harness="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
traces="$(realpath "${1:?trace directory}")"
out="$(realpath -m "${2:?new validation directory}")"
jar="${SOPHIA_SPECULA_TLA2TOOLS_JAR:?pinned Specula tools jar}"
community="${SOPHIA_SPECULA_COMMUNITY_JAR:?pinned Specula community jar}"
[[ "$(sha256sum "$jar" | cut -d ' ' -f 1)" == e22f8ffb4bacdea0a871f444dd94fe5fb0d8013b3388ae39e82e26f852c735d5 ]]
[[ "$(sha256sum "$community" | cut -d ' ' -f 1)" == 044e8ecdfbca92d51d7eb4469422c2a7da1fe25dc8ad39c4a90e6622d6da4d99 ]]
shopt -s nullglob
files=("$traces"/*.ndjson)
(( ${#files[@]} > 0 ))
mkdir "$out"
check="$(mktemp -d)"
trap 'rm -rf "$check"' EXIT
cp "$harness"/../spec/{Client,Trace,MC}.tla "$harness"/../spec/{Trace,MC}.cfg "$check/"
cd "$check"
timeout 60s java -cp "$jar:$community" tlc2.TLC -workers 1 -config MC.cfg MC.tla >"$out/model.log" 2>&1
for file in "${files[@]}"; do
    [[ -s "$file" ]]
    JSON="$file" timeout 60s java -cp "$jar:$community" tlc2.TLC -workers 1 \
        -config Trace.cfg Trace.tla >"$out/$(basename "$file").log" 2>&1
done
printf 'Validated %s nonempty runtime traces.\n' "${#files[@]}"
