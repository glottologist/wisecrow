{ lib, beamPackages, overrides ? (x: y: {}) }:

let
  buildRebar3 = lib.makeOverridable beamPackages.buildRebar3;
  buildMix = lib.makeOverridable beamPackages.buildMix;
  buildErlangMk = lib.makeOverridable beamPackages.buildErlangMk;

  self = packages // (overrides self packages);

  packages = with beamPackages; with self; {

    optimus = buildMix rec {
      name = "optimus";
      version = "0.3.0";

      src = fetchHex {
        pkg = "${name}";
        version = "${version}";
        sha256 = "0v4l0ppwjbfxj0j4xaim46a0kq8mvai3wiiw2zhn3r38y3yjdl6h";
      };

      beamDeps = [];
    };
  };
in self

