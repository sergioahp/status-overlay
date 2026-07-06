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
    let
      # home-manager module: installs the user service but does NOT enable it.
      # Trigger is Hyprland's exec-once -> `systemctl --user start status-overlay.service`,
      # so the overlay only runs once Hyprland is up and its env vars have been
      # imported into the systemd user environment. Running under systemd also gives
      # journald logs (journalctl --user -u status-overlay) and on-failure restart.
      homeManagerModule = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.status-overlay;
        in {
          options.programs.status-overlay = {
            enable = lib.mkEnableOption "status-overlay (Hyprland-triggered, systemd-supervised)";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.system}.default;
              description = "The status-overlay package to install and run.";
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];

            # Unit defined but Install.WantedBy intentionally omitted so it does not
            # auto-start with graphical-session.target. PartOf still binds shutdown:
            # when the graphical session ends, the overlay is torn down.
            systemd.user.services.status-overlay = {
              Unit = {
                Description = "GTK4 Wayland status overlay for Hyprland";
                PartOf = [ "graphical-session.target" ];
                After = [ "graphical-session.target" ];
              };
              Service = {
                ExecStart = "${cfg.package}/bin/status-overlay";
                Restart = "on-failure";
                RestartSec = 2;
              };
            };
          };
        };

      perSystem = flake-utils.lib.eachDefaultSystem (system:
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
    in
    perSystem // {
      # home-manager exposes modules via either name depending on version; expose both.
      homeManagerModules.default = homeManagerModule;
      homeModules.default = homeManagerModule;
    };
}
