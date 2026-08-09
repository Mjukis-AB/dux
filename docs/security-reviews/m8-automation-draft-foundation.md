# M8 automation draft-foundation security review

- Review date: 2026-08-09
- Scope: SQLite schema v20, core automation-draft domain/persistence/engine,
  UniFFI v62, and the native read-only Automations Settings section
- Result: approved only as an inert disabled-draft foundation

## Decision

This checkpoint may store and present automation preferences, but it creates no
automation or cleanup authority. A draft is not a scheduled operation, saved
cleanup approval, task, trigger, plan, or candidate selection.

The previous `schedules` table was reserved in schema v1 but had no shipped
reader, writer, scheduler, Settings control, or executor. Its rows therefore
cannot prove consent. Schema v20 deliberately discards that unadmitted shape
and replaces it with a strict representation whose only valid state is
`disabled_draft`.

## Admitted data

One draft contains only:

- a core-generated opaque ID and optimistic revision;
- an exact rule ID/revision or code-owned category scope;
- weekly, monthly, or low-disk-only cadence preference;
- minimum age, minimum reclaimable bytes, and maximum bytes per run;
- a bounded canonical set of exact rule revisions excluded from a category;
- pre-run notification preference and remaining first-run notices;
- confirmation-required or fully-automatic preference; and
- creation/update times.

The registry is capped at 64 drafts and 32 rule exclusions per draft. It has no
path, node, scan, candidate, plan, approval, journal, cleanup session, task,
trigger, last-run, next-run, AI, callback, or effect field.

`fully_automatic` is inert preference data in this checkpoint. It does not
open either runtime gate and cannot be consumed by a scheduler or executor.

## Mutation and compatibility boundary

- Reads do not create rows or rewrite defaults.
- IDs are generated inside core; clients cannot nominate a persisted ID on
  creation.
- Replacement and deletion require the exact positive revision last reviewed
  by the caller.
- Canonical list/exclusion ordering, row bounds, timestamps, scopes, and
  numeric limits are validated when reading as well as writing.
- Every mutation reconciles an uncertain commit against the exact original and
  expected state; an unprovable outcome is returned as `OutcomeUnknown` and is
  never retried automatically.
- The v20 migration is checksummed and fingerprinted. It preserves all
  non-schedule data and discards only never-admitted legacy schedule rows.

## Runtime and client boundary

Core owns the complete overview and reports both `global_enabled=false` and
`execution_available=false`. UniFFI v62 repeats those checks and emits
`enabled=false` for every draft. Swift validates the record version, IDs,
limits, canonical ordering, timestamps, notification state, and all three
closed gates before publishing anything.

The native UI is read-only. It explains that drafts cannot run cleanup and
offers no enable, run, trigger, cadence editor, or master-switch control. No
shipped candidate rule is currently marked schedule-eligible, so the overview
truthfully reports zero statically eligible rules.

The DUX CLI, `AppRuntime`, `MaintenanceScheduler`,
`CapacitySamplingScheduler`, AI path, planner, journal, and cleanup executor do
not import or consume a draft. The existing maintenance scheduler continues to
coordinate private-store upkeep only and is not user automation.

## Required later reviews

This approval does not cover schedule creation/editing in Settings, static or
runtime eligibility, history-based suggestions, a persisted global kill
switch, enabling/pausing, wake handling, timing, notifications, low-battery or
thermal gates, missed-run behavior, fresh planning/revalidation, execution,
failure pausing, or result history. Each remains subject to the complete
ROADMAP §15, SECURITY_DESIGN §10, and Milestone 8 exit criteria.

## Verification evidence

- `dux-core`: 1,646 tests passed with four intentional platform/performance
  ignores in one interference-free serialized run.
- `dux-ffi`: all 141 runnable tests passed with two intentional isolated
  Rust-target cleanup ignores.
- Native app: all 867 hosted tests passed with no failures or skips.
- Repository policy: all 129 Python tests passed; the destructive-call audit
  covered 404 source files with no violation.
- Rust formatting, workspace check, and warning-denied Clippy passed.
- Debug and Release binding generation emitted byte-identical Swift at
  SHA-256
  `0919eda855512644623096495ad2e88ec08890a0ba783ce0e70ab861c880c27f`.
- Clean Debug and Release macOS apps built as exact arm64/x86_64 universals,
  target macOS 14.0, preserve `LSUIElement=true`, and embed Sparkle 2.9.5 with
  the frozen DUX public key. Both apps contain byte-identical Sparkle and
  bundled CLI payloads; the CLI SHA-256 is
  `f12b9f2737ff705b1af95edb65a506ee9dd4a32d9105b13ecb6708c60381f307`.
