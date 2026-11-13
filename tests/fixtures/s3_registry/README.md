# S3 registry observations

`expected.tsv` records R 4.6.1's NAMESPACE entries and runtime S3 registration
bindings for this fixture. Ordinary Rust tests compare the shared registry with
these observations. The observer uses only base R, and no production code runs R.
The implicit implementation has a distinct body so R's `identical()` can recover
its binding independently of `handler`.

Verify the observations with:

```sh
compute run -- cargo test --test s3_registry live_r_registration_observations -- --ignored
```

To refresh deliberately, run `Rscript --vanilla observe.R .` from this
directory, write stdout to `expected.tsv`, and review every changed row. The
columns are source, generic, class, implementation binding, and delayed generic
package (`-` for ordinary lookup). Ordinary exports remain independent of every
registration.
