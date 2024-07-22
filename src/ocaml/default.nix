{
  name,
  ver,
  homepage,
  description,
  license,
  maintainers,
  nix-filter,
  pkgs ? import <nixpkgs> {},
}:
pkgs.ocamlPackages.buildDunePackage rec {
  pname = name;
  version = ver;
  src = ./.;

  buildInputs = [
    pkgs.ocamlPackages.ocaml
  ];

  propagatedBuildInputs = with pkgs.ocamlPackages; [
    dune_2
    /*
    Add your other opam dependencies here
    */
  ];
}
