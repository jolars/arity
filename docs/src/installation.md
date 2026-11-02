# Installation

<span id="getting-started"></span>

Arity is distributed as a single binary. Use the installer below for the command
line, or choose a package manager you already use.

For VS Code or Positron, the [Arity extension](guide/editors.md#vs-codepositron)
includes the binary, so you can start with [editor setup](guide/editors.md). For
formatting inside RStudio, install the [R package](#r-package-cran).

## Install script

The installer selects the release for your platform and installs to a user-local
directory.

::: code-group

```sh [macOS/Linux]
curl --proto '=https' --tlsv1.2 -sSf https://arity.cc/install | sh
```

```powershell [Windows]
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://arity.cc/install.ps1 | iex"
```

:::

Set `ARITY_INSTALL_DIR` to change the destination or `ARITY_TAG` to pin a
release.

## Package managers

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

## Prebuilt binaries

Download an archive from the [releases
page](https://github.com/jolars/arity/releases) and put the `arity` binary on
your `PATH`.

## Editor extensions

The [VS Code and Open VSX extension](guide/editors.md#vs-codepositron) bundles
the binary and starts the language server automatically. The [Zed
extension](guide/editors.md#zed) can download it or use an installed copy.

## From source

Clone the repository and build a release binary:

```bash
git clone https://github.com/jolars/arity
cd arity
cargo build --release
```

The binary is written to `target/release/arity`.

## R package (CRAN)

To use the formatter inside R, install the [`arity`
package](https://CRAN.R-project.org/package=arity):

```r
install.packages("arity")
```

See the [R package documentation](https://github.com/jolars/arity-r) for details
and [RStudio setup](guide/editors.md#rstudio) for use in the editor.

## Verify the installation {#first-run}

Check that Arity is available in your terminal:

```sh
arity --version
```

Continue with [Getting started](guide/getting-started.md) to format and lint
your first file, or [Editor setup](guide/editors.md) to use Arity in your
editor.
