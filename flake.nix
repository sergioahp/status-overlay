{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, fenix, crane }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        toolchain = fenix.packages.${system}.stable.toolchain;
        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          buildInputs = [ pkgs.gtk4 pkgs.gtk4-layer-shell ];
          nativeBuildInputs = [ pkgs.pkg-config ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        status-overlay = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });
      in
      {
        packages.default = status-overlay;

        checks = {
          inherit status-overlay;
          clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "-- --deny warnings";
          });
          fmt = craneLib.cargoFmt { src = ./.; };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ status-overlay ];
          packages = [ toolchain pkgs.rust-analyzer ];
        };
      });
}
