# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Git 规范

- Commit 信息必须使用英文，便于国际开发者协作
- Commit 格式遵循 Conventional Commits：`type: description`（如 `feat: add voice interrupt support`、`fix: resolve TTS cache invalidation`）
- Commit之后请自动推送到GitHub

## 项目概述

Kokoro Engine 是一个基于 Tauri v2 的跨平台桌面虚拟角色沉浸交互引擎。前端使用 React + TypeScript，后端使用 Rust。整合了 Live2D、LLM、TTS、STT、Vision、ImageGen、MCP 等多种 AI 能力，并通过 MOD 系统支持 UI 和功能扩展。

## 常用命令

```bash
# 安装依赖
npm install

# 启动开发环境（前端 + Tauri 后端）
npm run tauri dev

# 构建发行版
npm run tauri build

# 前端测试（Vitest）
npm run test

# 运行单个前端测试文件
npx vitest run src/ui/mods/ModMessageBus.test.ts

# 后端测试（Rust）
cd src-tauri && cargo test

# 运行单个后端测试
cd src-tauri && cargo test test_name

# 前端类型检查
npx tsc --noEmit

# Rust 编译检查
cd src-tauri && cargo check

# Rust lint
cd src-tauri && cargo clippy
```

## 架构

双层架构，通过 Tauri IPC 通信：

```
Frontend (React + TS)  ←→  kokoro-bridge.ts (Typed IPC)  ←→  Backend (Rust/Tauri)
```

### 前端 (`src/`)

- `core/` — 服务初始化、单例服务实例、共享类型定义
- `features/live2d/` — Live2D 渲染（PixiJS 6 + Cubism SDK），包含表情驱动、口型同步、动作控制
- `ui/layout/` — 声明式布局引擎，JSON 配置驱动的 grid/layer/component 树
- `ui/registry/` — 组件注册表（单例模式），按名称注册和解析组件，支持 MOD 注入
- `ui/theme/` — 主题系统，CSS 变量注入，支持 MOD 主题覆盖
- `ui/mods/` — MOD UI 组件，iframe 沙箱通信
- `lib/kokoro-bridge.ts` — 所有 Tauri IPC 调用的类型化封装层，是前后端通信的唯一入口

### 后端 (`src-tauri/src/`)

- `commands/` — 20 个 IPC 命令处理模块（chat、tts、stt、llm、vision、imagegen、mcp、mods、telegram 等）
- `ai/` — AI 编排器：上下文管理（context.rs）、情感系统（emotion.rs，持久化）、记忆系统（memory.rs）、自主行为（好奇心、主动发言、闲置动作）
- `llm/` — LLM 适配器（OpenAI 兼容接口、Ollama），支持多 Provider + 预设切换、Fast/Smart/Cheap 模型路由
- `tts/` — TTS 服务，6 个提供商（GPT-SoVITS、VITS、OpenAI、Azure、ElevenLabs、Browser TTS），trait 化插件架构
- `stt/` — 语音识别（Whisper、OpenAI）
- `vision/` — 屏幕截图 + VLM 分析（xcap 截图，像素差异检测）
- `imagegen/` — 图片生成（Stable Diffusion WebUI、OpenAI DALL-E、Google Gemini）
- `mcp/` — Model Context Protocol 客户端（stdio + Streamable HTTP 传输，JSON-RPC 2.0）
- `mods/` — MOD 系统：manifest 解析、`mod://` 协议、QuickJS (ES2020) 脚本沙箱
- `telegram/` — Telegram Bot 远程交互（teloxide 长轮询，Chat ID 白名单，文字/语音/图片消息桥接）
- `actions/` — 动作注册表，用于 LLM tool calling

### MOD 系统 (`mods/`)

MOD 结构：`mod.json`（清单）、`theme.json`（主题）、`layout.json`（布局覆盖）、HTML 组件（iframe 沙箱）、QuickJS 脚本。
MOD JS API：`Kokoro.on()`、`Kokoro.emit()`、`Kokoro.ui.send()`、`Kokoro.character.setExpression()`。

## 关键模式

### Rust 端

- **共享状态**: `Arc<RwLock<T>>` 用于跨异步任务的可变状态
- **Tauri 状态管理**: 服务通过 `app.manage()` 注册（AIOrchestrator、TtsService、ModManager 等）
- **trait 化插件**: `TtsProvider` trait 实现提供商可插拔替换
- **IPC 返回值**: 统一使用 `Result<T, String>` 兼容 Tauri invoke
- **异步优先**: 所有 I/O 使用 tokio 异步运行时
- **聊天流式传输**: SSE 流 → Tauri 事件（`chat-delta`、`chat-done`、`chat-error`）

