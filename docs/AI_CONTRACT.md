# DUX AI explanation contract

Status: provider-neutral v1 contract, core-owned privacy shaper, exact-review
disclosure preview, and fixed FFI v60 one-shot bridge implemented and tested;
every runtime provider remains disabled.

This document defines the JSON boundary for optional AI explanations. It does
not approve a provider, authorize transmission, or add provider invocation to
the application.
The normative security rules remain [SECURITY_DESIGN.md](../SECURITY_DESIGN.md)
§11 and the accepted ADRs.

## Authority boundary

The v1 contract is presentation-only. It contains no structured path,
candidate, rule revision, safety tier, blocker, action, cleanup mode, plan,
approval, schedule, exclusion, operation result, tool request, provider command,
filesystem handle, or effect capability.

The provider-neutral contract is a crate-private top-level `dux-core::ai`
module. Its contract source imports no other DUX module and is not exported as
a parser or proof constructor by the crate root, engine, CLI, UniFFI, or Swift.
Parsing a request proves only its wire shape, bounds, cross-field accounting,
and digest. It does **not** prove that labels are redacted, that sensitive
categories are absent, or that the request may be sent to a provider.

The contract's private privacy child is the only code that can mint the
separate non-cloneable `PrivacyShapedAiInputV1` proof. It imports only the
immutable validated snapshot review observation and the typed scan-coverage
fact. The proof and its constructor remain private. One narrow engine module
may wrap it in an exact-review preview and expose its already-shaped canonical
JSON for local inspection; there is no provider, network, CLI, cache, planner,
or cleanup consumer.

The schemas are:

- `dux-core/schema/ai-explanation-input-v1.schema.json`
- `dux-core/schema/ai-explanation-output-v1.schema.json`

Both use JSON Schema Draft 2020-12, require every field, and deny unknown fields
at every object level. Rust validation is still authoritative at runtime because
JSON Schema cannot reject duplicate keys or enforce cross-document references,
digests, byte limits, and checked arithmetic by itself.

## Input v1

The outer envelope is:

```json
{
  "schema_version": 1,
  "task": "explain_storage_cluster",
  "input_digest_sha256": "64 lowercase hexadecimal characters",
  "metadata": {}
}
```

`metadata` contains exactly:

- one path-free display `root_label` (the privacy shaper, not this parser,
  derives it);
- total observed logical bytes;
- five non-overlapping logical-byte age buckets;
- `complete`, `partial`, or `unknown` scan coverage;
- one direct child level with request-local opaque node IDs;
- explicit `children_complete`, omitted-child count, omitted logical bytes, and
  five omitted-child age buckets;
- optional deterministic classification labels bound to supplied child IDs;
- `protected: false`; and
- `content_included: false`.

V1 has no recursive children. Descendants beyond the direct-child limit must be
represented only by the omission fields or already-computed aggregate facts.
The input does not carry a scan ID, durable snapshot node ID, candidate ID, or
path-derived stable hash. The `n-…` IDs exist only to bind this one request to
presentation groups in its response.

File and directory labels are hostile text. The shaper derives
non-hierarchical display labels, encodes them only as JSON values, and no code
concatenates them into provider instructions. Shape validation rejects empty
text, leading/trailing whitespace, C0/C1 controls, Unicode directional-
formatting controls, common solidus variants, `/`, `\\`, `~`, `..`, percent-
encoded separators, and drive/scheme-shaped colon forms such as `C:cache`.
That conservative grammar is defense in depth, not a secret detector or proof
that redaction ran.

### Input digest

The digest prevents a response, cache record, or later adapter from being
silently rebound to different metadata. It is independent of JSON field order,
whitespace, string escaping, and Rust/Serde declaration behavior. SHA-256
receives the domain `dux-ai-explanation-input-v1\0`, metadata-record byte
`0x01`, then this frozen typed encoding:

