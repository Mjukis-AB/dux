# Synthetic rule-catalog fixtures

Every rule in this directory is fictional test data. Fixture IDs begin with
`fixture.`, provenance uses `example.com`, and matcher names do not describe a
researched cleanup target.

Production code does not embed, enumerate, or load this directory. These files
exercise the checked JSON Schema and the internal invariant-preserving loader;
passing validation does not make a rule trusted, shipped, actionable, or safe
to execute.

The buckets have distinct contracts:

- `schema-valid`: the JSON document must satisfy the checked schema and load.
- `schema-invalid`: the JSON document must fail both schema and loader checks.
- `domain-invalid`: the loader must reject a domain or registry invariant even
  when that constraint is not completely expressible in JSON Schema.
