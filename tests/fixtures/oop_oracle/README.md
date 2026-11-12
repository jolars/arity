# OOP runtime oracle

This developer-only oracle records R's behavior for S3, S4, Reference Classes,
R6, and S7. It complements the CLI and LSP coverage matrix in `../oop/`.
`fixture_names()` in `tests/oop_oracle.rs` registers each directory. Every case
contains executable `input.R`, a runtime observation script `observe.R`, and a
reviewed `expected.json` pin.

`versions.json` pins R 4.6.1, R6 2.6.1, and S7 0.2.2. The live driver checks all
three versions before evaluating a fixture. Missing dependencies or different
versions fail the explicitly requested oracle; they never silently skip cases.
Use an R installation matching the manifest, and install the exact R6 and S7
source releases into a developer library. `RSCRIPT` can select that
installation's `Rscript` executable, and `R_LIBS_USER` can select the library.
The oracle does not install packages, fetch dependencies, or execute R in
production.

For example, with the matching release tarballs already downloaded, install them
under a matching R installation:

```sh
mkdir -p /tmp/arity-oop-library
compute run -- R CMD INSTALL --library=/tmp/arity-oop-library R6_2.6.1.tar.gz S7_0.2.2.tar.gz
R_LIBS_USER=/tmp/arity-oop-library compute run -- task oop-oracle
```

Each original and formatted input runs in a fresh `Rscript --vanilla` process.
The driver creates a lexical owner environment with a stable package name so
runtime registrations cannot leak between cases. The observer uses runtime
introspection to collect:

- Class names, generator or constructor bindings, direct user-defined parents,
  and member names and kinds. S3 fields come from representative instances. S4
  slots, Reference Class fields, and S7 properties include inherited members. R6
  reports the generator's own fields, methods, and active bindings, including
  the generated `clone` method. Reference Class infrastructure methods and the
  implicit `envRefClass` parent are omitted.
- Constructor formals in order, with missing defaults represented by `null` and
  explicit defaults represented by R's deparsed expression. R6 records both
  `$new(...)` and `initialize` because the generator forwards arguments.
- Method signatures, formals, and enclosing frames. Each `environment` array
  lists the sorted bindings in successive frames, stopping before the fixture's
  lexical owner. An empty array means the method directly encloses that owner.
  This preserves R6's `self`, `private`, and `super` bindings and Reference
  Classes' extra `callSuper` frame without recording memory addresses.
- Dispatch expressions, representative receiver classes, and scalar results as
  strings. Cases cover S3 inheritance, default and operator methods, S4
  `callNextMethod()` and multiple dispatch, Reference Class `callSuper()`, R6
  overrides and active bindings, and S7 inherited and multiple dispatch.

The live test compares all facts with the pins and requires formatting to
preserve them. A separate live test verifies rejection of each mismatched
version. To refresh deliberately:

```sh
R_LIBS_USER=/tmp/arity-oop-library compute run -- task oop-oracle-refresh
git diff -- tests/fixtures/oop_oracle
R_LIBS_USER=/tmp/arity-oop-library compute run -- task oop-oracle
```

Refresh computes and validates every case before writing any pins. Review every
changed fact and its source; a difference needs an explanation, not automatic
acceptance. Input and observer text are pinned too, so ordinary Rust tests
reject stale observations after either script changes. A version update requires
an intentional manifest change, regeneration, and review of every affected pin.

Ordinary CI needs only Rust:

```sh
compute run -- cargo test --test oop_oracle
```

These tests deserialize the runtime facts, compare the currently supported S4,
Reference Class, and R6 class projection with R, and check lossless parsing and
formatter idempotence for every input. Insertions and deletions at character
boundaries exercise both successful incremental reparses and full-parse
fallback, comparing entire green trees and diagnostics with a fresh parse. The
richer member, environment, constructor, and dispatch facts are available for
the next semantic-model stages. The R6 environment cases also compare injected
bindings with semantic availability, including inherited method copies, private
methods, active bindings, defaults, nested closures, ordinary field functions,
and non-portable classes. The Reference Class environment case covers bare
fields, inherited methods and helpers, `.self`, `.refClassDef`, nested closures,
nonlocal field updates, and the ordinary lexical lookup of external methods. No
syntax gap was demonstrated by these cases.

The dispatch environment case records the metadata visible to S3 ordinary,
internal, group, and registered methods, S4 ordinary, primitive, and group
methods, and replacement methods. It also checks parameter shadowing, lazy
defaults, nested closures, and generic creation without an assignment. Rust
tests compare metadata availability with these observations and keep class-name
strings out of lexical bindings. Primitive names come from
`scripts/dump_base_symbols.R`, alongside the default-package export lists.