| Tag | Metadata value and encoding |
|---:|---|
| `01` | `root_label` as tagged string |
| `02` | `total_logical_bytes` as tagged u64 |
| `03` | root `age_summary` as tagged age record |
| `04` | `coverage` as tagged enum |
| `05` | `children_complete` as tagged bool |
| `06` | `omitted_child_count` as tagged u64 |
| `07` | `omitted_logical_bytes` as tagged u64 |
| `08` | `omitted_age_summary` as tagged age record |
| `09` | child-vector header, then child records in input order |
| `0a` | classification-vector header, then classification records in input order |
| `0b` | `protected` as tagged bool |
| `0c` | `content_included` as tagged bool |

A tagged string is its one-byte field tag, little-endian u64 UTF-8 byte length,
then exact UTF-8 bytes. A tagged u64 is its tag plus little-endian value. A bool
is its tag plus `00` or `01`. A vector header is its tag plus little-endian u64
element count. An age record is its tag followed by five little-endian u64s in
`within_7_days`, `days_8_to_30`, `days_31_to_90`, `older_than_90_days`, then
`unknown_age` order. Coverage tags are `01` complete, `02` partial, `03`
unknown; node-kind tags are `01` directory, `02` file, `03` symlink, `04`
other, and `05` unavailable.

Each child starts with record byte `a1`, then local tags `01` node ID, `02`
label, `03` kind, `04` logical bytes, `05` age record, `06` coverage, and `07`
protected. Each classification starts with `a2`, then local tags `01` node ID,
`02` classification ID, and `03` label. The digest itself remains outside
`metadata`, avoiding a circular value. Any encoding change requires a new
schema/digest version and new golden fixtures.

### Input limits

| Item | V1 limit |
|---|---:|
| Raw request before parsing | 256 KiB |
| Direct children | 128 |
| Known classifications | 128 |
| Request-local node ID | 3–64 ASCII bytes, `n-` prefixed |
| Root, child, or classification label | 1–512 UTF-8 bytes |
| Every JSON integer | `0...9,007,199,254,740,991` |
| Child depth | exactly one direct level |

The integer ceiling is JavaScript's exact integer range, so Rust and a future
Swift/JavaScript-adjacent adapter cannot silently round byte counts or IDs.
Every age bucket must be in range and its checked sum must exactly equal the
corresponding logical-byte observation. Child logical bytes plus omitted
logical bytes must exactly equal the root observation, and each child-plus-
omitted age bucket must exactly equal its root bucket. Child count plus omitted
child count must also fit the exact 53-bit range. Complete children require zero
omission count, bytes, and buckets; incomplete children require a positive
omitted count.

## Privacy shaper and retained-review preview v1

The shaper accepts one selected directory from an already validated,
immutable `SnapshotReviewDocument` and a typed `ScanCoverage` whose status is
complete, whose measurement is present, and whose issue set is empty. It does
not accept arbitrary JSON, caller-authored privacy booleans, display
projections, candidate classifications, live paths, file handles, or cleanup
types. The selected node must exist and be a directory. The selected root and
at most 200,000 selected-subtree nodes are inspected using strict lossless
Unix-byte or Windows-UTF-16 decoding. Unsupported encodings, ambiguous path
forms, error nodes, inaccessible or timed-out observations, mount boundaries,
invalid accounting, and inspection overflow reject the whole request without a
proof.

Sensitive-path policy revision 1 is independent of cleanup's protected-root
policy. It denies protected system roots, exact per-user Library/AppData roots,
and recognizable credentials/tokens, keychains, browser profiles,
Messages/Mail/Notes data, password managers,
security or device-management state, VM/container disk state, and cloud
documents. iCloud, CloudStorage, OneDrive, Dropbox, Google Drive, and iCloud
placeholder observations are denied regardless of upload or placeholder state.
No cleanup category or caller-provided classification can weaken the deny
decision. The checked versioned policy corpus is
`dux-core/tests/fixtures/ai/privacy/v1/sensitive-path-policy.json`.