### TypeScript 端

- **类型化 IPC**: `kokoro-bridge.ts` 封装所有 `invoke()` 调用，是前后端通信的唯一入口
- **组件注册表**: 单例 `ComponentRegistry`，使用 `useSyncExternalStore` 订阅变更
- **声明式布局**: JSON 配置 → `LayoutRenderer` → React 组件树
- **主题上下文**: `ThemeProvider` 注入 CSS 变量，支持 MOD 覆盖

## 配置文件位置

运行时配置存储在 `~/.local/share/com.chyin.kokoro/`（或对应 OS 的 app data 目录）：
`llm_config.json`、`tts_config.json`、`stt_config.json`、`vision_config.json`、`imagegen_config.json`、`mcp_servers.json`、`telegram_config.json`、`emotion_state.json`、`memory_system_config.json`、`memory_upgrade_config.json`

其中 `memory_upgrade_config.json` 用于记忆系统升级开关（如可观测性、事件触发、结构化记忆、检索评估等分阶段能力）。

## 数据库

SQLite，主要表：`memories`（嵌入向量 + 重要性评分）、`conversations`（聊天历史）、`characters`（角色元数据）。
本地嵌入使用 FastEmbed（all-MiniLM-L6-v2，ONNX 离线推理）。

Phase 1 起额外引入记忆可观测性表：`memory_write_events`（写入/合并事件统计）、`memory_retrieval_logs`（检索召回统计），用于记忆系统升级与质量评估。

## 版本管理规范

### 版本号规则（语义化版本 SemVer）

格式：`vMAJOR.MINOR.PATCH[-stage]`

| 版本段 | 何时递增 | 示例 |
|--------|----------|------|
| MAJOR | 破坏性变更（配置格式不兼容、API 重构） | v1.0.0 → v2.0.0 |
| MINOR | 新增功能，向后兼容 | v0.1.0 → v0.2.0 |
| PATCH | Bug 修复、性能优化、小改动 | v0.1.0 → v0.1.1 |

阶段标签：
- `-alpha`：功能未完整，内部测试
- `-beta`：功能完整，公开测试
- 无标签：稳定正式版

### 发布流程（由 Claude 执行）

用户说"发布 vX.Y.Z"时，按以下步骤操作：

**1. 更新版本号**
```bash
# 前端版本（package.json）
# src-tauri/tauri.conf.json 中的 version 字段
# src-tauri/Cargo.toml 中的 version 字段
# 三处必须保持一致
```

**2. 提交版本变更**
```
chore(release): bump version to vX.Y.Z
```

**3. 打 Git Tag**
```bash
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push && git push --tags
```

**4. 创建 GitHub Release**
```bash
gh release create vX.Y.Z \
  --title "Kokoro Engine vX.Y.Z" \
  --notes "..." \
  --prerelease  # 仅 alpha/beta 加此标志
```

### Release Notes 模板

```
## 新功能
- ...

## 修复
- ...

## 已知限制
- ...

## 安装
下载对应平台安装包，解压后直接运行。
```

### 当前版本历史参考

- `v0.1.0-beta`：首个公开测试版，核心功能完整，部分商业 API 未经完整测试

## CI/CD 工作流

### 自动化构建流程

Kokoro Engine 使用 GitHub Actions 实现跨平台自动化构建和发布。

#### 工作流文件

| 文件 | 触发条件 | 功能 |
|------|--------|------|
| `ci.yml` | 每次 push 到 main/develop，PR | 前端测试、后端测试、三平台编译检查 |
| `build-windows.yml` | push 到 main、标签 v*、手动触发 | Windows NSIS 安装包构建 |
| `build-macos.yml` | push 到 main、标签 v*、手动触发 | macOS DMG 应用构建 |
| `build-linux.yml` | push 到 main、标签 v*、手动触发 | Linux AppImage + DEB 构建 |

#### 发布流程（自动化）

1. **本地提交** - 推送代码到 main 分支
2. **CI 检查** - `ci.yml` 运行所有测试和编译检查
3. **创建标签** - 运行 `/tauri-release` 创建 Git 标签 `vX.Y.Z`
4. **自动构建** - 三个平台的构建工作流并行运行
5. **自动发布** - 构建完成后自动上传到 GitHub Release

#### 手动触发构建

如果需要手动构建（不创建 Release），可以在 GitHub Actions 页面点击 "Run workflow"：

```
Actions → Build Windows/macOS/Linux → Run workflow
```

#### 构建产物

