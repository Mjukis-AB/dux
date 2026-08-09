#!/bin/bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"

build_output=$(cargo test --color never -p dux-ffi --no-run --locked 2>&1)
printf '%s\n' "$build_output"
test_binaries=$(
    printf '%s\n' "$build_output" |
        sed -n 's/^  Executable unittests src\/lib\.rs (\(.*\))$/\1/p'
)
test_binary_count=$(printf '%s\n' "$test_binaries" | sed '/^$/d' | wc -l | tr -d ' ')

if [[ "$test_binary_count" != 1 ]]; then
    echo "expected exactly one Cargo-reported dux-ffi test binary" >&2
    exit 1
fi
test_binary=$test_binaries
if [[ "$test_binary" != /* ]]; then
    test_binary="$repo_root/$test_binary"
fi
if [[ ! -x "$test_binary" || -d "$test_binary" ]]; then
    echo "Cargo reported an invalid dux-ffi test binary" >&2
    exit 1
fi

validate_exact_test_output() {
    local output=$1
    local test_name=$2
    case "$output" in
    *"running 1 test"*"test $test_name ... ok"*"test result: ok. 1 passed; 0 failed; 0 ignored;"*)
        return 0
        ;;
    *)
        return 1
        ;;
    esac
}

success_status=0
# DUX-DESTRUCTIVE: allow=test-ffi-cleanup-success-binary -- execute only the compiled exact FFI cleanup success regression
success_output=$("$test_binary" \
    --ignored \
    --exact \
    tests::rust_target_cleanup_is_engine_bound_consume_once_path_free_and_history_correlated 2>&1) ||
    success_status=$?
printf '%s\n' "$success_output"
if [[ "$success_status" != 0 ]] ||
    ! validate_exact_test_output "$success_output" tests::rust_target_cleanup_is_engine_bound_consume_once_path_free_and_history_correlated; then
    echo "the exact FFI cleanup success regression did not run and pass once" >&2
    exit 1
fi

refusal_status=0
# DUX-DESTRUCTIVE: allow=test-ffi-cleanup-refusal-binary -- execute only the compiled exact FFI cleanup refusal regression
refusal_output=$("$test_binary" \
    --ignored \
    --exact \
    tests::rust_target_cleanup_refusal_is_one_shot_and_close_drains_start_operation 2>&1) ||
    refusal_status=$?
printf '%s\n' "$refusal_output"
if [[ "$refusal_status" != 0 ]] ||
    ! validate_exact_test_output "$refusal_output" tests::rust_target_cleanup_refusal_is_one_shot_and_close_drains_start_operation; then
    echo "the exact FFI cleanup refusal regression did not run and pass once" >&2
    exit 1
fi
