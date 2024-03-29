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
pkgs.rustPlatform.buildRustPackage rec {
  pname = name;
  version = ver;

  src = ./.;

  # Specify the binary that will be installed
  cargoBinName = name;

  # The package manager needs to know the SHA-256 hash of your dependencies
  cargoSha256 = "ZY2NSSgXFRMf7Z4uMaI9AJddGph88aHEqaE//txVgbc=";

  meta = with pkgs.stdenv.lib; {
    homepage = homepage;
    description = description;
    license = license;
    maintainers = maintainers;
  };
}
