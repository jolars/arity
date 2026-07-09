# arity-cli

[Arity](https://arity.cc) is a language server, formatter, and linter for the R
programming language.

## Install

```sh
npm install -g arity-cli
```

This installs the `arity` command globally. The package detects your platform at
install time and pulls in a prebuilt binary via npm's optional dependencies---no
Rust toolchain or postinstall download required.

You can also use it without a global install:

```sh
npx arity-cli format file.R
```

## Usage

```sh
arity format file.R             # format in place
arity format <file.R            # read stdin, write stdout
arity format --check path/      # check without writing
arity lint file.R               # lint
arity lsp                       # start the language server
```

See `arity --help` and the [documentation](https://arity.cc) for the full
feature list and configuration reference.

## Supported platforms

Prebuilt binaries are shipped for:

- Linux x64 (glibc and musl)
- Linux arm64 (glibc and musl)
- macOS x64 (Intel) and arm64 (Apple Silicon)
- Windows x64 and arm64

If your platform isn't covered, install via
[Cargo](https://crates.io/crates/arity),
[PyPI](https://pypi.org/project/arity/), or one of the other methods listed at
<https://arity.cc>.

## License

MIT---see [LICENSE](./LICENSE).
