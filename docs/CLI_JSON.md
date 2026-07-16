# DUX inspection CLI and JSON contract

This document defines schema version 1 of the noninteractive inspection
commands introduced during Milestone 2. It is distinct from the DUX product
version, SQLite schema, snapshot format, and UniFFI protocol.

## Commands

```text
dux status [--json]
dux history [--json] [--limit N]
```

`--json` is explicit. Redirection or a non-TTY stdout does not change the
format. The existing `dux [PATH]` TUI remains the default. Because `status` and
`history` are now command names, use `dux ./status` or `dux -- status` to scan a
directory with one of those literal names.

The commands resolve shared engine data at the platform application-data and
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
- Durable history is an observation only. It is not current filesystem
  evidence, a cleanup candidate, a plan, approval, or execution authority.
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
`corrupt_database`, `query_limit_exceeded`, and `internal_error`. Only the code
and `retryable` value are machine contracts; message wording may evolve.

## Compatibility

Optional fields may be added compatibly to schema 1. Removing/renaming a field,
changing its type or meaning, changing history composition/order, weakening the
path-disclosure policy, or changing null semantics requires a root schema
increment. Consumers must tolerate unknown enum strings. Golden tests lock the
empty, populated, newer-schema, and structured-error shapes.
