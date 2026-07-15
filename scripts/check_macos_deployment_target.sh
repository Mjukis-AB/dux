#!/bin/bash

set -euo pipefail

readonly EXPECTED_VERSION="14.0"

if [[ "$#" -eq 0 ]]; then
    echo "usage: $0 <Mach-O artifact> [...]" >&2
    exit 2
fi

if ! command -v xcrun >/dev/null 2>&1; then
    echo "error: required command 'xcrun' was not found" >&2
    exit 1
fi

for artifact in "$@"; do
    if [[ ! -f "$artifact" ]]; then
        echo "error: Mach-O artifact was not found: $artifact" >&2
        exit 1
    fi

    build_versions="$(xcrun vtool -show-build "$artifact")"
    minimum_versions="$(
        awk '$1 == "minos" { print $2 }' <<<"$build_versions" \
            | sort -u
    )"

    if [[ "$minimum_versions" != "$EXPECTED_VERSION" ]]; then
        echo "error: '$artifact' does not target exactly macOS $EXPECTED_VERSION" >&2
        echo "reported minimum versions: ${minimum_versions:-none}" >&2
        exit 1
    fi

    echo "Verified $artifact targets macOS $EXPECTED_VERSION"
done
