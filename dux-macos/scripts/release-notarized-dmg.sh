#!/bin/bash

set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly MACOS_ROOT="$REPO_ROOT/dux-macos"
readonly PROJECT_PATH="$MACOS_ROOT/Dux.xcodeproj"
readonly PROJECT_SPEC="$MACOS_ROOT/project.yml"
readonly PACKAGE_RESOLVED="$PROJECT_PATH/project.xcworkspace/xcshareddata/swiftpm/Package.resolved"
readonly ENTITLEMENTS_PATH="$MACOS_ROOT/Config/Release.entitlements"
readonly PRODUCTION_IDENTITY_PATH="$MACOS_ROOT/Config/ProductionIdentity.json"
readonly DEPLOYMENT_CHECK="$REPO_ROOT/scripts/check_macos_deployment_target.sh"
readonly CLI_METADATA_FINALIZER="$SCRIPT_DIR/finalize-bundled-cli-metadata.py"
readonly BUNDLED_CLI_NAME="dux-cli-bundled"
readonly BUNDLED_CLI_METADATA_NAME="dux-cli-bundled-metadata.json"
readonly DEVELOPMENT_CLI_SIGNING_IDENTIFIER="se.mjukis.dux.spike.cli.debug"
readonly RELEASE_PARENT="$REPO_ROOT/target/dux-macos-release"
readonly PRODUCTION_BUNDLE_IDENTIFIER="se.mjukis.dux"
readonly PRODUCTION_TEAM_ID="SMQ3E8Y57T"
readonly PRODUCTION_SIGNING_IDENTITY="Developer ID Application: MJUKIS AB (SMQ3E8Y57T)"
readonly PRODUCTION_DESIGNATED_REQUIREMENT='identifier "se.mjukis.dux" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = SMQ3E8Y57T'
readonly PRODUCTION_SPARKLE_PUBLIC_KEY="UmMI6TWBdBm2fKEmmk5xi2T+lu7K5KJl1abwIBRLSQo="
readonly PREPARED_FORMAT_VERSION=1
readonly PREPARED_ARCHIVE_NAME="DUX-unsigned.app.zip"
readonly PREPARED_CHECKSUM_NAME="$PREPARED_ARCHIVE_NAME.sha256"
readonly PREPARED_MANIFEST_NAME="prepared-manifest.txt"

release_succeeded=false
mounted_image=false
mount_point=""
staging_root=""

usage() {
    cat <<'EOF'
Prepare or sign, notarize, staple, and verify the primary DUX DMG.

Usage:
  release-notarized-dmg.sh --prepare /absolute/path/to/new-envelope
  release-notarized-dmg.sh --verify-prepared /absolute/path/to/envelope /absolute/path/to/new-inspection
  release-notarized-dmg.sh --sign-prepared /absolute/path/to/envelope

Required for both phases:
  DUX_VERSION             Stable X.Y.Z version; HEAD must be tagged vX.Y.Z
  DUX_BUILD_NUMBER        Positive integer CFBundleVersion
  DUX_BUNDLE_IDENTIFIER   Frozen production identifier (the spike ID is rejected)
  DUX_TEAM_ID             Ten-character Apple Developer Team ID

Required only for --sign-prepared:
  DUX_SIGNING_IDENTITY    Exact Developer ID Application identity in Keychain
  DUX_NOTARYTOOL_PROFILE  notarytool Keychain profile name

Optional environment:
  DUX_NOTARYTOOL_KEYCHAIN Absolute path to a dedicated file Keychain containing
                          the named profile. Omit to use the default Keychain.

Create the Keychain profile interactively before release:
  xcrun notarytool store-credentials PROFILE_NAME

The prepare phase requires a clean tagged revision. It runs every repository
release gate, builds a universal macOS 14 app without release credentials, and
seals the exact app archive with a SHA-256 manifest. The signing phase verifies
that seal, executes no build/test/dependency code, signs code inside-out,
notarizes and staples the app and Applications-link DMG, verifies Gatekeeper,
and atomically publishes immutable output under:
  target/dux-macos-release/vX.Y.Z/

It never accepts passwords or API private-key paths on the command line.
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 \
        || die "required command '$1' was not found"
}

require_value() {
    local name="$1"
    local value="$2"
    [[ -n "$value" ]] || die "required environment variable '$name' is missing"
}

run_notarytool() {
    local subcommand="$1"
    shift
    if [[ -n "$notary_keychain" ]]; then
        xcrun notarytool "$subcommand" --keychain "$notary_keychain" "$@"
    else
        xcrun notarytool "$subcommand" "$@"
    fi
}

