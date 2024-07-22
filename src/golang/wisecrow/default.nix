{pkgs ? import <nixpkgs> {}}: let
  lib = pkgs.lib;
  common = import ../common.nix {inherit pkgs;};
in
  pkgs.buildGoPackage rec {
    name = "wisecrow";
    src = ./.;
    goPackagePath = "github.com/glottologist/wisecrow/src/golang/wisecrow";
    meta = {
      description = "A simple Go project built with Nix";
      homepage = "https://github.com/glottologist/wisecrow";
      license = pkgs.lib.licenses.mit;
    };
  }
