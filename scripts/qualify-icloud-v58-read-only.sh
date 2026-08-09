#!/usr/bin/env bash

set -euo pipefail
umask 077

if (( $# != 0 )); then
    echo "error: iCloud v58 read-only qualification accepts no arguments" >&2
    exit 2
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: iCloud v58 read-only qualification requires macOS" >&2
    exit 1
fi

for command_path in \
    /usr/bin/arch \
    /usr/bin/env \
    /usr/bin/git \
    /usr/bin/lockf \
    /usr/bin/python3 \
    /usr/bin/shasum \
    /usr/bin/sw_vers \
    /usr/bin/xcodebuild; do
    if [[ ! -x "$command_path" ]]; then
        echo "error: a required qualification tool is unavailable" >&2
        exit 1
    fi
done

readonly SCRIPT_DIRECTORY="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly REPOSITORY_ROOT="$(cd "$SCRIPT_DIRECTORY/.." && pwd -P)"
readonly VALIDATOR="$SCRIPT_DIRECTORY/validate-icloud-v58-evidence.py"
readonly SCHEMA_PATH="$REPOSITORY_ROOT/spikes/icloud-v58-read-only-qualification/evidence-v1.schema.json"
readonly PROJECT_PATH="$REPOSITORY_ROOT/dux-macos/Dux.xcodeproj"

if [[ ! -f "$VALIDATOR" || ! -f "$SCHEMA_PATH" || ! -d "$PROJECT_PATH" ]]; then
    echo "error: the qualification contract is incomplete" >&2
    exit 1
fi

readonly TOP_LEVEL="$(/usr/bin/git -C "$REPOSITORY_ROOT" rev-parse --show-toplevel 2>/dev/null)"
if [[ "$TOP_LEVEL" != "$REPOSITORY_ROOT" ]]; then
    echo "error: qualification must run from its exact repository" >&2
    exit 1
fi
if [[ -n "$(/usr/bin/git -C "$REPOSITORY_ROOT" status --porcelain=v1 --untracked-files=all)" ]]; then
    echo "error: qualification requires an exactly clean worktree" >&2
    exit 1
fi
readonly SOURCE_COMMIT="$(/usr/bin/git -C "$REPOSITORY_ROOT" rev-parse --verify 'HEAD^{commit}')"
if [[ ! "$SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
    echo "error: qualification could not bind the source commit" >&2
    exit 1
fi

readonly PRODUCT_VERSION="$(/usr/bin/sw_vers -productVersion)"
readonly PRODUCT_BUILD="$(/usr/bin/sw_vers -buildVersion)"
readonly ARCHITECTURE="$(/usr/bin/arch)"
if [[ ! "$PRODUCT_VERSION" =~ ^[0-9]+(\.[0-9]+){1,3}$ ]]; then
    echo "error: qualification could not bind the macOS version" >&2
    exit 1
fi
readonly PRODUCT_MAJOR="${PRODUCT_VERSION%%.*}"
if (( PRODUCT_MAJOR < 14 )); then
    echo "error: iCloud v58 qualification requires macOS 14 or newer" >&2
    exit 1
fi
if [[ ! "$PRODUCT_BUILD" =~ ^[A-Za-z0-9._-]{1,31}$ ]]; then
    echo "error: qualification could not bind the macOS build" >&2
    exit 1
fi
case "$ARCHITECTURE" in
    arm64 | x86_64) ;;
    *)
        echo "error: qualification requires a supported Mac architecture" >&2
        exit 1
        ;;
esac

readonly ACCOUNT_HOME="$(/usr/bin/python3 -c 'import os, pwd, sys; sys.stdout.write(pwd.getpwuid(os.geteuid()).pw_dir)')"
if [[ ! "$ACCOUNT_HOME" =~ ^/ || ! -d "$ACCOUNT_HOME" ]]; then
    echo "error: qualification could not bind the OS account home" >&2
    exit 1
fi
readonly SCHEMA_OUTPUT="$(/usr/bin/shasum -a 256 "$SCHEMA_PATH")"
readonly SCHEMA_SHA256="${SCHEMA_OUTPUT%% *}"
if [[ ! "$SCHEMA_SHA256" =~ ^[0-9a-f]{64}$ ]]; then
    echo "error: qualification could not bind the evidence schema" >&2
    exit 1
fi
readonly RUN_STARTED_AT_UNIX_NS="$(/usr/bin/python3 -c 'import time; print(time.time_ns())')"
if [[ ! "$RUN_STARTED_AT_UNIX_NS" =~ ^[1-9][0-9]{17,18}$ ]]; then
    echo "error: qualification could not bind the run start" >&2
    exit 1
fi

/usr/bin/python3 "$VALIDATOR" preflight

readonly STATE_DIRECTORY="$DUX_ICLOUD_PRIVATE_STATE_DIRECTORY"
readonly LOCK_PATH="$STATE_DIRECTORY/qualification-v1.lock"
readonly DERIVED_DATA_PATH="$STATE_DIRECTORY/derived-data-v1"
readonly LIVE_TEST="DuxICloudQualificationTests/ICloudV58ReadOnlyQualificationTests/testLiveReadOnlyQualification"

echo "Starting isolated read-only iCloud v58 qualification."
/usr/bin/lockf -kn "$LOCK_PATH" \
    /usr/bin/env -i \
        PATH="/usr/bin:/bin:/usr/sbin:/sbin" \
        HOME="$ACCOUNT_HOME" \
        TMPDIR="/private/tmp" \
        LANG="C" \
        LC_ALL="C" \
        DUX_ICLOUD_REAL_DEVICE_TESTS="1" \
        DUX_ICLOUD_DISPOSABLE_ACCOUNT_CONFIRMED="YES" \
        DUX_ICLOUD_DISPOSABLE_FIXTURE_CONFIRMED="YES" \
        DUX_ICLOUD_EXTERNAL_CONTENT_REFERENCE_CONFIRMED="YES" \
        DUX_ICLOUD_FIXTURE_ROOT="$DUX_ICLOUD_FIXTURE_ROOT" \
        DUX_ICLOUD_FIXTURE_PATH="$DUX_ICLOUD_FIXTURE_PATH" \
        DUX_ICLOUD_ACCOUNT_LABEL="$DUX_ICLOUD_ACCOUNT_LABEL" \
        DUX_ICLOUD_FIXTURE_LABEL="$DUX_ICLOUD_FIXTURE_LABEL" \
        DUX_ICLOUD_PHASE="$DUX_ICLOUD_PHASE" \
        DUX_ICLOUD_NETWORK="$DUX_ICLOUD_NETWORK" \
        DUX_ICLOUD_EXPECTED_SYNC_ELIGIBILITY="$DUX_ICLOUD_EXPECTED_SYNC_ELIGIBILITY" \
        DUX_ICLOUD_EXPECTED_IDENTITY_READINESS="$DUX_ICLOUD_EXPECTED_IDENTITY_READINESS" \
        DUX_ICLOUD_PRIVATE_STATE_DIRECTORY="$STATE_DIRECTORY" \
        DUX_ICLOUD_EVIDENCE_OUTPUT="$DUX_ICLOUD_EVIDENCE_OUTPUT" \
        DUX_ICLOUD_SOURCE_COMMIT="$SOURCE_COMMIT" \
        DUX_ICLOUD_EVIDENCE_SCHEMA_SHA256="$SCHEMA_SHA256" \
        DUX_ICLOUD_RUN_STARTED_AT_UNIX_NS="$RUN_STARTED_AT_UNIX_NS" \
        DUX_ICLOUD_EXPECTED_OS_VERSION="$PRODUCT_VERSION" \
        DUX_ICLOUD_EXPECTED_OS_BUILD="$PRODUCT_BUILD" \
        DUX_ICLOUD_EXPECTED_ARCHITECTURE="$ARCHITECTURE" \
        DUX_ICLOUD_EXCLUSIVE_SERIALIZATION="1" \
        /usr/bin/xcodebuild \
            -project "$PROJECT_PATH" \
            -scheme DuxICloudReadOnlyQualification \
            -configuration Debug \
            -derivedDataPath "$DERIVED_DATA_PATH" \
            -destination 'platform=macOS' \
            -disableAutomaticPackageResolution \
            -onlyUsePackageVersionsFromResolvedFile \
            -only-testing:"$LIVE_TEST" \
            test

if [[ "$(/usr/bin/git -C "$REPOSITORY_ROOT" rev-parse --verify 'HEAD^{commit}')" != "$SOURCE_COMMIT" \
    || -n "$(/usr/bin/git -C "$REPOSITORY_ROOT" status --porcelain=v1 --untracked-files=all)" ]]; then
    echo "error: qualification source changed while the test was running" >&2
    exit 1
fi

DUX_ICLOUD_SOURCE_COMMIT="$SOURCE_COMMIT" \
DUX_ICLOUD_EVIDENCE_SCHEMA_SHA256="$SCHEMA_SHA256" \
DUX_ICLOUD_RUN_STARTED_AT_UNIX_NS="$RUN_STARTED_AT_UNIX_NS" \
DUX_ICLOUD_EXPECTED_OS_VERSION="$PRODUCT_VERSION" \
DUX_ICLOUD_EXPECTED_OS_BUILD="$PRODUCT_BUILD" \
DUX_ICLOUD_EXPECTED_ARCHITECTURE="$ARCHITECTURE" \
DUX_ICLOUD_EXCLUSIVE_SERIALIZATION="1" \
    /usr/bin/python3 "$VALIDATOR" evidence

echo "iCloud v58 read-only qualification evidence passed."
