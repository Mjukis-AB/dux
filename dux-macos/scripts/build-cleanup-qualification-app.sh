#!/bin/bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly PROJECT_SPEC="$REPO_ROOT/dux-macos/project.yml"
readonly PROJECT_PATH="$REPO_ROOT/dux-macos/Dux.xcodeproj"
readonly DEPLOYMENT_CHECK="$REPO_ROOT/scripts/check_macos_deployment_target.sh"
readonly EXPECTED_BUNDLE_ID="se.mjukis.dux"
readonly EXPECTED_PRODUCT="DUX.app"
readonly EXPECTED_SPARKLE_VERSION="2.9.5"

die() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage:
  DUX_QUALIFICATION_VERSION=X.Y.Z \
  DUX_QUALIFICATION_BUILD_NUMBER=N \
  DUX_QUALIFICATION_COMMIT=FULL_40_CHARACTER_COMMIT \
  build-cleanup-qualification-app.sh /absolute/new/DerivedData

Builds one unsigned, universal CleanupQualification app from the exact clean
HEAD. The output is not releasable or qualified until it is explicitly signed,
notarized, installed, exercised on disposable data, and verified under the
signed-app destructive qualification protocol.
EOF
}

require_command() {
    command -v "$1" >/dev/null 2>&1 \
        || die "required command '$1' was not found"
}

assert_exact_universal() {
    local artifact="$1"
    local architectures
    architectures="$(lipo -archs "$artifact")"
    case "$architectures" in
        "arm64 x86_64"|"x86_64 arm64") ;;
        *) die "'$artifact' must contain exactly arm64 and x86_64 (found: $architectures)" ;;
    esac
}

is_macho() {
    file -b "$1" | grep -q 'Mach-O'
}

assert_exact_code_inventory() {
    local app="$1"
    local executable="$2"
    local cli="$app/Contents/Resources/dux-cli-bundled"
    local sparkle="$app/Contents/Frameworks/Sparkle.framework/Versions/B"
    local count=0
    local candidate
    while IFS= read -r candidate; do
        if is_macho "$candidate"; then
            count=$((count + 1))
            case "$candidate" in
                "$executable"|\
                "$cli"|\
                "$sparkle/Sparkle"|\
                "$sparkle/Autoupdate"|\
                "$sparkle/Updater.app/Contents/MacOS/Updater"|\
                "$sparkle/XPCServices/Downloader.xpc/Contents/MacOS/Downloader"|\
                "$sparkle/XPCServices/Installer.xpc/Contents/MacOS/Installer") ;;
                *) die "qualification app contains unreviewed Mach-O code: $candidate" ;;
            esac
            assert_exact_universal "$candidate"
        fi
    done < <(find "$app/Contents" -type f -print)
    [[ "$count" -eq 7 ]] || die "qualification app must contain exactly seven Mach-O files"
}