validate_public_inputs() {
    require_value DUX_VERSION "$release_version"
    require_value DUX_BUILD_NUMBER "$build_number"
    require_value DUX_BUNDLE_IDENTIFIER "$bundle_identifier"
    require_value DUX_TEAM_ID "$team_id"

    grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' \
        <<<"$release_version" \
        || die "DUX_VERSION must be a stable X.Y.Z version"
    [[ "$build_number" =~ ^[1-9][0-9]*$ ]] \
        || die "DUX_BUILD_NUMBER must be a positive integer without leading zeroes"
    [[ "$bundle_identifier" =~ ^[A-Za-z0-9][A-Za-z0-9.-]*[A-Za-z0-9]$ ]] \
        || die "DUX_BUNDLE_IDENTIFIER has an invalid shape"
    [[ "$bundle_identifier" == *.* ]] \
        || die "DUX_BUNDLE_IDENTIFIER must be a reverse-DNS identifier"
    [[ "$bundle_identifier" != *..* ]] \
        || die "DUX_BUNDLE_IDENTIFIER cannot contain an empty component"
    [[ ${#bundle_identifier} -le 255 ]] \
        || die "DUX_BUNDLE_IDENTIFIER exceeds 255 characters"
    [[ "$bundle_identifier" != "se.mjukis.dux.spike" ]] \
        || die "the temporary spike bundle identifier cannot be released"
    [[ "$bundle_identifier" != com.apple.* ]] \
        || die "DUX_BUNDLE_IDENTIFIER cannot use Apple's reserved namespace"
    [[ "$team_id" =~ ^[A-Z0-9]{10}$ ]] \
        || die "DUX_TEAM_ID must contain exactly ten uppercase letters or digits"
    [[ "$bundle_identifier" == "$PRODUCTION_BUNDLE_IDENTIFIER" ]] \
        || die "DUX_BUNDLE_IDENTIFIER does not match the frozen production identity"
    [[ "$team_id" == "$PRODUCTION_TEAM_ID" ]] \
        || die "DUX_TEAM_ID does not match the frozen production identity"
}

validate_signing_inputs() {
    require_value DUX_SIGNING_IDENTITY "$signing_identity"
    require_value DUX_NOTARYTOOL_PROFILE "$notary_profile"

    [[ "$signing_identity" == "Developer ID Application: "*" ($team_id)" ]] \
        || die "DUX_SIGNING_IDENTITY must be a Developer ID Application identity for DUX_TEAM_ID"
    [[ "$signing_identity" != *$'\n'* ]] \
        || die "identity and profile values must be single-line strings"
    [[ "$signing_identity" != *$'\r'* ]] \
        || die "identity and profile values must be single-line strings"
    [[ "$notary_profile" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] \
        || die "DUX_NOTARYTOOL_PROFILE must be a safe Keychain profile name"
    if [[ -n "$notary_keychain" ]]; then
        [[ "$notary_keychain" == /* ]] \
            || die "DUX_NOTARYTOOL_KEYCHAIN must be an absolute path"
        [[ "$notary_keychain" != *$'\n'* ]] \
            || die "DUX_NOTARYTOOL_KEYCHAIN must be a single-line path"
        [[ "$notary_keychain" != *$'\r'* ]] \
            || die "DUX_NOTARYTOOL_KEYCHAIN must be a single-line path"
        [[ -f "$notary_keychain" && ! -L "$notary_keychain" ]] \
            || die "DUX_NOTARYTOOL_KEYCHAIN must be an existing regular file, not a symlink"
    fi
    [[ "$signing_identity" == "$PRODUCTION_SIGNING_IDENTITY" ]] \
        || die "DUX_SIGNING_IDENTITY does not match the frozen production identity"
}

assert_exact_universal() {
    local artifact="$1"
    local architectures
    architectures="$(lipo -archs "$artifact")"
    local count=0
    local has_arm64=false
    local has_x86_64=false
    local architecture
    for architecture in $architectures; do
        count=$((count + 1))
        case "$architecture" in
            arm64) has_arm64=true ;;
            x86_64) has_x86_64=true ;;
        esac
    done
    [[ "$count" -eq 2 ]] \
        || die "'$artifact' must contain exactly two architectures (found: $architectures)"
    [[ "$has_arm64" == true ]] \
        || die "'$artifact' is missing arm64"
    [[ "$has_x86_64" == true ]] \
        || die "'$artifact' is missing x86_64"
}

verify_bundled_cli_payload() {
    local app="$1"
    local cli="$app/Contents/Resources/$BUNDLED_CLI_NAME"
    local manifest="$app/Contents/Resources/$BUNDLED_CLI_METADATA_NAME"
    [[ -f "$cli" && ! -L "$cli" && -x "$cli" ]] \
        || die "bundled CLI is missing, linked, or not executable: $cli"
    [[ -f "$manifest" && ! -L "$manifest" ]] \
        || die "bundled CLI metadata is missing or linked: $manifest"
    assert_exact_universal "$cli"
    bash "$DEPLOYMENT_CHECK" "$cli"
    codesign --verify --all-architectures --strict --verbose=2 "$cli"

    local actual_sha256
    actual_sha256="$(shasum -a 256 "$cli" | awk '{print $1}')"
    local raw_metadata
    local metadata_stderr
    local expected_manifest
    raw_metadata="$(mktemp "$staging_root/cli-raw-metadata.XXXXXX")"
    metadata_stderr="$(mktemp "$staging_root/cli-metadata-stderr.XXXXXX")"
    expected_manifest="$(mktemp "$staging_root/cli-expected-manifest.XXXXXX")"
    # DUX-DESTRUCTIVE: allow=release-bundled-cli-metadata-inspect -- executes only the staged, signature-verified bundled CLI with its fixed inspection-only hidden command
    "$cli" __bundle-metadata >"$raw_metadata" 2>"$metadata_stderr" \
        || die "bundled CLI metadata command failed"
    [[ ! -s "$metadata_stderr" ]] \
        || die "bundled CLI metadata command wrote unexpected stderr"
    python3 "$CLI_METADATA_FINALIZER" \
        --input "$raw_metadata" \
        --expected-version "$release_version" \
        --sha256 "$actual_sha256" \
        >"$expected_manifest"
    cmp "$manifest" "$expected_manifest" >/dev/null \
        || die "bundled CLI manifest does not match its bytes, version, or schemas"
}

verify_prepared_bundled_cli_payload() {
    local app="$1"
    local cli="$app/Contents/Resources/$BUNDLED_CLI_NAME"
    local manifest="$app/Contents/Resources/$BUNDLED_CLI_METADATA_NAME"
    [[ -f "$cli" && ! -L "$cli" && -x "$cli" ]] \
        || die "bundled CLI is missing, linked, or not executable: $cli"
    [[ -f "$manifest" && ! -L "$manifest" ]] \
        || die "bundled CLI metadata is missing or linked: $manifest"
    assert_exact_universal "$cli"
    codesign --verify --all-architectures --strict --verbose=2 "$cli"

    local actual_sha256
    actual_sha256="$(shasum -a 256 "$cli" | awk '{print $1}')"
    jq -e \
        --arg version "$release_version" \
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
        || die "prepared bundled CLI manifest does not match its bytes, version, or schemas"
}

verify_sparkle_update_policy() {
    local app="$1"
    local plist="$app/Contents/Info.plist"
    [[ "$(plutil -extract SUPublicEDKey raw -o - "$plist")" == "$PRODUCTION_SPARKLE_PUBLIC_KEY" ]] \
        || die "built app does not contain the frozen Sparkle public key"
    [[ "$(plutil -extract SURequireSignedFeed raw -o - "$plist")" == true ]] \
        || die "built app must require a signed Sparkle feed"
    [[ "$(plutil -extract SUVerifyUpdateBeforeExtraction raw -o - "$plist")" == true ]] \
        || die "built app must verify Sparkle updates before extraction"
    [[ "$(plutil -extract SUSignedFeedFailureExpirationInterval raw -o - "$plist")" == 0 ]] \
        || die "built app must never expire signed-feed failures"
    if plutil -extract SUFeedURL raw -o - "$plist" >/dev/null 2>&1; then
        die "built app must not activate a Sparkle feed before the trust-model gate"
    fi
}

verify_development_bundled_cli() {
    local app="$1"
    local cli="$app/Contents/Resources/$BUNDLED_CLI_NAME"
    local details
    details="$(codesign --display --verbose=4 "$cli" 2>&1)"
    grep -Fxq "Identifier=$DEVELOPMENT_CLI_SIGNING_IDENTIFIER" <<<"$details" \
        || die "bundled CLI has the wrong development signing identifier"
    grep -Fxq "Signature=adhoc" <<<"$details" \
        || die "bundled CLI development signature is not ad-hoc"
    grep -Fxq "TeamIdentifier=not set" <<<"$details" \
        || die "bundled CLI development signature unexpectedly has a Team ID"
    grep -Eq 'flags=.*\([^)]*runtime' <<<"$details" \
        || die "bundled CLI development signature does not enable Hardened Runtime"
}

verify_signed_bundled_cli() {
    local app="$1"
    local cli="$app/Contents/Resources/$BUNDLED_CLI_NAME"
    codesign --verify --all-architectures --strict --verbose=2 "$cli"

    local details
    details="$(codesign --display --verbose=4 "$cli" 2>&1)"
    grep -Fxq "Identifier=$cli_signing_identifier" <<<"$details" \
        || die "bundled CLI identifier does not match $cli_signing_identifier"
    grep -Fxq "TeamIdentifier=$team_id" <<<"$details" \
        || die "bundled CLI TeamIdentifier does not match $team_id"
    grep -Fxq "Authority=$signing_identity" <<<"$details" \
        || die "bundled CLI authority does not match the selected identity"
    grep -Eq 'flags=.*\([^)]*runtime' <<<"$details" \
        || die "bundled CLI does not enable Hardened Runtime"
    grep -Eq '^Timestamp=' <<<"$details" \
        || die "bundled CLI has no secure signing timestamp"
}

sign_bundled_cli() {
    local app="$1"
    local cli="$app/Contents/Resources/$BUNDLED_CLI_NAME"
    local manifest="$app/Contents/Resources/$BUNDLED_CLI_METADATA_NAME"
    verify_prepared_bundled_cli_payload "$app"
    verify_development_bundled_cli "$app"
    codesign --force --sign "$signing_identity" --timestamp \
        --options runtime --generate-entitlement-der \
        --identifier "$cli_signing_identifier" "$cli"
    verify_signed_bundled_cli "$app"

    local signed_sha256
    signed_sha256="$(shasum -a 256 "$cli" | awk '{print $1}')"
    local rebound_manifest
    rebound_manifest="$(mktemp "$staging_root/cli-signed-manifest.XXXXXX")"
    jq -c --arg sha256 "$signed_sha256" '.sha256 = $sha256' \
        "$manifest" >"$rebound_manifest"
    # DUX-DESTRUCTIVE: allow=release-bundled-cli-manifest-publish -- replace only the fixed manifest inside the private unsigned release staging app
    mv "$rebound_manifest" "$manifest"
    verify_prepared_bundled_cli_payload "$app"
}

is_macho() {
    file -b "$1" | grep -q 'Mach-O'
}

assert_exact_code_inventory() {
    local app="$1"
    local main_executable="$2"
    local bundled_cli="$app/Contents/Resources/$BUNDLED_CLI_NAME"
    local sparkle_root="$app/Contents/Frameworks/Sparkle.framework/Versions/B"
    local macho_count=0
    local candidate
    while IFS= read -r candidate; do
        if is_macho "$candidate"; then
            macho_count=$((macho_count + 1))
            case "$candidate" in
                "$main_executable"|\
                "$bundled_cli"|\
                "$sparkle_root/Sparkle"|\
                "$sparkle_root/Autoupdate"|\
                "$sparkle_root/Updater.app/Contents/MacOS/Updater"|\
                "$sparkle_root/XPCServices/Downloader.xpc/Contents/MacOS/Downloader"|\
                "$sparkle_root/XPCServices/Installer.xpc/Contents/MacOS/Installer") ;;
                *) die "app contains an unreviewed Mach-O executable: $candidate" ;;
            esac
        fi
    done < <(find "$app/Contents" -type f -print)
    [[ "$macho_count" -eq 7 ]] \
        || die "app must contain exactly seven reviewed Mach-O executables"

    local framework="$app/Contents/Frameworks/Sparkle.framework"
    [[ "$(find "$app/Contents" -type d -name '*.framework' -print | wc -l | tr -d ' ')" == 1 ]] \
        || die "app must contain exactly the reviewed Sparkle framework"
    [[ -d "$framework" && ! -L "$framework" ]] \
        || die "reviewed Sparkle framework is missing or linked"
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
        die "unreviewed nested code bundle requires an explicit entitlement/signing policy: $nested"
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
    local candidate
    [[ -d "$version" ]] || die "Sparkle.framework version B is missing"
    [[ "$(readlink "$framework/Versions/Current")" == B ]] \
        || die "Sparkle.framework Current symlink does not select version B"
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
        || die "app contains an unreviewed symlink"

    for candidate in \
        "$version/Sparkle" \
        "$version/Autoupdate" \
        "$version/Updater.app/Contents/MacOS/Updater" \
        "$version/XPCServices/Downloader.xpc/Contents/MacOS/Downloader" \
        "$version/XPCServices/Installer.xpc/Contents/MacOS/Installer"; do
        [[ -f "$candidate" ]] || die "reviewed Sparkle executable is missing: $candidate"
        assert_exact_universal "$candidate"
    done

    [[ "$(
        plutil -extract CFBundleShortVersionString raw -o - \
            "$version/Resources/Info.plist"
    )" == 2.9.5 ]] || die "embedded Sparkle version is not the reviewed 2.9.5"
    assert_no_unreviewed_nested_bundles "$app"
}

sign_sparkle() {
    local app="$1"
    local framework="$app/Contents/Frameworks/Sparkle.framework"
    local version="$framework/Versions/B"
    local candidate

    verify_sparkle_shape "$app"
    for candidate in \
        "$version/XPCServices/Installer.xpc" \
        "$version/Autoupdate" \
        "$version/Updater.app"; do
        codesign --force --sign "$signing_identity" --timestamp \
            --options runtime --generate-entitlement-der "$candidate"
        codesign --verify --all-architectures --strict --verbose=2 "$candidate"
    done
    candidate="$version/XPCServices/Downloader.xpc"
    codesign --force --sign "$signing_identity" --timestamp \
        --options runtime --preserve-metadata=entitlements \
        --generate-entitlement-der "$candidate"
    codesign --verify --all-architectures --strict --verbose=2 "$candidate"

    codesign --force --sign "$signing_identity" --timestamp \
        --options runtime --generate-entitlement-der "$framework"
    codesign --verify --deep --all-architectures --strict --verbose=2 "$framework"
}

sign_nested_code() {
    local app="$1"
    local main_executable="$2"

    assert_no_unreviewed_nested_bundles "$app"
    assert_exact_code_inventory "$app" "$main_executable"
    sign_sparkle "$app"
}

extract_entitlements() {
    local app="$1"
    local output="$2"
    codesign --display --entitlements "$output" --xml "$app" 2>/dev/null
    if [[ ! -s "$output" ]]; then
        cp "$ENTITLEMENTS_PATH" "$output"
    fi
}

verify_empty_entitlements() {
    local entitlements="$1"
    plutil -lint "$entitlements" >/dev/null
    plutil -convert json -o - "$entitlements" \
        | jq -e 'type == "object" and length == 0' >/dev/null \
        || die "release entitlements must remain the reviewed empty dictionary"
}

verify_unsigned_app_shape() {
    local app="$1"
    [[ -d "$app" ]] || die "expected app bundle was not produced: $app"
    local executable_name
    executable_name="$(
        plutil -extract CFBundleExecutable raw -o - "$app/Contents/Info.plist"
    )"
    local executable="$app/Contents/MacOS/$executable_name"
    [[ -f "$executable" ]] || die "app main executable is missing: $executable"
    assert_exact_universal "$executable"
    assert_exact_code_inventory "$app" "$executable"
    bash "$DEPLOYMENT_CHECK" "$executable"
    verify_bundled_cli_payload "$app"
    verify_development_bundled_cli "$app"
    verify_sparkle_shape "$app"
    verify_sparkle_update_policy "$app"
}

verify_prepared_app_shape() {
    local app="$1"
    [[ -d "$app" && ! -L "$app" ]] \
        || die "prepared app bundle is missing or linked: $app"
    local executable_name
    executable_name="$(
        plutil -extract CFBundleExecutable raw -o - "$app/Contents/Info.plist"
    )"
    local executable="$app/Contents/MacOS/$executable_name"
    [[ -f "$executable" && ! -L "$executable" ]] \
        || die "prepared app main executable is missing or linked: $executable"
    assert_exact_universal "$executable"
    assert_exact_code_inventory "$app" "$executable"
    verify_prepared_bundled_cli_payload "$app"
    verify_development_bundled_cli "$app"
    verify_sparkle_shape "$app"
    verify_sparkle_update_policy "$app"
    [[ "$(plutil -extract CFBundleIdentifier raw -o - "$app/Contents/Info.plist")" == "$bundle_identifier" ]] \
        || die "prepared app bundle identifier is incorrect"
    [[ "$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")" == "$release_version" ]] \
        || die "prepared app marketing version is incorrect"
    [[ "$(plutil -extract CFBundleVersion raw -o - "$app/Contents/Info.plist")" == "$build_number" ]] \
        || die "prepared app build number is incorrect"
}

compare_app_layouts() {
    local debug_app="$1"
    local release_app="$2"
    local debug_inventory="$staging_root/debug-app-layout.txt"
    local release_inventory="$staging_root/release-app-layout.txt"
    (cd "$debug_app" && find Contents -print | sort) >"$debug_inventory"
    (cd "$release_app" && find Contents -print | sort) >"$release_inventory"
    cmp "$debug_inventory" "$release_inventory" >/dev/null \
        || die "Debug and Release app bundle layouts differ"
}

compare_bundled_cli_payloads() {
    local debug_app="$1"
    local release_app="$2"
    cmp \
        "$debug_app/Contents/Resources/$BUNDLED_CLI_NAME" \
        "$release_app/Contents/Resources/$BUNDLED_CLI_NAME" >/dev/null \
        || die "Debug and Release bundled CLI bytes differ"
    cmp \
        "$debug_app/Contents/Resources/$BUNDLED_CLI_METADATA_NAME" \
        "$release_app/Contents/Resources/$BUNDLED_CLI_METADATA_NAME" >/dev/null \
        || die "Debug and Release bundled CLI metadata differs"
}

verify_permanent_cleanup_feature_gate() {
    local debug_settings
    local release_settings
    debug_settings="$(
        xcodebuild -project "$PROJECT_PATH" -scheme Dux \
            -configuration Debug -disableAutomaticPackageResolution \
            -onlyUsePackageVersionsFromResolvedFile -showBuildSettings
    )"
    release_settings="$(
        xcodebuild -project "$PROJECT_PATH" -scheme Dux \
            -configuration Release -disableAutomaticPackageResolution \
            -onlyUsePackageVersionsFromResolvedFile -showBuildSettings
    )"
    grep -Fq 'DUX_INTERNAL_PERMANENT_SAFE_CLEANUP' <<<"$debug_settings" \
        || die "internal cleanup UI must remain explicit in Debug builds"
    if grep -Fq 'DUX_INTERNAL_PERMANENT_SAFE_CLEANUP' <<<"$release_settings"; then
        die "permanent cleanup UI must not be compiled into public Release builds"
    fi
}

verify_signed_app() {
    local app="$1"
    local executable="$app/Contents/MacOS/$(
        plutil -extract CFBundleExecutable raw -o - "$app/Contents/Info.plist"
    )"
    [[ -f "$executable" ]] || die "signed app main executable is missing"
    assert_exact_universal "$executable"
    assert_exact_code_inventory "$app" "$executable"
    verify_prepared_bundled_cli_payload "$app"
    verify_signed_bundled_cli "$app"
    verify_sparkle_update_policy "$app"

    codesign --verify --all-architectures --strict --verbose=2 "$app"
    # Verification may use --deep as an audit; signing above is always explicit.
    codesign --verify --deep --all-architectures --strict --verbose=2 "$app"

    local details
    details="$(codesign --display --verbose=4 "$app" 2>&1)"
    grep -Fxq "Identifier=$bundle_identifier" <<<"$details" \
        || die "signed app identifier does not match $bundle_identifier"
    grep -Fxq "TeamIdentifier=$team_id" <<<"$details" \
        || die "signed app TeamIdentifier does not match $team_id"
    grep -Fxq "Authority=$signing_identity" <<<"$details" \
        || die "signed app authority does not match the selected identity"
    grep -Eq 'flags=.*\([^)]*runtime' <<<"$details" \
        || die "signed app does not enable Hardened Runtime"
    grep -Eq '^Timestamp=' <<<"$details" \
        || die "signed app has no secure signing timestamp"

    local actual_entitlements
    actual_entitlements="$(mktemp "$staging_root/actual-release.XXXXXX")"
    extract_entitlements "$app" "$actual_entitlements"
    verify_empty_entitlements "$actual_entitlements"

    [[ "$(plutil -extract CFBundleIdentifier raw -o - "$app/Contents/Info.plist")" == "$bundle_identifier" ]] \
        || die "built app bundle identifier is incorrect"
    [[ "$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")" == "$release_version" ]] \
        || die "built app marketing version is incorrect"
    [[ "$(plutil -extract CFBundleVersion raw -o - "$app/Contents/Info.plist")" == "$build_number" ]] \
        || die "built app build number is incorrect"

    local actual_designated_requirement
    actual_designated_requirement="$(
        codesign --display --requirements - "$app" 2>&1 \
            | awk '/^designated => / { sub(/^designated => /, ""); print }'
    )"
    [[ "$actual_designated_requirement" == "$PRODUCTION_DESIGNATED_REQUIREMENT" ]] \
        || die "signed app designated requirement does not match the frozen production identity"
}

verify_signed_dmg() {
    local dmg="$1"
    codesign --verify --strict --verbose=2 "$dmg"
    local details
    details="$(codesign --display --verbose=4 "$dmg" 2>&1)"
    grep -Fxq "Authority=$signing_identity" <<<"$details" \
        || die "signed DMG authority does not match the selected identity"
    grep -Eq '^Timestamp=' <<<"$details" \
        || die "signed DMG has no secure signing timestamp"
}

submit_and_require_accepted() {
    local artifact="$1"
    local label="$2"
    local response="$publish_root/DUX-$release_version-$label-notarization.json"
    local raw_response="$staging_root/$label-notarization.raw.json"
    local log="$publish_root/DUX-$release_version-$label-notarization-log.json"

    run_notarytool submit "$artifact" \
        --keychain-profile "$notary_profile" \
        --wait --timeout 1h --output-format json >"$raw_response"
    jq -e '.status == "Accepted" and (.id | type == "string" and length > 0)' \
        "$raw_response" >/dev/null \
        || die "$label notarization was not accepted; response retained at $raw_response"

    local submission_id
    submission_id="$(jq -r '.id' "$raw_response")"
    run_notarytool log "$submission_id" \
        --keychain-profile "$notary_profile" "$log"
    jq -e '
        .status == "Accepted"
        and ([.issues[]? | select(.severity == "error")] | length == 0)
    ' "$log" >/dev/null \
        || die "$label notarization log contains an error"

    jq '{id, status, message, createdDate, name}' "$raw_response" >"$response"
}

cleanup() {
    local status=$?
    trap - EXIT
    if [[ "$mounted_image" == true && -n "$mount_point" ]]; then
        hdiutil detach "$mount_point" >/dev/null 2>&1 || true
    fi
    if [[ "$status" -eq 0 ]]; then
        if [[ "$release_succeeded" == true ]]; then
            if [[ -n "$staging_root" ]]; then
                if [[ -d "$staging_root" ]]; then
                    # DUX-DESTRUCTIVE: allow=release-dmg-staging-remove -- mktemp-owned release staging is removed only after atomic publication succeeds
                    rm -rf "$staging_root"
                fi
            fi
        fi
    elif [[ -n "$staging_root" ]]; then
        echo "release failed; diagnostics and staging were retained at: $staging_root" >&2
    fi
    exit "$status"
}

verify_frozen_identity() {
    verify_empty_entitlements "$ENTITLEMENTS_PATH"
    [[ -f "$PRODUCTION_IDENTITY_PATH" && ! -L "$PRODUCTION_IDENTITY_PATH" ]] \
        || die "frozen production identity record is missing or linked"
    [[ "$(jq -r '.record_version' "$PRODUCTION_IDENTITY_PATH")" == 1 ]] \
        || die "frozen production identity record version is unsupported"
    [[ "$(jq -r '.bundle_identifier' "$PRODUCTION_IDENTITY_PATH")" == "$PRODUCTION_BUNDLE_IDENTIFIER" ]] \
        || die "frozen production bundle identifier record drifted"
    [[ "$(jq -r '.team_id' "$PRODUCTION_IDENTITY_PATH")" == "$PRODUCTION_TEAM_ID" ]] \
        || die "frozen production Team ID record drifted"
    [[ "$(jq -r '.developer_id_application' "$PRODUCTION_IDENTITY_PATH")" == "$PRODUCTION_SIGNING_IDENTITY" ]] \
        || die "frozen Developer ID identity record drifted"
    [[ "$(jq -r '.designated_requirement' "$PRODUCTION_IDENTITY_PATH")" == "$PRODUCTION_DESIGNATED_REQUIREMENT" ]] \
        || die "frozen designated requirement record drifted"
    [[ "$(jq -r '.sparkle_public_ed_key' "$PRODUCTION_IDENTITY_PATH")" == "$PRODUCTION_SPARKLE_PUBLIC_KEY" ]] \
        || die "frozen Sparkle public key record drifted"
}

verify_clean_tagged_source() {
    [[ -z "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=all)" ]] \
        || die "release preparation requires a clean worktree, including untracked files"
    source_commit="$(git -C "$REPO_ROOT" rev-parse HEAD)"
    local tag_commit
    tag_commit="$(git -C "$REPO_ROOT" rev-list -n 1 "$release_tag" 2>/dev/null || true)"
    [[ -n "$tag_commit" ]] || die "release tag does not exist: $release_tag"
    [[ "$tag_commit" == "$source_commit" ]] || die "HEAD must be exactly tagged $release_tag"

    local workspace_versions
    workspace_versions="$(
        cargo metadata --manifest-path "$REPO_ROOT/Cargo.toml" \
            --locked --no-deps --format-version 1 \
            | jq -r '.packages[] | select(.name | startswith("dux-")) | .version' \
            | sort -u
    )"
    [[ "$workspace_versions" == "$release_version" ]] \
        || die "all DUX workspace package versions must equal $release_version"
}

validate_new_prepared_path() {
    local path="$1"
    [[ "$path" == /* ]] \
        || die "prepared envelope path must be an absolute single-line path"
    [[ "$path" != *$'\n'* ]] \
        || die "prepared envelope path must be an absolute single-line path"
    [[ "$path" != *$'\r'* ]] \
        || die "prepared envelope path must be an absolute single-line path"
    [[ ! -e "$path" && ! -L "$path" ]] \
        || die "prepared envelope path already exists: $path"
    local parent
    parent="$(dirname "$path")"
    mkdir -p "$parent"
    [[ ! -L "$parent" ]] || die "prepared envelope parent cannot be a symlink"
    [[ "$(cd "$parent" && pwd -P)" == "$parent" ]] \
        || die "prepared envelope parent must use its canonical path"
}

verify_prepared_envelope() {
    local path="$1"
    local expected_commit="$2"
    [[ "$path" == /* && -d "$path" && ! -L "$path" ]] \
        || die "prepared envelope must be an existing absolute directory, not a symlink"
    [[ "$(cd "$path" && pwd -P)" == "$path" ]] \
        || die "prepared envelope must use its canonical path"
    [[ "$(find "$path" -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')" == 3 ]] \
        || die "prepared envelope must contain exactly three files"

    local archive="$path/$PREPARED_ARCHIVE_NAME"
    local checksum="$path/$PREPARED_CHECKSUM_NAME"
    local manifest="$path/$PREPARED_MANIFEST_NAME"
    local candidate
    for candidate in "$archive" "$checksum" "$manifest"; do
        [[ -f "$candidate" && ! -L "$candidate" ]] \
            || die "prepared envelope member is missing, linked, or not a regular file: $candidate"
    done

    local archive_sha256
    archive_sha256="$(shasum -a 256 "$archive" | awk '{print $1}')"
    [[ "$(cat "$checksum")" == "$archive_sha256  $PREPARED_ARCHIVE_NAME" ]] \
        || die "prepared archive checksum sidecar is not exact"
    [[ "$(wc -l <"$checksum" | tr -d ' ')" == 1 ]] \
        || die "prepared archive checksum sidecar must contain exactly one line"
    (cd "$path" && shasum -a 256 -c "$PREPARED_CHECKSUM_NAME") >/dev/null

    [[ "$expected_commit" =~ ^[0-9a-f]{40}$ ]] \
        || die "expected prepared commit must be a full lowercase Git SHA"
    [[ "$(wc -l <"$manifest" | tr -d ' ')" == 13 ]] \
        || die "prepared manifest must contain exactly thirteen lines"
    grep -Fxq "format=$PREPARED_FORMAT_VERSION" "$manifest"
    grep -Fxq "version=$release_version" "$manifest"
    grep -Fxq "build=$build_number" "$manifest"
    grep -Fxq "tag=$release_tag" "$manifest"
    grep -Fxq "commit=$expected_commit" "$manifest"
    grep -Fxq "bundle_identifier=$bundle_identifier" "$manifest"
    grep -Fxq "team_id=$team_id" "$manifest"
    grep -Fxq "architectures=arm64,x86_64" "$manifest"
    grep -Fxq "deployment_target=14.0" "$manifest"
    grep -Eq '^xcode_version=[0-9][0-9A-Za-z.+-]*$' "$manifest"
    grep -Eq '^xcode_build=[0-9A-Za-z.+-]+$' "$manifest"
    grep -Fxq "sparkle_version=2.9.5" "$manifest"
    grep -Fxq "archive_sha256=$archive_sha256" "$manifest"
}

prepare_release() {
    local prepared_path="$1"
    validate_public_inputs

    local command
    for command in awk bash cargo cat codesign cmp dirname ditto file find git \
        grep jq lipo mkdir mktemp mv plutil python3 readlink rm shasum sort tr uname \
        wc xcodebuild xcodegen; do
        require_command "$command"
    done
    verify_frozen_identity
    verify_clean_tagged_source
    validate_new_prepared_path "$prepared_path"

    local host_arch
    host_arch="$(uname -m)"
    case "$host_arch" in
        arm64|x86_64) ;;
        *) die "unsupported macOS release host architecture: $host_arch" ;;
    esac

    local xcode_version_output
    local xcode_version
    local xcode_build
    xcode_version_output="$(xcodebuild -version)"
    xcode_version="$(awk '/^Xcode / { print $2 }' <<<"$xcode_version_output")"
    xcode_build="$(awk '/^Build version / { print $3 }' <<<"$xcode_version_output")"
    [[ "$xcode_version" =~ ^[0-9][0-9A-Za-z.+-]*$ ]] \
        || die "xcodebuild returned an unrecognized version"
    [[ "$xcode_build" =~ ^[0-9A-Za-z.+-]+$ ]] \
        || die "xcodebuild returned an unrecognized version"

    local prepared_parent
    prepared_parent="$(dirname "$prepared_path")"
    staging_root="$(mktemp -d "$prepared_parent/.dux-prepared-staging.XXXXXX")"
    trap cleanup EXIT
    local debug_derived_data="$staging_root/DerivedData-Debug"
    local debug_app="$debug_derived_data/Build/Products/Debug/DUX.app"
    local swift_test_data="$staging_root/DerivedData-Tests"
    local derived_data="$staging_root/DerivedData"
    local unsigned_app="$derived_data/Build/Products/Release/DUX.app"

    cargo fmt --manifest-path "$REPO_ROOT/Cargo.toml" --all -- --check
    cargo clippy --manifest-path "$REPO_ROOT/Cargo.toml" \
        --workspace --all-targets --locked -- -D warnings
    cargo test --manifest-path "$REPO_ROOT/Cargo.toml" \
        --workspace --all-targets --locked
    python3 -m unittest discover -s "$REPO_ROOT/scripts/tests" -p 'test_*.py'
    python3 "$REPO_ROOT/scripts/check_destructive_calls.py"

    CONFIGURATION=Debug "$SCRIPT_DIR/generate-bindings.sh"
    xcodegen generate --spec "$PROJECT_SPEC"
    git -C "$REPO_ROOT" diff --exit-code -- \
        dux-macos/Dux.xcodeproj/project.pbxproj \
        dux-macos/Dux.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved \
        dux-macos/Dux/Generated/DuxFFI.swift
    verify_permanent_cleanup_feature_gate
    xcodebuild test -project "$PROJECT_PATH" -scheme Dux \
        -destination "platform=macOS,arch=$host_arch" \
        -disableAutomaticPackageResolution \
        -onlyUsePackageVersionsFromResolvedFile \
        -derivedDataPath "$swift_test_data" CODE_SIGNING_ALLOWED=NO
    xcodebuild -project "$PROJECT_PATH" -scheme Dux -configuration Debug \
        -destination 'generic/platform=macOS' -derivedDataPath "$debug_derived_data" \
        -disableAutomaticPackageResolution \
        -onlyUsePackageVersionsFromResolvedFile \
        ARCHS='arm64 x86_64' ONLY_ACTIVE_ARCH=NO ENABLE_DEBUG_DYLIB=NO \
        CODE_SIGNING_ALLOWED=NO PRODUCT_BUNDLE_IDENTIFIER="$bundle_identifier" \
        MARKETING_VERSION="$release_version" \
        CURRENT_PROJECT_VERSION="$build_number" build
    verify_unsigned_app_shape "$debug_app"

    CONFIGURATION=Release "$SCRIPT_DIR/generate-bindings.sh"
    git -C "$REPO_ROOT" diff --exit-code -- \
        dux-macos/Dux/Generated/DuxFFI.swift
    xcodebuild -project "$PROJECT_PATH" -scheme Dux -configuration Release \
        -destination 'generic/platform=macOS' -derivedDataPath "$derived_data" \
        -disableAutomaticPackageResolution \
        -onlyUsePackageVersionsFromResolvedFile \
        ARCHS='arm64 x86_64' ONLY_ACTIVE_ARCH=NO CODE_SIGNING_ALLOWED=NO \
        PRODUCT_BUNDLE_IDENTIFIER="$bundle_identifier" \
        MARKETING_VERSION="$release_version" \
        CURRENT_PROJECT_VERSION="$build_number" build

    verify_unsigned_app_shape "$unsigned_app"
    compare_app_layouts "$debug_app" "$unsigned_app"
    compare_bundled_cli_payloads "$debug_app" "$unsigned_app"
    verify_clean_tagged_source

    local envelope_root="$staging_root/envelope"
    mkdir "$envelope_root"
    local archive="$envelope_root/$PREPARED_ARCHIVE_NAME"
    ditto -c -k --keepParent "$unsigned_app" "$archive"
    local archive_sha256
    archive_sha256="$(shasum -a 256 "$archive" | awk '{print $1}')"
    printf '%s  %s\n' "$archive_sha256" "$PREPARED_ARCHIVE_NAME" \
        >"$envelope_root/$PREPARED_CHECKSUM_NAME"
    printf '%s\n' \
        "format=$PREPARED_FORMAT_VERSION" \
        "version=$release_version" \
        "build=$build_number" \
        "tag=$release_tag" \
        "commit=$source_commit" \
        "bundle_identifier=$bundle_identifier" \
        "team_id=$team_id" \
        "architectures=arm64,x86_64" \
        "deployment_target=14.0" \
        "xcode_version=$xcode_version" \
        "xcode_build=$xcode_build" \
        "sparkle_version=2.9.5" \
        "archive_sha256=$archive_sha256" \
        >"$envelope_root/$PREPARED_MANIFEST_NAME"
    verify_prepared_envelope "$envelope_root" "$source_commit"

    [[ ! -e "$prepared_path" && ! -L "$prepared_path" ]] \
        || die "prepared envelope appeared during preparation: $prepared_path"
    # DUX-DESTRUCTIVE: allow=release-prepared-envelope-publish -- atomically publish only the newly created three-file sealed envelope at its caller-selected absent path
    mv "$envelope_root" "$prepared_path"
    release_succeeded=true
    echo "Prepared sealed unsigned release envelope: $prepared_path"
}

verify_prepared_release() {
    local prepared_path="$1"
    local inspection_path="$2"
    validate_public_inputs
    require_value DUX_RELEASE_COMMIT "$expected_commit"

    local command
    for command in awk cat codesign dirname ditto file find grep jq lipo mkdir \
        plutil readlink shasum tr wc; do
        require_command "$command"
    done
    verify_frozen_identity
    verify_prepared_envelope "$prepared_path" "$expected_commit"

    [[ "$inspection_path" == /* ]] \
        || die "prepared inspection path must be absolute"
    [[ "$inspection_path" != *$'\n'* ]] \
        || die "prepared inspection path must be single-line"
    [[ "$inspection_path" != *$'\r'* ]] \
        || die "prepared inspection path must be single-line"
    [[ ! -e "$inspection_path" && ! -L "$inspection_path" ]] \
        || die "prepared inspection path already exists: $inspection_path"
    local inspection_parent
    inspection_parent="$(dirname "$inspection_path")"
    mkdir -p "$inspection_parent"
    [[ ! -L "$inspection_parent" ]] \
        || die "prepared inspection parent cannot be a symlink"
    [[ "$(cd "$inspection_parent" && pwd -P)" == "$inspection_parent" ]] \
        || die "prepared inspection parent must use its canonical path"
    mkdir "$inspection_path"
    ditto -x -k "$prepared_path/$PREPARED_ARCHIVE_NAME" "$inspection_path"
    [[ "$(find "$inspection_path" -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')" == 1 ]] \
        || die "prepared archive must extract exactly one top-level app"
    verify_prepared_app_shape "$inspection_path/DUX.app"
    echo "Verified sealed unsigned release envelope: $prepared_path"
}

sign_prepared_release() {
    local prepared_path="$1"
    validate_public_inputs
    validate_signing_inputs
    require_value DUX_RELEASE_COMMIT "$expected_commit"

    local command
    for command in awk cat codesign cp dirname ditto file find grep hdiutil jq \
        lipo ln mkdir mktemp mv plutil readlink rm security shasum sort spctl tr \
        wc xcrun; do
        require_command "$command"
    done
    verify_frozen_identity
    verify_prepared_envelope "$prepared_path" "$expected_commit"

    local identity_inventory
    identity_inventory="$(security find-identity -v -p codesigning)"
    awk -v wanted="$signing_identity" \
        'index($0, "\"" wanted "\"") { found = 1 } END { exit !found }' \
        <<<"$identity_inventory" \
        || die "the requested Developer ID Application identity is not valid in Keychain"

    run_notarytool history --keychain-profile "$notary_profile" \
        --output-format json >/dev/null

    if [[ -L "$REPO_ROOT/target" ]]; then
        die "refusing a symlinked repository target directory"
    fi
    mkdir -p "$RELEASE_PARENT"
    [[ ! -L "$RELEASE_PARENT" ]] || die "refusing a symlinked release output parent"
    [[ "$(cd "$RELEASE_PARENT" && pwd -P)" == "$RELEASE_PARENT" ]] \
        || die "release output parent is not repository-owned"

    local output_dir="$RELEASE_PARENT/$release_tag"
    [[ ! -e "$output_dir" && ! -L "$output_dir" ]] \
        || die "immutable release output already exists: $output_dir"

    staging_root="$(mktemp -d "$RELEASE_PARENT/.staging.XXXXXX")"
    trap cleanup EXIT
    local extracted_root="$staging_root/prepared"
    local staged_app="$extracted_root/DUX.app"
    local app_zip="$staging_root/DUX-$release_version.zip"
    local image_root="$staging_root/image-root"
    local dmg_name="DUX-$release_version.dmg"
    publish_root="$staging_root/publish"
    local staged_dmg="$publish_root/$dmg_name"
    mkdir "$extracted_root"
    mkdir -p "$publish_root"
    ditto -x -k "$prepared_path/$PREPARED_ARCHIVE_NAME" "$extracted_root"
    [[ "$(find "$extracted_root" -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')" == 1 ]] \
        || die "prepared archive must extract exactly one top-level app"
    verify_prepared_app_shape "$staged_app"
    local main_name
    main_name="$(plutil -extract CFBundleExecutable raw -o - "$staged_app/Contents/Info.plist")"
    local main_executable="$staged_app/Contents/MacOS/$main_name"
    [[ -f "$main_executable" ]] || die "main executable is missing from DUX.app"
    assert_exact_universal "$main_executable"
    sign_bundled_cli "$staged_app"
    sign_nested_code "$staged_app" "$main_executable"
    codesign --force --sign "$signing_identity" --timestamp \
        --options runtime --generate-entitlement-der \
        --entitlements "$ENTITLEMENTS_PATH" "$staged_app"
    verify_signed_app "$staged_app"

    ditto -c -k --keepParent "$staged_app" "$app_zip"
    submit_and_require_accepted "$app_zip" app
    xcrun stapler staple "$staged_app"
    xcrun stapler validate "$staged_app"
    spctl --assess --type execute --verbose=4 "$staged_app"
    verify_signed_app "$staged_app"

    mkdir -p "$image_root"
    ditto "$staged_app" "$image_root/DUX.app"
    ln -s /Applications "$image_root/Applications"
    hdiutil create -srcfolder "$image_root" -volname DUX \
        -format UDZO "$staged_dmg"
    codesign --force --sign "$signing_identity" --timestamp "$staged_dmg"
    verify_signed_dmg "$staged_dmg"
    hdiutil verify "$staged_dmg"

    submit_and_require_accepted "$staged_dmg" dmg
    xcrun stapler staple "$staged_dmg"
    xcrun stapler validate "$staged_dmg"
    verify_signed_dmg "$staged_dmg"
    spctl --assess --type open --context context:primary-signature \
        --verbose=4 "$staged_dmg"
    hdiutil verify "$staged_dmg"

    mount_point="$staging_root/mounted-dmg"
    mkdir "$mount_point"
    hdiutil attach "$staged_dmg" -readonly -nobrowse -mountpoint "$mount_point" >/dev/null
    mounted_image=true
    [[ -L "$mount_point/Applications" ]] \
        || die "final DMG does not contain an Applications link"
    [[ "$(readlink "$mount_point/Applications")" == /Applications ]] \
        || die "final DMG Applications link has the wrong target"
    [[ -d "$mount_point/DUX.app" ]] || die "final DMG does not contain DUX.app"
    verify_signed_app "$mount_point/DUX.app"
    xcrun stapler validate "$mount_point/DUX.app"
    spctl --assess --type execute --verbose=4 "$mount_point/DUX.app"
    hdiutil detach "$mount_point" >/dev/null
    mounted_image=false

    local dmg_checksum
    local cli_sha256
    local cli_database_schema_version
    local cli_snapshot_format_version
    local prepared_manifest="$prepared_path/$PREPARED_MANIFEST_NAME"
    local xcode_version
    local xcode_build
    xcode_version="$(awk -F= '$1 == "xcode_version" { print $2 }' "$prepared_manifest")"
    xcode_build="$(awk -F= '$1 == "xcode_build" { print $2 }' "$prepared_manifest")"
    dmg_checksum="$(shasum -a 256 "$staged_dmg" | awk '{print $1}')"
    cli_sha256="$(jq -r '.sha256' "$staged_app/Contents/Resources/$BUNDLED_CLI_METADATA_NAME")"
    cli_database_schema_version="$(jq -r '.database_schema_version' "$staged_app/Contents/Resources/$BUNDLED_CLI_METADATA_NAME")"
    cli_snapshot_format_version="$(jq -r '.snapshot_format_version' "$staged_app/Contents/Resources/$BUNDLED_CLI_METADATA_NAME")"
    printf '%s  %s\n' "$dmg_checksum" "$dmg_name" \
        >"$publish_root/$dmg_name.sha256"
    printf '%s\n' \
        "version=$release_version" \
        "build=$build_number" \
        "tag=$release_tag" \
        "commit=$expected_commit" \
        "bundle_identifier=$bundle_identifier" \
        "team_id=$team_id" \
        "signing_identity=$signing_identity" \
        "designated_requirement=$PRODUCTION_DESIGNATED_REQUIREMENT" \
        "architectures=arm64,x86_64" \
        "deployment_target=14.0" \
        "xcode_version=$xcode_version" \
        "xcode_build=$xcode_build" \
        "sparkle_version=2.9.5" \
        "cli_version=$release_version" \
        "cli_identifier=$cli_signing_identifier" \
        "cli_architectures=arm64,x86_64" \
        "cli_database_schema_version=$cli_database_schema_version" \
        "cli_snapshot_format_version=$cli_snapshot_format_version" \
        "cli_sha256=$cli_sha256" \
        "dmg_sha256=$dmg_checksum" \
        >"$publish_root/release-manifest.txt"

    [[ ! -e "$output_dir" && ! -L "$output_dir" ]] \
        || die "release output appeared during the build: $output_dir"
    # DUX-DESTRUCTIVE: allow=release-dmg-publish-move -- atomically publish a new versioned directory only after every release verification succeeds
    mv "$publish_root" "$output_dir"
    release_succeeded=true
    echo "Published signed, notarized, and stapled release: $output_dir/$dmg_name"
}

main() {
    case "${1:-}" in
        --help|-h)
            [[ "$#" -eq 1 ]] || die "--help does not accept another argument"
            usage
            ;;
        --prepare)
            [[ "$#" -eq 2 ]] || die "--prepare requires one absolute envelope path"
            prepare_release "$2"
            ;;
        --verify-prepared)
            [[ "$#" -eq 3 ]] \
                || die "--verify-prepared requires envelope and inspection paths"
            verify_prepared_release "$2" "$3"
            ;;
        --sign-prepared)
            [[ "$#" -eq 2 ]] || die "--sign-prepared requires one absolute envelope path"
            sign_prepared_release "$2"
            ;;
        "")
            die "a phase is required; use --help"
            ;;
        *)
            die "unexpected argument '$1'; use --help"
            ;;
    esac
}

release_version="${DUX_VERSION:-}"
build_number="${DUX_BUILD_NUMBER:-}"
bundle_identifier="${DUX_BUNDLE_IDENTIFIER:-}"
cli_signing_identifier="${bundle_identifier}.cli"
team_id="${DUX_TEAM_ID:-}"
signing_identity="${DUX_SIGNING_IDENTITY:-}"
notary_profile="${DUX_NOTARYTOOL_PROFILE:-}"
notary_keychain="${DUX_NOTARYTOOL_KEYCHAIN:-}"
expected_commit="${DUX_RELEASE_COMMIT:-}"
release_tag="v$release_version"
source_commit=""
publish_root=""

main "$@"
