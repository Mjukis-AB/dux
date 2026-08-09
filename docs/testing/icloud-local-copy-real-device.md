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

The repository-owned `scripts/qualify-icloud-v58-read-only.sh` harness makes
one matrix-row observation reproducible. It is a non-shipping XCTest target
that compiles the exact production Foundation reader directly. The harness is
default-skipped, does not initialize the app or Rust engine, and does not make
a successful local build count as real-device evidence. The two required host
rows and the operator-controlled transitions below still have to be performed
and reviewed explicitly.

## Gates

The read-only capability suite requires all of the following:

- an explicit `DUX_ICLOUD_REAL_DEVICE_TESTS=1` opt-in;
- a dedicated disposable Apple Account named only in the private operator
  manifest, plus an opaque `acct-` label in repository-safe evidence;
- a test root created solely for DUX under iCloud Drive;
- one physical/disposable test environment on macOS 14 and one on the newest
  macOS version supported by the release (intermediate supported versions are
  not part of this gate unless a failure requires more characterization);
- network connectivity plus a documented offline phase;
- no parallel DUX iCloud test run;
- captured OS/build, hardware, account-fixture, and DUX commit identifiers;
- manual confirmation that every fixture is disposable;
- manual confirmation that the content-integrity reference is stored outside
  iCloud Drive; and
- an exact clean source commit. A dirty tree, detached source description, or
  evidence path that already exists fails closed.

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

## Repository-owned read-only harness

Run only the checked-in wrapper from a clean commit. It accepts no arguments
and requires all inputs through its fixed environment contract. At minimum the
operator supplies:

- the three exact confirmations for the disposable account, disposable
  fixture, and external content reference;
- canonical physical paths for the dedicated fixture root, one strict
  descendant regular-file fixture, a private state directory, and a new
  evidence JSON file;
- fixed-format opaque account and fixture labels;
- one allowlisted phase, network state, expected sync-eligibility result, and
  expected identity-readiness result.

The private state and evidence parents MUST be existing, user-owned `0700`
directories outside the fixture root, iCloud Drive, and DUX application data.
The wrapper derives the clean commit, evidence-schema digest, run time, and
platform facts; clears the inherited environment; serializes the exact XCTest
with `lockf`; and validates the newly created evidence file afterward. The
live XCTest is:

```text
DuxICloudQualificationTests/
ICloudV58ReadOnlyQualificationTests/testLiveReadOnlyQualification
```

Ordinary `xcodebuild test` without the exact opt-in skips that test before the
fixture or File Provider is accessed. Any presence of
`DUX_ICLOUD_DESTRUCTIVE_TESTS` is a hard refusal, regardless of value. Do not
invoke the XCTest directly to create accepted evidence: only the wrapper binds
the clean commit, closed schema, exclusive lock, and platform expectations.

For cross-process and cross-reboot comparison, the qualification-only reader
may transform each bounded identity into a domain-separated HMAC-SHA256 tag
inside the reader. The fixed 32-byte random key and private tag reference are
stored only in the private state directory as user-owned, single-link `0600`
files. Raw account, domain, item, generation, and version values never cross
the reader boundary. Neither raw values, keys, tags, paths, filenames, content,
content hashes, nor underlying error text may enter the evidence report or
test logs. These test-only tags are comparison state, not production evidence
or cleanup authority, and the qualification compile condition is absent from
the app's Debug, Release, and cleanup-qualification targets.

## Read-only v45/v58 matrix

At each of the two required OS endpoints, run every matrix row as exactly three
consecutive bracketed observations. A skipped test, fewer observations, a
changed stat/allocation witness, non-identical result within the three-read
row, or expected/observed policy mismatch does not count as evidence.

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
2. let the qualification XCTest process exit, then run the process-restart row;
3. reboot the Mac and run the post-reboot row;
4. sign out and back in only on the disposable account, then observe again.

Compare only redacted continuity classifications derived by the
qualification-only keyed comparator. Raw archived tokens and identifiers are
opaque; do not assume their bytes are a documented canonical cross-process or
cross-OS representation. A matching HMAC proves equality only for the same
private qualification key and baseline; it does not make the archived form a
documented canonical identity. Record whether continuity matches the baseline,
changes, is unavailable, or is contradicted within a read.

### Item transitions

Observe before and after each isolated transition:

- no-op reopen and metadata refresh;
- content edit followed by completed upload;
- rename within the same directory;
- move within the same iCloud Drive tree;
- move between containers, if a supported public test container exists;
- download on another device;
- network loss and restoration;
- sync pause and resume;
- sharing enable/disable;
- account change.

Local eviction/redownload and deliberately created conflicts belong only to
the future destructive suite. They are not prerequisites for the read-only
identity gate. Unsupported-OS behavior belongs in deterministic injected unit
tests, not a real-device row below the macOS 14 deployment target.

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
- preserves or deliberately changes identity across rename, move, reboot, and
  content-version changes;
- fails closed for unavailable domains, placeholders, shared items, account
  changes, and supported-host capability failures;
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

Store the review record outside the application database. It MUST conform to
`spikes/icloud-v58-read-only-qualification/evidence-v1.schema.json` and pass
`scripts/validate-icloud-v58-evidence.py`. The closed record contains only:

- DUX commit and FFI contract version;
- macOS version/build and hardware architecture;
- test-matrix row and opaque fixture label;
- timestamps and network/sync state;
- observed capability classifications and expected policy result;
- exactly three consecutive observation results;
- fixed read-only, non-production isolation facts; and
- stat/allocation and redaction invariants proving that no unauthorized effect
  or content read occurred.

The record is test evidence only. It MUST NOT be imported into production
storage, candidate admission, AI payloads, or cleanup history.

The repository-safe record omits reviewer prose and actual account identity;
those stay in the private operator manifest. Before accepting the matrix,
review the paired macOS 14 and newest-supported records together with that
manifest, confirm every required phase is present, and confirm all records bind
the same clean source commit and protocol contract. Until that review is
complete, the roadmap may say only **qualification harness ready**. It MUST NOT
mark the real-device matrix, durable evidence design, candidate admission, or
eviction executor complete.
