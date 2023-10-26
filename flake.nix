{
  description = "Wisecrow Flake";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nix-filter.url = "github:numtide/nix-filter";
  };
  outputs = {
    self,
    nixpkgs,
    flake-utils,
    nix-filter,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        name = "wisecrow";
        ver = "0.1.0";
        homepage = "https://github.com/glottologist/wisecrow";
        description = "An intensive language learning app";
        license = pkgs.lib.licenses.mit;
        maintainers = [maintainers.glottologist];
        buildF = path: pkgs.callPackage path {inherit name ver homepage description license maintainers nix-filter;};
        runScylla = pkgs.callPackage {inherit pkgs;};
      in rec {
        packages = {
          haskellCmdApp = buildF ./cmd/haskell;
          ocamlCmdApp = buildF ./cmd/ocaml;
          rustCmdApp = buildF ./cmd/rust;
        };

        devShells = {
          haskell = pkgs.mkShell {
            buildInputs = with pkgs; [
              runScylla
              ghc
              cabal-install
              cabal2nix
            ];
          };
          ocaml = pkgs.mkShell {
            buildInputs = with pkgs; [
              runScylla
              opam2nix
              ocaml
              ocamlPackages.opam
              ocamlPackages.core
              ocamlPackages.merlin
            ];
          };
          rust = pkgs.mkShell {
            buildInputs = [
              runScylla.dockerComposeEnvironmentpkgs
              pkgs.rustc
              pkgs.cargo
            ];
          };

          docker-scylla = pkgs.mkShell {
            buildInputs = [pkgs.docker-compose];

            shellHook = ''
              echo "Starting Docker Compose..."
              docker-compose up -d
            '';
          };
        };

        defaultPackage = packages.rustCmdApp;
        defaultDevShell = devShells.rust;
      }
    );
}
