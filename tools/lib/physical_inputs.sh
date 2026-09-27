# shellcheck shell=bash
# The one way a physical runner obtains its binaries and the pinned Sophia
# tree: a prepared, read-only input directory made by
# `xtask prepare-physical-inputs`. No runner builds anything, reads a source
# checkout's target directory, or uses a shared or predictable /tmp cache.
#
# Requires ROOT_DIR (this repository) and SOPHIA_INTEGRATION_XTASK (the
# absolute prebuilt recipe tool of this checkout; tools/lib/physical_runner.sh
# checks it).
#
# physical_inputs_prepare HELPER-OPTION...
#     Runs the helper to build a new input directory: explicit, private
#     SOPHIA_GATE_BUILD_DIR (0700, owned, outside every source tree), the
#     pinned SOPHIA_SOURCE checkout, the provisioned CARGO_HOME. The helper
#     stages every source as its exact signed tree (git archive, tree-hash
#     proven), builds offline and --locked at nice 19 with two jobs into
#     private targets below the build directory, proves the trees again after
#     the builds, and writes a new read-only output there. This call runs it
#     at nice 19 with two jobs under a KILL deadline, then loads the result.
# physical_inputs_use DIR MANIFEST-SHA256
#     For runners that take an already prepared directory: loads it.
# physical_inputs_load DIR MANIFEST-SHA256
#     `verify` (every file re-derived; the expected manifest sha256 is
#     required), then the strict inputs.env reader (never `source`d): only
#     the known keys, each once, each value an absolute path or hex. Sets PI[]
#     and exports SOPHIA_PHYSICAL_INPUTS and SOPHIA_PHYSICAL_INPUTS_SHA256 so
#     the gate that archives the run can verify the same directory again.
# physical_inputs_verify_exported
#     Before archiving: verifies SOPHIA_PHYSICAL_INPUTS against
#     SOPHIA_PHYSICAL_INPUTS_SHA256 again (required; no default).

# The inputs.env keys (the same list as xtask::physical_inputs::ENV_KEYS).
PHYSICAL_INPUTS_KEYS="SOPHIA_PHYSICAL_INPUTS SOPHIA_ROOT SOPHIA_COMMIT SOPHIA_INTEGRATION_COMMIT SOPHIA_BIN SOPHIA_WM_DEMO_BIN SOPHIA_HAGIA_BIN SOPHIA_HAGIA_COMMIT SOPHIA_NARTHEX_BIN SOPHIA_NARTHEX_COMMIT SOPHIA_PROFILE_DIR"
# The helper's deadline: every staged tree and build, bounded.
PHYSICAL_INPUTS_DEADLINE=7200

physical_inputs_fail() {
    echo "physical inputs: $*" >&2
    exit 2
}

