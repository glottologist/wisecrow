{
  description = "A flake for the Ocaml variant of wisecrow";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-21.11";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        ocaml = pkgs.ocaml-ng.ocamlPackages_4_12.ocaml;
        dune = pkgs.ocaml-ng.ocamlPackages_4_12.dune;
        utop = pkgs.ocaml-ng.ocamlPackages_4_12.utop;
        merlin = pkgs.ocaml-ng.ocamlPackages_4_12.merlin;
        ocp-indent = pkgs.ocaml-ng.ocamlPackages_4_12.ocp-indent;
      in
      {
        devShell = pkgs.mkShell {
          buildInputs = [
            ocaml
            dune
            utop
            merlin
            ocp-indent
          ];

          OCAMLFIND_DESTDIR = "${pkgs.lib.concatMapStringsSep ":" (p: "${p}/lib/ocaml") [ ocaml ]}";
          DUNE_BUILD_DIR = "_build";
        };
      });
}