The shaper emits only the fixed root label `Selected folder`, fixed kind plus
ordinal child labels such as `Directory 1`, and fresh `n-…` request-local IDs.
It emits no source basename and no known classification in v1. If a direct
child or any descendant is sensitive, that entire direct child is excluded.
Its name, byte count, age, and descendant facts do not survive in the payload,
root totals, or omission totals. The local disclosure records only the policy
revision and aggregate inspected/included/excluded/eligible-omitted counts; it
contains no sensitive sizes or identifiers.

After exclusion, eligible direct children are deterministically ordered by
logical bytes and snapshot ordinal, then capped at 128. Root and eligible-
omission logical bytes are recomputed exactly. Age buckets are computed from
the logical bytes and modification time of non-directory leaf observations,
not aggregate directory modification times; missing and future times are
unknown. The frozen typed digest and exact JSON encoding are created only after
the resulting metadata passes every v1 semantic check.

Source acquisition is now limited to one exact retained Explorer review. The
review lease reloads the identical succeeded scan row under a current history
guard, derives complete typed coverage from that row, requires the same
retained snapshot reference, and revalidates the retained snapshot. The engine
repeats the exact coverage and live-lease proof after shaping. Callers supply
only a selected snapshot node ID; they cannot supply coverage, JSON, digest,
privacy flags, provider data, or a path.

The resulting non-cloneable preview retains its parent owner/session/scan
identity and freezes a two-minute monotonic and wall-clock deadline capped by
the parent lease. Its exact canonical JSON and the sealed proof share one
bounded backing allocation. Its request-local `n-…` to snapshot-node mapping
stays private for later validated overlays. UniFFI contract v59 exposes an
opaque releaseable child, exact JSON bytes, digest, generic structured
projection, aggregate disclosure, and explicit false content/path/name flags.
At most one preview is available per engine; close and reset drain it before
parent reviews. The native service adapter validates and releases that object
but publishes no UI state. Parsed input can never be upgraded to a proof, and
the preview cannot be consumed as a provider request in this checkpoint.

## Output v1

The response requires exactly:

- `schema_version: 1`;
- `task: "explain_storage_cluster"`;
- the exact input digest;
- a plain-text summary;
- presentation labels;
- presentation-only groups;
- questions;
- uncertainties; and
- suggestions for future human rule research.

A group contains only a title, one or more input node IDs, and a reason. Every
ID must exist in the exact checked input, IDs must be unique inside a group, and
groups must be disjoint. One unknown, duplicate, or overlapping reference
rejects the whole response; DUX never publishes a partially accepted model
answer.

Provider and model identity never come from model output. A future approved
adapter must attach its trusted identity outside this JSON document. AI labels
and prose remain visibly AI-authored, inert text. Clients must not parse prose
for paths or commands, create clickable cleanup actions from it, translate a
label into a DUX safety category, or use it as rule/planner evidence.

V1 applies the same path-shaped checks to provider text and conservatively
rejects ASCII word families associated with approval, cleanup, deletion,
destruction, discarding, disposal, emptying, erasure, eviction, execution,
installation, killing, purging, reclaiming, removal, scheduling, shell use,
Trash, unlinking, and wiping. It also rejects the exact command tokens `chmod`,
`chown`, `mv`, `ran`, `rm`, `rmdir`, `run`, `running`, `sudo`, and `xargs`.
Invisible formatting characters that could split those tokens are rejected
before the vocabulary check. This lexical filter cannot understand every
natural-language instruction and is not an authorization boundary. Every
future UI must render the accepted result as non-linkified inert text and must
never interpret it.

### Output limits

| Item | V1 limit |
|---|---:|
| Raw response before parsing | 64 KiB |
| Summary | 1–4,096 UTF-8 bytes |
| Labels | 16 unique stable labels, each 1–64 bytes |
| Groups | 32 |
| Node IDs per group | 1–128 unique IDs |
| Group title | 1–256 UTF-8 bytes |
| Group reason | 1–1,024 UTF-8 bytes |
| Questions | 16 unique values, each 1–512 bytes |
| Uncertainties | 16 unique values, each 1–512 bytes |
| Research suggestions | 8 unique values, each 1–512 bytes |

