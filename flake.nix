{
  description = "synctr: rclone profiles, ignore rules, CLI and TUI";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      each = nixpkgs.lib.genAttrs systems;
    in {
      packages = each (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          lib = pkgs.lib;
          synctr = pkgs.rustPlatform.buildRustPackage {
            pname = "synctr";
            version = "0.1.0";
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./synctr
                ./synctr-engine
                ./contrib
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [ "--package" "synctr" ];
            cargoTestFlags = [ "--workspace" ];
            doCheck = true;
            buildInputs = lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
              pkgs.libiconv
            ];
            meta = {
              description = "rclone profiles with gitignore-style ignore rules";
              mainProgram = "synctr";
              platforms = systems;
            };
          };
        in {
          default = synctr;
          inherit synctr;
        });

      apps = each (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.synctr}/bin/synctr";
        };
      });

      devShells = each (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in {
          default = pkgs.mkShell {
            packages = [
              pkgs.cargo
              pkgs.rustc
              pkgs.rustfmt
              pkgs.clippy
              pkgs.rclone
            ];
          };
        });
    };
}
