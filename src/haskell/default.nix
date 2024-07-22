{
  name,
  ver,
  homepage,
  description,
  license,
  maintainers,
  nix-filter,
  pkgs ? import <nixpkgs> {},
}: let
  wiseCrowPackage = pkgs.haskellPackages.mkDerivation {
    pname = name;
    version = ver;
    src = ./.;
    isLibrary = false;
    isExecutable = true;
    executableHaskellDepends = with pkgs.haskellPackages; [
      base
    ];
    license = license;
    maintainers = maintainers;
  };
in
  wiseCrowPackage