Malformed UTF-8/JSON, duplicate keys, trailing documents, unknown fields,
unsupported versions/tasks, bad digests, control/invisible-format characters,
path markers, conservative action-language matches, invalid identifiers,
excessive counts, and cross-reference failures are typed all-or-error failures.
Errors contain positions and bounded taxonomy, not input labels, node IDs,
provider prose, or filesystem paths. Schema `maxLength` uses Unicode code
points; `x-dux-maxUtf8Bytes` documents the stricter authoritative Rust byte
limit, while `x-dux-forbidsActionLanguage` marks semantic text checks that JSON
Schema cannot express. `x-dux-forbidsPathLikeText` likewise marks the semantic
path-shaped grammar beyond the schema's character class. Draft 2020-12 also
considers `1.0` an integer; DUX wire
integers use canonical unsigned decimal JSON lexemes only (for example `1`, not
`1.0` or `1e0`), as marked by `x-dux-canonicalUnsignedIntegerLexeme` and
enforced by Rust deserialization.

## Approved future remote boundary

[ADR 0013](adr/0013-metadata-only-remote-ai-transport.md) selects a fixed,
direct-vendor HTTPS architecture for the first provider implementation. It
does not expose this contract or approve a concrete adapter by itself. The
gated FFI v60 bridge may consume only one exact retained-review preview into an
opaque fixed Anthropic Messages v1 revision-1 attempt. All request information
comes from moving the sealed Rust proof; arbitrary or parsed JSON cannot become
an authorized request. When the implementation and generated binding pass, the
production-compiled handoff is still not runtime invocation authority. Before
any product caller is added, a separate consent
UI must let the user inspect that proof's exact path-free disclosure and require
an explicit Explain-selection action for one transmission.

The transport surface is closed rather than generic. Each adapter owns an
exact provider ID, bounded model selection, HTTPS origin and path, authentication
shape, request envelope, response extractor, and provider-retention disclosure.
It accepts no caller URL, command, header map, cookie, upload, file, image,
remote URL, tool/function/MCP definition, streaming or background option. DUX-
managed credentials use a generic-password data-protection Keychain item with
`kSecUseDataProtectionKeychain=true`, fixed service
`se.mjukis.dux.ai-provider-key.v1`, adapter-ID account, no access group,
`kSecAttrSynchronizable=false`, and
`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`; they are not represented by
this schema. Every operation supplies a fresh `LAContext` with
`interactionNotAllowed=true`; the redacted secret object accepts only 1–512
bytes of visible ASCII. Settings credential verification is local-only. The
exact native orchestrator must start the one 60-second lifecycle deadline
before consuming the opaque attempt or starting Keychain lookup. It may send
exactly one fixed-adapter-produced request through an ephemeral session that
rejects redirects, disables cookies and caches, applies fixed byte/deadline
limits, owns one cancellation/task teardown, and does not retry.

Wire provider/model choice is fixed by adapter-owned metadata outside the model
response; the sealed Rust attempt independently attests the same binding and
offers no selector. The fixed adapter extracts exactly one response, then the
opaque FFI attempt accepts those raw bytes once into the existing all-or-error
Rust v1 validator.
A response with a mismatched input digest, tool-shaped content, unknown
reference, malformed body, or over-limit output is discarded completely. Rust
maps each accepted request-local group ID to the corresponding sealed snapshot
node ID internally; neither mapping nor a caller-supplied node ID crosses the
validation call. Provider failure does not modify the deterministic Explorer
tree, selection, candidates, or recommendations. Accepted prose is visibly
provider-labeled, non-linkified inert presentation and never becomes rule,
plan, approval, schedule, or executor input.

