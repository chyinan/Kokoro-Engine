# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Git 规范

- Commit 信息必须使用英文，便于国际开发者协作
- Commit 格式遵循 Conventional Commits：`type: description`（如 `feat: add voice interrupt support`、`fix: resolve TTS cache invalidation`）

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

- `commands/` — 18 个 IPC 命令处理模块（chat、tts、stt、llm、vision、imagegen、mcp、mods 等）
- `ai/` — AI 编排器：上下文管理（context.rs）、情感系统（emotion.rs）、记忆系统（memory.rs）、自主行为（好奇心、主动发言、闲置动作）
- `llm/` — LLM 适配器（OpenAI 兼容接口、Ollama），支持 Fast/Smart/Cheap 模型路由
- `tts/` — TTS 服务，15+ 提供商（GPT-SoVITS、OpenAI、Azure、Google、Browser TTS、RVC 等），trait 化插件架构
- `stt/` — 语音识别（Whisper、OpenAI）
- `vision/` — 屏幕截图 + VLM 分析（xcap 截图，像素差异检测）
- `imagegen/` — 图片生成（Stable Diffusion WebUI、OpenAI DALL-E、Google Gemini）
- `mcp/` — Model Context Protocol 客户端（stdio 传输，JSON-RPC 2.0）
- `mods/` — MOD 系统：manifest 解析、`mod://` 协议、QuickJS (ES2020) 脚本沙箱
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
`llm_config.json`、`tts_config.json`、`stt_config.json`、`vision_config.json`、`imagegen_config.json`、`mcp_servers.json`

## 数据库

SQLite，主要表：`memories`（嵌入向量 + 重要性评分）、`conversations`（聊天历史）、`characters`（角色元数据）。
本地嵌入使用 FastEmbed（all-MiniLM-L6-v2，ONNX 离线推理）。

## 文档

- `docs/architecture.md` — 系统架构设计
- `docs/API specification.md` — IPC 命令参考
- `docs/MOD_system_design.md` — MOD 框架详细设计
- `docs/extending-tts.md` — TTS 提供商扩展指南
- `docs/PRD.md` — 产品需求文档
