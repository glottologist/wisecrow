{pkgs ? import <nixpkgs> {}}: let
  lib = pkgs.lib;
in {
  name = "wisecrow";
  ver = "0.1.0";
  homepage = "https://github.com/glottologist/wisecrow";
  description = "Wisecrow Language Learning";
  license = lib.licences.agpl3Plus;
  maintainers = with lib.maintainers; [
    {
      name = "Jason Ridgway-Taylor";
      email = "jason@glottologist.co.uk";
      github = "glottologist";
    }
  ];
}
