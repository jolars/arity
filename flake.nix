{
  description = "A language server, formatter, and linter for R";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        arity = pkgs.rustPlatform.buildRustPackage {
          pname = "arity";
          version = "0.23.0";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
            # Git dependencies (the test-only air differential oracle plus its
            # transitive biome and tree-sitter-r deps). Keyed by git rev, so one
            # entry per source repo covers all crates sharing that rev.
            outputHashes = {
              "air_r_parser-0.0.0" = "sha256-44l09NxdvlWNHcpgGdH9hAi7WQKA08IFI4VdU6fa9hY=";
              "biome_console-0.5.7" = "sha256-/X2QmQxqSMn33RH6KvkAOxOCxSDQgHL9U6nS33U//Y4=";
              "tree-sitter-r-1.1.0" = "sha256-IkWhya0Mj+9Idu3OBN5wTuIY+nJA50BWbJhsw9N4Mv4=";
            };
          };

          nativeBuildInputs = [ pkgs.installShellFiles ];

          postInstall = ''
            installShellCompletion --cmd arity \
              --bash target/completions/arity.bash \
              --fish target/completions/arity.fish \
              --zsh target/completions/_arity

            installManPage target/man/*
          '';

          meta = with pkgs.lib; {
            description = "An LSP, formatter, and linter for R";
            homepage = "https://github.com/jolars/arity";
            license = licenses.mit;
            maintainers = [ ];
          };
        };
      in
      {
        packages = {
          default = arity;
          arity = arity;
        };

        apps = {
          default = {
            type = "app";
            program = "${arity}/bin/arity";
          };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            go-task
            R
          ];
        };
      }
    );
}
