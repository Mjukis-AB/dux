#!/bin/bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly DEPLOYMENT_CHECK="$REPO_ROOT/scripts/check_macos_deployment_target.sh"
readonly IDENTITY_RECORD="$REPO_ROOT/dux-macos/Config/ProductionIdentity.json"
readonly EXPECTED_APP="/Applications/DUX Cleanup Qualification.app"
readonly EXPECTED_BUNDLE_ID="se.mjukis.dux"
readonly EXPECTED_TEAM_ID="SMQ3E8Y57T"
readonly EXPECTED_IDENTITY="Developer ID Application: MJUKIS AB (SMQ3E8Y57T)"
readonly EXPECTED_SPARKLE_KEY="UmMI6TWBdBm2fKEmmk5xi2T+lu7K5KJl1abwIBRLSQo="
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
  DUX_QUALIFICATION_ARCHIVE_SHA256=64_LOWERCASE_HEX \
  verify-signed-cleanup-qualification.sh \
    '/Applications/DUX Cleanup Qualification.app'

Read-only verification for the installed, Developer ID-signed, notarized,
stapled cleanup-qualification app. This does not launch DUX or perform cleanup.
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

assert_no_unreviewed_nested_bundles() {
    local app="$1"
    local nested
    local reviewed
    while IFS= read -r nested; do
        for reviewed in \
            "$app/Contents/Frameworks/Sparkle.framework/Versions/B/Updater.app" \
            "$app/Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices/Downloader.xpc" \
            "$app/Contents/Frameworks/Sparkle.framework/Versions/B/XPCServices/Installer.xpc"; do
            if [[ "$nested" == "$reviewed" ]]; then
                continue 2
            fi
        done
        die "qualification app contains an unreviewed nested bundle: $nested"
    done < <(
        find "$app/Contents" -depth -type d \
            \( -name '*.app' -o -name '*.appex' -o -name '*.xpc' -o -name '*.bundle' \) \
            -print
    )
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
    assert_no_unreviewed_nested_bundles "$app"
}

verify_bundled_cli_manifest() {
    local app="$1"
    local expected_version="$2"
    local cli="$app/Contents/Resources/dux-cli-bundled"
    local manifest="$app/Contents/Resources/dux-cli-bundled-metadata.json"
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
        || die "bundled CLI metadata does not match its signed bytes, version, or schemas"
}

verify_signature_identity() {
    local artifact="$1"
    local expected_identifier="$2"
    local details
    details="$(codesign --display --verbose=4 "$artifact" 2>&1)"
    grep -Fxq "Identifier=$expected_identifier" <<<"$details" \
        || die "'$artifact' has the wrong signing identifier"
    grep -Fxq "TeamIdentifier=$EXPECTED_TEAM_ID" <<<"$details" \
        || die "'$artifact' has the wrong Team ID"
    grep -Fxq "Authority=$EXPECTED_IDENTITY" <<<"$details" \
        || die "'$artifact' has the wrong Developer ID authority"
    grep -Eq 'flags=.*\([^)]*runtime' <<<"$details" \
        || die "'$artifact' does not enable Hardened Runtime"
    grep -Eq '^Timestamp=' <<<"$details" \
        || die "'$artifact' has no secure timestamp"
}

verify_empty_entitlements() {
    local artifact="$1"
    local entitlements
    entitlements="$(codesign --display --entitlements - --xml "$artifact" 2>/dev/null)"
    if [[ -z "$entitlements" ]]; then
        entitlements='<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict/></plist>'
    fi
    plutil -convert json -o - -- - <<<"$entitlements" \
        | jq -e 'type == "object" and length == 0' >/dev/null \
        || die "'$artifact' entitlements are not the reviewed empty dictionary"
}