verify_sparkle_shape() {
    local app="$1"
    local framework="$app/Contents/Frameworks/Sparkle.framework"
    local version="$framework/Versions/B"
    [[ -d "$version" && ! -L "$version" ]] || die "Sparkle.framework version B is missing or linked"

    local expected_link
    local expected_target
    while IFS='|' read -r expected_link expected_target; do
        [[ -L "$framework/$expected_link" ]] \
            || die "reviewed Sparkle symlink is missing: $expected_link"
        [[ "$(readlink "$framework/$expected_link")" == "$expected_target" ]] \
            || die "reviewed Sparkle symlink has the wrong target: $expected_link"
    done <<'EOF'
Resources|Versions/Current/Resources
Versions/Current|B
Autoupdate|Versions/Current/Autoupdate
Updater.app|Versions/Current/Updater.app
XPCServices|Versions/Current/XPCServices
Sparkle|Versions/Current/Sparkle
EOF
    [[ "$(find "$app" -type l -print | wc -l | tr -d ' ')" == 6 ]] \
        || die "qualification app contains an unreviewed symlink"
    [[ "$(
        plutil -extract CFBundleShortVersionString raw -o - \
            "$version/Resources/Info.plist"
    )" == "$EXPECTED_SPARKLE_VERSION" ]] \
        || die "embedded Sparkle version is not the reviewed $EXPECTED_SPARKLE_VERSION"
}

verify_bundled_cli_manifest() {
    local app="$1"
    local expected_version="$2"
    local cli="$app/Contents/Resources/dux-cli-bundled"
    local manifest="$app/Contents/Resources/dux-cli-bundled-metadata.json"
    [[ -f "$cli" && ! -L "$cli" && -x "$cli" ]] \
        || die "bundled CLI is missing, linked, or not executable"
    [[ -f "$manifest" && ! -L "$manifest" ]] \
        || die "bundled CLI metadata is missing or linked"
    local actual_sha256
    actual_sha256="$(shasum -a 256 "$cli" | awk '{print $1}')"
    jq -e \
        --arg version "$expected_version" \
        --arg sha256 "$actual_sha256" '
        type == "object"
        and keys == [
            "architectures",
            "database_schema_version",
            "product",
            "record_version",
            "sha256",
            "snapshot_format_version",
            "version"
        ]
        and .record_version == 1
        and .product == "dux-cli"
        and .version == $version
        and (.database_schema_version | type == "number" and . >= 1 and . <= 4294967295 and floor == .)
        and (.snapshot_format_version | type == "number" and . >= 1 and . <= 4294967295 and floor == .)
        and .sha256 == $sha256
        and .architectures == ["arm64", "x86_64"]
    ' "$manifest" >/dev/null \
        || die "bundled CLI metadata does not match its bytes, version, or schemas"
}

main() {
    # DUX-DESTRUCTIVE: allow=qualification-builder-help-argument -- compare the first argument only to fixed help flags before any build work
    if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then
        usage
        exit 0
    fi
    [[ "$#" -eq 1 ]] || die "expected one fresh absolute DerivedData path"

    local version="${DUX_QUALIFICATION_VERSION:-}"
    local build_number="${DUX_QUALIFICATION_BUILD_NUMBER:-}"
    local commit="${DUX_QUALIFICATION_COMMIT:-}"
    local derived_data="$1"

    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
        || die "DUX_QUALIFICATION_VERSION must be X.Y.Z"
    [[ "$build_number" =~ ^[1-9][0-9]*$ ]] \
        || die "DUX_QUALIFICATION_BUILD_NUMBER must be a positive integer"
    [[ "$commit" =~ ^[0-9a-f]{40}$ ]] \
        || die "DUX_QUALIFICATION_COMMIT must be a lowercase full commit"
    [[ "$derived_data" == /* ]] || die "DerivedData path must be absolute"
    [[ ! -e "$derived_data" && ! -L "$derived_data" ]] \
        || die "DerivedData path must not already exist"

    local parent
    parent="$(dirname "$derived_data")"
    [[ -d "$parent" && ! -L "$parent" ]] \
        || die "DerivedData parent must be an existing physical directory"
    [[ "$(cd "$parent" && pwd -P)" == "$parent" ]] \
        || die "DerivedData parent must be canonical"

    local command
    for command in awk bash dirname file find git grep jq lipo mkdir plutil readlink shasum tr uname wc xcodebuild xcodegen; do
        require_command "$command"
    done
    [[ "$(uname -s)" == Darwin ]] || die "qualification app builds only on macOS"
    [[ "$(git -C "$REPO_ROOT" rev-parse HEAD)" == "$commit" ]] \
        || die "DUX_QUALIFICATION_COMMIT does not match HEAD"
    [[ -z "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=all)" ]] \
        || die "qualification build requires a clean worktree, including untracked files"

    mkdir "$derived_data"
    CARGO_TARGET_DIR="$derived_data/RustTarget" CONFIGURATION=Release \
        "$SCRIPT_DIR/generate-bindings.sh"
    xcodegen generate --spec "$PROJECT_SPEC"
    [[ -z "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=all)" ]] \
        || die "generation changed tracked source; review and commit it first"

    xcodebuild \
        -project "$PROJECT_PATH" \
        -scheme Dux \
        -configuration CleanupQualification \
        -derivedDataPath "$derived_data" \
        -disableAutomaticPackageResolution \
        -onlyUsePackageVersionsFromResolvedFile \
        ARCHS="arm64 x86_64" \
        ONLY_ACTIVE_ARCH=NO \
        CODE_SIGNING_ALLOWED=NO \
        MARKETING_VERSION="$version" \
        CURRENT_PROJECT_VERSION="$build_number" \
        DUX_CLEANUP_QUALIFICATION_SOURCE_COMMIT="$commit" \
        build

    local app="$derived_data/Build/Products/CleanupQualification/$EXPECTED_PRODUCT"
    local info="$app/Contents/Info.plist"
    [[ -d "$app" && ! -L "$app" ]] || die "qualification app was not produced"
    [[ "$(plutil -extract CFBundleIdentifier raw -o - "$info")" == "$EXPECTED_BUNDLE_ID" ]] \
        || die "qualification app does not use the frozen production bundle ID"
    [[ "$(plutil -extract CFBundleDisplayName raw -o - "$info")" == "DUX Cleanup Qualification" ]] \
        || die "qualification app is not visibly distinguished"
    [[ "$(plutil -extract DUXCleanupQualificationProtocolVersion raw -o - "$info")" == 1 ]] \
        || die "qualification app protocol marker is missing"
    [[ "$(plutil -extract DUXCleanupQualificationSourceCommit raw -o - "$info")" == "$commit" ]] \
        || die "qualification app source commit is not bound"

    local executable_name
    executable_name="$(plutil -extract CFBundleExecutable raw -o - "$info")"
    local executable="$app/Contents/MacOS/$executable_name"
    [[ -f "$executable" && ! -L "$executable" ]] \
        || die "qualification executable is missing or linked"
    assert_exact_code_inventory "$app" "$executable"
    verify_sparkle_shape "$app"
    verify_bundled_cli_manifest "$app" "$version"
    bash "$DEPLOYMENT_CHECK" "$executable"

    printf 'Built unsigned cleanup qualification app:\n%s\n' "$app"
}

main "$@"
