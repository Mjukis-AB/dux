# ADR 0013: Metadata-only remote AI transport

- Status: Accepted
- Decision date: 2026-08-09

## Context

DUX is an unsandboxed storage application whose stable production identity may
receive Full Disk Access. AI is useful for explaining unfamiliar storage and
grouping already observed rows, but it must not inherit DUX's filesystem
authority or enter the cleanup authority graph.

The provider-neutral v1 schemas and the core privacy shaper
define a bounded, path-free, content-free metadata document. ADR 0009 then
proved that launching Claude CLI, Codex CLI, or another same-user command is
not confinement: a hostile child could read a known 0600 canary outside its
empty working directory. Fixed arguments, a scrubbed environment, closed
descriptors, and provider tool-disable flags do not change that result.

The remaining architecture choices were a remote API, a separately App-
Sandboxed component, or a virtual machine. A sandboxed component or VM would
add another executable, signing/update boundary, credential broker, Intel and
Apple-Silicon artifact, and macOS 14/newest-release qualification matrix before
DUX could explain anything. A direct vendor HTTPS request executes no provider
code under DUX's local authority and can carry only the already redacted
metadata document.

This decision chooses that remote shape. It does not approve a concrete model,
add a credential, expose the privacy proof, or enable network traffic. Those
are later implementation and adapter-review gates.

## Decision

DUX v1 AI explanations MUST use reviewed, built-in, metadata-only direct-vendor
HTTPS adapters. The fixed Anthropic Messages v1 revision-1 adapter, complete
core-to-transport orchestration, and explicit Explorer preview/consent gates
have passed. It is the only enabled runtime provider path; disabled remains a
fully supported state.

The first adapter reviews may use only these frozen identities and endpoints:

| Adapter ID | Method and endpoint | Authentication owned by adapter |
| --- | --- | --- |
| `anthropic-messages-v1` | `POST https://api.anthropic.com/v1/messages` | `x-api-key` plus a fixed reviewed `anthropic-version` |
| `openai-responses-v1` | `POST https://api.openai.com/v1/responses` | `Authorization: Bearer` |

Listing an adapter here reserves its closed network identity; it does not
enable it. Each adapter still requires a review of its current stable API,
bounded model allowlist, exact request envelope, response extractor, error
mapping, tools-disabled behavior, provider retention policy, and fixtures.
Adding or changing an origin, path, method, authentication scheme, provider,
or provider SDK requires a new adapter revision and review. A caller cannot
supply or override any of them.

DUX MUST NOT offer an arbitrary URL, OpenAI-compatible endpoint, custom header
map, proxy implemented by DUX, local command, executable picker, shell string,
provider plugin, SDK, MCP server, or generic `Data -> Data` transport. A future
App-Sandboxed component or VM remains a different, unapproved architecture.

## Disclosure and consent boundary

AI remains optional and off by default. Scans, disk-pressure transitions,
notifications, schedules, app launch, and deterministic recommendations MUST
NOT invoke it. One transmission requires all of the following:

1. one exact retained succeeded-snapshot Explorer review and complete typed
   scan coverage;
2. a core-minted, non-cloneable `PrivacyShapedAiInputV1` proof for the selected
   directory;
3. a preview of the exact path-free metadata, provider, model, adapter revision,
   response limit, and provider data-retention disclosure; and
4. an explicit user invocation of **Explain selection** for that preview.

Parsed JSON, Swift-authored metadata, cache rows, provider output, AI text, and
caller booleans can never be upgraded into the proof. Consent is single-use and
bound to the exact input digest, review lease, provider, model, and adapter
revision. Changing any value requires a fresh preview and invocation.

The orchestration boundary MUST retain the proof and review lease inside
the engine until the user consumes one opaque request capability. It may then
give one fixed adapter only the exact encoded v1 input and non-secret adapter
configuration. There MUST be no reusable public network service or FFI method
that accepts caller bytes, URLs, paths, headers, candidates, plans, or actions.

## Credential boundary

