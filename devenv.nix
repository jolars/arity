{
  pkgs,
  ...
}:

{
  packages = [
    pkgs.bashInteractive
    pkgs.perf
    pkgs.cargo-flamegraph
    pkgs.cargo-llvm-cov
    pkgs.cargo-audit
    pkgs.cargo-deny
    pkgs.cargo-insta
    pkgs.cmark
    pkgs.go-task
    pkgs.jarl
    pkgs.llvmPackages.bintools
    pkgs.prettier
    pkgs.quartoMinimal
    pkgs.air-formatter
    pkgs.ruff
    pkgs.shfmt
    pkgs.wasm-pack
    pkgs.stylua
    pkgs.hyperfine
    pkgs.yamlfmt
    pkgs.vsce
    (pkgs.rWrapper.override {
      packages = with pkgs.rPackages; [
        knitr
        rmarkdown
        bookdown
        languageserver
      ];
    })
  ];

  languages = {
    rust = {
      enable = true;
    };

    r = {
      enable = true;

      package = (
        pkgs.rWrapper.override {
          packages = with pkgs.rPackages; [
            languageserver
          ];
        }
      );
    };

    javascript = {
      enable = true;

      # corepack.enable = true;

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

      eslint = {
        enable = true;
      };
    };
  };
}
