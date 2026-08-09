# Permanent-safe cleanup qualification

Status: required macOS CI gate; public Release action remains disabled.

## Purpose

This lane proves that the production Rust-target cleanup capability crosses the
Rust API layer of the `dux-ffi` crate without accepting caller-supplied
filesystem authority. It complements the ordinary parallel workspace,
generated-binding, and native test suites. It does not exercise Swift/C ABI
marshalling and does not authorize another cleanup rule, platform, scheduler,
CLI command, or AI consumer.

The tests operate only on disposable directories they create for themselves.
They never inspect or remove an existing project `target` directory or other
user data.

## CI entry point

The macOS FFI app job runs:

```sh
./scripts/test_ffi_rust_target_cleanup.sh
```

The workflow gives this step a 15-minute deadline and does not permit failure.
The harness first asks Cargo, with terminal color explicitly disabled, for the
one exact `dux-ffi` unit-test executable, then invokes that executable directly.
The explicit color setting keeps the parser stable under CI's global
`CARGO_TERM_COLOR=always`. Direct invocation ensures no Cargo or rustc process
from the build command remains alive while the process-quiescence guard is
being tested.

The two effect-capable fixtures remain `#[ignore]` in the ordinary test binary.
The harness requires exactly one successful execution of each fully qualified
test name and rejects missing, duplicated, filtered, ignored, or failed output.
No wildcard, caller-selected test, caller-selected path, or retry is accepted.

## Evidence covered

The success fixture proves:

- foreign-engine and same-store review handles cannot start the task;
- the owning engine consumes the exact opaque review once;
- the FFI task result and exact-session cleanup history correlate through a
  path-free session identifier;
- the disposable stale Cargo payload is removed;
- `CACHEDIR.TAG`, `Cargo.toml`, `Cargo.lock`, and project source remain; and
- a terminal operation cannot be retried through task cancellation.

The refusal fixture proves:

- an admitted start participates in the shared operation tracker;
- the review becomes unavailable as soon as the owning start consumes it;
- a typed busy refusal cannot restore or reuse that authority; and
- engine close waits for the admitted start boundary to drain.

The broader focused core and Swift suites separately cover final-inventory
drift, symlinks, hard links, recent content, cancellation, partial and unknown
outcomes, journal settlement/quarantine, exact confirmation binding, close
survival, shutdown quiescence, and path-free presentation. Those layers remain
required; this lane does not replace them.

## What this does not prove

This lane is unsigned and runs on a disposable CI workspace. It does not prove
a signed/notarized stable-install flow, the separate ADR 0010 read-only active-
cleanup census, an active private vulnerability-reporting channel, or public
Release UI availability. ADR 0011 deliberately retains prior-boot and other
unproven cleanup debt as non-executable rather than reconciling it; this lane
does not widen or independently prove that policy boundary. It does not test
cleanup against existing user data. The Release action must remain compiled
out until every item in `SECURITY_DESIGN.md` section 17.3 is closed and recorded
independently.

## Local use

Run the same command on macOS only when no unrelated `cargo` or `rustc` process
is active. A refusal caused by an active guarded process is a valid product
safety response but does not satisfy this qualification lane. Do not bypass the
process guard; stop the unrelated build and rerun the exact harness.
