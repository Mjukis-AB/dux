#!/bin/bash

set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly SOURCE_BINARY="$REPO_ROOT/dux-macos/Generated/dux-cli-bundled"
readonly SOURCE_METADATA="$REPO_ROOT/dux-macos/Generated/dux-cli-bundled-metadata.json"
readonly METADATA_FINALIZER="$SCRIPT_DIR/finalize-bundled-cli-metadata.py"
readonly DEPLOYMENT_CHECK="$REPO_ROOT/scripts/check_macos_deployment_target.sh"
readonly DEVELOPMENT_SIGNING_IDENTIFIER="se.mjukis.dux.spike.cli.debug"

die() {
    echo "error: $*" >&2
    exit 1
}

assert_exact_universal() {
    local binary="$1"
    local architectures
    architectures="$(lipo -archs "$binary")"
    case "$architectures" in
        "x86_64 arm64"|"arm64 x86_64") ;;
        *) die "'$binary' must contain exactly arm64 and x86_64 (found: $architectures)" ;;
    esac
}

verify_pair() {
    local binary="$1"
    local metadata="$2"
    [[ -f "$binary" && ! -L "$binary" && -x "$binary" ]] \
        || die "bundled CLI is missing, linked, or not executable: $binary"
    [[ -f "$metadata" && ! -L "$metadata" ]] \
        || die "bundled CLI metadata is missing or linked: $metadata"
    assert_exact_universal "$binary"
    bash "$DEPLOYMENT_CHECK" "$binary"
    codesign --verify --all-architectures --strict --verbose=2 "$binary"

    local details
    details="$(codesign --display --verbose=4 "$binary" 2>&1)"
    grep -Fxq "Identifier=$DEVELOPMENT_SIGNING_IDENTIFIER" <<<"$details" \
        || die "bundled CLI has the wrong development signing identifier"
    grep -Fxq "Signature=adhoc" <<<"$details" \
        || die "bundled CLI development signature is not ad-hoc"
    grep -Fxq "TeamIdentifier=not set" <<<"$details" \
        || die "bundled CLI development signature unexpectedly has a Team ID"
    grep -Eq 'flags=.*\([^)]*runtime' <<<"$details" \
        || die "bundled CLI development signature does not enable Hardened Runtime"

    local version
    version="$(
        cargo metadata \
            --manifest-path "$REPO_ROOT/Cargo.toml" \
            --locked \
            --no-deps \
            --format-version 1 \
            | python3 -c 'import json,sys; values=[p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "dux-cli"]; sys.stdout.write(values[0] if len(values) == 1 else "")'
    )"
    [[ -n "$version" ]] || die "could not resolve one dux-cli package version"
    local sha256
    local raw_metadata
    local metadata_stderr
    sha256="$(shasum -a 256 "$binary" | awk '{print $1}')"
    raw_metadata="$(mktemp "$staging_root/cli-raw-metadata.XXXXXX")"
    metadata_stderr="$(mktemp "$staging_root/cli-metadata-stderr.XXXXXX")"
    # DUX-DESTRUCTIVE: allow=build-bundled-cli-embed-metadata-inspect -- executes only the copied, signature-verified private CLI stage with its fixed inspection-only hidden command
    "$binary" __bundle-metadata >"$raw_metadata" 2>"$metadata_stderr" \
        || die "bundled CLI metadata command failed"
    [[ ! -s "$metadata_stderr" ]] \
        || die "bundled CLI metadata command wrote unexpected stderr"
    cmp "$metadata" <(
        python3 "$METADATA_FINALIZER" \
            --mode verify \
            --input "$metadata" \
            --expected-version "$version" \
            --sha256 "$sha256"
    ) >/dev/null || die "bundled CLI manifest is malformed or does not match its bytes"
    cmp "$metadata" <(
        python3 "$METADATA_FINALIZER" \
            --input "$raw_metadata" \
            --expected-version "$version" \
            --sha256 "$sha256"
    ) >/dev/null || die "bundled CLI manifest does not match trusted executable metadata"
}

main() {
    [[ "$#" -eq 0 ]] || die "this embedder accepts no arguments"
    [[ "${WRAPPER_NAME:-}" == "DUX.app" ]] \
        || die "WRAPPER_NAME must identify the fixed DUX.app product"
    [[ "${UNLOCALIZED_RESOURCES_FOLDER_PATH:-}" == "DUX.app/Contents/Resources" ]] \
        || die "unexpected application resources path"
    [[ "${TARGET_BUILD_DIR:-}" == /* && -d "${TARGET_TEMP_DIR:-}" ]] \
        || die "Xcode build directories are unavailable"

    local staging_root
    staging_root="$(mktemp -d "$TARGET_TEMP_DIR/dux-cli-embed.XXXXXX")"
    # DUX-DESTRUCTIVE: allow=build-bundled-cli-embed-staging-remove -- exact Xcode-target mktemp staging directory contains only copied generated resources
    trap 'rm -rf "$staging_root"' EXIT
    local staged_binary="$staging_root/dux-cli-bundled"
    local staged_metadata="$staging_root/dux-cli-bundled-metadata.json"
    cp -p "$SOURCE_BINARY" "$staged_binary"
    cp -p "$SOURCE_METADATA" "$staged_metadata"
    verify_pair "$staged_binary" "$staged_metadata"

    local resources="$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH"
    mkdir -p "$resources"
    local physical_target_build_dir
    local physical_resources
    physical_target_build_dir="$(cd "$TARGET_BUILD_DIR" && pwd -P)"
    physical_resources="$(cd "$resources" && pwd -P)"
    [[ ! -L "$resources" ]] \
        || die "refusing a symlinked Xcode application resources directory"
    [[ "$physical_resources" == "$physical_target_build_dir/$UNLOCALIZED_RESOURCES_FOLDER_PATH" ]] \
        || die "refusing an unexpected Xcode application resources directory"
    local destination_binary="$resources/dux-cli-bundled"
    local destination_metadata="$resources/dux-cli-bundled-metadata.json"
    [[ ! -L "$destination_binary" && ! -L "$destination_metadata" ]] \
        || die "refusing linked bundled CLI destinations"
    cp -p "$staged_binary" "$destination_binary"
    cp -p "$staged_metadata" "$destination_metadata"
    chmod 0755 "$destination_binary"
    chmod 0644 "$destination_metadata"
    verify_pair "$destination_binary" "$destination_metadata"
    cmp "$staged_binary" "$destination_binary" >/dev/null \
        || die "Xcode embedded different bundled CLI bytes"
    cmp "$staged_metadata" "$destination_metadata" >/dev/null \
        || die "Xcode embedded different bundled CLI metadata"
    trap - EXIT
    # DUX-DESTRUCTIVE: allow=build-bundled-cli-embed-final-staging-remove -- remove only the verified Xcode-target mktemp staging directory
    rm -rf "$staging_root"
}

main "$@"
