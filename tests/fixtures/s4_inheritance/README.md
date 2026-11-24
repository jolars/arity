# S4 inheritance observations

`observe.R` records R's inherited method choices, ambiguity conditions,
primitive and group dispatch, S3 registration bridges, and Reference Class
storage. The pins were produced with R 4.6.1. The Rust tests read `expected.tsv`
without R.

R can choose a method based on superclass order, including when equally near
superclasses supply methods for one argument. Arity preserves those alternatives
as ambiguous. For every observation, R's chosen method must occur among Arity's
best candidates. A definite Arity selection must agree with R.

The built-in generic signatures, full formals, callable identities, and class
relationships are generated separately by `scripts/dump_s4_builtins.R`.
Production analysis reads these static tables. Use these commands to refresh and
verify them.

```sh
Rscript --vanilla scripts/dump_s4_builtins.R
Rscript --vanilla tests/fixtures/s4_inheritance/observe.R tests/fixtures/s4_inheritance
cargo test --test s4_inheritance -- --ignored
```

The namespace oracle installs temporary packages under a temporary library. It
checks that class metadata, method tables, generic ownership, and lexical
bindings retain their separate meanings across package boundaries. It removes
the temporary library on exit.

References: R's [method selection
documentation](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/Methods_Details.html),
[method registration
documentation](https://stat.ethz.ch/R-manual/R-devel/library/methods/html/setMethod.html),
and [namespace
implementation](https://github.com/wch/r-source/blob/trunk/src/library/base/R/namespace.R).
