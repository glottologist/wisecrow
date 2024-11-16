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
      "https://devenv.cachix.org"
    ];
    extra-trusted-public-keys = [
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
          rust = pkgs.callPackage ./rust/default.nix {inherit pkgs;};
          default = self'.packages.rust;
        };
        apps = {
          rustApp = flake-utils.lib.mkApp {drv = self'.packages.${system}.rust;};
        };

        devenv.shells.default = devenv.shells.rust;
        devenv.shells.rust = {
          devenv.root = let
            devenvRootFileContent = builtins.readFile devenv-root.outPath;
          in
            pkgs.lib.mkIf (devenvRootFileContent != "") devenvRootFileContent;
          name = "Wisecrow shell for Rust";
          env.GREET = "devenv for the Rust flavour of Wisecrow";
          env.PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
          env.POSTGRES_DB = POSTGRES_DB;
          env.POSTGRES_USER = POSTGRES_USER;
          env.POSTGRES_PASSWORD = POSTGRES_PASSWORD;
          env.DB_ADDR = DB_ADDR;
          packages = with pkgs; [
            git
            podman
            podman-tui
            podman-compose
            mdbook
            mdbook-i18n-helpers
            mdbook-mermaid
            openssh
            openssl
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
            ntest.exec = ''
              cargo nextest run --all --nocapture
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
              cargo watch -c -q -w ./rust/src -x build
            '';
          };
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
            settings.rust.cargoManifestPath = "./rust/Cargo.toml";
          };
        };
      };
    };
}
