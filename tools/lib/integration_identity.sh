# shellcheck shell=bash
# This repository's identity in new physical archives (the finding-A ruling).
#
# A CURRENT archive declares `integration_schema=1` in its manifest, names
# `integration_commit=<40 lowercase hex>` exactly once, and carries the raw
# commit object as `integration.commit`, listed in its SHA256SUMS. The commit
# is the one the runner bound (SOPHIA_INTEGRATION_COMMIT) and must be a signed
# commit of the explicit integration source the runner used
# (SOPHIA_INTEGRATION_SOURCE, an absolute git repository; no default).
#
# An archive without `integration_schema` predates this binding. It verifies
# ONLY in the explicit legacy mode (`--legacy`), which reports "integration
# identity unavailable"; legacy is never inferred from a missing field, and a
# legacy archive that carries any integration field is refused as inconsistent.

integration_source_repo() {
    local repo="${SOPHIA_INTEGRATION_SOURCE:-}"
    if [[ "$repo" != /* ]] || ! git -C "$repo" rev-parse --git-dir >/dev/null 2>&1; then
        echo "SOPHIA_INTEGRATION_SOURCE must name the absolute integration repository (no default)." >&2
        return 2
    fi
    printf '%s\n' "$repo"
}

# integration_identity_bind RUN_DIR: writes RUN_DIR/integration.commit and
# prints the bound commit.
integration_identity_bind() {
    local run_dir="$1" commit="${SOPHIA_INTEGRATION_COMMIT:-}" repo
    repo="$(integration_source_repo)" || return 2
    [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || {
        echo "SOPHIA_INTEGRATION_COMMIT must name the runner's integration commit (40 lowercase hex)." >&2
        return 1
    }
    git -C "$repo" cat-file -e "$commit^{commit}" 2>/dev/null || {
        echo "integration commit $commit is not in the integration source $repo" >&2
        return 1
    }
    git -C "$repo" verify-commit "$commit" >/dev/null 2>&1 || {
        echo "integration commit $commit lacks a valid signature" >&2
        return 1
    }
    git -C "$repo" cat-file commit "$commit" >"$run_dir/integration.commit"
    chmod 600 "$run_dir/integration.commit"
    printf '%s\n' "$commit"
}

# integration_identity_verify RUN MODE (current|legacy): verifies the archive's
# integration identity, or reports its absence in legacy mode.
integration_identity_verify() {
    local run="$1" mode="$2" manifest="$1/manifest" schema commit repo raw_commit
    schema="$(sed -n 's/^integration_schema=//p' "$manifest")"
    if [[ "$(grep -c '^integration_schema=' "$manifest")" == 0 ]]; then
        if [[ "$mode" != legacy ]]; then
            echo "archive predates the integration binding (no integration_schema); verify it explicitly with --legacy: $run" >&2
            return 1
        fi
        if grep -q '^integration_commit=' "$manifest" || [[ -e "$run/integration.commit" ]]; then
            echo "legacy archive carries integration fields without integration_schema: $run" >&2
            return 1
        fi
        echo "integration identity unavailable: legacy archive $run"
        return 0
    fi
    [[ "$(grep -c '^integration_schema=' "$manifest")" == 1 && "$schema" == 1 ]] || {
        echo "archive declares an unsupported integration_schema: $run" >&2
        return 1
    }
    [[ "$(grep -c '^integration_commit=' "$manifest")" == 1 ]] || {
        echo "archive has a missing or repeated integration_commit: $run" >&2
        return 1
    }
    commit="$(sed -n 's/^integration_commit=//p' "$manifest")"
    [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || {
        echo "archive has a malformed integration_commit: $run" >&2
        return 1
    }
    grep -Eq '^[0-9a-f]{64}  integration\.commit$' "$run/SHA256SUMS" || {
        echo "archive does not seal its integration commit object: $run" >&2
        return 1
    }
    repo="$(integration_source_repo)" || return 2
    raw_commit="$(git -C "$repo" hash-object -t commit --stdin <"$run/integration.commit")"
    [[ "$raw_commit" == "$commit" ]] || {
        echo "archive integration_commit does not match its commit object: $run" >&2
        return 1
    }
    git -C "$repo" cat-file -e "$commit^{commit}" 2>/dev/null || {
        echo "archive integration_commit is not in the integration source: $run" >&2
        return 1
    }
    git -C "$repo" verify-commit "$commit" >/dev/null 2>&1 || {
        echo "archive integration_commit lacks a valid signature: $run" >&2
        return 1
    }
}

# integration_verify_mode "$@": sets INTEGRATION_MODE (current|legacy) and
# INTEGRATION_ARGS (the remaining arguments).
integration_verify_mode() {
    INTEGRATION_MODE=current
    if [[ "${1:-}" == --legacy ]]; then
        INTEGRATION_MODE=legacy
        shift
    fi
    INTEGRATION_ARGS=("$@")
}
