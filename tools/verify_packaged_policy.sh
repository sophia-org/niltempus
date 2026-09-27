#!/usr/bin/env bash
# Provenance: moved from Sophia tools/verify_packaged_policy.sh at a6edbbcad02ad9e9bb4c790e7cfd714cf333d30b (original copy; source unchanged at the pin de776c68) (Sophia rule 13).
set -euo pipefail

release="${1:-}"
[[ -n "$release" && -d "$release" ]] || {
    echo "usage: tools/verify_packaged_policy.sh RELEASE_DIR" >&2
    exit 1
}
release="$(cd "$release" && pwd)"
manifest="$release/manifest"
[[ -f "$manifest" ]] || {
    echo "Packaged policy manifest is missing: $manifest" >&2
    exit 1
}

field() {
    sed -n "s/^$1=//p" "$manifest" | head -n 1
}
require_sha256() {
    local name="$1" actual="$2" expected
    expected="$(field "$name")"
    [[ "$expected" =~ ^[0-9a-f]{64}$ && "$actual" == "$expected" ]] || {
        echo "Packaged policy has an invalid $name." >&2
        exit 1
    }
}

field_count() {
    grep -c "^$1=" "$manifest" || true
}

# Schema 7 binds Hagia's vendored C SDK. A schema-6 release is refused as a
# candidate here; historical schema-6 releases are read only by the Go
# verifier's legacy path in Sophia, never accepted by this repository.
manifest_schema="$(field schema)"
[[ "$manifest_schema" == 7 && "$(field_count schema)" == 1 ]] || {
    echo "Packaged policy requires native-only release manifest schema 7." >&2
    exit 1
}
for legacy_field in \
    xmonad_version xmonad_source_version xmonad_contrib_source_version \
    xmonad_config_sha256 xmonad_cabal_sha256 xmonad_project_sha256 \
    xmonad_core_config_sha256 xmonad_desktop_profile_sha256 \
    xmonad_binary_sha256 xmobar_version xmobar_source_commit \
    xmobar_config_sha256 xmobar_binary_sha256; do
    [[ -z "$(field "$legacy_field")" ]] || {
        echo "Native-only package contains legacy field: $legacy_field" >&2
        exit 1
    }
done
for legacy_path in \
    "$release/target/release/sophia-x11-wm-bridge" \
    "$release/target/release/xmonad" \
    "$release/target/release/xmobar" \
    "$release/share/sophia-policy/xmonad"; do
    [[ ! -e "$legacy_path" ]] || {
        echo "Native-only package contains a legacy policy artifact: $legacy_path" >&2
        exit 1
    }
done

case "$(field hagia_included)" in
    true)
        hagia="$release/target/release/hagia"
        hagia_shell="$release/target/release/narthex"
        for executable in "$hagia" "$hagia_shell"; do
            [[ -x "$executable" ]] || {
                echo "Packaged Hagia executable is missing: $executable" >&2
                exit 1
            }
        done
        require_sha256 hagia_binary_sha256 \
            "$(sha256sum "$hagia" | awk '{print $1}')"
        require_sha256 hagia_shell_binary_sha256 \
            "$(sha256sum "$hagia_shell" | awk '{print $1}')"
        hagia_profile="$release/share/sophia-policy/hagia/default.kdl"
        [[ -f "$hagia_profile" && ! -L "$hagia_profile" ]] || {
            echo "Packaged Hagia default profile is missing: $hagia_profile" >&2
            exit 1
        }
        [[ "$(field hagia_source_commit)" =~ ^[0-9a-f]{40}$ ]] || {
            echo "Packaged Hagia source commit is invalid." >&2
            exit 1
        }
        require_sha256 hagia_default_profile_sha256 \
            "$(sha256sum "$hagia_profile" | awk '{print $1}')"
        # Hagia's vendored C SDK: both fields present once and well formed,
        # the sealed manifest hashes to the recorded digest and names the
        # recorded revision.
        sdk_manifest="$release/share/sophia-policy/hagia/c-sdk.manifest.json"
        [[ -f "$sdk_manifest" && ! -L "$sdk_manifest" ]] || {
            echo "Packaged Hagia C SDK manifest is missing: $sdk_manifest" >&2
            exit 1
        }
        for sdk_field in hagia_c_sdk_revision hagia_c_sdk_manifest_sha256; do
            [[ "$(field_count "$sdk_field")" == 1 ]] || {
                echo "Packaged policy needs exactly one $sdk_field." >&2
                exit 1
            }
        done
        sdk_revision="$(field hagia_c_sdk_revision)"
        [[ "$sdk_revision" =~ ^[0-9a-f]{40}$ ]] || {
            echo "Packaged Hagia C SDK revision is invalid." >&2
            exit 1
        }
        require_sha256 hagia_c_sdk_manifest_sha256 \
            "$(sha256sum "$sdk_manifest" | awk '{print $1}')"
        named_revisions="$(grep -oE '"revision"[[:space:]]*:[[:space:]]*"[0-9a-f]{40}"' \
            "$sdk_manifest" | grep -oE '[0-9a-f]{40}' || true)"
        [[ "$named_revisions" == "$sdk_revision" ]] || {
            echo "Packaged Hagia C SDK manifest does not name revision $sdk_revision." >&2
            exit 1
        }
        "$hagia" config check --config="$hagia_profile" >/dev/null
        "$release/target/release/sophia" config check \
            --desktop-profile="$hagia_profile" >/dev/null
        ;;
    false)
        for absent in \
            "$release/target/release/hagia" \
            "$release/target/release/narthex" \
            "$release/share/sophia-policy/hagia/default.kdl" \
            "$release/share/sophia-policy/hagia/c-sdk.manifest.json"; do
            [[ ! -e "$absent" ]] || {
                echo "Package declares hagia_included=false but contains: $absent" >&2
                exit 1
            }
        done
        for sdk_field in hagia_c_sdk_revision hagia_c_sdk_manifest_sha256; do
            [[ "$(field_count "$sdk_field")" == 0 ]] || {
                echo "Package declares hagia_included=false but records $sdk_field." >&2
                exit 1
            }
        done
        ;;
    *)
        echo "Packaged policy has an invalid hagia_included field." >&2
        exit 1
        ;;
esac

echo "Native packaged policy executables verified."
