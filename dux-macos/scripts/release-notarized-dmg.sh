#!/bin/bash

set -euo pipefail
umask 077

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly MACOS_ROOT="$REPO_ROOT/dux-macos"
readonly PROJECT_PATH="$MACOS_ROOT/Dux.xcodeproj"
readonly PROJECT_SPEC="$MACOS_ROOT/project.yml"
readonly ENTITLEMENTS_PATH="$MACOS_ROOT/Config/Release.entitlements"
readonly DEPLOYMENT_CHECK="$REPO_ROOT/scripts/check_macos_deployment_target.sh"
readonly RELEASE_PARENT="$REPO_ROOT/target/dux-macos-release"

release_succeeded=false
mounted_image=false
mount_point=""
staging_root=""

usage() {
    cat <<'EOF'
Build, sign, notarize, staple, and verify the primary DUX DMG.

Required environment:
  DUX_VERSION             Stable X.Y.Z version; HEAD must be tagged vX.Y.Z
  DUX_BUILD_NUMBER        Positive integer CFBundleVersion
  DUX_BUNDLE_IDENTIFIER   Frozen production identifier (the spike ID is rejected)
  DUX_TEAM_ID             Ten-character Apple Developer Team ID
  DUX_SIGNING_IDENTITY    Exact Developer ID Application identity in Keychain
  DUX_NOTARYTOOL_PROFILE  notarytool Keychain profile name

Create the Keychain profile interactively before release:
  xcrun notarytool store-credentials PROFILE_NAME

The script requires a clean tagged revision. It runs the repository release
gates, creates a universal macOS 14 app, signs code inside-out, notarizes and
staples the app, packages an Applications-link DMG, notarizes and staples the
DMG, verifies Gatekeeper, and atomically publishes immutable output under:
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

validate_inputs() {
    require_value DUX_VERSION "$release_version"
    require_value DUX_BUILD_NUMBER "$build_number"
    require_value DUX_BUNDLE_IDENTIFIER "$bundle_identifier"
    require_value DUX_TEAM_ID "$team_id"
    require_value DUX_SIGNING_IDENTITY "$signing_identity"
    require_value DUX_NOTARYTOOL_PROFILE "$notary_profile"

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
    [[ "$signing_identity" == "Developer ID Application: "*" ($team_id)" ]] \
        || die "DUX_SIGNING_IDENTITY must be a Developer ID Application identity for DUX_TEAM_ID"
    [[ "$signing_identity" != *$'\n'* ]] \
        || die "identity and profile values must be single-line strings"
    [[ "$signing_identity" != *$'\r'* ]] \
        || die "identity and profile values must be single-line strings"
    [[ "$notary_profile" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] \
        || die "DUX_NOTARYTOOL_PROFILE must be a safe Keychain profile name"
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

is_macho() {
    file -b "$1" | grep -q 'Mach-O'
}

assert_no_unreviewed_nested_bundles() {
    local app="$1"
    local nested
    nested="$(
        find "$app/Contents" -depth -type d \
            \( -name '*.app' -o -name '*.appex' -o -name '*.xpc' -o -name '*.bundle' \) \
            -print -quit
    )"
    [[ -z "$nested" ]] \
        || die "unreviewed nested code bundle requires an explicit entitlement/signing policy: $nested"
}

sign_nested_code() {
    local app="$1"
    local main_executable="$2"
    local candidate

    assert_no_unreviewed_nested_bundles "$app"

    while IFS= read -r candidate; do
        [[ "$candidate" != "$main_executable" ]] || continue
        if is_macho "$candidate"; then
            assert_exact_universal "$candidate"
            case "$candidate" in
                *.framework/*) continue ;;
            esac
            codesign --force --sign "$signing_identity" --timestamp \
                --options runtime --generate-entitlement-der "$candidate"
            codesign --verify --all-architectures --strict --verbose=2 "$candidate"
        fi
    done < <(find "$app/Contents" -depth -type f -print)

    while IFS= read -r candidate; do
        codesign --force --sign "$signing_identity" --timestamp \
            --generate-entitlement-der "$candidate"
        codesign --verify --all-architectures --strict --verbose=2 "$candidate"
    done < <(find "$app/Contents" -depth -type d -name '*.framework' -print)
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
    bash "$DEPLOYMENT_CHECK" "$executable"
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

verify_signed_app() {
    local app="$1"
    local executable="$app/Contents/MacOS/$(
        plutil -extract CFBundleExecutable raw -o - "$app/Contents/Info.plist"
    )"
    [[ -f "$executable" ]] || die "signed app main executable is missing"
    assert_exact_universal "$executable"
    bash "$DEPLOYMENT_CHECK" "$executable"

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

    xcrun notarytool submit "$artifact" \
        --keychain-profile "$notary_profile" \
        --wait --timeout 2h --output-format json >"$raw_response"
    jq -e '.status == "Accepted" and (.id | type == "string" and length > 0)' \
        "$raw_response" >/dev/null \
        || die "$label notarization was not accepted; response retained at $raw_response"

    local submission_id
    submission_id="$(jq -r '.id' "$raw_response")"
    xcrun notarytool log "$submission_id" \
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

main() {
    if [[ "${1:-}" == "--help" ]]; then
        usage
        exit 0
    fi
    if [[ "${1:-}" == "-h" ]]; then
        usage
        exit 0
    fi
    [[ "$#" -eq 0 ]] || die "unexpected argument '$1'; use --help"

    validate_inputs

    local command
    for command in awk cargo cat codesign cmp cp ditto file find git grep hdiutil \
        jq lipo ln mkdir mktemp mv plutil python3 readlink security shasum sort \
        spctl xcodebuild xcodegen xcrun; do
        require_command "$command"
    done

    verify_empty_entitlements "$ENTITLEMENTS_PATH"

    [[ -z "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=all)" ]] \
        || die "release requires a clean worktree, including untracked files"
    local release_tag="v$release_version"
    local head_commit
    local tag_commit
    head_commit="$(git -C "$REPO_ROOT" rev-parse HEAD)"
    tag_commit="$(git -C "$REPO_ROOT" rev-list -n 1 "$release_tag" 2>/dev/null || true)"
    [[ -n "$tag_commit" ]] || die "release tag does not exist: $release_tag"
    [[ "$tag_commit" == "$head_commit" ]] || die "HEAD must be exactly tagged $release_tag"

    local workspace_versions
    workspace_versions="$(
        cargo metadata --manifest-path "$REPO_ROOT/Cargo.toml" \
            --locked --no-deps --format-version 1 \
            | jq -r '.packages[] | select(.name | startswith("dux-")) | .version' \
            | sort -u
    )"
    [[ "$workspace_versions" == "$release_version" ]] \
        || die "all DUX workspace package versions must equal $release_version"

    local identity_inventory
    identity_inventory="$(security find-identity -v -p codesigning)"
    awk -v wanted="$signing_identity" \
        'index($0, "\"" wanted "\"") { found = 1 } END { exit !found }' \
        <<<"$identity_inventory" \
        || die "the requested Developer ID Application identity is not valid in Keychain"

    xcrun notarytool history --keychain-profile "$notary_profile" \
        --output-format json >/dev/null

    if [[ -L "$REPO_ROOT/target" ]]; then
        die "refusing a symlinked repository target directory"
    fi
    mkdir -p "$RELEASE_PARENT"
    [[ ! -L "$RELEASE_PARENT" ]] || die "refusing a symlinked release output parent"
    [[ "$(cd "$RELEASE_PARENT" && pwd -P)" == "$RELEASE_PARENT" ]] \
        || die "release output parent is not repository-owned"

    readonly output_dir="$RELEASE_PARENT/$release_tag"
    [[ ! -e "$output_dir" && ! -L "$output_dir" ]] \
        || die "immutable release output already exists: $output_dir"

    staging_root="$(mktemp -d "$RELEASE_PARENT/.staging.XXXXXX")"
    trap cleanup EXIT
    readonly debug_derived_data="$staging_root/DerivedData-Debug"
    readonly debug_app="$debug_derived_data/Build/Products/Debug/DUX.app"
    readonly swift_test_data="$staging_root/DerivedData-Tests"
    readonly derived_data="$staging_root/DerivedData"
    readonly unsigned_app="$derived_data/Build/Products/Release/DUX.app"
    readonly staged_app="$staging_root/DUX.app"
    readonly app_zip="$staging_root/DUX-$release_version.zip"
    readonly image_root="$staging_root/image-root"
    readonly dmg_name="DUX-$release_version.dmg"
    readonly publish_root="$staging_root/publish"
    readonly staged_dmg="$publish_root/$dmg_name"
    mkdir -p "$publish_root"

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
        dux-macos/Dux/Generated/DuxFFI.swift
    xcodebuild test -project "$PROJECT_PATH" -scheme Dux \
        -destination 'platform=macOS,arch=arm64' \
        -derivedDataPath "$swift_test_data" CODE_SIGNING_ALLOWED=NO
    xcodebuild -project "$PROJECT_PATH" -scheme Dux -configuration Debug \
        -destination 'generic/platform=macOS' -derivedDataPath "$debug_derived_data" \
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
        ARCHS='arm64 x86_64' ONLY_ACTIVE_ARCH=NO CODE_SIGNING_ALLOWED=NO \
        PRODUCT_BUNDLE_IDENTIFIER="$bundle_identifier" \
        MARKETING_VERSION="$release_version" \
        CURRENT_PROJECT_VERSION="$build_number" build

    verify_unsigned_app_shape "$unsigned_app"
    compare_app_layouts "$debug_app" "$unsigned_app"
    ditto "$unsigned_app" "$staged_app"
    local main_name
    main_name="$(plutil -extract CFBundleExecutable raw -o - "$staged_app/Contents/Info.plist")"
    local main_executable="$staged_app/Contents/MacOS/$main_name"
    [[ -f "$main_executable" ]] || die "main executable is missing from DUX.app"
    assert_exact_universal "$main_executable"
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
    dmg_checksum="$(shasum -a 256 "$staged_dmg" | awk '{print $1}')"
    printf '%s  %s\n' "$dmg_checksum" "$dmg_name" \
        >"$publish_root/$dmg_name.sha256"
    printf '%s\n' \
        "version=$release_version" \
        "build=$build_number" \
        "tag=$release_tag" \
        "commit=$head_commit" \
        "bundle_identifier=$bundle_identifier" \
        "team_id=$team_id" \
        "architectures=arm64,x86_64" \
        "deployment_target=14.0" \
        "dmg_sha256=$dmg_checksum" \
        >"$publish_root/release-manifest.txt"

    [[ ! -e "$output_dir" && ! -L "$output_dir" ]] \
        || die "release output appeared during the build: $output_dir"
    # DUX-DESTRUCTIVE: allow=release-dmg-publish-move -- atomically publish a new versioned directory only after every release verification succeeds
    mv "$publish_root" "$output_dir"
    release_succeeded=true
    echo "Published signed, notarized, and stapled release: $output_dir/$dmg_name"
}

release_version="${DUX_VERSION:-}"
build_number="${DUX_BUILD_NUMBER:-}"
bundle_identifier="${DUX_BUNDLE_IDENTIFIER:-}"
team_id="${DUX_TEAM_ID:-}"
signing_identity="${DUX_SIGNING_IDENTITY:-}"
notary_profile="${DUX_NOTARYTOOL_PROFILE:-}"
publish_root=""

main "$@"