| 平台 | 产物 | 位置 |
|------|------|------|
| Windows | NSIS 安装包 (.exe) | `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/` |
| macOS | DMG 镜像 (.dmg) | `src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/` |
| Linux | AppImage (.AppImage) + DEB (.deb) | `src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/` |

#### 工作流状态

查看构建状态：https://github.com/chyin/kokoro-engine/actions

## 代码质量要求

### 编译检查（提交前必须通过）

**前端**：
```bash
npx tsc --noEmit
npm run test
```

**后端**：
```bash
cd src-tauri && cargo check
cd src-tauri && cargo clippy -- -D warnings
cd src-tauri && cargo fmt -- --check
cd src-tauri && cargo test
```

### 平台兼容性（Windows 开发必读）

本项目支持 Windows、macOS、Linux。**在 Windows 上开发时特别注意**：

#### ❌ 禁止的做法
```rust
// 硬编码 Windows 路径
let path = "C:\\Users\\...";
let path = "D:\\Program\\...";

// 硬编码 Unix 路径
let path = "/Users/...";
let path = "/home/...";

// 使用 Windows 特定的 API 而不检查平台
use std::os::windows::...;
```

#### ✅ 正确的做法
```rust
// 使用 Tauri 的跨平台 API
use tauri::api::path::{app_dir, config_dir};

// 使用条件编译
#[cfg(target_os = "windows")]
fn windows_specific() { ... }

#[cfg(target_os = "macos")]
fn macos_specific() { ... }

#[cfg(target_os = "linux")]
fn linux_specific() { ... }

// 使用 std::path::PathBuf 处理路径
use std::path::PathBuf;
let path = PathBuf::from("relative/path");
```

#### 常见陷阱
- **路径分隔符**：使用 `/` 或 `PathBuf`，不要硬编码 `\\`
- **行尾符**：Rust 自动处理，不要手动指定 `\r\n`
- **环境变量**：使用 `std::env::var()`，不要假设 `HOME` 或 `USERPROFILE` 存在
- **文件权限**：Windows 没有 Unix 权限模型，不要使用 `chmod`

### 发布前检查清单

在运行 `/tauri-release` 前，自动验证：
- [ ] 所有测试通过：`npm run test && cd src-tauri && cargo test`
- [ ] 版本号一致：package.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml
- [ ] 代码格式正确：`cd src-tauri && cargo fmt -- --check`
- [ ] 没有 clippy 警告：`cd src-tauri && cargo clippy -- -D warnings`
- [ ] 没有未提交的更改：`git status` 干净
- [ ] 本地构建成功（至少在 Windows 上）
- [ ] Git 标签不存在：`git tag | grep vX.Y.Z` 无结果
- [ ] 远程分支已同步：`git fetch && git status` 显示 "up to date"

## 文档

- `docs/architecture.md` — 系统架构设计
- `docs/API specification.md` — IPC 命令参考
- `docs/MOD_system_design.md` — MOD 框架详细设计
- `docs/extending-tts.md` — TTS 提供商扩展指南
- `docs/PRD.md` — 产品需求文档

## 版本管理规范

### 版本号规则（语义化版本 SemVer）

格式：`vMAJOR.MINOR.PATCH[-stage]`

| 版本段 | 何时递增 | 示例 |
|--------|----------|------|
| MAJOR | 破坏性变更（配置格式不兼容、API 重构） | v1.0.0 → v2.0.0 |
| MINOR | 新增功能，向后兼容 | v0.1.0 → v0.2.0 |
| PATCH | Bug 修复、性能优化、小改动 | v0.1.0 → v0.1.1 |

阶段标签：
- `-alpha`：功能未完整，内部测试
- `-beta`：功能完整，公开测试
- 无标签：稳定正式版

### 发布流程（由 Claude 执行）

用户说"发布 vX.Y.Z"时，按以下步骤操作：

**1. 更新版本号**
```bash
# 前端版本（package.json）
# src-tauri/tauri.conf.json 中的 version 字段
# src-tauri/Cargo.toml 中的 version 字段
# 三处必须保持一致
```

**2. 提交版本变更**
```
chore(release): bump version to vX.Y.Z
```

**3. 打 Git Tag**
```bash
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push && git push --tags
```

**4. 创建 GitHub Release**
```bash
gh release create vX.Y.Z \
  --title "Kokoro Engine vX.Y.Z" \
  --notes "..." \
  --prerelease  # 仅 alpha/beta 加此标志
```

### Release Notes 模板

```
## 新功能
- ...

## 修复
- ...

## 已知限制
- ...

## 安装
下载对应平台安装包，解压后直接运行。
```

