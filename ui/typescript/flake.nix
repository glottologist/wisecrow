{
  description = "A flake for the typescript variant of wisecrow UI";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }: {
    devShell = nixpkgs.mkShell {
      buildInputs = with nixpkgs; [
        nodejs-14_x
        yarn
      ];
      shellHook = ''
        export NODE_PATH="${NODE_PATH:+$NODE_PATH:}$HOME/.yarn/global/node_modules"
        export PATH="$PATH:$HOME/.yarn/bin"
      '';
    };

    packages = {
      my-typescript-project = nixpkgs.mkDerivation {
        name = "my-typescript-project";
        src = ./.;
        buildInputs = with nixpkgs; [
          typescript
          ts-node
          rimraf
        ];
      };
    };
  };
}