No AI cache write is currently admitted. The reserved SQLite row's 16-MiB
payload and missing privacy/input revision fields are insufficient for v1. A
future migration and sealed boundary must cap canonical validated output at
64 KiB and bind the input digest plus privacy-policy, input schema/digest,
output schema, provider, adapter, and exact model revisions before applying the
30-day/user-clearable retention policy.

## Current non-capabilities

This checkpoint adds no runtime provider/model selection, executable probe,
subprocess, environment handling, temporary directory, cache write/read,
database migration, UI, CLI command, candidate, plan, approval, schedule, or
cleanup edge. The intended FFI v60 attempt and exact native orchestrator are
complete only after the implementation, generated binding, focused tests, and
architecture-policy guard all pass. Even then they remain production-compiled
but unreachable: no AppModel, controller, view, Settings flow, CLI, or
scheduler may construct the orchestrator, so disabled/no-provider remains the
only runtime state.

The closed Keychain store owns the fixed DUX service and adapter-account tuple,
redacted 1–512-byte visible-ASCII secret, local presence, explicit replace/
delete, and single-request read capabilities. The Foundation lifecycle owns one
ephemeral data task, fixed request/response caps, one original 60-second
monotonic deadline, redirect/auth-challenge refusal, cancellation/invalidation,
late-callback fencing, and no retry. `SecItem*` stays only in the credential
store; `URLSession` stays only in the lifecycle. Because Keychain work is
synchronous, timeout fences and discards a late result rather than claiming to
cancel the OS operation. Tests inject Keychain/network/clock edges and create
neither a real credential nor a live request.

The fixed Anthropic Messages v1 adapter revision 1 owns only `POST
https://api.anthropic.com/v1/messages`, API version `2023-06-01`, the pinned
`claude-sonnet-4-6` model, one-request `x-api-key` authentication, JSON media
types, 8,192 output tokens, a constant injection-resistant instruction, one
metadata text value, and a constant provider-compatible structural output
schema. Its request has no tool/function/server-tool, file/image/URL, prompt
cache, beta, thinking, stream, background, persistence, metadata, or fallback
option. Its strict duplicate-free response extractor returns only one exact
non-empty assistant-text block from the reviewed model after `end_turn`; every
tool-shaped block, other stop reason, unknown envelope, model mismatch,
malformed JSON, and over-limit body fails closed. These bytes are still
untrusted and have no presentation path before the opaque attempt's consume-
once Rust v1 validator. Only the exact one-shot orchestrator may use its
production bridge;
the DEBUG harness may still exercise inert fixture metadata and a fake key
without `URLSession` or Keychain. The reviewed provider policy and suspension
triggers are in
[Anthropic Messages v1 provider review](provider-reviews/anthropic-messages-v1.md).

The permitted opaque Rust attempt retains the moved sealed proof and its
private request-local-ID mapping, exact review affinity, expiry, input digest,
and fixed adapter binding. Its public FFI surface is limited to bounded info,
consume-once validation of extracted response bytes, and release. Close/reset
drains attempts before previews and reviews. `EngineService` is the sole
non-generated production user of that FFI surface;
`NativeAIAnthropicMessagesV1Orchestrator` is the sole production consumer of
the narrow credential-reader, adapter, lifecycle, and core-attempt protocols.
There is no generic URL, provider, model, header, request/body, callback, or
validator API and no cache, persistence, action, or partial-admission
authority.

The 2026-08-09 adversarial macOS subprocess-confinement spike returned no-go
for a direct local command. A hostile child retained ordinary same-user read
access outside its empty working directory despite a minimal environment and
closed nonstandard descriptors. That failure is decisive before any favorable
TCC assumption: ADR 0009 closes the conditional direct Claude/Codex adapters
without implementing them. ADR 0013 now approves only the future remote
architecture described above; it does not itself enable an adapter. The exact
one-shot bridge remains a testable production dependency graph only, not a
product call site. The consent preview, explicit Explain-selection action,
inert overlays, cache migration/boundary, and structural proof that AI cannot
reach planning remain open. Disabled/no-provider remains the only runtime
provider state.
