#!/bin/bash

set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly GENERATED_ROOT="$REPO_ROOT/dux-macos/Generated"
readonly OUTPUT_BINARY="$GENERATED_ROOT/dux-cli-bundled"
readonly OUTPUT_METADATA="$GENERATED_ROOT/dux-cli-bundled-metadata.json"
readonly METADATA_FINALIZER="$SCRIPT_DIR/finalize-bundled-cli-metadata.py"
readonly DEPLOYMENT_CHECK="$REPO_ROOT/scripts/check_macos_deployment_target.sh"
readonly ARM64_TARGET="aarch64-apple-darwin"
readonly X86_64_TARGET="x86_64-apple-darwin"
readonly DEVELOPMENT_SIGNING_IDENTIFIER="se.mjukis.dux.spike.cli.debug"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
export MACOSX_DEPLOYMENT_TARGET="14.0"

die() {
    echo "error: $*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 \
        || die "required command '$1' was not found"
}

require_rust_target() {
    local target="$1"
    rustup target list --installed | grep -Fxq "$target" \
        || die "Rust target '$target' is not installed; run: rustup target add $target"
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

sign_and_verify_development_binary() {
    local binary="$1"
    codesign --force --sign - \
        --identifier "$DEVELOPMENT_SIGNING_IDENTIFIER" \
        --options runtime \
        --generate-entitlement-der \
        "$binary"
    codesign --verify --all-architectures --strict --verbose=2 "$binary"
    local details
    details="$(codesign --display --verbose=4 "$binary" 2>&1)"
    grep -Fxq "Identifier=$DEVELOPMENT_SIGNING_IDENTIFIER" <<<"$details" \
        || die "bundled CLI has the wrong ad-hoc development identifier"
    grep -Fxq "Signature=adhoc" <<<"$details" \
        || die "bundled CLI does not have an ad-hoc development signature"
    grep -Eq 'flags=.*\([^)]*runtime' <<<"$details" \
        || die "bundled CLI development signature does not enable Hardened Runtime"
}

main() {
    [[ "$#" -eq 0 ]] || die "this builder accepts no arguments"
    [[ "$(uname -s)" == "Darwin" ]] || die "the bundled CLI can be built only on macOS"

    local command
    for command in awk cargo chmod codesign grep lipo mkdir mktemp mv python3 rustup shasum uname; do
        require_command "$command"
    done
    [[ -x "$METADATA_FINALIZER" || -f "$METADATA_FINALIZER" ]] \
        || die "metadata finalizer is missing: $METADATA_FINALIZER"
    [[ -x "$DEPLOYMENT_CHECK" || -f "$DEPLOYMENT_CHECK" ]] \
        || die "deployment-target verifier is missing: $DEPLOYMENT_CHECK"
    require_rust_target "$ARM64_TARGET"
    require_rust_target "$X86_64_TARGET"

    local physical_parent
    physical_parent="$(cd "$(dirname "$GENERATED_ROOT")" && pwd -P)"
    [[ "$physical_parent" == "$(dirname "$GENERATED_ROOT")" ]] \
        || die "generated output grandparent is not repository-owned"
    [[ ! -L "$GENERATED_ROOT" ]] || die "refusing a symlinked generated output directory"
    mkdir -p "$GENERATED_ROOT"
    [[ "$(cd "$GENERATED_ROOT" && pwd -P)" == "$GENERATED_ROOT" ]] \
        || die "generated output directory is not repository-owned"
    for output in "$OUTPUT_BINARY" "$OUTPUT_METADATA"; do
        [[ ! -L "$output" ]] || die "refusing a symlinked generated output: $output"
        [[ ! -e "$output" || -f "$output" ]] \
            || die "generated output is not a regular file: $output"
    done

    local staging_root
    staging_root="$(mktemp -d "$GENERATED_ROOT/.dux-cli-stage.XXXXXX")"
    # DUX-DESTRUCTIVE: allow=build-bundled-cli-staging-remove -- exact mktemp-created repository-generated staging directory
    trap 'rm -rf "$staging_root"' EXIT

    local target
    for target in "$ARM64_TARGET" "$X86_64_TARGET"; do
        cargo build \
            --manifest-path "$REPO_ROOT/Cargo.toml" \
            --locked \
            --package dux-cli \
            --target "$target" \
            --release
    done

    local arm64_binary="$CARGO_TARGET_DIR/$ARM64_TARGET/release/dux"
    local x86_64_binary="$CARGO_TARGET_DIR/$X86_64_TARGET/release/dux"
    [[ -f "$arm64_binary" ]] || die "arm64 CLI was not produced: $arm64_binary"
    [[ -f "$x86_64_binary" ]] || die "x86_64 CLI was not produced: $x86_64_binary"

    local staged_binary="$staging_root/dux-cli-bundled"
    local raw_metadata="$staging_root/raw-metadata.json"
    local staged_metadata="$staging_root/dux-cli-bundled-metadata.json"
    local metadata_stderr="$staging_root/metadata.stderr"
    lipo -create "$arm64_binary" "$x86_64_binary" -output "$staged_binary"
    chmod 0755 "$staged_binary"
    assert_exact_universal "$staged_binary"
    bash "$DEPLOYMENT_CHECK" "$staged_binary"
    sign_and_verify_development_binary "$staged_binary"
    assert_exact_universal "$staged_binary"
    bash "$DEPLOYMENT_CHECK" "$staged_binary"

    # DUX-DESTRUCTIVE: allow=build-bundled-cli-metadata-inspect -- executes only the just-built, signature-verified private CLI stage with its fixed inspection-only hidden command
    "$staged_binary" __bundle-metadata >"$raw_metadata" 2>"$metadata_stderr" \
        || die "trusted CLI metadata command failed"
    [[ ! -s "$metadata_stderr" ]] \
        || die "trusted CLI metadata command wrote unexpected stderr"

    local package_version
    package_version="$(
        cargo metadata \
            --manifest-path "$REPO_ROOT/Cargo.toml" \
            --locked \
            --no-deps \
            --format-version 1 \
            | python3 -c 'import json,sys; values=[p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "dux-cli"]; sys.stdout.write(values[0] if len(values) == 1 else "")'
    )"
    [[ -n "$package_version" ]] || die "could not resolve one dux-cli package version"

    local binary_sha256
    binary_sha256="$(shasum -a 256 "$staged_binary" | awk '{print $1}')"
    python3 "$METADATA_FINALIZER" \
        --input "$raw_metadata" \
        --expected-version "$package_version" \
        --sha256 "$binary_sha256" \
        >"$staged_metadata"
    [[ -s "$staged_metadata" ]] || die "final bundled CLI metadata is empty"

    # Publish the manifest last. If publication is interrupted, consumers reject
    # the mismatched pair instead of trusting stale metadata.
    # DUX-DESTRUCTIVE: allow=build-bundled-cli-binary-publish -- replace only the fixed validated generated bundled-CLI destination
    mv -f "$staged_binary" "$OUTPUT_BINARY"
    # DUX-DESTRUCTIVE: allow=build-bundled-cli-metadata-publish -- replace only the fixed validated generated bundled-CLI manifest destination
    mv -f "$staged_metadata" "$OUTPUT_METADATA"
    trap - EXIT
    # DUX-DESTRUCTIVE: allow=build-bundled-cli-final-staging-remove -- remove the exact emptied mktemp-created staging directory after publication
    rm -rf "$staging_root"

    echo "Created $OUTPUT_BINARY (arm64,x86_64; version $package_version)"
    echo "Created $OUTPUT_METADATA (sha256 $binary_sha256)"
}

main "$@"
