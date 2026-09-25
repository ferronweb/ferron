{
  description = "Ferron web server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forEachSystem = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
    in
    {
      packages = forEachSystem (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          ferron = pkgs.callPackage ./nix/package.nix { };
          ferron-bin = pkgs.callPackage ./nix/package-bin.nix { };
          # Prebuilt release binaries by default: fast install,
          # PGO-optimized, version-pinned (see nix/package-bin.nix).
          # Use `.#ferron` for the source build.
          default = self.packages.${system}.ferron-bin;
        }
      );

      overlays.default = final: prev: {
        ferron = final.callPackage ./nix/package.nix { };
        ferron-bin = final.callPackage ./nix/package-bin.nix { };
      };

      nixosModules.default = import ./nix/module.nix;
      nixosModules.ferron = self.nixosModules.default;
    };
}
