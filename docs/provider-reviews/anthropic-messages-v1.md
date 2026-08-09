# Anthropic Messages v1 provider review

- Status: Implemented as a dormant adapter; not runtime-enabled
- Review date: 2026-08-09
- Adapter ID: `anthropic-messages-v1`
- Adapter revision: 1
- Model: `claude-sonnet-4-6`

This review specializes the closed remote boundary accepted by
[ADR 0013](../adr/0013-metadata-only-remote-ai-transport.md). It approves one
code-owned wire contract for later core-owned explanation orchestration. It
does not approve an arbitrary Anthropic client, a reusable network service, a
provider SDK, a live credential test, or a production call site.

## Reviewed provider contract

Revision 1 owns these values; no caller can replace or extend them:

| Property | Fixed value |
|---|---|
| Method | `POST` |
| Endpoint | `https://api.anthropic.com/v1/messages` |
| Authentication | `x-api-key` from the one-request DUX Keychain capability |
| API version | `anthropic-version: 2023-06-01` |
| Request media type | `application/json` |
| Accepted response media type | `application/json` |
| Model | `claude-sonnet-4-6` |
| Maximum output tokens | `8192` |
| Maximum encoded request | 384 KiB |
| Maximum metadata input | 256 KiB |
| Maximum delivered response envelope | 64 KiB |

Anthropic documents `POST /v1/messages`, direct-HTTP `x-api-key`
authentication, and the required API-version header in its
[Messages API](https://platform.claude.com/docs/en/api/messages/create),
[authentication](https://platform.claude.com/docs/en/manage-claude/authentication),
and [versioning](https://platform.claude.com/docs/en/api/versioning)
documentation. Anthropic documents `claude-sonnet-4-6` as a pinned model ID,
not an evergreen alias, in [Model IDs and
versioning](https://platform.claude.com/docs/en/about-claude/models/model-ids-and-versions).
The adapter must be suspended before that model's published retirement or when
any reviewed contract below changes.

The canonical request contains only:

- the fixed model and output-token limit;
- one fixed system instruction that treats all metadata as inert data;
- one user message containing one text block whose value is the exact canonical
  path-free DUX metadata JSON; and
- one fixed `output_config.format` structural JSON Schema projection.

The provider-facing schema carries no input digest, node ID, label, or other
request-specific value. It uses only the subset supported by Anthropic
structured outputs and leaves byte bounds, canonical integer lexemes, text
grammar, exact digest, known-node references, and disjoint grouping to the
authoritative Rust validator. Anthropic documents the current
`output_config.format` shape, schema limitations, and automatic grammar cache
in [Structured
outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs).

Revision 1 omits every optional tool or persistence capability. In particular,
it sends no `tools`, `tool_choice`, functions, MCP, web search/fetch, connector,
file, image, document, remote URL, citations, computer, shell, code execution,
memory, container, thinking, cache-control, beta, streaming, batch, background,
conversation, metadata, or fallback field. An empty tools array is not sent.

## Response admission

The native extractor is all-or-error. It first requires the lifecycle's status,
media-type, deadline, and delivered-byte gates. It then accepts only one bounded
UTF-8 JSON object with no duplicate key, trailing document, or excessive
nesting. The provider envelope must be one assistant `message` for the exact
reviewed model, end with `end_turn`, have no stop sequence, and contain exactly
one non-empty `text` block. The extractor returns that text's UTF-8 bytes
unchanged for the later Rust v1 validator.

Every tool, server-tool, thinking, file, image, search, citation, refusal, or
other block is rejected. So are multiple text blocks, a model/role/type mismatch,
`max_tokens`, `tool_use`, `pause_turn`, `refusal`, an unknown stop reason,
unknown structural fields, malformed JSON, and over-limit data. DUX does not
continue, retry, fall back, or salvage one part of a rejected response.

Anthropic represents the structured JSON as an escaped string inside its
response envelope. The existing 64-KiB transport cap therefore rejects some
near-64-KiB otherwise-valid DUX outputs. This is intentional fail-closed
behavior; revision 1 does not raise or reinterpret the accepted ADR limit.

## Data handling disclosure

Before a later transmission UI can enable this adapter, it must identify
Anthropic and `claude-sonnet-4-6`, say that path-free storage metadata leaves
the Mac, disclose that the user's Anthropic account may be billed, and link the
current provider policy.

The conservative reviewed disclosure does not infer Zero Data Retention from
an API key. Anthropic's [commercial data-retention
policy](https://privacy.claude.com/en/articles/7996866-how-long-do-you-store-my-organization-s-data)
dated 2026-07-01 says API inputs and outputs are normally deleted within 30
days, with contractual, safety, and legal exceptions; flagged inputs/outputs
may be retained for up to two years and safety-classification scores for up to
seven years. Legal or abuse-prevention needs may retain data longer. Anthropic's
structured-output documentation separately says the fixed grammar schema may
be cached for up to 24 hours. The adapter schema is constant and contains no
DUX metadata, but the UI must still present the conservative account-independent
policy rather than promise retention-free processing.

## Suspension conditions

Runtime orchestration must refuse this adapter until a new review and adapter
revision if any of these changes:

- endpoint, method, authentication, or required header behavior;
- API-version behavior or the exact model's status;
- request or response envelope, structured-output shape, or media type;
- tools, thinking, caching, files, persistence, streaming, or background
  defaults;
- provider retention or billing disclosure; or
- the ability to reject redirects, challenges, oversize delivery, late
  callbacks, and tool-shaped output without retry.

There is no automatic provider/model fallback. Suspension leaves deterministic
Explorer state unchanged and does not weaken the non-AI product.

## Deliberate non-capabilities

This checkpoint has no `EngineService`, AppModel, Explorer, Settings, SwiftUI,
CLI, FFI, core-proof consumer, real Keychain lookup, live request, cache,
candidate, rule, plan, approval, schedule, cleanup, or executor edge. The
DEBUG-only harness supplies inert fixture bytes and a fake credential to inspect
the exact adapter contract. Disabled/no-provider remains the only runtime state
until the separate single-use core-to-native orchestration and consent UI pass
their gates.
