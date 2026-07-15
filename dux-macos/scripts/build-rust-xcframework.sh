#!/bin/bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly CRATE_NAME="dux-ffi"
readonly LIBRARY_NAME="libdux_ffi.a"
readonly ARM64_TARGET="aarch64-apple-darwin"
readonly X86_64_TARGET="x86_64-apple-darwin"
readonly CONFIGURATION="${CONFIGURATION:-Release}"
readonly EXPECTED_OUTPUT_PATH="$REPO_ROOT/dux-macos/Generated/DuxFFI.xcframework"
readonly OUTPUT_PATH="${1:-$EXPECTED_OUTPUT_PATH}"
readonly HEADERS_PATH="${DUX_FFI_HEADERS_PATH:-}"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
# Rust build scripts that compile C/C++ dependencies must receive the same
# minimum OS as the Xcode target. Without this, clang stamps bundled objects
# with the host SDK version and the universal app cannot actually run on 14.
export MACOSX_DEPLOYMENT_TARGET="14.0"

if [[ "$OUTPUT_PATH" != "$EXPECTED_OUTPUT_PATH" ]]; then
    echo "error: output must be the repository-owned XCFramework path '$EXPECTED_OUTPUT_PATH'" >&2
    exit 1
fi

readonly OUTPUT_PARENT="$(dirname "$EXPECTED_OUTPUT_PATH")"
readonly OUTPUT_GRANDPARENT="$(dirname "$OUTPUT_PARENT")"
readonly PHYSICAL_OUTPUT_GRANDPARENT="$(cd "$OUTPUT_GRANDPARENT" && pwd -P)"
if [[ "$PHYSICAL_OUTPUT_GRANDPARENT" != "$OUTPUT_GRANDPARENT" || -L "$OUTPUT_PARENT" ]]; then
    echo "error: refusing a symlinked XCFramework output parent '$OUTPUT_PARENT'" >&2
    exit 1
fi
mkdir -p "$OUTPUT_PARENT"
readonly PHYSICAL_OUTPUT_PARENT="$(cd "$OUTPUT_PARENT" && pwd -P)"
if [[ "$PHYSICAL_OUTPUT_PARENT" != "$OUTPUT_PARENT" ]]; then
    echo "error: XCFramework output parent is not repository-owned '$OUTPUT_PARENT'" >&2
    exit 1
fi

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: required command '$1' was not found" >&2
        exit 1
    fi
}

require_rust_target() {
    local target="$1"
    if ! rustup target list --installed | grep -Fxq "$target"; then
        echo "error: Rust target '$target' is not installed" >&2
        echo "install it with: rustup target add $target" >&2
        exit 1
    fi
}

assert_universal_library() {
    local library="$1"
    local architectures
    architectures="$(lipo -archs "$library")"

    case " $architectures " in
        *" arm64 "*) ;;
        *)
            echo "error: '$library' is missing arm64" >&2
            exit 1
            ;;
    esac

    case " $architectures " in
        *" x86_64 "*) ;;
        *)
            echo "error: '$library' is missing x86_64" >&2
            exit 1
            ;;
    esac

    local architecture_count=0
    local architecture
    for architecture in $architectures; do
        architecture_count=$((architecture_count + 1))
    done
    if [[ "$architecture_count" -ne 2 ]]; then
        echo "error: '$library' has unexpected architectures: $architectures" >&2
        exit 1
    fi
}

require_command cargo
require_command rustup
require_command grep
require_command lipo
require_command xcodebuild
require_command plutil
require_rust_target "$ARM64_TARGET"
require_rust_target "$X86_64_TARGET"

case "$CONFIGURATION" in
    Debug|debug)
        readonly PROFILE_DIRECTORY="debug"
        ;;
    Release|release)
        readonly PROFILE_DIRECTORY="release"
        ;;
    *)
        echo "error: unsupported CONFIGURATION '$CONFIGURATION' (expected Debug or Release)" >&2
        exit 1
        ;;
esac

readonly STAGING_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/dux-xcframework.XXXXXX")"
# DUX-DESTRUCTIVE: allow=build-xcframework-staging-remove -- mktemp-created staging directory is exclusively owned by this build
trap 'rm -rf "$STAGING_ROOT"' EXIT

for target in "$ARM64_TARGET" "$X86_64_TARGET"; do
    if [[ "$PROFILE_DIRECTORY" == "release" ]]; then
        cargo build \
            --manifest-path "$REPO_ROOT/Cargo.toml" \
            --locked \
            --package "$CRATE_NAME" \
            --target "$target" \
            --release
    else
        cargo build \
            --manifest-path "$REPO_ROOT/Cargo.toml" \
            --locked \
            --package "$CRATE_NAME" \
            --target "$target"
    fi
done

readonly ARM64_LIBRARY="$CARGO_TARGET_DIR/$ARM64_TARGET/$PROFILE_DIRECTORY/$LIBRARY_NAME"
readonly X86_64_LIBRARY="$CARGO_TARGET_DIR/$X86_64_TARGET/$PROFILE_DIRECTORY/$LIBRARY_NAME"
readonly UNIVERSAL_LIBRARY="$STAGING_ROOT/$LIBRARY_NAME"
readonly STAGED_XCFRAMEWORK="$STAGING_ROOT/DuxFFI.xcframework"

lipo -create "$ARM64_LIBRARY" "$X86_64_LIBRARY" -output "$UNIVERSAL_LIBRARY"
assert_universal_library "$UNIVERSAL_LIBRARY"

if [[ -n "$HEADERS_PATH" ]]; then
    if [[ ! -f "$HEADERS_PATH/DuxFFILowLevel.h" || ! -f "$HEADERS_PATH/module.modulemap" ]]; then
        echo "error: DUX_FFI_HEADERS_PATH must contain DuxFFILowLevel.h and module.modulemap" >&2
        exit 1
    fi
    xcodebuild -create-xcframework \
        -library "$UNIVERSAL_LIBRARY" \
        -headers "$HEADERS_PATH" \
        -output "$STAGED_XCFRAMEWORK"
else
    xcodebuild -create-xcframework \
        -library "$UNIVERSAL_LIBRARY" \
        -output "$STAGED_XCFRAMEWORK"
fi

plutil -lint "$STAGED_XCFRAMEWORK/Info.plist" >/dev/null

readonly PACKAGED_LIBRARY="$(find "$STAGED_XCFRAMEWORK" -type f -name "$LIBRARY_NAME" -print -quit)"
if [[ -z "$PACKAGED_LIBRARY" ]]; then
    echo "error: XCFramework does not contain '$LIBRARY_NAME'" >&2
    exit 1
fi
assert_universal_library "$PACKAGED_LIBRARY"
readonly PACKAGED_ARCHITECTURES="$(lipo -archs "$PACKAGED_LIBRARY")"

if [[ -e "$OUTPUT_PATH" || -L "$OUTPUT_PATH" ]]; then
    # DUX-DESTRUCTIVE: allow=build-xcframework-output-remove -- exact repository-generated XCFramework destination was validated above
    rm -rf "$OUTPUT_PATH"
fi
# DUX-DESTRUCTIVE: allow=build-xcframework-publish-move -- publish staged XCFramework only to the validated repository destination
mv "$STAGED_XCFRAMEWORK" "$OUTPUT_PATH"

echo "Created $OUTPUT_PATH ($PACKAGED_ARCHITECTURES)"
