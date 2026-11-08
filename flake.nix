{
  description = "Lucy: a model-agnostic personal AI assistant";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # Only for checking the home-manager module.
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      home-manager,
      ...
    }:
    let
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
    in
    {
      # `pkgs.lucy` and `pkgs.lucy-sprite`.
      overlays.default = final: _prev: {
        lucy = final.callPackage ./nix/lucy.nix { inherit version; };
        lucy-sprite = final.callPackage ./nix/lucy-sprite.nix { inherit version; };
      };

      # `programs.lucy`; see nix/hm-module.nix.
      homeModules.default = import ./nix/hm-module.nix self;
      homeManagerModules.default = self.homeModules.default;
    }
    //
      flake-utils.lib.eachSystem
        [
          "x86_64-linux"
          "aarch64-linux"
          "aarch64-darwin"
        ]
        (
          system:
          let
            pkgs = import nixpkgs {
              inherit system;
              overlays = [ self.overlays.default ];
            };
            inherit (pkgs.stdenv.hostPlatform) isLinux;

            # Native libraries Tauri needs on Linux (for apps/sprite).
            tauriLinuxDeps = with pkgs; [
              at-spi2-atk
              atkmm
              cairo
              gdk-pixbuf
              glib
              gtk3
              gtk-layer-shell
              harfbuzz
              libayatana-appindicator
              librsvg
              libsoup_3
              pango
              webkitgtk_4_1
              openssl
            ];
          in
          {
            packages = {
              inherit (pkgs) lucy;
              default =
                if isLinux then
                  pkgs.symlinkJoin {
                    name = "lucy-${version}";
                    paths = [
                      pkgs.lucy
                      pkgs.lucy-sprite
                    ];
                    meta.mainProgram = "lucy";
                  }
                else
                  pkgs.lucy;
            }
            // pkgs.lib.optionalAttrs isLinux { inherit (pkgs) lucy-sprite; };

            checks = {
              inherit (pkgs) lucy;
            }
            // pkgs.lib.optionalAttrs isLinux {
              inherit (pkgs) lucy-sprite;
              home-manager-module = import ./nix/hm-module-check.nix {
                inherit pkgs self home-manager;
              };
            };

            devShells.default = pkgs.mkShell {
              packages =
                with pkgs;
                [
                  cargo
                  rustc
                  clippy
                  rustfmt
                  rust-analyzer
                  pkg-config
                  nodejs_24
                  cargo-tauri
                ]
                ++ pkgs.lib.optionals isLinux tauriLinuxDeps;

              RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            };
          }
        );
}
