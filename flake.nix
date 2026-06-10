{
  description = "Sputnik - a note taking app";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Rust toolchain with rust-src for rust-analyzer
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
        };

        # Runtime-only libraries needed by the Iced GUI
        runtimeDeps = with pkgs; [
          vulkan-loader
          libxkbcommon
          wayland
          dbus
          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libXi
          stdenv.cc.cc.lib  # provides libgcc_s.so.1
        ];

        # Cargo lock hashes for git dependencies
        cargoOutputHashes = {
          "cosmic-text-0.15.0" = "sha256-IcaVn8r6qGWhgNnZchRHIgcMSNYE61Bfc3n29X9N7xY=";
          "cryoglyph-0.1.0"    = "sha256-iBpeC4g/C2rkMWxoOahPJ4aECqsE2rnxDeFEmuBPj3k=";
          "iced-0.15.0-dev"          = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_core-0.15.0-dev"     = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_debug-0.15.0-dev"    = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_futures-0.15.0-dev"  = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_graphics-0.15.0-dev" = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_program-0.15.0-dev"  = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_renderer-0.15.0-dev" = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_runtime-0.15.0-dev"  = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_tiny_skia-0.15.0-dev"= "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_wgpu-0.15.0-dev"     = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_widget-0.15.0-dev"   = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "iced_winit-0.15.0-dev"    = "sha256-CvQC4s4+AheJucvkUhHgAXD0g5FKaBSX8v398mLv61Q=";
          "winit-0.30.8" = "sha256-pQn1lCFSJMkjUfHoggEzMHnm5k+Chnzi5JEDjahnjUA=";
          "dpi-0.1.1"    = "sha256-pQn1lCFSJMkjUfHoggEzMHnm5k+Chnzi5JEDjahnjUA=";
        };

      in
      {
        # -------------------------------
        # Development shell: nix develop
        # -------------------------------
        devShells.default = pkgs.mkShell {
          buildInputs = [ rustToolchain ] ++ runtimeDeps;

          nativeBuildInputs = with pkgs; [
            pkg-config    # needed so Cargo can find system libraries
            cargo-watch   # live rebuild on file change
            cargo-edit    # cargo add/rm/upgrade
            imagemagick   # icon generation
            just          # command runner
          ];

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          # Required at runtime for wgpu / Wayland / X11
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeDeps;
        };

        # -------------------------------
        # Package: nix build / nix profile install
        # -------------------------------
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "sputnik";
          version = "0.1.0";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = cargoOutputHashes;
          };

          # Use the 'production' profile defined in Cargo.toml
          buildType = "production";

          # Build-time tools
          nativeBuildInputs = with pkgs; [
            pkg-config
            makeWrapper
            copyDesktopItems
            autoPatchelfHook  # embeds runtimeDeps paths as RPATH in the binary
          ];

          # Runtime libraries linked into the binary
          buildInputs = runtimeDeps;

          postInstall = ''
            # Wrap the binary so it can find shared libraries at runtime
            wrapProgram $out/bin/sputnik \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeDeps}

            # Install the application icon
            install -Dm644 assets/Logo.png \
              $out/share/icons/hicolor/256x256/apps/sputnik.png
          '';

          desktopItems = [
            (pkgs.makeDesktopItem {
              name        = "sputnik";
              exec        = "sputnik";
              icon        = "sputnik";
              desktopName = "Sputnik";
              genericName = "Note taking app";
              categories  = [ "Utility" ];
              comment     = "A lightweight note taking application";
            })
          ];
        };

        # -------------------------------
        # Apps
        # -------------------------------

        # nix run
        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
        };

        # nix run .#generate-icon
        apps.generate-icon = {
          type = "app";
          program = "${pkgs.writeShellScriptBin "generate-icon" ''
            PATH=${pkgs.imagemagick}/bin:$PATH
            ./scripts/generate_icon.sh
          ''}/bin/generate-icon";
        };

        # -------------------------------
        # nix fmt
        # -------------------------------
        formatter = pkgs.nixpkgs-fmt;
      });
}
