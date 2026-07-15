# DUX path-validation fuzzing

This isolated, non-shipping crate exercises only bounded in-memory lexical and
protected-policy invariants. It never probes or mutates the filesystem, and its
`cfg(fuzzing)`-gated adapter is absent from normal builds and returns no
cleanup authority.

Install the pinned runner and use a pinned nightly:

```bash
cargo install cargo-fuzz --version 0.13.2 --locked
cargo +nightly-2026-07-14 fuzz build path_validation
cargo +nightly-2026-07-14 fuzz run path_validation -- \
  -seed=6845581 -runs=20000 -max_len=65536 -timeout=2
```

Run those commands from this `fuzz/` directory. Promote every minimized crash
or invariant violation into the versioned dangerous-path JSON corpus before
fixing it, so ordinary cross-platform CI retains the regression permanently.
