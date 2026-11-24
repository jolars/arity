# S4 dispatch observations

`expected.tsv` pins R 4.6.1's generic packages, selected arguments, defaults,
normalized method signatures, and S4 group generic metadata. Ordinary Rust tests
compare the shared index with every row. Registrations remain separate even when
R replaces a previous registration with the same signature. The observer also
checks that changing or explicitly forwarding an argument does not change the
signature that `callNextMethod()` uses to find the next method, that
`selectMethod()` ignores signature names, and that qualified `standardGeneric()`
calls retain defaults. Partial argument names in registrations are pinned too.

Run the live oracle with:

```sh
cargo test --test s4_dispatch live_r_s4_dispatch_observations -- --ignored
```

To refresh deliberately, run `Rscript --vanilla observe.R .` from this
directory, write stdout to `expected.tsv`, and review every changed row.
Production analysis never invokes R. The ordinary tests do not require an R
installation.

The semantics follow R's documentation for
[`setGeneric()`](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/setGeneric.html),
[`setMethod()`](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/setMethod.html),
[`standardGeneric()`](https://stat.ethz.ch/R-manual/R-devel/library/base/html/standardGeneric.html),
[`callNextMethod()`](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/NextMethod.html),
and [S4 group
generics](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/S4groupGeneric.html).