### 当前版本历史参考

- `v0.1.0-beta`：首个公开测试版，核心功能完整，部分商业 API 未经完整测试

## CI/CD 工作流

### 自动化构建流程

Kokoro Engine 使用 GitHub Actions 实现跨平台自动化构建和发布。

#### 工作流文件

| 文件 | 触发条件 | 功能 |
|------|--------|------|
| `ci.yml` | 每次 push 到 main/develop，PR | 前端测试、后端测试、三平台编译检查 |
| `build-windows.yml` | push 到 main、标签 v*、手动触发 | Windows NSIS 安装包构建 |
| `build-macos.yml` | push 到 main、标签 v*、手动触发 | macOS DMG 应用构建 |
| `build-linux.yml` | push 到 main、标签 v*、手动触发 | Linux AppImage + DEB 构建 |

#### 发布流程（自动化）

1. **本地提交** - 推送代码到 main 分支
2. **CI 检查** - `ci.yml` 运行所有测试和编译检查
3. **创建标签** - 运行 `/tauri-release` 创建 Git 标签 `vX.Y.Z`
4. **自动构建** - 三个平台的构建工作流并行运行
5. **自动发布** - 构建完成后自动上传到 GitHub Release

#### 手动触发构建

如果需要手动构建（不创建 Release），可以在 GitHub Actions 页面点击 "Run workflow"：

```
Actions → Build Windows/macOS/Linux → Run workflow
```

#### 构建产物

| 平台 | 产物 | 位置 |
|------|------|------|
| Windows | NSIS 安装包 (.exe) | `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/` |
| macOS | DMG 镜像 (.dmg) | `src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/` |
| Linux | AppImage (.AppImage) + DEB (.deb) | `src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/` |

#### 工作流状态

查看构建状态：https://github.com/chyin/kokoro-engine/actions

## 代码质量要求

### 编译检查（提交前必须通过）

**前端**：
```bash
npx tsc --noEmit
npm run test
```

**后端**：
```bash
cd src-tauri && cargo check
cd src-tauri && cargo clippy -- -D warnings
cd src-tauri && cargo fmt -- --check
cd src-tauri && cargo test
```

### 平台兼容性（Windows 开发必读）

本项目支持 Windows、macOS、Linux。**在 Windows 上开发时特别注意**：

#### ❌ 禁止的做法
```rust
// 硬编码 Windows 路径
let path = "C:\\Users\\...";
let path = "D:\\Program\\...";

// 硬编码 Unix 路径
let path = "/Users/...";
let path = "/home/...";

// 使用 Windows 特定的 API 而不检查平台
use std::os::windows::...;
```

#### ✅ 正确的做法
```rust
// 使用 Tauri 的跨平台 API
use tauri::api::path::{app_dir, config_dir};

// 使用条件编译
#[cfg(target_os = "windows")]
fn windows_specific() { ... }

#[cfg(target_os = "macos")]
fn macos_specific() { ... }

#[cfg(target_os = "linux")]
fn linux_specific() { ... }

// 使用 std::path::PathBuf 处理路径
use std::path::PathBuf;
let path = PathBuf::from("relative/path");
```

#### 常见陷阱
- **路径分隔符**：使用 `/` 或 `PathBuf`，不要硬编码 `\\`
- **行尾符**：Rust 自动处理，不要手动指定 `\r\n`
- **环境变量**：使用 `std::env::var()`，不要假设 `HOME` 或 `USERPROFILE` 存在
- **文件权限**：Windows 没有 Unix 权限模型，不要使用 `chmod`

### 发布前检查清单

在运行 `/tauri-release` 前，自动验证：
- [ ] 所有测试通过：`npm run test && cd src-tauri && cargo test`
- [ ] 版本号一致：package.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml
- [ ] 代码格式正确：`cd src-tauri && cargo fmt -- --check`
- [ ] 没有 clippy 警告：`cd src-tauri && cargo clippy -- -D warnings`
- [ ] 没有未提交的更改：`git status` 干净
- [ ] 本地构建成功（至少在 Windows 上）
- [ ] Git 标签不存在：`git tag | grep vX.Y.Z` 无结果
- [ ] 远程分支已同步：`git fetch && git status` 显示 "up to date"

## 文档

- `docs/architecture.md` — 系统架构设计
- `docs/API specification.md` — IPC 命令参考
- `docs/MOD_system_design.md` — MOD 框架详细设计
- `docs/extending-tts.md` — TTS 提供商扩展指南
- `docs/PRD.md` — 产品需求文档