A DUX-managed provider API key MUST be a data-protection Keychain generic-
password item. Every add/read/update/delete query includes
`kSecClassGenericPassword`, `kSecUseDataProtectionKeychain=true`, the fixed
service `se.mjukis.dux.ai-provider-key.v1`, the fixed adapter ID as
`kSecAttrAccount`, `kSecAttrSynchronizable=false`, and no access group. Add and
replacement values use `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`.
Selecting the data-protection Keychain is mandatory on macOS; the accessibility
attribute is not a valid device-only guarantee without it. This exact tuple
prevents a different service/account item from satisfying lookup. A credential
does not sync, migrate to another device, or become available while the device
is locked.

The credential MUST NOT appear in `UserDefaults`, SQLite, snapshot/cache files,
environment variables, command arguments, request bodies, model input,
diagnostics, logs, errors, crash reports, analytics, or clipboard operations.
It is loaded only for the one request, applied through the adapter-owned
authentication header, and released with the request task. Settings may verify
only local presence/readability, replace, or delete the selected adapter's
exact Keychain item after an explicit action; it may display only absent/
present/failed state. Credential verification performs no network request.

DUX MUST NOT copy credentials from Claude Code, Codex, shell configuration,
environment variables, browser sessions, or another application's storage.

## Network and lifecycle boundary

The native transport MUST use Foundation `URLSession` directly, not a
provider SDK, with one `URLSessionConfiguration.ephemeral` session per request.
It MUST additionally set no cookie storage, no URL cache, no credential
storage, no connectivity waiting, no discretionary/background behavior, and a
reload-ignoring-local-cache request policy. Default platform TLS validation is
required; DUX adds neither a trust bypass nor certificate pinning.

The transport contract is fixed:

- one `POST` to the adapter endpoint and no redirect; the delegate cancels the
  first redirect before an authorization header can be replayed;
- one canonical JSON request no larger than 384 KiB, including the at-most
  256-KiB v1 input, instructions, and output schema;
- a 60-second monotonic wall deadline covering credential lookup, request,
  response, validation handoff, and cancellation settlement;
- a 64-KiB decompressed response-body limit enforced incrementally even when
  `Content-Length` is missing or false;
- status `200` plus the adapter's exact JSON media-type policy;
- cancellation through the engine-owned task, native task cancellation, and a
  generation fence that discards every late callback;
- no automatic retry, redirect follow, authentication challenge fallback,
  streaming response, background continuation, resumable task, upload task,
  file body, multipart body, remote input URL, image, or attachment; and
- bounded path-free error categories only. Response bodies, provider prose,
  request bodies, keys, headers, URLs with query data, and underlying error
  strings never enter logs or persistence.

The session and delegate become unreachable after one terminal result. There
is no process tree to terminate because ADR 0009 continues to prohibit local
provider processes.

## Provider capability boundary

An adapter sends no client tool, function, MCP, web-search, web-fetch, file,
computer-use, shell, code-execution, memory, connector, remote-tool, or server-
tool declaration. It does not request optional prompt caching, files,
persistent conversation state, background mode, or provider-side storage.
Where an API has an explicit storage control, the adapter fixes it to the non-
storing value. Provider-mandated caching or abuse-monitoring retention is not a
request capability DUX can disable; the adapter must disclose the current
worst-case policy and MUST NOT call the request retention-free.

Tools-disabled behavior is defense in depth and MUST be validated against the
current provider API before an adapter is enabled. The response extractor
accepts only the adapter's exact terminal assistant-text/structured-output
shape. A tool call, function call, server-tool result, file/image block,
remote-URL reference, multiple answer bodies, continuation, or unexpected
finish reason rejects the complete response. DUX never executes a model-
requested operation.

Provider and model identity come only from the trusted adapter selection and
the response's exact adapter-validated echo where one exists. Model-authored
text cannot choose or rename a provider.

## Validation and authority boundary

The provider response is untrusted bytes. The adapter extracts at most one
bounded JSON document and returns it directly to the engine. The existing Rust
v1 output validator MUST accept its schema, semantic bounds, conservative
path/action grammar, node references, disjoint groups, and exact input digest
before any result is displayed or cached. Validation is all-or-error; DUX does
not salvage partial prose or groups.

