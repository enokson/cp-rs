{
  description = "cp-rs";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };
  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ 
        "x86_64-linux"
        # add other supported systems here
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = nixpkgs.legacyPackages;
    in {
      packages = forAllSystems (system: {
        default = pkgsFor.${system}.callPackage ./. { };
      });
      devShells = forAllSystems (system: {
        default = pkgsFor.${system}.callPackage ./shell.nix { };
      });
    };
}

