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
          pkgs.sqlite
        ];

        commonArgs = {
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              (craneLib.filterCargoSources path type)
              || pkgs.lib.hasSuffix ".css" path;
          };
          buildInputs = [ pkgs.gtk4 pkgs.gtk4-layer-shell pkgs.sqlite ];
          nativeBuildInputs = [ pkgs.pkg-config pkgs.wrapGAppsHook4 ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        status-overlay = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          preCheck = ''
            export LD_LIBRARY_PATH=${runtimeLibs}:$LD_LIBRARY_PATH
          '';
          postInstall = ''
            wrapProgram $out/bin/status-overlay \
              --prefix LD_LIBRARY_PATH : ${runtimeLibs} \
              --set STATUS_OVERLAY_CSS $out/share/status-overlay/style.css
            install -Dm644 src/style.css $out/share/status-overlay/style.css
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
          LD_LIBRARY_PATH = runtimeLibs;
        };
      });
}
