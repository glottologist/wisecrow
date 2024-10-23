{pkgs ? import <nixpkgs> {}}: let
  lib = pkgs.lib;
  common = import ../common.nix {inherit pkgs;};
in
  pkgs.rustPlatform.buildRustPackage rec {
    inherit (common) name ver;
    pname = name;
    version = ver;
    src = pkgs.lib.cleanSource ./.;
    # Specify the binary that will be installed
    cargoBinName = name;

    nativeBuildInputs = with pkgs; [
      pkg-config
    ];

    buildInputs = with pkgs; [
      openssl
      openssl.dev
      openssh
    ];

    cargoLock = {
      lockFile = ./Cargo.lock;
    };

    # Temporary fake hash, replace with real one after first build
    cargoSha256 = pkgs.lib.fakeSha256;

    meta = with pkgs.stdenv.lib; {
      inherit (common) maintainers homepage description licenses;
    };
  }
