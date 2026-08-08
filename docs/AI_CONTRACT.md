# DUX AI explanation contract

Status: provider-neutral v1 contract implemented; privacy shaping and every
provider remain disabled.

This document defines the JSON boundary for optional AI explanations. It does
not approve a provider, authorize transmission, or add AI to the application.
The normative security rules remain [SECURITY_DESIGN.md](../SECURITY_DESIGN.md)
§11 and the accepted ADRs.

## Authority boundary

The v1 contract is presentation-only. It contains no structured path,
candidate, rule revision, safety tier, blocker, action, cleanup mode, plan,
approval, schedule, exclusion, operation result, tool request, provider command,
filesystem handle, or effect capability.

The implementation is a crate-private top-level `dux-core::ai` module. It does
not import another DUX module and is not exported by the crate root, engine,
CLI, UniFFI, or Swift. Parsing a request proves only its wire shape, bounds,
cross-field accounting, and digest. It does **not** prove that labels are
redacted, that sensitive categories are absent, or that the request may be sent
to a provider. A later core-owned privacy shaper must mint that separate proof.

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

- one path-free display `root_label` (the future privacy shaper, not this
  parser, must derive it);
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

File and directory labels are hostile text. A future shaper must derive
non-hierarchical display labels, encode them only as JSON values, and never
concatenate them into provider instructions. Shape validation rejects empty
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

## Current non-capabilities

This checkpoint adds no snapshot-to-input shaper, privacy authorization,
sensitive-path policy, provider selection, executable probe, subprocess,
network request, environment handling, temporary directory, timeout,
cancellation, output pipe, cache write/read, database migration, task,
`EngineHandle` method, FFI record, Swift model, UI, CLI command, or cleanup edge.

The next Milestone 7 slice is privacy redaction and sensitive-category
exclusion. The adversarial macOS TCC/confinement spike remains a separate
shipping gate after that. Until both gates pass, disabled/no-provider is the
only permitted provider state.
