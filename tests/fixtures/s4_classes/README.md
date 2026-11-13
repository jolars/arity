# S4 class observations

`expected.tsv` records observations from R 4.6.1. The ordinary tests in
`tests/s4_classes.rs` compare Arity's class metadata, inherited slots,
prototypes, and relationships with these observations. R runs only in the
ignored developer oracle:

```sh
compute run -- cargo test --test s4_classes live_r_s4_observations -- --ignored
```

The fixture covers legacy representations, multiple inheritance, virtual and
sealed classes, union membership, conditional and coercing `setIs()` relations,
explicit coercions, old-class bridges, copied S4 definitions, data slots, and
slot reads and replacements. It also covers reused old-class ancestors,
prototype-backed old classes, self-template bridges, copied relation edges, the
`ANY`, `S4`, and `NULL` data slots, and first-parent implicit defaults. Validity
and coercion bodies stay opaque in Arity.
