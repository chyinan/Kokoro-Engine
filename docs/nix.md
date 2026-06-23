# Nix

## Enabling Flakes

On NixOS:

```nix
{
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
}
```

Apply the configuration:

```bash
sudo nixos-rebuild switch
```

On non-NixOS systems, enable temporarily:

```bash
nix --extra-experimental-features "nix-command flakes" develop
```

## Development

```bash
nix develop
npm install
npm run tauri dev
```

The default development shell is optimized for day-to-day debugging. It
provides Node.js, Rust, Tauri CLI, WebKitGTK, GStreamer, ONNX Runtime, and the
GTK/GIO runtime variables required by Tauri on NixOS.

For a fully pinned shell that also prefetches the `sherpa-onnx` native archive
used by the Nix package build, run:

```bash
nix develop .#full
```

Use the full shell when debugging the offline Nix package build itself. The
first run may take longer because it downloads the `sherpa-onnx` archive.

## Build

```bash
nix build .#default
./result/bin/tauri-appkokoro-engine
```

Or run directly:

```bash
nix run .#default
```

## NixOS Flake Installation

Add the repository to your system flake inputs:

```nix
{
  inputs.kokoro-engine.url = "github:chyinan/Kokoro-Engine";
}
```

Then install it in your `configuration.nix` module:

```nix
{ pkgs, inputs, ... }:
{
  environment.systemPackages = [
    inputs.kokoro-engine.packages.${pkgs.system}.default
  ];
}
```

## Home Manager Installation

```nix
{ pkgs, inputs, ... }:
{
  home.packages = [
    inputs.kokoro-engine.packages.${pkgs.system}.default
  ];
}
```

## Notes

- The flake currently supports Linux only.
- The package includes runtime configuration for WebKitGTK, glib-networking, GStreamer, and ONNX Runtime.