Accepted output is visibly provider-labeled, non-linkified inert presentation.
It cannot create or mutate a path, candidate, rule, classification, safety tier,
blocker, cleanup mode, plan, approval, schedule, exclusion, notification,
journal row, operation result, driver request, callback, or executor input.
Provider failure, cancellation, timeout, refusal, malformed output, and
credential/network errors leave the deterministic Explorer tree, selection,
recommendations, and cleanup state unchanged.

No request or response persistence is approved by this ADR. The existing
`ai_insights` reservation is not an admissible cache schema: it permits a
16-MiB payload and lacks privacy-policy and input-contract revisions. Before
the first cache write, a migration plus a sealed validated insert/load boundary
MUST cap the canonical validated output at 64 KiB and bind the input digest,
privacy-policy revision, input schema/digest revision, output schema revision,
provider, adapter revision, and exact model revision. The later cache must use
the existing 30-day/user-clearable retention policy, expose clear controls,
and retain no credential or raw transport diagnostics.

## Provider privacy and retention disclosure

Metadata-only does not mean provider-local or retention-free. Before every
transmission the UI MUST identify the provider/model, state that path-free
storage metadata will leave the Mac, link the provider's current API data-
handling terms, disclose that the user's provider account may be billed, and
avoid claiming zero retention unless the exact account and endpoint prove it.

The OpenAI adapter review MUST force `store: false` and must not use background
mode, while truthfully disclosing that `store: false` does not itself disable
provider prompt caching or abuse-monitoring retention. As of this decision,
the public policy permits up to 30 days of abuse-monitoring retention and says
supported-model queries use extended prompt caching unless the organization
has Zero Data Retention; the UI must show the current reviewed policy rather
than infer ZDR from an API key. The Anthropic adapter review MUST omit optional
prompt caching, files, tools, and beta features unless a later adapter revision
separately approves them.
Changes in provider storage defaults or tool behavior suspend the affected
adapter until review; they do not fall back to another provider automatically.

## Implementation sequence

This ADR authorizes architecture work in this order:

1. bind the core shaper to an exact retained Explorer review and expose
   a preview-only, path-free, opaque single-use capability through UniFFI;
2. prove by module/source guards that AI cannot import or mint cleanup authority
   (**implemented 2026-08-09** through the dependency-free
   `DuxAIExplanationPresentation` target, private raw membership, one reviewed
   transport SPI import, immutable context projection, and opaque action
   admissions);
3. implement the bounded native lifecycle and credential store with injected
   network/Keychain fakes, still without a live credential in tests;
4. implement and review one fixed provider adapter and its tools-disabled,
   envelope, extraction, limit, cancellation, redirect, and retention tests;
5. add **Explain selection**, provider settings, metadata preview, inert group
   overlays, failure states, and accessibility (**implemented 2026-08-09**); and
6. add digest-bound cache/clear controls only after the uncached flow passes.

No step may introduce a generic transport and promise to narrow it later.

## Consequences

Benefits:

- provider code never executes under DUX's broad local filesystem authority;
- users can eventually choose a familiar built-in provider while inspecting
  exactly what leaves the Mac;
- credentials use the platform secret store rather than plaintext settings;
- the closed transport is small enough to test adversarially; and
- AI failure cannot weaken deterministic storage analysis or cleanup safety.

Costs:

- remote use requires network access, a provider account/key, and may cost
  money or subject metadata to provider retention;
- Claude Code/Codex subscriptions or installed commands cannot be reused;
- arbitrary OpenAI-compatible/self-hosted endpoints are not supported in v1;
- model/provider API changes require adapter revision and review; and
- the non-streaming 60-second/64-KiB contract may reject otherwise usable slow
  or verbose responses rather than weakening bounds.

## Alternatives considered

### Direct Claude, Codex, or custom subprocess

Rejected by ADR 0009. Cooperative flags do not remove ambient filesystem/TCC
authority from the provider host or its descendants.

### Separately App-Sandboxed XPC provider

Deferred. It may eventually support local models, but it requires a separate
signed component, narrow egress and credential brokering, descendant/platform
qualification, update compatibility, and proof that no powerful host IPC leaks
through the boundary.

