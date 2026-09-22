{
  description = "Ditox — terminal-first clipboard manager (Zig backend + OpenTUI/Bun frontend)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    bun2nix = {
      url = "github:baileyluTCD/bun2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # Advertise the binary cache so `nix run/build github:grok-insider/ditox` can pull
  # prebuilt closures instead of compiling. Users must trust these (Nix will
  # prompt, or add them to nix.settings on NixOS).
  nixConfig = {
    extra-substituters = [
      "https://cache.fabrials.com/fabrials"
      "https://nix-community.cachix.org"
    ];
    extra-trusted-public-keys = [
      "fabrials:vpcHFnI0zvmDmw7Hetdbew29BbS9JObpVNkzClA8Q3k="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
    ];
  };

  outputs = { self, nixpkgs, bun2nix }:
    let
      lib = nixpkgs.lib;
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = lib.genAttrs systems;

      packageFor = system:
        let
          pkgs = import nixpkgs { inherit system; };
          b2n = bun2nix.packages.${system}.default;

          runtimePath = lib.makeBinPath [
            pkgs.bun
            pkgs.hyprland
            pkgs.wl-clipboard
          ];

          # Hermetic, content-addressed bun cache built from the committed
          # tui/bun.lock (regenerate tui/bun.nix with `bun2nix` after any
          # dependency change — see the devShell + README).
          bunDeps = b2n.fetchBunDeps {
            bunNix = ./tui/bun.nix;
          };
        in
        pkgs.stdenv.mkDerivation {
          pname = "ditox";
          version = "0.1.0";
          src = ./.;

          nativeBuildInputs = [
            pkgs.makeBinaryWrapper
            pkgs.pkg-config
            pkgs.zig_0_16
            pkgs.bun
            b2n.hook
          ];
          buildInputs = [ pkgs.sqlite ];

          # bun2nix hook configuration: install node_modules into tui/ from the
          # offline cache only. opentui/resvg ship prebuilt native binaries, so
          # lifecycle scripts are unnecessary; the TUI bundle is produced by a
          # custom build.ts, not the hook's default `bun build`.
          inherit bunDeps;
          bunRoot = "tui";
          bunInstallFlags = "--linker=hoisted";
          dontRunLifecycleScripts = true;
          dontUseBunBuild = true;
          dontUseBunCheck = true;
          dontUseBunInstall = true;

          buildPhase = ''
            runHook preBuild

            export DITOX_SQLITE3_INCLUDE_DIR="${pkgs.sqlite.dev}/include"
            export DITOX_SQLITE3_LIB_DIR="${pkgs.sqlite.out}/lib"
            export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-cache"

            # 1. Bundle the OpenTUI frontend from the hermetic node_modules
            #    installed by the bun2nix hook into tui/.
            ( cd tui && bun run build )

            # 2. Build the Zig binaries (ditox + ditoxd) and install the TUI
            #    bundle/config assets under share/ditox/tui via build.zig.
            zig build -Doptimize=ReleaseSafe --prefix "$out"

            # 3. Vendor the exact @opentui modules (incl. the matching native
            #    libopentui.so) next to the bundle. The installed CLI launches
            #    `bun --no-install --cwd share/ditox/tui ./dist/index.js`, so the
            #    FFI lib resolves from the store and never the user's bun cache.
            mkdir -p "$out/share/ditox/tui/node_modules"
            cp -rL tui/node_modules/@opentui "$out/share/ditox/tui/node_modules/@opentui"

            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            runHook postInstall
          '';

          postFixup = ''
            wrapProgram "$out/bin/ditox" \
              --prefix PATH : "${runtimePath}" \
              --set-default DITOXD "$out/bin/ditoxd"
            wrapProgram "$out/bin/ditoxd" \
              --prefix PATH : "${runtimePath}"
          '';

          meta = {
            description = "Ditox clipboard manager (Zig backend + OpenTUI/Bun frontend)";
            mainProgram = "ditox";
            platforms = systems;
          };
        };

      # Separate from packageFor so ditoxd and its service do not link WebKit.
      guiFor = system:
        let
          pkgs = import nixpkgs { inherit system; };
          b2n = bun2nix.packages.${system}.default;
          bunDeps = b2n.fetchBunDeps {
            bunNix = ./gui/bun.nix;
          };
        in
        pkgs.rustPlatform.buildRustPackage {
          pname = "ditox-gui";
          version = "0.1.0";
          src = pkgs.lib.cleanSourceWith {
            src = pkgs.lib.cleanSource ./gui;
            filter = path: _type:
              let
                base = baseNameOf path;
              in
              base != "node_modules"
              && base != "target"
              && base != "gen"
              && (base != "dist" || pkgs.lib.hasInfix "/vendor/" path);
          };
          cargoLock.lockFile = ./gui/src-tauri/Cargo.lock;
          cargoRoot = "src-tauri";
          buildAndTestSubdir = "src-tauri";
          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.wrapGAppsHook4
            pkgs.bun
            b2n.hook
          ];
          buildInputs = [
            pkgs.webkitgtk_4_1
            pkgs.libayatana-appindicator
            pkgs.gtk3
            pkgs.openssl
          ];
          inherit bunDeps;
          bunRoot = ".";
          bunInstallFlags = "--offline --linker=hoisted";
          dontRunLifecycleScripts = true;
          dontUseBunBuild = true;
          dontUseBunCheck = true;
          dontUseBunInstall = true;
          doCheck = false;
          preBuild = ''
            bun run build
          '';
          preFixup = ''
            gappsWrapperArgs+=(
              --set-default WEBKIT_DISABLE_DMABUF_RENDERER 1
              --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ]}"
            )
          '';
          meta = {
            description = "Ditox clipboard window";
            mainProgram = "ditox-gui";
            platforms = systems;
          };
        };
    in
    {
      packages = forAllSystems (system: rec {
        default = packageFor system;
        ditox = default;
        ditox-gui = guiFor system;
      });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/ditox";
        };
      });

      homeManagerModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.ditox;
        in
        {
          options.programs.ditox = {
            enable = lib.mkEnableOption "Ditox clipboard manager";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "ditox.packages.\${pkgs.stdenv.hostPlatform.system}.default";
              description = "Ditox package to install.";
            };

            systemd.enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Whether to run the Ditox clipboard watcher as a user service.";
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];

            systemd.user.services.ditox = lib.mkIf cfg.systemd.enable {
              Unit = {
                Description = "Ditox clipboard watcher";
                After = [ "graphical-session.target" ];
                PartOf = [ "graphical-session.target" ];
              };

              Service = {
                # Single long-lived DB owner running the full clipboard capture
                # loop against its one connection (avoids multi-writer SQLITE_BUSY).
                ExecStart = "${cfg.package}/bin/ditoxd daemon";
                Restart = "on-failure";
                RestartSec = 2;
              };

              Install.WantedBy = [ "graphical-session.target" ];
            };
          };
        };

      checks = forAllSystems (system: {
        default = self.packages.${system}.default;
      });

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.bun
              pkgs.zig_0_16
              pkgs.sqlite
              pkgs.pkg-config
              pkgs.wl-clipboard
              pkgs.hyprland
              # Regenerate tui/bun.nix after dependency changes:
              #   (cd tui && bun2nix -o bun.nix)
              bun2nix.packages.${system}.default
            ];

            SQLITE3_INCLUDE_DIR = "${pkgs.sqlite.dev}/include";
            SQLITE3_LIB_DIR = "${pkgs.sqlite.out}/lib";

            shellHook = ''
              export DITOX_SQLITE3_INCLUDE_DIR="$SQLITE3_INCLUDE_DIR"
              export DITOX_SQLITE3_LIB_DIR="$SQLITE3_LIB_DIR"
            '';
          };
        });
    };
}