physical_inputs_prepare() {
    local build="${SOPHIA_GATE_BUILD_DIR:-}" out summary sha
    [[ "$build" == /* && -d "$build" && ! -L "$build" ]] ||
        physical_inputs_fail "SOPHIA_GATE_BUILD_DIR must name an absolute private build directory (no default)"
    [[ "${SOPHIA_INTEGRATION_XTASK:-}" == /* && -x "${SOPHIA_INTEGRATION_XTASK:-}" ]] ||
        physical_inputs_fail "SOPHIA_INTEGRATION_XTASK must name the absolute prebuilt recipe tool"
    [[ "${SOPHIA_SOURCE:-}" == /* ]] ||
        physical_inputs_fail "SOPHIA_SOURCE must name the absolute pinned Sophia checkout"
    [[ "${CARGO_HOME:-}" == /* ]] ||
        physical_inputs_fail "CARGO_HOME must name the provisioned private CARGO_HOME"
    out="$build/inputs-$(date -u +%Y%m%dT%H%M%SZ)-$$"
    echo "Preparing physical inputs (signed trees, private builds) in $out ..."
    summary="$(CARGO_BUILD_JOBS=2 timeout -s KILL "$PHYSICAL_INPUTS_DEADLINE" \
        nice -n 19 "$SOPHIA_INTEGRATION_XTASK" prepare-physical-inputs \
        --sophia-root="$SOPHIA_SOURCE" --build-dir="$build" --out="$out" "$@")" ||
        physical_inputs_fail "prepare-physical-inputs failed"
    printf '%s\n' "$summary"
    sha="$(sed -n 's/^physical_inputs status=prepared manifest_sha256=\([0-9a-f]\{64\}\) .*/\1/p' <<<"$summary")"
    [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || physical_inputs_fail "the helper printed no manifest digest"
    physical_inputs_load "$out" "$sha"
}

physical_inputs_use() {
    physical_inputs_load "${1:-}" "${2:-}"
}

physical_inputs_load() {
    local dir="${1:-}" sha="${2:-}" line key value
    [[ "$dir" == /* && -d "$dir" && ! -L "$dir" ]] ||
        physical_inputs_fail "the input directory must be absolute and real: ${dir:-unset}"
    [[ "$sha" =~ ^[0-9a-f]{64}$ ]] || physical_inputs_fail "the expected manifest sha256 is required"
    timeout -s KILL 600 nice -n 19 "$SOPHIA_INTEGRATION_XTASK" prepare-physical-inputs verify \
        --out="$dir" --manifest-sha256="$sha" ||
        physical_inputs_fail "$dir is not the expected prepared inputs"
    declare -gA PI=()
    [[ -f "$dir/inputs.env" && ! -L "$dir/inputs.env" ]] || physical_inputs_fail "no regular inputs.env"
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ "$line" =~ ^([A-Z][A-Z0-9_]*)=([A-Za-z0-9._/+-]+)$ ]] ||
            physical_inputs_fail "malformed inputs.env line"
        key="${BASH_REMATCH[1]}"
        value="${BASH_REMATCH[2]}"
        [[ " $PHYSICAL_INPUTS_KEYS " == *" $key "* ]] || physical_inputs_fail "unknown inputs.env key $key"
        [[ -z "${PI[$key]+set}" ]] || physical_inputs_fail "inputs.env repeats $key"
        [[ "$value" == /* || "$value" =~ ^[0-9a-f]{40}$ ]] ||
            physical_inputs_fail "inputs.env $key is neither an absolute path nor a commit"
        PI[$key]="$value"
    done <"$dir/inputs.env"
    [[ "${PI[SOPHIA_PHYSICAL_INPUTS]:-}" == "$dir" ]] || physical_inputs_fail "inputs.env names another directory"
    SOPHIA_PHYSICAL_INPUTS="$dir"
    SOPHIA_PHYSICAL_INPUTS_SHA256="$sha"
    export SOPHIA_PHYSICAL_INPUTS SOPHIA_PHYSICAL_INPUTS_SHA256
}

physical_inputs_verify_exported() {
    [[ "${SOPHIA_PHYSICAL_INPUTS:-}" == /* && "${SOPHIA_PHYSICAL_INPUTS_SHA256:-}" =~ ^[0-9a-f]{64}$ ]] ||
        physical_inputs_fail "SOPHIA_PHYSICAL_INPUTS and SOPHIA_PHYSICAL_INPUTS_SHA256 are required before archiving"
    [[ "${SOPHIA_INTEGRATION_XTASK:-}" == /* && -x "${SOPHIA_INTEGRATION_XTASK:-}" ]] ||
        physical_inputs_fail "SOPHIA_INTEGRATION_XTASK must name the absolute prebuilt recipe tool"
    timeout -s KILL 600 nice -n 19 "$SOPHIA_INTEGRATION_XTASK" prepare-physical-inputs verify \
        --out="$SOPHIA_PHYSICAL_INPUTS" --manifest-sha256="$SOPHIA_PHYSICAL_INPUTS_SHA256" ||
        physical_inputs_fail "the prepared inputs changed before archiving"
}

# physical_inputs_bound INTEGRATION-COMMIT: the loaded inputs were prepared
# from this repository's signed INTEGRATION-COMMIT, and the checkout is still
# clean at it (the final HEAD check; the helper proved every staged tree).
physical_inputs_bound() {
    local commit="${1:-}"
    [[ "${PI[SOPHIA_INTEGRATION_COMMIT]:-}" == "$commit" ]] ||
        physical_inputs_fail "the inputs were prepared from another integration commit"
    [[ -z "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=all)" &&
        "$(git -C "$ROOT_DIR" rev-parse HEAD)" == "$commit" ]] ||
        physical_inputs_fail "the integration checkout changed while the inputs were prepared"
}

# physical_inputs_nim_options NAME ROOT: the helper options for one Nim half
# (hagia or narthex) at ROOT's HEAD, with its REVIEWED dependency manifest
# and independently supplied sha256 from SOPHIA_<NAME>_NIM_DEPS and
# SOPHIA_<NAME>_NIM_DEPS_SHA256 (no default). Prints one option per line.
physical_inputs_nim_options() {
    local name="$1" root="$2" upper deps sha commit
    upper="${name^^}"
    deps="SOPHIA_${upper}_NIM_DEPS"
    sha="SOPHIA_${upper}_NIM_DEPS_SHA256"
    [[ "${!deps:-}" == /* && -f "${!deps:-}" ]] ||
        physical_inputs_fail "$deps must name the absolute reviewed dependency manifest (no default)"
    [[ "${!sha:-}" =~ ^[0-9a-f]{64}$ ]] ||
        physical_inputs_fail "$sha must be the manifest's independently supplied sha256"
    commit="$(git -C "$root" rev-parse --verify HEAD)" || physical_inputs_fail "no HEAD in $root"
    printf '%s\n' "--$name=$root" "--$name-commit=$commit" \
        "--$name-nim-deps=${!deps}" "--$name-nim-deps-sha256=${!sha}"
}

# physical_inputs_preflight SOPHIA-BIN SOPHIA-ROOT LOG: the atomic-scanout
# preflight (no DRM master, no modeset) run by the prepared binary, its record
# verified by Sophia's retained verifier from the staged pinned tree. Sophia's
# own tools/atomic_scanout_preflight.sh is never called: it `cargo run`s in the
# tree it lives in. LOG is the caller's private evidence path.
physical_inputs_preflight() {
    local bin="${1:-}" root="${2:-}" log="${3:-}" status
    [[ "$bin" == /* && -x "$bin" && "$root" == /* && -d "$root/tools" && "$log" == /* ]] ||
        physical_inputs_fail "the preflight needs the absolute prepared SOPHIA_BIN, SOPHIA_ROOT and a log path"
    echo "Sophia atomic scanout preflight (prepared binary; no DRM master, no modeset)"
    set +e
    "$bin" atomic-scanout-preflight 2>&1 | tee "$log"
    status="${PIPESTATUS[0]}"
    set -e
    [[ "$status" == 0 ]] || {
        echo "Atomic scanout preflight failed; output left at $log" >&2
        return "$status"
    }
    "$root/tools/verify_atomic_scanout_preflight.sh" "$log" || {
        echo "Atomic scanout preflight did not find a smoke-ready host; output left at $log" >&2
        return 1
    }
}