### Virtual machine or bundled model runtime

Deferred. It adds large artifacts, CPU/RAM/storage cost, update and licensing
work, model supply-chain risk, and a substantially larger confinement surface.

### DUX-operated relay

Rejected for v1. A relay would make DUX an online service and add server-side
credential, privacy, abuse, retention, availability, and incident-response
obligations without strengthening the local proof boundary.

### Arbitrary OpenAI-compatible endpoint or provider plugin

Rejected. Caller-controlled egress and authentication are a credential-leak,
SSRF, privacy-bypass, and unbounded protocol surface. New providers use reviewed
built-in adapter revisions.

### Provider SDK

Rejected for v1. SDKs add transitive code, implicit retries/telemetry/defaults,
and update churn. The required HTTP surface is deliberately small.

## Validation criteria

This decision remains correctly implemented while:

- only disabled or the fixed consent-gated Anthropic Messages v1 revision-1
  path can be a runtime provider state; no generic selector or fallback exists;
- the privacy proof can reach the provider/network consumer only through the
  exact retained-review orchestration after local preview and one-shot consent;
- production source contains no direct local AI launch or generic caller-
  controlled network transport;
- adapter endpoints, methods, authentication, models, envelopes, headers, and
  extractors are code-owned and revisioned;
- every credential operation uses the exact data-protection Keychain class,
  service/account, synchronizability, accessibility, and access-group contract,
  and all tests use injected fakes rather than real credentials;
- lifecycle tests cover redirect, oversize/missing-length/compressed response,
  deadline, cancellation, late callback, status/media type, and no retry;
- provider fixtures prove no tools or persistent/background features are sent
  and tool-shaped output is rejected;
- the existing Rust validator is the only response-to-presentation admission;
- `DuxAIExplanationPresentation` compiles without an app/generated-FFI/action
  dependency, Browser/action views own no AI state or result type, raw group
  membership remains private behind one reviewed transport SPI import, the host
  can project only immutable context and render inert type-erased presentation,
  and no AI value can construct the opaque Trash, plan-review, cleanup, or
  dry-run admissions; and
- the app and CLI remain fully functional with AI disabled and provider failure
  changes no deterministic state.

## Reconsider when

Use a new ADR before supporting a new origin/provider, arbitrary/self-hosted
endpoint, DUX relay, provider SDK, streaming/background request, local provider,
App-Sandboxed helper, or VM. Reconsider the fixed limits only with measured
fixtures and an equally bounded replacement. Suspend an adapter when its API,
tool defaults, authentication, or retention behavior no longer matches its
reviewed revision.

## References

- [ADR 0003: Primary build without App Sandbox](0003-primary-build-without-app-sandbox.md)
- [ADR 0004: Shared Rust engine](0004-shared-rust-engine.md)
- [ADR 0005: UniFFI for Swift/Rust transport](0005-uniffi-swift-rust-transport.md)
- [ADR 0009: Reject direct local AI subprocess adapters](0009-reject-direct-local-ai-subprocesses.md)
- [DUX AI explanation contract](../AI_CONTRACT.md)
- [Anthropic Messages v1 provider review](../provider-reviews/anthropic-messages-v1.md)
- [Anthropic Messages API](https://platform.claude.com/docs/en/api/messages)
- [Anthropic tool use](https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview)
- [OpenAI Responses API guide](https://developers.openai.com/api/docs/guides/migrate-to-responses)
- [OpenAI structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs)
- [OpenAI API data controls](https://developers.openai.com/api/docs/guides/your-data)
- [Apple URLSession](https://developer.apple.com/documentation/foundation/urlsession)
- [Apple URLSessionTaskDelegate redirects](https://developer.apple.com/documentation/foundation/urlsessiontaskdelegate/urlsession(_:task:willperformhttpredirection:newrequest:completionhandler:))
- [Apple Keychain Services](https://developer.apple.com/documentation/security/keychain-services)
- [Apple data-protection Keychain selector](https://developer.apple.com/documentation/security/ksecusedataprotectionkeychain)
