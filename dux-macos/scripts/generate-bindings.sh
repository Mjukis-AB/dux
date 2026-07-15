#!/bin/bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly XCFRAMEWORK_PATH="$REPO_ROOT/dux-macos/Generated/DuxFFI.xcframework"
readonly SWIFT_OUTPUT="$REPO_ROOT/dux-macos/Dux/Generated/DuxFFI.swift"
readonly STAGING_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/dux-bindings.XXXXXX")"
readonly GENERATED_ROOT="$STAGING_ROOT/Generated"
readonly HEADERS_ROOT="$STAGING_ROOT/Headers"

# DUX-DESTRUCTIVE: allow=build-bindings-staging-remove -- mktemp-created staging directory is exclusively owned by this build
trap 'rm -rf "$STAGING_ROOT"' EXIT

if ! command -v perl >/dev/null 2>&1; then
    echo "error: required command 'perl' was not found" >&2
    exit 1
fi

"$SCRIPT_DIR/build-rust-xcframework.sh" "$XCFRAMEWORK_PATH"

readonly UNIVERSAL_LIBRARY="$(find "$XCFRAMEWORK_PATH" -type f -name libdux_ffi.a -print -quit)"
if [[ -z "$UNIVERSAL_LIBRARY" ]]; then
    echo "error: universal libdux_ffi.a was not found in '$XCFRAMEWORK_PATH'" >&2
    exit 1
fi

mkdir -p "$GENERATED_ROOT" "$HEADERS_ROOT" "$(dirname "$SWIFT_OUTPUT")"

cargo run \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    --package dux-ffi \
    --features bindgen \
    --bin uniffi-bindgen-swift \
    -- \
    "$UNIVERSAL_LIBRARY" \
    "$GENERATED_ROOT" \
    --swift-sources \
    --headers \
    --modulemap \
    --module-name DuxFFILowLevel \
    --modulemap-filename module.modulemap

for generated_file in DuxFFI.swift DuxFFILowLevel.h module.modulemap; do
    if [[ ! -s "$GENERATED_ROOT/$generated_file" ]]; then
        echo "error: UniFFI did not generate '$generated_file'" >&2
        exit 1
    fi
done

perl -pi -e 's/[ \t]+$//' \
    "$GENERATED_ROOT/DuxFFI.swift" \
    "$GENERATED_ROOT/DuxFFILowLevel.h" \
    "$GENERATED_ROOT/module.modulemap"

cp "$GENERATED_ROOT/DuxFFILowLevel.h" "$HEADERS_ROOT/DuxFFILowLevel.h"
cp "$GENERATED_ROOT/module.modulemap" "$HEADERS_ROOT/module.modulemap"
cp "$GENERATED_ROOT/DuxFFI.swift" "$SWIFT_OUTPUT"

DUX_FFI_HEADERS_PATH="$HEADERS_ROOT" \
    "$SCRIPT_DIR/build-rust-xcframework.sh" "$XCFRAMEWORK_PATH"

readonly PACKAGED_HEADER="$(find "$XCFRAMEWORK_PATH" -type f -name DuxFFILowLevel.h -print -quit)"
readonly PACKAGED_MODULEMAP="$(find "$XCFRAMEWORK_PATH" -type f -name module.modulemap -print -quit)"
if [[ -z "$PACKAGED_HEADER" || -z "$PACKAGED_MODULEMAP" ]]; then
    echo "error: generated XCFramework is missing UniFFI headers" >&2
    exit 1
fi

echo "Generated $SWIFT_OUTPUT and $XCFRAMEWORK_PATH"
