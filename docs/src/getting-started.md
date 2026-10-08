# Getting Started

## Installation

### Cargo

Install the Arity CLI from [crates.io](https://crates.io/crates/arity) with
Cargo:

```bash
cargo install arity
```

### Homebrew

On macOS or Linux:

```bash
brew install jolars/tap/arity
```

### npm

The `arity-cli` package bundles a prebuilt binary:

```bash
npm install -g arity-cli
```

### PyPI

Install the binary as a Python tool:

```bash
uv tool install arity
```

Alternatively, use `pipx install arity`.

### Arch Linux

Install the prebuilt
[`arity-bin`](https://aur.archlinux.org/packages/arity-bin/) package with an AUR
helper:

```bash
paru -S arity-bin
```

### Nix

Arity is available as `arity` in
[Nixpkgs](https://search.nixos.org/packages?channel=unstable&show=arity). For a
shell with Arity available:

```bash
nix shell nixpkgs#arity
```

For a persistent NixOS installation, add `pkgs.arity` to
`environment.systemPackages`.

### mise and Aqua

[mise](https://mise.jdx.dev/dev-tools/backends/aqua.html) can install Arity
through its Aqua backend:

```bash
mise use aqua:jolars/arity
```

Commit the resulting `mise.toml` to share the selected version. With
[Aqua](https://aquaproj.github.io/docs/tutorial/) directly, run `aqua init` if
the project has no `aqua.yaml`, then add and install Arity:

```bash
aqua g -i jolars/arity
aqua install
```

Commit `aqua.yaml` to share the selected version. Both tools use the
[`jolars/arity` registry
entry](https://github.com/aquaproj/aqua-registry/tree/main/pkgs/jolars/arity).

### Install Script

The installer selects the release for your platform and installs to a user-local
directory. On macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://arity.cc/install | sh
```

On Windows, run this in PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://arity.cc/install.ps1 | iex"
```

Set `ARITY_INSTALL_DIR` to change the destination or `ARITY_TAG` to pin a
release.

### Prebuilt Binaries

Download an archive from the [releases
page](https://github.com/jolars/arity/releases) and put the `arity` binary on
your `PATH`.

### Editor Extensions

The [VS Code and Open VSX extension](guide/editors.md#vs-codepositron) bundles
the binary and starts the language server automatically. The [Zed
extension](guide/editors.md#zed) can download it or use an installed copy.

### From Source

Clone the repository and build a release binary:

```bash
git clone https://github.com/jolars/arity
cd arity
cargo build --release
```

The binary is written to `target/release/arity`.

### R Package (CRAN)

To use the formatter inside R, install the [`arity`
package](https://CRAN.R-project.org/package=arity):

```r
install.packages("arity")
```

See the [R package documentation](https://github.com/jolars/arity-r) for details
and [RStudio setup](guide/editors.md#rstudio) for use in the editor.

## First Run

With the CLI installed, run these commands in a terminal.

Format a file in place:

```bash
arity format file.R
```

Check formatting without writing changes:

```bash
arity format --check file.R
```

Lint a file (or pipe from stdin):

```bash
arity lint file.R
```

Run the language server over stdio (for editor integration):

```bash
arity lsp
```

See the [CLI Reference](reference/cli.md) for the full set of commands and
options.
