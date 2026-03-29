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
        version = "0.1.4";

        src = lib.cleanSource ./.;

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
            hash = "sha256-LdtyukW15us0UBRXT/MIhS7IacxlgEJm1HNo65TKdSM=";
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
          ORT_PREFER_DYNAMIC_LINK = "1";
          ORT_SKIP_DOWNLOAD = "1";

          postInstall = ''
            for bin in "$out"/bin/*; do
              wrapProgram "$bin" \
                --set GIO_MODULE_DIR ${pkgs.glib-networking}/lib/gio/modules \
                --prefix GIO_EXTRA_MODULES : ${pkgs.glib-networking}/lib/gio/modules \
                --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.gst_all_1.gst-plugins-bad}/lib/gstreamer-1.0 \
                --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.gst_all_1.gstreamer}/lib/gstreamer-1.0 \
                --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.gst_all_1.gst-plugins-base}/lib/gstreamer-1.0 \
                --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : ${pkgs.gst_all_1.gst-plugins-good}/lib/gstreamer-1.0 \
                --set GST_PLUGIN_SCANNER ${pkgs.gst_all_1.gstreamer.out}/libexec/gstreamer-1.0/gst-plugin-scanner \
                --prefix XDG_DATA_DIRS : ${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name} \
                --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibraries}
            done
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

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
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

          buildInputs = runtimeLibraries;

          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

          shellHook = ''
            export XDG_DATA_DIRS="$GSETTINGS_SCHEMAS_PATH''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
            export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules"
            export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
            export GST_PLUGIN_SYSTEM_PATH_1_0="${pkgs.gst_all_1.gst-plugins-bad}/lib/gstreamer-1.0:${pkgs.gst_all_1.gstreamer}/lib/gstreamer-1.0:${pkgs.gst_all_1.gst-plugins-base}/lib/gstreamer-1.0:${pkgs.gst_all_1.gst-plugins-good}/lib/gstreamer-1.0''${GST_PLUGIN_SYSTEM_PATH_1_0:+:$GST_PLUGIN_SYSTEM_PATH_1_0}"
            export GST_PLUGIN_SCANNER="${pkgs.gst_all_1.gstreamer.out}/libexec/gstreamer-1.0/gst-plugin-scanner"
            export ORT_LIB_LOCATION="${pkgs.onnxruntime}/lib"
            export ORT_PREFER_DYNAMIC_LINK=1
            export ORT_SKIP_DOWNLOAD=1
            export LD_LIBRARY_PATH=${lib.makeLibraryPath runtimeLibraries}:$LD_LIBRARY_PATH
          '';
        };
      }
    );
}
