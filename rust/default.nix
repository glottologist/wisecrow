{pkgs ? import <nixpkgs> {}}: let
  lib = pkgs.lib;
  common = import ./../../common.nix {inherit pkgs;};
in
  pkgs.rustPlatform.buildRustPackage rec {
    inherit (common) name ver;
    pname = name;
    version = ver;
    src = pkgs.lib.cleanSource ./.;
    # Specify the binary that will be installed
    cargoBinName = name;

    buildInputs = [pkgs.openssl];

    cargoLock = {
      lockFile = ./Cargo.lock;
    };

    # The package manager needs to know the SHA-256 hash of your dependencies
    cargoSha256 = pkgs.lib.fakeSha256;

    meta = with pkgs.stdenv.lib; {
      inherit (common) maintainers homepage description licenses;
    };
  }
