# DUX inspection CLI and JSON contract

This document defines schema version 1 of the noninteractive inspection and
candidate-review commands introduced during Milestones 2 and 9. It is distinct
from the DUX product version, SQLite schema, snapshot format, and UniFFI
protocol.

## Commands

```text
dux status [--json]
dux history [--json] [--limit N]
dux scan-detail --scan-id SCAN_ID [--offset N] [--limit N] [--json]
dux candidates --scan-id SCAN_ID [--cursor N] [--limit N] [--json]
dux review-state --scan-id SCAN_ID --candidate-id CANDIDATE_ID \
  --command select|clear-selection|dismiss|restore [--json]
dux cleanup-history list [--limit N] \
  [--after-started-at-unix-ms MS --after-session-id SESSION_ID] [--json]
dux cleanup-history show --session-id SESSION_ID [--json]
```

`--json` is explicit. Redirection or a non-TTY stdout does not change the
format. The existing `dux [PATH]` TUI remains the default. The noninteractive
names are reserved command names. Prefix a literal directory with `./` or place
it after `--`, for example `dux ./candidates` or `dux -- cleanup-history`.

These commands resolve shared engine data at the platform application-data and
cache locations and prepare missing standard parent directories on first use.
The core still validates the exact publication parent independently before
creating private DUX storage. On macOS these are:

```text
~/Library/Application Support/Dux/dux.sqlite3
~/Library/Application Support/Dux/snapshots/
~/Library/Caches/Dux/
```

The legacy TUI cache remains independent during the engine migration.

## Shared guarantees

- Every serialized JSON object contains `schema_version: 1`.
- Output contains no scan root, database path, snapshot path/name/digest, SQL,
  or underlying provider/storage message. `path_disclosure` is therefore
  `none`.
- Scan coverage output preserves only whether an issue was global, at the scan
  root, or below it. It deliberately omits stored relative name components.
  Candidate output omits paths and evidence payloads. Candidate IDs are stable
  local pseudonyms derived partly from path bytes, not anonymous identifiers
  suitable for remote disclosure.
- Durable history is an observation only. It is not current filesystem
  evidence, a cleanup candidate, a plan, approval, or execution authority.
- Candidate summaries and cleanup records are sensitive local history and are
  not AI- or remote-safe by default, even though their command schemas are
  path-free.
- Timestamps are non-negative Unix milliseconds and byte/count fields are
  non-negative 64-bit integers. JSON consumers must not assume IEEE-754 number
  precision is sufficient.
- Unknown allocation is `null`, never fabricated from logical bytes or encoded
  as zero.
- `snapshot_recorded` means a validated SQLite snapshot reference exists. It
  does not claim the referenced file currently exists or passes a fresh
  integrity check.
- A retained `running` row is reported as `running`; the CLI does not infer a
  crash or rewrite it as interrupted.

## `status` schema

The root object contains:

- `schema_version`, `command`, `dux_version`, and `path_disclosure`;
- versioned `database` compatibility facts;
- versioned `capabilities.scan_history`, which is `available` or
  `unavailable_newer_schema`;
- `latest_scan`, the most recently started durable record, or `null`.

`latest_scan: null` means no record only while scan history is available. A
newer database remains a successful status response with read-only
compatibility facts and an explicit unavailable capability.

## `history` schema

History version 1 is scan-only and ordered by
`started_at_desc_scan_id_asc`. `--limit` defaults to 20 and accepts 1 through
200. The response contains at most that many records and `has_more`; increase
the limit to inspect a larger recent window. When `has_more` remains true at
200, older records exist but are not addressable through schema v1. Version 1
deliberately does not freeze a cursor representation.

Each scan contains:

- its opaque scan ID and start/optional completion time;
- `queued`, `running`, `succeeded`, `failed`, `cancelled`, or `interrupted`;
- counts only for a succeeded scan; non-success counts are `null` rather than
  exposing SQLite defaults as measurements;
- the complete validated coverage summary, including measured permille,
  distinct issue-record count, and summed issue occurrences;
- `snapshot_recorded` with the limited meaning above.

No scans is a successful empty response (`items: []`, `has_more: false`).

## `scan-detail` schema

`scan-detail` reads one exact scan's bounded coverage-issue page. `--offset`
defaults to 0; `--limit` defaults to 20 and accepts 1 through 64. Results are
ordered by durable issue ordinal and contain:

- the scan ID, requested page, `has_more`, and optional `next_offset`;
- the complete coverage totals and measured-permille observation; and
- issue ordinal, typed kind, occurrence count, location scope, whether a
  location was recorded, and whether descendant context was truncated.

The command does not open a snapshot or expose any stored location component.
The issue page is historical coverage evidence only.

## `candidates` schema

`candidates` reads the exact scan's deterministic discovery observation.
`--cursor` is a zero-based immutable evaluation ordinal, defaults to 0, and
`--limit` defaults to 20 with a range of 1 through 64. Results are ordered by
stored evaluation ordinal.

The root preserves the source scan status and optional scheduling/completion
times. Its versioned `evaluation` distinguishes `not_run`, `pending`,
`succeeded`, and `failed`; only a successful evaluation can contain candidate
items. A candidate summary contains its opaque ID, rule ID/revision, category,
estimate, optional newest modification observation, safety tier, proposed
action, scheduling eligibility, path count, evidence kinds, blockers, creation
time, and durable review/lifecycle status. It contains neither a path nor an
evidence payload.

Pagination is applied only after the core has completely bounded and validated
the durable evaluation graph. `next_cursor: null` means the immutable
observation is exhausted.

## `review-state` schema

`review-state` is the sole state-changing command in this contract. It accepts
one exact scan/candidate pair and one semantic command: `select`,
`clear-selection`, `dismiss`, or `restore`. There is no arbitrary status
setter. DUX retains and validates the same exact snapshot-review lease used by
the app while the transition is made.

The result contains only the IDs, requested command, resulting durable status,
and `cleanup_performed: false`. It deliberately omits a previous status or
`changed` flag because a separate pre-read would not be atomic. Selection is
review intent only: it does not construct a plan, approve work, schedule work,
or authorize a filesystem effect. An `outcome_unknown` response must be
resolved by reading `candidates` again; callers must not automatically retry
the mutation.

## `cleanup-history` schemas

`cleanup-history list` returns a newest-first page ordered by
`started_at_desc_session_id_asc`. `--limit` defaults to 20 and accepts 1
through 64. Older pages use the exact two-part keyset cursor returned by
`next_cursor`; `--after-started-at-unix-ms` and `--after-session-id` must be
supplied together.

Each summary preserves the opaque session and plan IDs, complete-versus-legacy
record format, optional source scan, lifecycle times, mode, trigger, status,
estimated bytes, optional signed verified-capacity delta, optional cancellation
observation, graph totals, and exhaustive item/path status counts.

`cleanup-history show` selects one exact session ID copied from the list. It
returns the same validated summary, contiguous path-free item summaries, and
typed warnings. Legacy records keep unavailable policy fields as `null`; the
CLI never fabricates them. Both operations omit target paths, evidence
payloads, candidate IDs, execution owners, generations, heartbeats, claims,
receipts, and effect fences. A session ID is an observation selector only; no
cleanup, retry, recovery, approval, or executor operation accepts it.

## Failures and exit status

Successful JSON is written only to stdout. Runtime failures write one compact,
versioned error object to stderr when `--json` was requested; human commands
write one human-readable stderr line. Expected categories never contain paths,
SQL, or raw storage errors.

| Exit | Meaning |
| --- | --- |
| 0 | Success, including empty history and a clean broken pipe |
| 2 | Command-line usage or limit error (emitted by Clap) |
| 3 | Unavailable, unsafe, incompatible, corrupt, or resource-limited storage |
| 4 | Temporary storage contention |
| 70 | Unexpected internal failure |

Stable runtime error codes are `storage_busy`, `storage_unavailable`,
`unsafe_storage`, `incompatible_database_schema`,
`corrupt_database`, `query_limit_exceeded`, `scan_not_found`,
`candidate_not_found`, `candidate_not_reviewable`,
`cleanup_session_not_found`, `cursor_out_of_range`, `outcome_unknown`, and
`internal_error`. Only `storage_busy` is retryable. `outcome_unknown` is
explicitly non-retryable because repeating a mutation could misrepresent the
committed result. Only the code and `retryable` value are machine contracts;
message wording may evolve.

## Compatibility

Optional fields may be added compatibly to schema 1. Removing/renaming a field,
changing its type or meaning, changing history composition/order or cursor
meaning, weakening the path-disclosure policy, or changing null semantics
requires a root schema increment. Consumers must tolerate unknown enum strings.
Golden tests lock empty, populated, paged, legacy, newer-schema, and
structured-error shapes.
