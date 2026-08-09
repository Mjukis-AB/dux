# iCloud local-copy real-device protocol

## Purpose

This protocol characterizes the public macOS identity and synchronization
facts needed by ADR 0006. It does not grant cleanup authority. Contract v45 is
read-only; contract v58 adds read-only public File Provider domain and item
stability classifications without transporting or persisting either raw
identifier.

Run this protocol only on a dedicated physical test Mac or disposable test
volume using a dedicated disposable Apple Account and disposable iCloud Drive
files. Never use a personal or production account, irreplaceable data, or an
ordinary CI worker.

## Gates

The read-only capability suite requires all of the following:

- an explicit `DUX_ICLOUD_REAL_DEVICE_TESTS=1` opt-in;
- a dedicated disposable Apple Account named in the test record;
- a test root created solely for DUX under iCloud Drive;
- macOS 14 and the newest macOS version supported by the release;
- network connectivity plus a documented offline phase;
- no parallel DUX iCloud test run;
- captured OS/build, hardware, account-fixture, and DUX commit identifiers;
- manual confirmation that every fixture is disposable.

The read-only flag MUST NOT authorize
`FileManager.evictUbiquitousItem(at:)`,
`NSFileProviderManager.evictItem(identifier:)`, deletion, Trash, conflict
creation, or account mutation. A future destructive suite requires a separate
`DUX_ICLOUD_DESTRUCTIVE_TESTS=1` opt-in, an additional interactive confirmation
naming the disposable test root, and a reviewed executor that cannot address
files outside that root. Automation MUST fail closed when either scope proof is
missing.

## Fixture

Create uniquely named fixtures inside the dedicated test root:

1. a small uploaded current file;
2. a larger uploaded current file with known local allocation;
3. a nested uploaded current file;
4. a shared item, if the account configuration supports one;
5. an item excluded from synchronization, if public UI or API allows the state;
6. an item with upload or download activity long enough to observe;
7. a remote-only placeholder created by a separate device;
8. a conflict fixture created only by the separately reviewed test procedure.

Record hashes of test content outside iCloud Drive. Do not put paths, filenames,
contents, archived identity tokens, or persistent identifiers in ordinary
logs. Give each fixture an opaque test label.

## Read-only v45/v58 matrix

For every supported OS version, repeat each observation enough times to
distinguish deterministic behavior from a one-off cache result.

### Baseline and within-read stability

1. Wait until Finder reports the baseline fixture uploaded and current.
2. Run the exact v58 bracket:
   account A → File Provider identity A → resource sample A → file version A →
   resource sample B → file version B → File Provider identity B → account B.
3. Verify both resource samples are complete or the whole result fails.
4. Verify account, File Provider domain, File Provider item, generation, and
   file-version outputs are only stable, unavailable, changed during read, or
   unsupported.
5. Verify shared and sync-paused facts remain independent tri-state values.
6. Verify domain and provider-item classifications are independent and no raw
   identifier appears in app state, logs, persistence, AI input, or test
   output.
7. Verify favorable sync eligibility does not make identity readiness true
   unless every separately required identity and sharing/sync fact is stable
   or favorable in the same bracket.
8. Verify no database, candidate, plan, journal/history, provider command, or
   effect record is created and file allocation/content does not change.

### Process restart and reboot

For the same unchanged fixture:

1. record a baseline observation;
2. quit DUX completely and observe again after launch;
3. reboot the Mac and observe again;
4. sign out and back in only on the disposable account, then observe again.

Compare only the capability classifications required by the test. Raw archived
tokens and identifiers are opaque; do not assume their bytes are a documented
canonical cross-process or cross-OS representation. Record whether equality is
stable enough to support a future design, unavailable, or contradicted.

### Item transitions

Observe before and after each isolated transition:

- no-op reopen and metadata refresh;
- content edit followed by completed upload;
- rename within the same directory;
- move within the same iCloud Drive tree;
- move between containers, if a supported public test container exists;
- download on another device;
- local eviction and redownload, only in the future destructive suite;
- network loss and restoration;
- sync pause and resume;
- sharing enable/disable;
- account change.

For each transition, record expected and observed changes in generation, current
file version, sync facts, filesystem identity, and local allocation. Treat an
unexpectedly stable value as characterization data, not proof that the value is
a durable authority witness.

## Public File Provider characterization

Contract v58 evaluates
`NSFileProviderManager.getIdentifierForUserVisibleFile(at:)` separately from
the Foundation v45 archive facts. Establish on both required macOS versions
that the API:

- accepts arbitrary user-selected iCloud Drive items, not only app-owned
  domains or containers;
- returns a documented stable domain/container and item identity;
- distinguishes providers and accounts without using path or display text;
- preserves or deliberately changes identity across rename, move, reboot,
  eviction, redownload, and content-version changes;
- fails closed for unavailable domains, placeholders, shared items, account
  changes, and unsupported OS versions;
- requires no private entitlement, provider-private database, Finder scraping,
  extended attribute, or pathname inference.

Until every item passes, the in-memory stable classifications remain capability
evidence only and no durable provider evidence or candidate admission may be
designed around the API.

## Future destructive suite

The destructive suite is out of scope for v45 and v58. Before it is enabled,
require:

- the separate destructive opt-in and interactive disposable-root
  confirmation;
- a reviewed journal-fenced one-shot executor;
- a fresh final filesystem and provider proof after durable `effect_started`;
- exactly one supported eviction API call and no automatic retry;
- process-lifetime quarantine for every post-entry ambiguity;
- persistence-only restart reconciliation that cannot call eviction;
- local-edit, active-writer, upload, conflict, account-change,
  target-replacement, API-error, crash, and restart races;
- content-hash verification after redownload;
- explicit verification that cloud content remains and no delete/Trash call
  occurred;
- truthful capacity telemetry that does not equate API success with verified
  bytes freed.

Any failure, undocumented behavior, or ambiguous local-only-change outcome
keeps the effect unreachable.

## Evidence record

Store the review record outside the application database and redact account
names, paths, filenames, token archives, item identifiers, and content. Include:

- DUX commit and FFI contract version;
- macOS version/build and hardware architecture;
- test-matrix row and opaque fixture label;
- timestamps and network/sync state;
- observed capability classifications and expected policy result;
- whether the test was read-only or separately destructive;
- proof that no unauthorized effect occurred;
- reviewer and reproduction notes.

The record is test evidence only. It MUST NOT be imported into production
storage, candidate admission, AI payloads, or cleanup history.