main() {
    # DUX-DESTRUCTIVE: allow=qualification-verifier-help-argument -- compare the first argument only to fixed help flags before read-only verification
    if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then
        usage
        exit 0
    fi
    [[ "$#" -eq 1 ]] || die "expected the exact installed qualification app path"

    local version="${DUX_QUALIFICATION_VERSION:-}"
    local build_number="${DUX_QUALIFICATION_BUILD_NUMBER:-}"
    local commit="${DUX_QUALIFICATION_COMMIT:-}"
    local archive_sha256="${DUX_QUALIFICATION_ARCHIVE_SHA256:-}"
    local app="$1"
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
        || die "DUX_QUALIFICATION_VERSION must be X.Y.Z"
    [[ "$build_number" =~ ^[1-9][0-9]*$ ]] \
        || die "DUX_QUALIFICATION_BUILD_NUMBER must be a positive integer"
    [[ "$commit" =~ ^[0-9a-f]{40}$ ]] \
        || die "DUX_QUALIFICATION_COMMIT must be a lowercase full commit"
    [[ "$archive_sha256" =~ ^[0-9a-f]{64}$ ]] \
        || die "DUX_QUALIFICATION_ARCHIVE_SHA256 must be a lowercase SHA-256"
    [[ "$app" == "$EXPECTED_APP" ]] \
        || die "qualification app must be installed at '$EXPECTED_APP'"
    [[ -d "$app" && ! -L "$app" ]] || die "installed qualification app is missing or linked"
    [[ "$(cd "$(dirname "$app")" && pwd -P)/$(basename "$app")" == "$EXPECTED_APP" ]] \
        || die "installed qualification app path is not canonical"

    local command
    for command in awk basename bash codesign dirname file find grep jq lipo plutil readlink shasum spctl tr wc xcrun; do
        require_command "$command"
    done

    local info="$app/Contents/Info.plist"
    [[ "$(plutil -extract CFBundleIdentifier raw -o - "$info")" == "$EXPECTED_BUNDLE_ID" ]] \
        || die "qualification app has the wrong bundle ID"
    [[ "$(plutil -extract CFBundleDisplayName raw -o - "$info")" == "DUX Cleanup Qualification" ]] \
        || die "qualification app is not visibly distinguished"
    [[ "$(plutil -extract CFBundleShortVersionString raw -o - "$info")" == "$version" ]] \
        || die "qualification app version does not match"
    [[ "$(plutil -extract CFBundleVersion raw -o - "$info")" == "$build_number" ]] \
        || die "qualification app build number does not match"
    [[ "$(plutil -extract DUXCleanupQualificationProtocolVersion raw -o - "$info")" == 1 ]] \
        || die "qualification protocol marker is not one"
    [[ "$(plutil -extract DUXCleanupQualificationSourceCommit raw -o - "$info")" == "$commit" ]] \
        || die "qualification source commit does not match"
    [[ "$(plutil -extract LSUIElement raw -o - "$info")" == true ]] \
        || die "qualification app is not menu-bar-only"
    [[ "$(plutil -extract SUPublicEDKey raw -o - "$info")" == "$EXPECTED_SPARKLE_KEY" ]] \
        || die "qualification app has the wrong Sparkle public key"
    [[ "$(plutil -extract SURequireSignedFeed raw -o - "$info")" == true ]] \
        || die "qualification app does not require signed feeds"
    [[ "$(plutil -extract SUVerifyUpdateBeforeExtraction raw -o - "$info")" == true ]] \
        || die "qualification app does not verify before extraction"
    [[ "$(plutil -extract SUSignedFeedFailureExpirationInterval raw -o - "$info")" == 0 ]] \
        || die "qualification app permits a signed-feed failure window"
    if plutil -extract SUFeedURL raw -o - "$info" >/dev/null 2>&1; then
        die "qualification app must not contain an update feed"
    fi

    local executable_name
    executable_name="$(plutil -extract CFBundleExecutable raw -o - "$info")"
    local executable="$app/Contents/MacOS/$executable_name"
    local cli="$app/Contents/Resources/dux-cli-bundled"
    [[ -f "$executable" && ! -L "$executable" ]] \
        || die "qualification executable is missing or linked"
    [[ -f "$cli" && ! -L "$cli" ]] || die "bundled CLI is missing or linked"
    assert_exact_universal "$executable"
    assert_exact_universal "$cli"
    bash "$DEPLOYMENT_CHECK" "$executable" "$cli"
    assert_exact_code_inventory "$app" "$executable"
    verify_sparkle_shape "$app"
    verify_bundled_cli_manifest "$app" "$version"

    codesign --verify --all-architectures --strict --verbose=2 "$app"
    codesign --verify --deep --all-architectures --strict --verbose=2 "$app"
    verify_signature_identity "$app" "$EXPECTED_BUNDLE_ID"
    verify_signature_identity "$cli" "$EXPECTED_BUNDLE_ID.cli"
    local sparkle="$app/Contents/Frameworks/Sparkle.framework/Versions/B"
    verify_signature_identity \
        "$app/Contents/Frameworks/Sparkle.framework" \
        "org.sparkle-project.Sparkle"
    verify_signature_identity \
        "$sparkle/Autoupdate" \
        "Autoupdate-55554944b10b2f3652903f61bfd5afadcbf1d8d0"
    verify_signature_identity \
        "$sparkle/Updater.app" \
        "org.sparkle-project.Sparkle.Updater"
    verify_signature_identity \
        "$sparkle/XPCServices/Downloader.xpc" \
        "org.sparkle-project.DownloaderService"
    verify_signature_identity \
        "$sparkle/XPCServices/Installer.xpc" \
        "org.sparkle-project.InstallerLauncher"
    verify_empty_entitlements "$app"
    verify_empty_entitlements "$cli"
    verify_empty_entitlements "$app/Contents/Frameworks/Sparkle.framework"
    verify_empty_entitlements "$sparkle/Autoupdate"
    verify_empty_entitlements "$sparkle/Updater.app"
    verify_empty_entitlements "$sparkle/XPCServices/Downloader.xpc"
    verify_empty_entitlements "$sparkle/XPCServices/Installer.xpc"

    local expected_requirement
    expected_requirement="$(jq -r '.designated_requirement' "$IDENTITY_RECORD")"
    local actual_requirement
    actual_requirement="$(
        codesign --display --requirements - "$app" 2>&1 \
            | awk '/^designated => / { sub(/^designated => /, ""); print }'
    )"
    [[ "$actual_requirement" == "$expected_requirement" ]] \
        || die "qualification app designated requirement does not match production"

    xcrun stapler validate "$app"
    local assessment
    assessment="$(spctl --assess --type execute --verbose=4 "$app" 2>&1)"
    grep -Fq 'accepted' <<<"$assessment" || die "Gatekeeper did not accept the app"
    grep -Fq 'source=Notarized Developer ID' <<<"$assessment" \
        || die "Gatekeeper did not report Notarized Developer ID"

    local signing_details
    signing_details="$(codesign --display --verbose=4 "$app" 2>&1)"
    local cdhash
    cdhash="$(awk -F= '$1 == "CDHash" { print $2 }' <<<"$signing_details")"
    [[ "$cdhash" =~ ^[0-9a-fA-F]{40}$ ]] || die "qualification app CDHash is malformed"
    local executable_sha256
    executable_sha256="$(shasum -a 256 "$executable" | awk '{print $1}')"

    printf 'qualification_protocol=1\n'
    printf 'source_commit=%s\n' "$commit"
    printf 'archive_sha256=%s\n' "$archive_sha256"
    printf 'version=%s\n' "$version"
    printf 'build_number=%s\n' "$build_number"
    printf 'team_id=%s\n' "$EXPECTED_TEAM_ID"
    printf 'cdhash=%s\n' "$cdhash"
    printf 'executable_sha256=%s\n' "$executable_sha256"
    printf 'gatekeeper=accepted-notarized-developer-id\n'
}

main "$@"
