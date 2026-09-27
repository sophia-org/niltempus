# shellcheck shell=bash
# Activation history for an install prefix, sourced by the activator and by
# rollback. Callers set PREFIX.
#
# $PREFIX/activated-releases holds one line per activated release:
#   RELEASE_ID MANIFEST_SHA256 SHA256SUMS_SHA256
# An entry exempts only that release_id with exactly those contents from the
# current packaged-policy verifier. The same ID with different contents is
# refused. Links grant nothing, except in a one-time bootstrap: when a prefix
# has no ledger yet, the existing current and previous targets are recorded
# once, and from then on only ledger entries count.

activation_ledger_path() {
    printf '%s/activated-releases\n' "$PREFIX"
}

# The release_id recorded in a release directory's manifest.
activation_release_id() {
    awk -F= '$1 == "release_id" { print $2; exit }' "$1/manifest"
}

# "MANIFEST_SHA256 SHA256SUMS_SHA256" for a release directory.
activation_release_digests() {
    local dir="$1" name digests=()
    for name in manifest SHA256SUMS; do
        [[ -f "$dir/$name" && ! -L "$dir/$name" ]] || {
            echo "Release has no regular $name: $dir" >&2
            return 1
        }
        digests+=("$(sha256sum -- "$dir/$name" | awk '{print $1}')")
    done
    printf '%s %s\n' "${digests[0]}" "${digests[1]}"
}

# Replace the ledger atomically with the given text.
activation_ledger_write() {
    local ledger temp
    ledger="$(activation_ledger_path)"
    temp="$PREFIX/.activated-releases.$$"
    printf '%s' "$1" >"$temp"
    chmod 644 "$temp"
    mv -Tf "$temp" "$ledger"
}

# One time only: with no ledger, record the current and previous targets.
activation_ledger_bootstrap() {
    local ledger releases link target id digests lines=""
    ledger="$(activation_ledger_path)"
    [[ ! -e "$ledger" && ! -L "$ledger" ]] || return 0
    releases="$(readlink -f "$PREFIX/releases" 2>/dev/null || true)"
    [[ -n "$releases" ]] || releases="$PREFIX/releases"
    for link in current previous; do
        target="$(readlink -f "$PREFIX/$link" 2>/dev/null || true)"
        [[ -n "$target" && -d "$target" && "$target" == "$releases/"* ]] || continue
        id="$(activation_release_id "$target")"
        [[ "$id" =~ ^[0-9A-Za-z._-]+$ && "$target" == "$releases/$id" ]] || continue
        digests="$(activation_release_digests "$target")" || continue
        [[ $'\n'"$lines" != *$'\n'"$id "* ]] || continue
        lines+="$id $digests"$'\n'
    done
    activation_ledger_write "$lines"
    echo "Recorded activation history for $PREFIX (one-time migration)." >&2
}

# The recorded "MANIFEST_SHA256 SHA256SUMS_SHA256" for an ID, or nothing.
activation_ledger_entry() {
    local ledger
    ledger="$(activation_ledger_path)"
    [[ -f "$ledger" ]] || return 0
    awk -v id="$1" '
        NF != 3 { bad = 1 }
        $1 == id { print $2, $3; count++ }
        END { exit (bad || count > 1) ? 1 : 0 }
    ' "$ledger"
}

# Print "recorded" when ID is in the ledger with exactly DIR's current
# contents, or "new" when it is not recorded. Refuse (status 1) a recorded ID
# whose contents changed, or a malformed ledger.
activation_ledger_status() {
    local id="$1" dir="$2" recorded actual
    recorded="$(activation_ledger_entry "$id")" || {
        echo "Activation ledger is malformed or repeats $id: $(activation_ledger_path)" >&2
        return 1
    }
    if [[ -z "$recorded" ]]; then
        echo new
        return 0
    fi
    actual="$(activation_release_digests "$dir")" || return 1
    [[ "$recorded" == "$actual" ]] || {
        echo "Release $id changed since it was activated (manifest or SHA256SUMS digest differs); refusing it." >&2
        return 1
    }
    echo recorded
}

# Record ID with DIGESTS ("MANIFEST_SHA256 SHA256SUMS_SHA256") if absent.
activation_ledger_record() {
    local id="$1" digests="$2" ledger existing=""
    ledger="$(activation_ledger_path)"
    [[ ! -f "$ledger" ]] || existing="$(cat -- "$ledger")"
    [[ -z "$existing" ]] || existing+=$'\n'
    [[ $'\n'"$existing" != *$'\n'"$id "* ]] || return 0
    activation_ledger_write "$existing$id $digests"$'\n'
}
