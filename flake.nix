{
  description = "discourse-rs: a Rust port of Discourse";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    # The revision shell.nix pins, so both build the same Rust release.
    rust-overlay = {
      url = "github:oxalica/rust-overlay/368fee9beaab04ca6fe7af28db63caa9badb22fa";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems =
        f:
        lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );
    in
    {
      packages = forAllSystems (pkgs: rec {
        discourse-rs = pkgs.callPackage ./nix/package.nix { };
        discourse-assets = pkgs.callPackage ./nix/assets.nix { };
        default = discourse-rs;
      });

      # services.discourse-rs, with this flake's builds as its packages.
      nixosModules.default =
        { pkgs, ... }:
        let
          packages = self.packages.${pkgs.stdenv.hostPlatform.system};
        in
        {
          imports = [ ./nix/module.nix ];
          services.discourse-rs = {
            package = lib.mkDefault packages.discourse-rs;
            assets = lib.mkDefault packages.discourse-assets;
          };
        };

      checks = forAllSystems (pkgs: {
        vm = import ./nix/vm-test.nix {
          inherit pkgs;
          module = self.nixosModules.default;
        };
      });
    };
}
