# Snapshot performance fixtures

This document defines the reproducible performance evidence for DUX's immutable
snapshot review path. Measurements are observations from isolated optimized
runs, not product latency promises and not substitutes for structural memory,
page, and lease bounds.

## Scope

The generated fixtures use the real production path without creating millions
of filesystem entries:

1. build one canonical depth-first `SnapshotDocument` in memory;
2. record its real durable running scan;
3. stream it through checksummed, no-replace snapshot publication;
4. drop the source document;
5. acquire the exact durable Explorer review lease;
6. decode and index the retained immutable file;
7. request bounded pages, treemap cells, and Large Files;
8. explicitly release the review and close the engine.

The fixture never creates a cleanup plan, resolves a live path, supplies an AI
input, or grants mutation authority. Fixture construction time is reported
separately from product-path timings.

Two shapes expose different risks:

- `balanced`: 999 root directories distribute the remaining file nodes. An
  exact one-million-node run is one root, 999 directories, and 999,000 files;
  every sortable fan-out is at most 1,000.
- `wide`: one root has 999,999 file children. This measures the worst admitted
  direct-child ordering and treemap path instead of hiding it behind a balanced
  tree.

Normal CI runs 10,000-node balanced and wide correctness fixtures. Exact large
runs are ignored by ordinary tests and must run alone in Release with one test
thread. This avoids turning a memory observation into a flaky parallel unit
test.

## Commands

Balanced one million:

```bash
/usr/bin/time -l cargo test --release -p dux-core --lib --locked \
  engine::registry::snapshot_performance_tests::million_node_snapshot_review_performance_fixture \
  -- --ignored --exact --nocapture --test-threads=1
```

Wide one million:

```bash
DUX_SNAPSHOT_PERF_SHAPE=wide \
/usr/bin/time -l cargo test --release -p dux-core --lib --locked \
  engine::registry::snapshot_performance_tests::million_node_snapshot_review_performance_fixture \
  -- --ignored --exact --nocapture --test-threads=1
```

Balanced five million:

```bash
DUX_SNAPSHOT_PERF_NODES=5000000 \
/usr/bin/time -l cargo test --release -p dux-core --lib --locked \
  engine::registry::snapshot_performance_tests::million_node_snapshot_review_performance_fixture \
  -- --ignored --exact --nocapture --test-threads=1
```

Each successful lane emits one `DUX_SNAPSHOT_PERFORMANCE_JSON=` record. On
macOS, `/usr/bin/time -l` reports maximum resident set size in bytes. Compile
the Release test first or repeat the command before recording RSS so a Rust
compiler child is not mistaken for fixture memory.

The `Snapshot performance` workflow runs balanced 1M weekly and offers manual
balanced-1M, wide-1M, and balanced-5M choices. It uploads the complete JSON and
resource log for 30 days. No workflow updates baselines automatically.

## Initial baseline

Measured 2026-07-18 on an Apple M1 Max MacBook Pro with 64 GB memory, macOS
26.5, arm64, and optimized Rust 1.96.0. All values are one isolated run; `ms`
values are integer wall-clock observations from the version-1 report.

| Shape | Nodes | Wire bytes | Peak RSS | Build | Publish | First review | First page | Cached page | Treemap | Large Files | Outcome |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| balanced | 1,000,000 | 123,996,236 | 422,805,504 | 172 ms | 8,609 ms | 1,510 ms | 1 ms | 1 ms | 1 ms | 6 ms | reviewed |
| wide | 1,000,000 | 124,000,228 | 459,816,960 | 175 ms | 9,172 ms | 1,649 ms | 7 ms | 1 ms | 15 ms | 6 ms | reviewed |
| balanced | 5,000,000 | 619,996,236 | 2,010,857,472 | 843 ms | 43,718 ms | 0 ms | — | — | — | — | typed review-budget refusal |

`Publish` includes validation, streamed encoding, immutable publication,
durability, and exact durable completion. `First review` includes retained-file
decode, checksum/graph validation, and child-index construction. These blocking
operations remain on DUX's serial utility executor rather than the main actor.

The native 64-cell layout has a separate repeated 60 Hz regression. Core pages
remain capped at 200 records and treemaps at 64 cells, so no million-node object
graph crosses UniFFI or enters SwiftUI state.

## Decisions and budgets

- The measured 999,999-child page required 7 ms and its 48-cell treemap
  required 15 ms on the baseline host. DUX therefore admits up to 999,999
  sortable direct children, retains one compact sorted `u32` index, and repeats
  exact lease/file validation after the potentially long sort or aggregation.
  Greater fan-out still fails with the existing typed, path-free resource error.
- Wall-clock observations are not unit-test assertions. Host load, storage, and
  durability vary. Correctness gates enforce exact counts, bounded result sizes,
  deterministic ordering, treemap accounting, typed resource refusal, lease
  release, and main-thread layout cost.
- Five million nodes remain a valid v1 persistence format but are not admitted
  to the current decoded Explorer cache. The 619,996,236-byte wire charges
  1,859,988,708 bytes under the conservative `wire × 3` estimate, above the
  fixed 1 GiB per-engine decoded-review budget. The measured publication lane
  already peaked near 2.01 GB. Raising the budget would convert a deliberate
  refusal into multi-gigabyte UI-process risk, so the fixture proves the refusal
  occurs before decode. Interactive 5M review requires a future streaming or
  indexed snapshot representation with its own format, migration, memory, and
  malformed-input review; this measurement does not authorize a budget increase.

When comparing later observations, investigate material regression in repeated
Release runs. Do not relax safety bounds or record a faster number merely to
make a noisy host pass.
