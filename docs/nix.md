# Nix

## 启用 Flakes

NixOS:

```nix
{
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
}
```

应用配置：

```bash
sudo nixos-rebuild switch
```

非 NixOS 临时启用：

```bash
nix --extra-experimental-features "nix-command flakes" develop
```

## 开发

```bash
nix develop
npm install
npm run tauri dev
```

## 构建

```bash
nix build .#default
./result/bin/tauri-appkokoro-engine
```

也可以：

```bash
nix run .#default
```

## NixOS Flake 安装

先把仓库加到你的系统 flake 输入里：

```nix
{
  inputs.kokoro-engine.url = "github:chyinan/Kokoro-Engine";
}
```

然后在 `configuration.nix` 对应模块里安装：

```nix
{ pkgs, inputs, ... }:
{
  environment.systemPackages = [
    inputs.kokoro-engine.packages.${pkgs.system}.default
  ];
}
```

## Home Manager 安装

```nix
{ pkgs, inputs, ... }:
{
  home.packages = [
    inputs.kokoro-engine.packages.${pkgs.system}.default
  ];
}
```

## 说明

- 当前 flake 主要支持 Linux。
- 包里已经带上 WebKitGTK、glib-networking、GStreamer 和 ONNX Runtime 相关运行时设置。
