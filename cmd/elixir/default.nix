{ name, ver, homepage, description, license, maintainers, nix-filter, pkgs ? import <nixpkgs> {} }:
with pkgs;
with beamPackages;
  mixRelease {
    pname = name;
    version = ver;
    src = ./.;
    mixNixDeps = import ./deps.nix {inherit lib beamPackages;};
  }
