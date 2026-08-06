{
  pkgs,
  ...
}:

{
  packages = [
    pkgs.bashInteractive
    pkgs.perf
    pkgs.google-lighthouse
    pkgs.cargo-flamegraph
    pkgs.cargo-llvm-cov
    pkgs.cargo-audit
    pkgs.cargo-deny
    pkgs.cargo-insta
    pkgs.go-task
    pkgs.jarl
    pkgs.mdbook
    pkgs.llvmPackages.bintools
    pkgs.biome
    pkgs.prettier
    pkgs.air-formatter
    pkgs.ruff
    pkgs.shfmt
    pkgs.wasm-pack
    pkgs.stylua
    pkgs.hyperfine
    pkgs.yamlfmt
    pkgs.vsce
  ];

  languages = {
    rust = {
      enable = true;

      toolchainFile = ./rust-toolchain.toml;
    };

    r = {
      enable = true;

      package = (
        pkgs.rWrapper.override {
          packages = with pkgs.rPackages; [
            languageserver
            # roxygen2 + commonmark back the roxygen oracle harness
            # (tests/roxygen_oracle.rs / tests/oracle/roxygen_oracle.R). Declared
            # explicitly rather than relying on languageserver pulling them in.
            roxygen2
            commonmark
            styler
            lintr
            flir
          ];
        }
      );
    };

    python = {
      enable = true;

      package = (
        pkgs.python3.withPackages (
          ps: with ps; [
            openai
          ]
        )
      );
    };

    javascript = {
      enable = true;

      pnpm = {
        enable = true;

        install = {
          enable = true;
        };
      };
    };

    typescript = {
      enable = true;
    };
  };

  git-hooks = {
    hooks = {
      clippy = {
        enable = true;
        settings = {
          allFeatures = true;
        };
      };

      rustfmt = {
        enable = true;
      };

      biome = {
        enable = true;
        # The hook feeds biome every staged js/ts/json file, but `biome.jsonc`
        # scopes it to a few paths. Without this, a commit whose only matching
        # file is out of scope (any JSON, say) fails with "no files processed".
        settings.flags = "--no-errors-on-unmatched";
      };

      # panache-format = {
      #   enable = true;
      # };
    };
  };
}
