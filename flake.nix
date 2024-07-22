{
  description = "Flake for Wisecrow";

  inputs = {
    devenv-root = {
      url = "file+file:///dev/null";
      flake = false;
    };
    nixpkgs.url = "github:cachix/devenv-nixpkgs/rolling";
    fenix.url = "github:nix-community/fenix";
    devenv.url = "github:cachix/devenv";
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-utils.url = "github:numtide/flake-utils";
    nix2container.url = "github:nlewo/nix2container";
    nix2container.inputs.nixpkgs.follows = "nixpkgs";
    mk-shell-bin.url = "github:rrbutani/nix-mk-shell-bin";
  };

  nixConfig = {
    extra-substituters = [
      "https://tweag-jupyter.cachix.org"
      "https://devenv.cachix.org"
    ];
    extra-trusted-public-keys = [
      "tweag-jupyter.cachix.org-1:UtNH4Zs6hVUFpFBTLaA4ejYavPo5EFFqgd7G7FxGW9g="
      "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw="
    ];
  };

  outputs = inputs @ {
    flake-parts,
    flake-utils,
    nixpkgs,
    devenv-root,
    ...
  }:
    flake-parts.lib.mkFlake {inherit inputs;} {
      imports = [
        inputs.devenv.flakeModule
      ];

      systems = inputs.nixpkgs.lib.systems.flakeExposed;

      perSystem = {
        config,
        self',
        inputs',
        pkgs,
        system,
        ...
      }: let
        POSTGRES_DB = "wisecrow";
        POSTGRES_USER = "wisecrow";
        POSTGRES_PASSWORD = "wisecrow";
        DB_HOST = "127.0.0.1";
        DB_PORT = 5432;
        DB_ADDR = "${DB_HOST}:${toString DB_PORT}";
      in rec {
        packages = rec {
          rust = pkgs.callPackage ./src/rust/default.nix {inherit pkgs;};
          ocaml = pkgs.callPackage ./src/ocaml/default.nix {inherit pkgs;};
          haskell = pkgs.callPackage ./src/haskell/default.nix {inherit pkgs;};
          golang = pkgs.callPackage ./src/golang/wisecrow/default.nix {inherit pkgs;};
          default = self'.packages.rust;
        };
        apps = {
          rustApp = flake-utils.lib.mkApp {drv = self'.packages.${system}.rust;};
          ocamlApp = flake-utils.lib.mkApp {drv = self'.packages.${system}.ocaml;};
          haskellApp = flake-utils.lib.mkApp {drv = self'.packages.${system}.haskell;};
          golangApp = flake-utils.lib.mkApp {drv = self'.packages.${system}.golang;};
        };

        devenv.shells.default = devenv.shells.rust;
        devenv.shells.db = {
          devenv.root = let
            devenvRootFileContent = builtins.readFile devenv-root.outPath;
          in
            pkgs.lib.mkIf (devenvRootFileContent != "") devenvRootFileContent;
          name = "Wisecrow shell for Postgres";
          env.POSTGRES_DB = POSTGRES_DB;
          env.POSTGRES_USER = POSTGRES_USER;
          env.POSTGRES_PASSWORD = POSTGRES_PASSWORD;
          env.DB_ADDR = DB_ADDR;
          services.postgres = {
            enable = true;
            listen_addresses = "${DB_HOST}";
            port = DB_PORT;
            initialDatabases = [{name = "${POSTGRES_DB}";}];
            initialScript = ''
              CREATE ROLE ${POSTGRES_USER} WITH LOGIN PASSWORD '${POSTGRES_PASSWORD}';
              GRANT ALL PRIVILEGES ON DATABASE ${POSTGRES_DB} TO ${POSTGRES_USER};
              \c ${POSTGRES_DB}
              GRANT ALL PRIVILEGES ON SCHEMA public TO ${POSTGRES_USER};

            '';
            settings = {
              log_connections = true;
              log_statement = "all";
            };
          };
        };
        devenv.shells.rust = {
          devenv.root = let
            devenvRootFileContent = builtins.readFile devenv-root.outPath;
          in
            pkgs.lib.mkIf (devenvRootFileContent != "") devenvRootFileContent;
          name = "Wisecrow shell for Rust";
          env.GREET = "devenv for the Rust flavour of Wisecrow";
          env.PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
          packages = with pkgs; [
            git
            podman
            podman-tui
            podman-compose
            mdbook
            mdbook-i18n-helpers
            mdbook-mermaid
            openssh
            pkg-config
          ];
          enterShell = ''
            cargo install cargo-watch
            cargo install cargo-modules
            cargo install cargo-audit
            cargo install cargo-nextest
            cargo install cargo-expand
            git --version
            nix --version
            rustc --version
            cargo --version
            mdbook --version
          '';
          languages = {
            rust.enable = true;
            rust.channel = "nightly";
            nix.enable = true;
          };
          scripts = {
            podup.exec = ''
              podman-compose -f ./datastore/docker-compose.yml up -d
            '';
            poddown.exec = ''
              podman-compose -f ./datastore/docker-compose.yml down
            '';

            nextest.exec = ''
              cargo nextest run
            '';
            audit.exec = ''
              cargo audit
            '';
            lib.exec = ''
              cargo modules structure --lib
            '';
            bin.exec = ''
              cargo modules structure --bin acp
            '';

            watch.exec = ''
              cargo watch -c -q -w ./src -x build
            '';
          };
          dotenv.enable = true;
          difftastic.enable = true;
          pre-commit = {
            hooks = {
              alejandra.enable = true;
              commitizen.enable = true;
              cargo-check.enable = true;
              clippy.enable = true;
              rustfmt.enable = true;
              nil.enable = true;
            };
            settings.rust.cargoManifestPath = "./src/rust/Cargo.toml";
          };
        };
        devenv.shells.haskell = {
          name = "Wisecrow shell for Haskell";
          env.GREET = "devenv for the Haskell flavour of Wisecrow";
          packages = with pkgs; [
            git
            podman
            podman-tui
            podman-compose
            mdbook
            mdbook-i18n-helpers
            mdbook-mermaid
          ];
          scripts = {
            podup.exec = ''
              podman-compose -f ./datastore/docker-compose.yml up -d
            '';
            poddown.exec = ''
              podman-compose -f ./datastore/docker-compose.yml down
            '';
          };
          enterShell = ''
            nix --version
            ghc --version
            mdbook --version
            podman-compose -f ./datastore/docker-compose.yml up -d
          '';
          languages = {
            haskell.enable = true;
            nix.enable = true;
          };
          dotenv.enable = true;
          difftastic.enable = true;
          pre-commit.hooks = {
            commitizen.enable = true;
            ormolu.enable = true;
            hlint.enable = true;
            cabal2nix.enable = true;
            nil.enable = true;
          };
        };
        devenv.shells.ocaml = {
          name = "Wisecrow shell for Ocaml";
          env.GREET = "devenv for the Ocaml flavour of Wisecrow";
          packages = with pkgs; [
            git
            podman
            podman-tui
            podman-compose
            mdbook
            mdbook-i18n-helpers
            mdbook-mermaid
          ];
          scripts = {
            podup.exec = ''
              podman-compose -f ./datastore/docker-compose.yml up -d
            '';
            poddown.exec = ''
              podman-compose -f ./datastore/docker-compose.yml down
            '';
          };
          enterShell = ''
            nix --version
            ocaml --version
            opam --version
            dune --version
            mdbook --version
            podman-compose -f ./datastore/docker-compose.yml up -d
          '';
          languages = {
            ocaml.enable = true;
            nix.enable = true;
          };
          dotenv.enable = true;
          difftastic.enable = true;
          pre-commit.hooks = {
            commitizen.enable = true;
            ocp-indent.enable = true;
            opam-lint.enable = true;
            dune-fmt.enable = true;
            nil.enable = true;
          };
        };
        devenv.shells.go = {
          name = "Wisecrow shell for Go";
          env.GREET = "devenv for the Go flavour of Wisecrow";
          packages = with pkgs; [
            git
            podman
            podman-tui
            podman-compose
            mdbook
            mdbook-i18n-helpers
            mdbook-mermaid
          ];
          scripts = {
            podup.exec = ''
              podman-compose -f ./datastore/docker-compose.yml up -d
            '';
            poddown.exec = ''
              podman-compose -f ./datastore/docker-compose.yml down
            '';
          };
          enterShell = ''
            nix --version
            go version
            mdbook --version
            podman-compose -f ./datastore/docker-compose.yml up -d
          '';
          languages = {
            go.enable = true;
            nix.enable = true;
          };
          dotenv.enable = true;
          difftastic.enable = true;
          pre-commit.hooks = {
            commitizen.enable = true;
            gofmt.enable = true;
            nil.enable = true;
          };
        };
      };
    };
}
