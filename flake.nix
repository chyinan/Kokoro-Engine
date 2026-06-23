{
  description = "Kokoro Engine Nix development shell and package";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachSystem [
      "x86_64-linux"
      "aarch64-linux"
    ] (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;

        pname = "kokoro-engine";
        version = "0.2.9";

        src = lib.cleanSource ./.;

        sherpaOnnxArchiveName = "sherpa-onnx-v1.12.33-linux-x64-static-lib.tar.bz2";
        sherpaOnnxArchive = pkgs.fetchurl {
          name = sherpaOnnxArchiveName;
          url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.12.33/${sherpaOnnxArchiveName}";
          hash = "sha256-wyFodE8ppr0ITelab48WdAWm+9OrKpGV/dHAZiy510c=";
        };
        sherpaOnnxArchiveDir = pkgs.linkFarm "sherpa-onnx-prebuilt-archives" [
          {
            name = sherpaOnnxArchiveName;
            path = sherpaOnnxArchive;
          }
        ];

        runtimeLibraries = with pkgs; [
          alsa-lib
          atk
          cairo
          gdk-pixbuf
          glib
          glib-networking
          gsettings-desktop-schemas
          gst_all_1.gst-plugins-bad
          gst_all_1.gst-plugins-base
          gst_all_1.gst-plugins-good
          gst_all_1.gstreamer
          gtk3
          libayatana-appindicator
          librsvg
          libsoup_3
          onnxruntime
          openssl
          pango
          webkitgtk_4_1
        ];

        devNativeBuildInputs = with pkgs; [
          cargo
          cargo-tauri
          clippy
          nodejs
          pkg-config
          rust-analyzer
          rustc
          rustfmt
        ] ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
          wrapGAppsHook4
        ];

        mkDevShell =
          { withSherpaArchive ? false }:
          pkgs.mkShell {
            nativeBuildInputs = devNativeBuildInputs;
            buildInputs = runtimeLibraries;

            RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

            shellHook = ''
              export XDG_DATA_DIRS="$GSETTINGS_SCHEMAS_PATH''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
              export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules"
              export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
              export GST_PLUGIN_SYSTEM_PATH_1_0="${pkgs.gst_all_1.gst-plugins-bad}/lib/gstreamer-1.0:${pkgs.gst_all_1.gstreamer}/lib/gstreamer-1.0:${pkgs.gst_all_1.gst-plugins-base}/lib/gstreamer-1.0:${pkgs.gst_all_1.gst-plugins-good}/lib/gstreamer-1.0''${GST_PLUGIN_SYSTEM_PATH_1_0:+:$GST_PLUGIN_SYSTEM_PATH_1_0}"
              export GST_PLUGIN_SCANNER="${pkgs.gst_all_1.gstreamer.out}/libexec/gstreamer-1.0/gst-plugin-scanner"
              export ORT_LIB_LOCATION="${pkgs.onnxruntime}/lib"
              export ORT_DYLIB_PATH="${pkgs.onnxruntime}/lib/libonnxruntime.so"
              export ORT_PREFER_DYNAMIC_LINK=1
              export ORT_SKIP_DOWNLOAD=1
              ${lib.optionalString withSherpaArchive ''
                export SHERPA_ONNX_ARCHIVE_DIR="${sherpaOnnxArchiveDir}"
              ''}
              ${lib.optionalString (!withSherpaArchive) ''
                unset SHERPA_ONNX_ARCHIVE_DIR
              ''}
              bad_nix_rpath="-rpath $PWD/outputs/out/lib"
              export NIX_LDFLAGS="''${NIX_LDFLAGS//$bad_nix_rpath/}"
              export LD_LIBRARY_PATH=${lib.makeLibraryPath runtimeLibraries}:$LD_LIBRARY_PATH
            '';
          };

        app = pkgs.rustPlatform.buildRustPackage {
          inherit pname version src;

          cargoLock = {
            lockFile = ./src-tauri/Cargo.lock;
          };

          cargoRoot = "src-tauri";
          buildAndTestSubdir = "src-tauri";

          npmDeps = pkgs.fetchNpmDeps {
            name = "${pname}-${version}-npm-deps";
            inherit src;
            hash = "sha256-1VCTyKmUKgx4f0bCmxz/tS/xojwiPzpEwO1iw4yYZYk=";
          };

          nativeBuildInputs = with pkgs; [
            cargo-tauri.hook
            nodejs
            npmHooks.npmConfigHook
            pkg-config
          ] ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
            wrapGAppsHook4
          ];

          buildInputs = runtimeLibraries;

          ORT_LIB_LOCATION = "${pkgs.onnxruntime}/lib";
          ORT_DYLIB_PATH = "${pkgs.onnxruntime}/lib/libonnxruntime.so";
          ORT_PREFER_DYNAMIC_LINK = "1";
          ORT_SKIP_DOWNLOAD = "1";
          SHERPA_ONNX_ARCHIVE_DIR = sherpaOnnxArchiveDir;

          preFixup = ''
            gappsWrapperArgs+=(
              --set GIO_MODULE_DIR ${pkgs.glib-networking}/lib/gio/modules
              --prefix GIO_EXTRA_MODULES : ${pkgs.glib-networking}/lib/gio/modules
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.gst_all_1.gst-plugins-bad}/lib/gstreamer-1.0
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.gst_all_1.gstreamer}/lib/gstreamer-1.0
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.gst_all_1.gst-plugins-base}/lib/gstreamer-1.0
              --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.gst_all_1.gst-plugins-good}/lib/gstreamer-1.0
              --set GST_PLUGIN_SCANNER ${pkgs.gst_all_1.gstreamer.out}/libexec/gstreamer-1.0/gst-plugin-scanner
              --prefix XDG_DATA_DIRS : ${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}
              --set ORT_DYLIB_PATH ${pkgs.onnxruntime}/lib/libonnxruntime.so
              --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibraries}
            )
          '';

          meta = with lib; {
            description = "Desktop virtual character interaction engine built with Tauri";
            homepage = "https://github.com/chyinan/Kokoro-Engine";
            platforms = platforms.linux;
            mainProgram = "tauri-appkokoro-engine";
          };
        };
      in
      {
        packages.default = app;

        apps.default = {
          type = "app";
          program = "${app}/bin/${app.meta.mainProgram}";
          meta = app.meta;
        };

        devShells = {
          default = mkDevShell { };
          full = mkDevShell { withSherpaArchive = true; };
        };
      }
    );
}
