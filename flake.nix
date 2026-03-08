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

        runtimeLibs = pkgs.lib.makeLibraryPath [
          pkgs.gtk4
          pkgs.gtk4-layer-shell
          pkgs.glib
          pkgs.cairo
          pkgs.pango
          pkgs.gdk-pixbuf
          pkgs.graphene
          pkgs.fontconfig
          pkgs.freetype
        ];

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          buildInputs = [ pkgs.gtk4 pkgs.gtk4-layer-shell ];
          nativeBuildInputs = [ pkgs.pkg-config pkgs.wrapGAppsHook4 ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        status-overlay = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          postInstall = ''
            wrapProgram $out/bin/status-overlay \
              --prefix LD_LIBRARY_PATH : ${runtimeLibs}
          '';
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
          packages = [ status-overlay toolchain pkgs.rust-analyzer ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.gtk4
            pkgs.gtk4-layer-shell
            pkgs.glib
            pkgs.cairo
            pkgs.pango
            pkgs.gdk-pixbuf
            pkgs.graphene
            pkgs.fontconfig
            pkgs.freetype
          ];
        };
      });
}
