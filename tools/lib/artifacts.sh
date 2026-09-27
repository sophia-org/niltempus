# shellcheck shell=bash
# Explicit inputs of the attended gates: the pinned Sophia source tree, a
# private build directory, and prepared product artifacts.
#
# Sourced by tools/run_current_lom_panel_gate_tty4.sh and
# tools/lom_gpu_content_hardware_proof.sh. Requires ROOT_DIR (this repository).
#
# Two separate claims are checked, and worded separately in every error:
#  * SOURCE AUTHORIZATION: a revision is signed by a key Git accepts
#    (`git verify-commit`, status G). For products this happened once, at
#    preparation (cargo xtask prepare-*-artifact); for Sophia it is checked
#    here, on the explicit checkout, for exactly the pinned revision.
#  * ARTIFACT BINDING: the files used are exactly the ones the operator
#    expects. The artifact manifest is NOT signed, so a manifest digest alone
#    binds nothing; every executed binary and every configuration must match
#    an operator-supplied SHA-256, and the raw commit object must hash to the
#    operator-supplied commit and name the recorded tree.

artifact_fail() {
    echo "artifact binding: $*" >&2
    exit 2
}

source_fail() {
    echo "source authorization: $*" >&2
    exit 2
}

artifact_hex() { # VALUE LENGTH
    [[ ${#1} -eq $2 && "$1" =~ ^[0-9a-f]+$ ]]
}

artifact_sha256() {
    sha256sum -- "$1" | cut -d' ' -f1
}

# The one Sophia revision pins/sophia.toml names.
pinned_sophia_rev() {
    local rev
    rev=$(sed -n 's/^rev = "\([0-9a-f]*\)"$/\1/p' "$ROOT_DIR/pins/sophia.toml")
    artifact_hex "$rev" 40 || source_fail "pins/sophia.toml has no full revision"
    printf '%s\n' "$rev"
}

# check_build_dir DIR: an explicit absolute private directory outside every
# source tree. It is created 0700 when missing.
check_build_dir() {
    local dir=$1 tree resolved
    [[ "$dir" == /* ]] || { echo "SOPHIA_GATE_BUILD_DIR must be absolute" >&2; exit 2; }
    resolved=$(realpath -m -- "$dir")
    for tree in "$ROOT_DIR" "${SOPHIA_SOURCE:-}"; do
        [[ -n "$tree" ]] || continue
        tree=$(realpath -m -- "$tree")
        if [[ "$resolved" == "$tree" || "$resolved" == "$tree"/* || "$tree" == "$resolved"/* ]]; then
            echo "SOPHIA_GATE_BUILD_DIR must be outside every source tree: $dir" >&2
            exit 2
        fi
    done
    mkdir -p -m 700 -- "$resolved"
    [[ -O "$resolved" && ! -L "$dir" ]] || { echo "SOPHIA_GATE_BUILD_DIR must be owned by this user" >&2; exit 2; }
    chmod 700 -- "$resolved"
}

# check_sophia_source DIR: an explicit absolute clean checkout whose HEAD is
# exactly the pinned revision, and that revision is signed.
check_sophia_source() {
    local dir=$1 rev head
    rev=$(pinned_sophia_rev)
    [[ "$dir" == /* && -d "$dir" ]] || source_fail "SOPHIA_SOURCE must be an absolute Sophia checkout"
    [[ -z "$(git -C "$dir" status --short)" ]] || source_fail "Sophia checkout must be clean"
    head=$(git -C "$dir" rev-parse --verify HEAD) || source_fail "Sophia checkout has no HEAD"
    [[ "$head" == "$rev" ]] || source_fail "Sophia HEAD $head is not the pinned revision $rev"
    git -C "$dir" verify-commit "$rev" >/dev/null 2>&1 || source_fail "Sophia $rev is not a good signed commit"
}

# stage_sophia_tree SOURCE DEST: the exact pinned tree, extracted with
# `git archive` into DEST (inside the private build directory) and proven
# equal to the revision's tree. Every Sophia file the gates read or execute
# comes from here, never from the operator's working tree, so the pin covers
# the complete transitive input set by construction.
stage_sophia_tree() {
    local source=$1 dest=$2 rev
    rev=$(pinned_sophia_rev)
    [[ ! -e "$dest" ]] || chmod -R u+w -- "$dest"
    rm -rf -- "$dest"
    mkdir -m 700 -- "$dest"
    git -C "$source" archive --format=tar "$rev" | tar -x --no-same-owner -C "$dest"
    verify_staged_tree "$source" "$dest"
    # Read-only from here on: builds write only to the private target.
    chmod -R a-w -- "$dest"
}

# verify_staged_tree SOURCE DEST: DEST hashes to exactly the pinned revision's
# tree. Re-run before and after each stage that executes staged files.
verify_staged_tree() {
    local source=$1 dest=$2 rev expected actual index
    rev=$(pinned_sophia_rev)
    expected=$(git -C "$source" rev-parse --verify "$rev^{tree}")
    index=$(mktemp -d)
    git init -q "$index"
    git --git-dir="$index/.git" --work-tree="$dest" -c core.autocrlf=false -c core.fileMode=true \
        add -A -f
    actual=$(git --git-dir="$index/.git" write-tree)
    rm -rf -- "$index"
    [[ "$actual" == "$expected" ]] || source_fail "staged Sophia tree $actual is not $rev's tree $expected"
}

# load_artifact KIND DIR COMMIT BINARY_SHA256 DEST [CONFIG_SHA256 DEST_CONFIG]
# KIND is bemenu, lom, provlita or hagia. The configuration pair is required
# exactly when the product ships a configuration (lom, provlita).
load_artifact() {
    local kind=$1 dir=$2 commit=$3 expected=$4 dest=$5 expected_config=${6:-} dest_config=${7:-}
    local binary manifest keys
    case "$kind" in
    bemenu)
        binary=bemenu-sophia
        manifest=bemenu-artifact.manifest
        keys="schema binary binary_sha256 source_commit source_tree signature_status signer_fingerprint sdk_revision sdk_manifest_sha256"
        ;;
    lom | provlita | hagia)
        binary=$kind
        manifest=product-artifact.manifest
        keys="schema product binary binary_sha256 source_commit source_tree signature_status signer_fingerprint config config_sha256"
        ;;
    *) artifact_fail "unknown kind $kind" ;;
    esac
    [[ "$dir" == /* && -d "$dir" && ! -L "$dir" ]] || artifact_fail "$kind artifact must be an absolute directory: $dir"
    artifact_hex "$commit" 40 || artifact_fail "$kind commit must be 40 lowercase hex"
    artifact_hex "$expected" 64 || artifact_fail "$kind expected binary SHA-256 must be 64 lowercase hex"
    [[ -f "$dir/$manifest" && ! -L "$dir/$manifest" ]] || artifact_fail "$kind manifest missing"

    local -A value=()
    local order=() line key
    while IFS= read -r line; do
        [[ "$line" == *=* ]] || artifact_fail "$kind manifest line: $line"
        key=${line%%=*}
        [[ -z "${value[$key]+set}" ]] || artifact_fail "$kind manifest repeats $key"
        value[$key]=${line#*=}
        order+=("$key")
    done <"$dir/$manifest"
    [[ "${order[*]}" == "$keys" ]] || artifact_fail "$kind manifest keys: ${order[*]}"
    [[ "${value[schema]}" == 1 ]] || artifact_fail "$kind manifest schema"
    [[ "$kind" == bemenu || "${value[product]}" == "$kind" ]] || artifact_fail "$kind manifest product"
    [[ "${value[binary]}" == "$binary" ]] || artifact_fail "$kind binary name"
    [[ "${value[source_commit]}" == "$commit" ]] || artifact_fail "$kind commit mismatch"
    # The preparation's authorization record. Unsigned: it is carried into the
    # evidence, never trusted to bind a file.
    [[ "${value[signature_status]}" == G ]] || artifact_fail "$kind preparation recorded no source authorization"
    [[ -n "${value[signer_fingerprint]}" && "${value[signer_fingerprint]}" =~ ^[0-9A-Fa-f]+$ ]] \
        || artifact_fail "$kind signer fingerprint"
    artifact_hex "${value[source_tree]}" 40 || artifact_fail "$kind manifest tree"
    [[ "${value[binary_sha256]}" == "$expected" ]] || artifact_fail "$kind manifest binary SHA-256 is not the expected one"
    if [[ "$kind" == bemenu ]]; then
        local pinned_sdk pinned_revision
        pinned_sdk="$ROOT_DIR/pins/c-desktop-sdk/manifest.json"
        pinned_revision=$(sed -n 's/^  "revision": "\([0-9a-f]*\)",$/\1/p' "$pinned_sdk")
        artifact_hex "$pinned_revision" 40 || artifact_fail "pins/c-desktop-sdk has no revision"
        [[ "${value[sdk_revision]}" == "$pinned_revision" ]] \
            || artifact_fail "bemenu SDK revision ${value[sdk_revision]} is not the pinned $pinned_revision"
        [[ "${value[sdk_manifest_sha256]}" == "$(artifact_sha256 "$pinned_sdk")" ]] \
            || artifact_fail "bemenu SDK manifest differs from pins/c-desktop-sdk"
    fi

    # The raw signed object is exactly the expected revision and tree.
    local raw=$dir/source.commit
    [[ -f "$raw" && ! -L "$raw" ]] || artifact_fail "$kind source.commit missing"
    [[ "$(git hash-object --stdin -t commit <"$raw")" == "$commit" ]] \
        || artifact_fail "$kind source.commit does not hash to $commit"
    [[ "$(head -n 1 "$raw")" == "tree ${value[source_tree]}" ]] || artifact_fail "$kind commit object tree"
    grep -q '^gpgsig ' "$raw" || artifact_fail "$kind commit object carries no signature"

    # The executed file is a private copy hashed after copying.
    [[ -f "$dir/$binary" && ! -L "$dir/$binary" ]] || artifact_fail "$kind binary is not a regular file"
    [[ ! -e "$dest" ]] || artifact_fail "$kind destination exists: $dest"
    cp -- "$dir/$binary" "$dest"
    chmod 700 "$dest"
    [[ "$(artifact_sha256 "$dest")" == "$expected" ]] || artifact_fail "$kind binary SHA-256 is not the expected one"

    if [[ "$kind" == lom || "$kind" == provlita ]]; then
        artifact_hex "$expected_config" 64 || artifact_fail "$kind expected configuration SHA-256 must be 64 lowercase hex"
        [[ "${value[config]}" == config.kdl && "${value[config_sha256]}" == "$expected_config" ]] \
            || artifact_fail "$kind manifest configuration is not the expected one"
        [[ -n "$dest_config" && ! -e "$dest_config" ]] || artifact_fail "$kind configuration destination"
        [[ -f "$dir/config.kdl" && ! -L "$dir/config.kdl" ]] || artifact_fail "$kind configuration missing"
        cp -- "$dir/config.kdl" "$dest_config"
        chmod 600 "$dest_config"
        [[ "$(artifact_sha256 "$dest_config")" == "$expected_config" ]] \
            || artifact_fail "$kind configuration SHA-256 is not the expected one"
    elif [[ "$kind" == hagia ]]; then
        [[ "${value[config]}" == none && "${value[config_sha256]}" == none && -z "$expected_config" ]] \
            || artifact_fail "hagia configuration record"
    fi
}
