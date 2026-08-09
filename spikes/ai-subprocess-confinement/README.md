# AI subprocess confinement spike

This directory contains a non-shipping adversarial positive control for
[ADR 0009](../../docs/adr/0009-reject-direct-local-ai-subprocesses.md). It does
not link into DUX, invoke a provider, use the network, or inspect personal data.

Build with the system C compiler and warnings as errors. The fixed output
directory keeps the reviewed host and child adjacent without adding a shipping
target or an indirect build-script command:

```sh
mkdir -p /private/tmp/dux-ai-subprocess-confinement-build
xcrun clang -std=c11 -Wall -Wextra -Werror -O2 \
  spikes/ai-subprocess-confinement/host.c \
  -o /private/tmp/dux-ai-subprocess-confinement-build/host
xcrun clang -std=c11 -Wall -Wextra -Werror -O2 \
  spikes/ai-subprocess-confinement/hostile_child.c \
  -o /private/tmp/dux-ai-subprocess-confinement-build/hostile-child
```

Run both the direct lane and the deprecated `sandbox-exec` comparison:

```sh
/private/tmp/dux-ai-subprocess-confinement-build/host
```

The host writes one path-free observation to standard output. Before that
observation becomes accepted evidence, add the closed schema's `source` object
with the repository base commit, SHA-256 digests of the schema, both source
files, and both tested binaries, and the exact compiler identity shown by
`xcrun clang --version`. Verify those digests with `shasum -a 256`; do not alter
the host-produced observation fields. The resulting record must validate
against `evidence-v1.schema.json`.

A direct-lane `ambient_read_succeeded` is the expected positive control and
produces the `no_go` decision. On macOS, `read_denied` in the deprecated lane
demonstrates that the probe can observe an applied kernel restriction; that
lane always reports `production_eligible=false`. An enclosing sandbox may
prevent `sandbox-exec` from applying, which reports an unavailable or failed
comparison and never changes the direct result.

The host creates a unique root only under `/private/tmp`, writes a fixed canary,
passes its known absolute path to the adversary, and reports no path or content.
Successful and setup-failure paths use descriptor- and identity-checked cleanup
of only those exact fixtures. If an identity cannot be proven or changes, the
host refuses the affected removal and may leave a private 0700 temporary root
containing only the fixed non-secret canary. Do not modify it to accept a user
path or to probe real TCC-protected data.
