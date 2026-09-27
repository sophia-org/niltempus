#!/bin/sh
# Provision the pinned Sophia crates and every crates.io dependency into
# .provision/ so that all later gates build with
#   cargo --config .provision/cargo-config.toml <command> --offline --locked
#
# usage: sh tools/provision.sh [--source ABSOLUTE-SOPHIA-REPO] [--generate-lockfile]
#
# Default route: fetch the pinned revision from its canonical public URL
# (pins/sophia.toml). Optional route: --source redirects that URL to an
# explicit local clone for the cargo and git children of this script only,
# through GIT_CONFIG_COUNT/KEY/VALUE in their environment. No Git or Cargo
# configuration file is written anywhere.
#
# --generate-lockfile creates Cargo.lock when it does not exist yet (first
# provisioning only); otherwise Cargo.lock is required and used --locked.
#
# The result is accepted only after `cargo xtask check-pins` passes against
# the vendored tree (and `audit-pins` when a local clone was named).
# Until then no .provision/cargo-config.toml exists, so no gate can build.
set -eu

usage() {
    echo "usage: sh tools/provision.sh [--source ABSOLUTE-SOPHIA-REPO] [--generate-lockfile]" >&2
    exit 2
}

source_repo=
generate=
while [ $# -gt 0 ]; do
    case $1 in
    --source)
        [ $# -ge 2 ] || usage
        source_repo=$2
        shift 2
        ;;
    --generate-lockfile)
        generate=1
        shift
        ;;
    *) usage ;;
    esac
done

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$repo"

pin() {
    value=$(sed -n "s/^$1 = \"\\([^\"]*\\)\"\$/\\1/p" pins/sophia.toml)
    [ -n "$value" ] || {
        echo "provision: pins/sophia.toml has no $1" >&2
        exit 1
    }
    printf '%s\n' "$value"
}
url=$(pin url)
rev=$(pin rev)

: "${CARGO_BUILD_JOBS:=2}"
export CARGO_BUILD_JOBS

if [ -n "$source_repo" ]; then
    case $source_repo in
    /*) ;;
    *)
        echo "provision: --source must be absolute: $source_repo" >&2
        exit 2
        ;;
    esac
    resolved=$(git -C "$source_repo" rev-parse --verify --end-of-options "$rev^{commit}")
    [ "$resolved" = "$rev" ] || {
        echo "provision: $source_repo does not hold $rev" >&2
        exit 1
    }
    run_cargo() {
        env CARGO_NET_GIT_FETCH_WITH_CLI=true \
            GIT_CONFIG_COUNT=2 \
            GIT_CONFIG_KEY_0="url.file://$source_repo.insteadOf" \
            GIT_CONFIG_VALUE_0="$url" \
            GIT_CONFIG_KEY_1=uploadpack.allowReachableSHA1InWant \
            GIT_CONFIG_VALUE_1=true \
            nice -n 19 cargo "$@"
    }
else
    run_cargo() {
        nice -n 19 cargo "$@"
    }
fi

rm -f .provision/cargo-config.toml .provision/cargo-config.toml.new
trap 'rm -f "$repo/.provision/cargo-config.toml.new"' EXIT
rm -rf .provision/vendor
mkdir -p .provision

if [ -n "$generate" ]; then
    if [ -e Cargo.lock ]; then
        echo "provision: Cargo.lock exists; refusing to regenerate it" >&2
        exit 1
    fi
    run_cargo generate-lockfile
elif [ ! -e Cargo.lock ]; then
    echo "provision: Cargo.lock is missing; pass --generate-lockfile once" >&2
    exit 1
fi

run_cargo vendor --locked --versioned-dirs "$repo/.provision/vendor" \
    >.provision/cargo-config.toml.new

# Accept the vendored tree only when every pin agrees, offline.
check() {
    nice -n 19 cargo --config .provision/cargo-config.toml.new \
        run --offline --locked --package xtask -- "$@"
}
check check-pins
if [ -n "$source_repo" ]; then
    check audit-pins "$source_repo"
fi
mv .provision/cargo-config.toml.new .provision/cargo-config.toml
echo "provision status=pass sophia=$rev config=.provision/cargo-config.toml"
