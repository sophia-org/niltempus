# shellcheck shell=bash
# Fail-closed intake of prepared product artifacts for the attended gates.
#
# Sourced by tools/run_current_lom_panel_gate_tty4.sh and
# tools/lom_gpu_content_hardware_proof.sh. An artifact directory is produced by
#   cargo xtask prepare-bemenu-artifact  SOURCE SIGNED-COMMIT NEW-DIR   (bemenu)
#   cargo xtask prepare-product-artifact lom|provlita|hagia SOURCE SIGNED-COMMIT NEW-DIR
# Signer authorization happened at preparation. This checks BINDING only: the
# manifest keys and values, the raw commit object hashing to the operator's
# expected commit and naming the recorded tree, and the private copies of the
# binary (and configuration) hashing to the recorded digests before use.
# Requires ROOT_DIR (this repository) to be set by the caller.

artifact_fail() {
    echo "artifact: $*" >&2
    exit 2
}

artifact_hex() { # VALUE LENGTH
    [[ ${#1} -eq $2 && "$1" =~ ^[0-9a-f]+$ ]]
}

artifact_sha256() {
    sha256sum -- "$1" | cut -d' ' -f1
}

# load_artifact KIND DIR COMMIT DEST_BINARY [DEST_CONFIG]
# KIND is bemenu, lom, provlita or hagia. DEST_CONFIG is required exactly when
# the product ships a configuration (lom, provlita).
load_artifact() {
    local kind=$1 dir=$2 commit=$3 dest=$4 dest_config=${5:-}
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
    [[ "${value[signature_status]}" == G ]] || artifact_fail "$kind preparation recorded no good signature"
    [[ -n "${value[signer_fingerprint]}" && "${value[signer_fingerprint]}" =~ ^[0-9A-Fa-f]+$ ]] \
        || artifact_fail "$kind signer fingerprint"
    artifact_hex "${value[source_tree]}" 40 && artifact_hex "${value[binary_sha256]}" 64 \
        || artifact_fail "$kind manifest digests"
    if [[ "$kind" == bemenu ]]; then
        artifact_hex "${value[sdk_revision]}" 40 || artifact_fail "bemenu SDK revision"
        [[ "${value[sdk_manifest_sha256]}" == "$(artifact_sha256 "$ROOT_DIR/pins/c-desktop-sdk/manifest.json")" ]] \
            || artifact_fail "bemenu SDK manifest differs from pins/c-desktop-sdk"
    fi

    # Binding: the raw signed object is exactly the expected revision and tree.
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
    [[ "$(artifact_sha256 "$dest")" == "${value[binary_sha256]}" ]] || artifact_fail "$kind binary SHA-256 mismatch"

    if [[ "$kind" == lom || "$kind" == provlita ]]; then
        [[ "${value[config]}" == config.kdl ]] && artifact_hex "${value[config_sha256]}" 64 \
            || artifact_fail "$kind configuration record"
        [[ -n "$dest_config" && ! -e "$dest_config" ]] || artifact_fail "$kind configuration destination"
        [[ -f "$dir/config.kdl" && ! -L "$dir/config.kdl" ]] || artifact_fail "$kind configuration missing"
        cp -- "$dir/config.kdl" "$dest_config"
        chmod 600 "$dest_config"
        [[ "$(artifact_sha256 "$dest_config")" == "${value[config_sha256]}" ]] \
            || artifact_fail "$kind configuration SHA-256 mismatch"
    elif [[ "$kind" == hagia ]]; then
        [[ "${value[config]}" == none && "${value[config_sha256]}" == none && -z "$dest_config" ]] \
            || artifact_fail "hagia configuration record"
    fi
}

# check_sophia_source DIR: an explicit, absolute, clean Sophia checkout whose
# HEAD is signed, holding the shared files at their pinned digests.
check_sophia_source() {
    local dir=$1 digest path
    [[ "$dir" == /* && -d "$dir" ]] || artifact_fail "SOPHIA_SOURCE must be an absolute Sophia checkout"
    [[ -z "$(git -C "$dir" status --short)" ]] || artifact_fail "Sophia source must be clean"
    git -C "$dir" verify-commit HEAD >/dev/null || artifact_fail "Sophia HEAD is not a good signed commit"
    while read -r digest path; do
        [[ -f "$dir/$path" && ! -L "$dir/$path" ]] || artifact_fail "Sophia $path missing"
        [[ "$(artifact_sha256 "$dir/$path")" == "$digest" ]] \
            || artifact_fail "Sophia $path differs from pins/sophia-shared.sha256"
    done <"$ROOT_DIR/pins/sophia-shared.sha256"
}
