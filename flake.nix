{
  description = "Ditox — clipboard manager. Rust daemon, Rune window.";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  nixConfig = {
    extra-substituters = [ "https://cache.fabrials.com/fabrials" ];
    extra-trusted-public-keys = [ "fabrials:vpcHFnI0zvmDmw7Hetdbew29BbS9JObpVNkzClA8Q3k=" ];
  };

  outputs = { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = lib.genAttrs systems;
      runtimePath = pkgs: lib.makeBinPath [ pkgs.hyprland pkgs.wl-clipboard ];
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          windowLibs = [ pkgs.libxkbcommon pkgs.vulkan-loader pkgs.wayland ];
          ditox = pkgs.rustPlatform.buildRustPackage {
            pname = "ditox";
            version = (lib.importTOML ./Cargo.toml).workspace.package.version;
            src = lib.cleanSourceWith {
              src = lib.cleanSource ./.;
              filter = path: _type:
                let base = baseNameOf path;
                in base != "target" && base != "result" && base != "docs" && base != ".github";
            };
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "rune-0.0.1" = "sha256-RDHDvgGnZ0WBUrY4HbjOQR2RACtREu/LFbIqSrexexM=";
                "rune-motion-0.1.0" = "sha256-RDHDvgGnZ0WBUrY4HbjOQR2RACtREu/LFbIqSrexexM=";
                "rune-staging-0.0.1" = "sha256-RDHDvgGnZ0WBUrY4HbjOQR2RACtREu/LFbIqSrexexM=";
                "rune-taffy-0.0.1" = "sha256-RDHDvgGnZ0WBUrY4HbjOQR2RACtREu/LFbIqSrexexM=";
              };
            };
            nativeBuildInputs = [ pkgs.pkg-config pkgs.makeBinaryWrapper ];
            # nixpkgs links libsqlite3-sys against its own SQLite; the tests need its FTS5.
            buildInputs = windowLibs ++ [ pkgs.sqlite ];
            doCheck = true;
            postInstall = ''
              # After PATH: the session's own hyprctl matches the running compositor.
              wrapProgram "$out/bin/ditoxd" --suffix PATH : "${runtimePath pkgs}"
              wrapProgram "$out/bin/ditox" \
                --suffix PATH : "${runtimePath pkgs}" \
                --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath windowLibs}"
              install -Dm644 ${./crates/ditox/ditox.desktop} "$out/share/applications/ditox.desktop"
            '';
            meta = {
              description = "Ditox clipboard manager: the ditoxd daemon and the ditox window";
              homepage = "https://github.com/grok-insider/ditox";
              license = lib.licenses.agpl3Plus;
              mainProgram = "ditox";
              platforms = systems;
            };
          };
        in
        {
          inherit ditox;
          default = ditox;
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
              description = "Ditox package to install. Provides ditox and ditoxd.";
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
              pkgs.rustc
              pkgs.cargo
              pkgs.clippy
              pkgs.rustfmt
              pkgs.pkg-config
              pkgs.wl-clipboard
              pkgs.hyprland
              pkgs.libxkbcommon
              pkgs.vulkan-loader
              pkgs.wayland
            ];
            LD_LIBRARY_PATH = lib.makeLibraryPath [ pkgs.libxkbcommon pkgs.vulkan-loader pkgs.wayland ];
          };
        });
    };
}
