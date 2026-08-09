#!/usr/bin/env bash

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: macOS is required for APFS boundary qualification" >&2
    exit 1
fi
if (( $# != 0 )); then
    echo "error: APFS boundary qualification accepts no arguments" >&2
    exit 2
fi

for command_name in cargo hdiutil mktemp python3; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "error: required command is unavailable: $command_name" >&2
        exit 1
    fi
done

readonly SCRIPT_DIRECTORY="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPOSITORY_ROOT="$(cd "$SCRIPT_DIRECTORY/.." && pwd -P)"
readonly TEST_NAME="path_validation::protected::tests::macos_apfs_boundary_qualification_fails_closed_after_nested_mount"
readonly ACCOUNT_HOME="$(python3 -c 'import os, pwd, sys; sys.stdout.write(pwd.getpwuid(os.geteuid()).pw_dir)')"

if [[ -z "$ACCOUNT_HOME" || ! -d "$ACCOUNT_HOME" ]]; then
    echo "error: the OS account database did not return an absolute existing home" >&2
    exit 1
fi
case "$ACCOUNT_HOME" in
    /*) ;;
    *)
        echo "error: the OS account database did not return an absolute existing home" >&2
        exit 1
        ;;
esac

readonly FIXTURE_ROOT="$(mktemp -d "$ACCOUNT_HOME/.dux-apfs-boundary-qualification.XXXXXX")"
readonly MOUNT_POINT="$FIXTURE_ROOT/scan-root"
readonly CONTROL_DIRECTORY="$FIXTURE_ROOT/control"
readonly DISK_IMAGE="$FIXTURE_ROOT/disposable-apfs.dmg"
readonly READY_FILE="$CONTROL_DIRECTORY/ready"
readonly CONTINUE_FILE="$CONTROL_DIRECTORY/mounted"
readonly CREATE_LOG="$CONTROL_DIRECTORY/hdiutil-create.log"
readonly ATTACH_LOG="$CONTROL_DIRECTORY/hdiutil-attach.log"

test_pid=""
detach_required=0
fixture_present=1

remove_fixture() {
    # DUX-DESTRUCTIVE: allow=qualification-apfs-fixture-cleanup -- remove only the harness-owned disposable image and empty control fixture after detaching it
    rm -rf -- "$FIXTURE_ROOT"
    fixture_present=0
}

cleanup() {
    local status=$?
    local preserve_fixture=0
    trap - EXIT INT TERM HUP

    if [[ -n "$test_pid" ]] && kill -0 "$test_pid" >/dev/null 2>&1; then
        kill "$test_pid" >/dev/null 2>&1 || true
        wait "$test_pid" >/dev/null 2>&1 || true
    fi
    if (( detach_required == 1 )); then
        if ! hdiutil detach "$MOUNT_POINT" >/dev/null; then
            echo "error: could not detach the disposable APFS image; fixture preserved at $FIXTURE_ROOT" >&2
            preserve_fixture=1
            status=1
        fi
    fi
    if (( preserve_fixture == 0 && fixture_present == 1 )); then
        remove_fixture
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

mkdir -m 700 "$MOUNT_POINT" "$CONTROL_DIRECTORY"
if ! hdiutil create \
    -size 128m \
    -fs APFS \
    -type UDIF \
    -volname DUXAPFSBoundaryQualification \
    "$DISK_IMAGE" > "$CREATE_LOG" 2>&1; then
    echo "error: could not create the disposable APFS image" >&2
    exit 1
fi

(
    cd "$REPOSITORY_ROOT"
    exec env \
        DUX_APFS_QUALIFICATION_MOUNT_POINT="$MOUNT_POINT" \
        DUX_APFS_QUALIFICATION_CONTROL_DIRECTORY="$CONTROL_DIRECTORY" \
        cargo test --quiet --locked -p dux-core --lib "$TEST_NAME" -- --exact --ignored --nocapture
) &
test_pid=$!

deadline=$((SECONDS + 30))
while [[ ! -f "$READY_FILE" ]]; do
    if ! kill -0 "$test_pid" >/dev/null 2>&1; then
        set +e
        wait "$test_pid"
        test_status=$?
        set -e
        test_pid=""
        echo "error: APFS qualification test exited before requesting the mount (status $test_status)" >&2
        exit 1
    fi
    if (( SECONDS >= deadline )); then
        echo "error: APFS qualification test did not become ready within 30 seconds" >&2
        exit 1
    fi
    sleep 0.05
done

detach_required=1
if ! hdiutil attach \
    -nobrowse \
    -noautoopen \
    -mountpoint "$MOUNT_POINT" \
    "$DISK_IMAGE" > "$ATTACH_LOG" 2>&1; then
    echo "error: could not attach the disposable APFS image" >&2
    exit 1
fi
printf 'mounted\n' > "$CONTINUE_FILE"

set +e
wait "$test_pid"
test_status=$?
set -e
test_pid=""
if (( test_status != 0 )); then
    echo "error: APFS boundary qualification failed (status $test_status)" >&2
    exit "$test_status"
fi

if ! hdiutil detach "$MOUNT_POINT" >/dev/null; then
    echo "error: could not detach the disposable APFS image; fixture preserved at $FIXTURE_ROOT" >&2
    exit 1
fi
detach_required=0
remove_fixture

echo "APFS boundary qualification passed."
echo "same_mount_descendant=accepted"
echo "symlink_alias=refused"
echo "system_data_home_spelling=refused"
echo "retained_witness_after_nested_mount=changed"
echo "nested_mount_home_witness=refused"
echo "home_relative_cross_volume_snapshot=refused"
