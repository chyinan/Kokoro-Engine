# Tauri 后端控制台日志分级着色 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Tauri 后端控制台提供“级别高亮 + 模块固定色”的统一日志输出，并保证 Windows/macOS/Linux 兼容与无色降级可读。

**Architecture:** 在 `src-tauri` 引入 `tracing` 体系，新增统一日志初始化与格式化模块，启动阶段一次性初始化。先通过纯函数格式化单测锁定输出结构，再分批迁移高频 `println!/eprintln!` 到 `info!/warn!/error!`（含 `target`），最后做三平台编译与日志烟测。

**Tech Stack:** Rust, Tauri v2, tracing, tracing-subscriber, ANSI color/no-color fallback, cargo test/check/clippy

---

> 执行约束：
> - 按 @superpowers:test-driven-development 执行（先写失败测试，再最小实现）。
> - 完成前按 @superpowers:verification-before-completion 逐项核验。
> - 每个任务结束后小步提交，避免大提交。

### Task 1: 引入日志依赖与日志模块骨架

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/utils/mod.rs`
- Create: `src-tauri/src/utils/logging.rs`
- Test: `src-tauri/src/utils/logging.rs`（同文件 `#[cfg(test)]`）

**Step 1: 写失败测试（格式化结构 + 模块映射 + 无色模式）**

在 `src-tauri/src/utils/logging.rs` 先写测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_line_keeps_structure_without_color() {
        let line = format_log_line("INFO", "ai", "Restored memory_enabled=true", false);
        assert!(line.starts_with("[INFO][ai] "));
        assert!(line.contains("Restored memory_enabled=true"));
        assert!(!line.contains("\u{1b}["));
    }

    #[test]
    fn module_palette_returns_default_for_unknown_target() {
        assert_eq!(module_palette("unknown-target"), ModulePalette::Default);
    }

    #[test]
    fn module_palette_maps_known_targets() {
        assert_eq!(module_palette("ai"), ModulePalette::Ai);
        assert_eq!(module_palette("mcp"), ModulePalette::Mcp);
    }
}
```

**Step 2: 运行测试确认失败**

Run:

```bash
cd src-tauri && cargo test utils::logging -- --nocapture
```

Expected: FAIL（`format_log_line` / `module_palette` / `ModulePalette` 尚未实现）

**Step 3: 写最小实现（模块骨架 + 纯函数）**

在 `src-tauri/src/utils/logging.rs` 实现：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulePalette {
    Ai,
    Tts,
    Stt,
    Mcp,
    Vision,
    ImageGen,
    Tools,
    Pet,
    Default,
}

pub fn module_palette(target: &str) -> ModulePalette {
    match target {
        "ai" => ModulePalette::Ai,
        "tts" => ModulePalette::Tts,
        "stt" => ModulePalette::Stt,
        "mcp" => ModulePalette::Mcp,
        "vision" => ModulePalette::Vision,
        "imagegen" => ModulePalette::ImageGen,
        "tools" => ModulePalette::Tools,
        "pet" => ModulePalette::Pet,
        _ => ModulePalette::Default,
    }
}

pub fn format_log_line(level: &str, target: &str, message: &str, with_color: bool) -> String {
    if with_color {
        // Task 2 再补齐真实颜色拼接，先保证结构
        format!("[{level}][{target}] {message}")
    } else {
        format!("[{level}][{target}] {message}")
    }
}
```

并在 `src-tauri/src/utils/mod.rs` 注册：

```rust
pub mod logging;
```

**Step 4: 运行测试确认通过**

Run:

```bash
cd src-tauri && cargo test utils::logging -- --nocapture
```

Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/utils/mod.rs src-tauri/src/utils/logging.rs
git commit -m "feat(logging): add logging module skeleton and formatter tests"
```

---

### Task 2: 实现 tracing subscriber 与跨平台颜色降级

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/utils/logging.rs`
- Test: `src-tauri/src/utils/logging.rs`

**Step 1: 写失败测试（有色模式应包含 ANSI、无色模式不包含）**

在 `src-tauri/src/utils/logging.rs` 增补测试：

```rust
#[test]
fn format_line_contains_ansi_when_color_enabled() {
    let line = format_log_line("ERROR", "mcp", "connection failed", true);
    assert!(line.contains("\u{1b}["));
}

#[test]
fn format_line_no_ansi_when_color_disabled() {
    let line = format_log_line("ERROR", "mcp", "connection failed", false);
    assert!(!line.contains("\u{1b}["));
}
```

**Step 2: 运行测试确认失败**

Run:

```bash
cd src-tauri && cargo test utils::logging::tests::format_line_contains_ansi_when_color_enabled -- --nocapture
```

Expected: FAIL（当前 `with_color=true` 仍无 ANSI）

**Step 3: 实现颜色渲染与 subscriber 初始化**

在 `src-tauri/Cargo.toml` 增加依赖：

```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
```

在 `src-tauri/src/utils/logging.rs` 实现：

- `pub fn init_logging()`：使用 `tracing_subscriber::fmt()` 初始化全局 subscriber。
- `pub fn color_enabled()`：根据 `std::io::IsTerminal` + `NO_COLOR` 环境变量决定是否上色。
- `format_log_line(...)`：当 `with_color=true` 时，为 level 与 target 注入 ANSI 色值。
- 重复初始化时安全返回（不 panic，忽略 `set_global_default` 二次调用错误）。

**Step 4: 运行测试确认通过**

Run:

```bash
cd src-tauri && cargo test utils::logging -- --nocapture
```

Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/utils/logging.rs
git commit -m "feat(logging): add tracing subscriber with color fallback"
```

---

### Task 3: 在应用入口初始化统一日志系统

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/lib.rs`（仅在可测试范围内添加轻量测试；否则以集成检查替代）

**Step 1: 写失败验证（启动日志结构检查）**

先新增一个可单测纯函数（若尚无）：

- 在 `utils/logging.rs` 暴露 `format_log_line`（已在前面完成）。
- 在 `lib.rs` 测试里新增最小断言，验证启动日志采用 `[LEVEL][TARGET]` 结构（通过调用格式化函数，不直接依赖全局 subscriber）。

**Step 2: 运行测试确认失败（如果尚未接入目标格式）**

Run:

```bash
cd src-tauri && cargo test lib::tests -- --nocapture
```

Expected: FAIL（若测试断言了尚未接入的日志 target/格式）

**Step 3: 最小实现（接入 init_logging）**

在 `src-tauri/src/lib.rs`：

- `use crate::utils::logging::init_logging;`
- 在 `run()` 最早阶段调用 `init_logging();`
- 保持现有业务初始化顺序不变。

**Step 4: 运行测试/编译确认通过**

Run:

```bash
cd src-tauri && cargo test lib::tests -- --nocapture
cd src-tauri && cargo check
```

Expected: PASS / Finished

**Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(logging): initialize global logging during app startup"
```

---

### Task 4: 迁移第一批高频日志（lib + context）到 tracing target

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/context.rs`
- Test: `src-tauri/src/utils/logging.rs`（补 target 映射测试）

**Step 1: 写失败测试（target 规范）**

在 `utils/logging.rs` 增补测试，明确 target 映射：

```rust
#[test]
fn module_palette_maps_context_related_ai_target() {
    assert_eq!(module_palette("ai"), ModulePalette::Ai);
}
```

**Step 2: 运行测试确认失败（若 target 规范未覆盖）**

Run:

```bash
cd src-tauri && cargo test utils::logging::tests::module_palette_maps_context_related_ai_target -- --nocapture
```

Expected: FAIL（若未覆盖）

**Step 3: 最小实现（替换高频 println/eprintln）**

在 `lib.rs` / `commands/context.rs`：

- 将关键 `println!` 替换为 `tracing::info!(target = "ai", ...)`、`tracing::info!(target = "mcp", ...)` 等
- 将关键 `eprintln!` 替换为 `tracing::error!(target = "ai"|"mcp"|"pet", ...)
`
- 保留原消息语义，不改业务逻辑

示例：

```rust
tracing::info!(target = "ai", "Restored proactive_enabled={}", enabled);
tracing::error!(target = "pet", "auto-start failed: {}", e);
```

**Step 4: 运行测试确认通过**

Run:

```bash
cd src-tauri && cargo test utils::logging -- --nocapture
cd src-tauri && cargo check
```

Expected: PASS / Finished

**Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/commands/context.rs src-tauri/src/utils/logging.rs
git commit -m "refactor(logging): migrate startup and context logs to tracing targets"
```

---

### Task 5: 迁移第二批高频日志（mcp/tts/stt）

**Files:**
- Modify: `src-tauri/src/commands/mcp.rs`
- Modify: `src-tauri/src/mcp/client.rs`
- Modify: `src-tauri/src/mcp/manager.rs`
- Modify: `src-tauri/src/tts/manager.rs`
- Modify: `src-tauri/src/stt/service.rs`
- Test: `src-tauri/src/utils/logging.rs`（必要时补 target 测试）

**Step 1: 写失败测试（模块映射完整性）**

新增/补齐测试：

```rust
#[test]
fn module_palette_maps_mcp_tts_stt_targets() {
    assert_eq!(module_palette("mcp"), ModulePalette::Mcp);
    assert_eq!(module_palette("tts"), ModulePalette::Tts);
    assert_eq!(module_palette("stt"), ModulePalette::Stt);
}
```

**Step 2: 运行测试确认失败（若未覆盖）**

Run:

```bash
cd src-tauri && cargo test utils::logging::tests::module_palette_maps_mcp_tts_stt_targets -- --nocapture
```

Expected: FAIL（若映射未覆盖）

**Step 3: 最小实现（分文件替换）**

将对应文件的高频 `println!/eprintln!` 替换为 `tracing` 宏并设置 target：

- MCP 相关：`target = "mcp"`
- TTS 相关：`target = "tts"`
- STT 相关：`target = "stt"`

**Step 4: 运行测试确认通过**

Run:

```bash
cd src-tauri && cargo test utils::logging -- --nocapture
cd src-tauri && cargo check
```

Expected: PASS / Finished

**Step 5: Commit**

```bash
git add src-tauri/src/commands/mcp.rs src-tauri/src/mcp/client.rs src-tauri/src/mcp/manager.rs src-tauri/src/tts/manager.rs src-tauri/src/stt/service.rs src-tauri/src/utils/logging.rs
git commit -m "refactor(logging): migrate mcp tts stt logs to tracing"
```

---

### Task 6: 迁移第三批与收尾验证（Vision/ImageGen/Pet/Tools）

**Files:**
- Modify: `src-tauri/src/vision/watcher.rs`
- Modify: `src-tauri/src/vision/server.rs`
- Modify: `src-tauri/src/imagegen/service.rs`
- Modify: `src-tauri/src/commands/imagegen.rs`
- Modify: `src-tauri/src/commands/pet.rs`
- Modify: `src-tauri/src/actions/registry.rs`
- Test: `src-tauri/src/utils/logging.rs`

**Step 1: 写失败测试（剩余模块映射）**

```rust
#[test]
fn module_palette_maps_vision_imagegen_pet_tools_targets() {
    assert_eq!(module_palette("vision"), ModulePalette::Vision);
    assert_eq!(module_palette("imagegen"), ModulePalette::ImageGen);
    assert_eq!(module_palette("pet"), ModulePalette::Pet);
    assert_eq!(module_palette("tools"), ModulePalette::Tools);
}
```

**Step 2: 运行测试确认失败（若未覆盖）**

Run:

```bash
cd src-tauri && cargo test utils::logging::tests::module_palette_maps_vision_imagegen_pet_tools_targets -- --nocapture
```

Expected: FAIL（若映射未覆盖）

**Step 3: 最小实现（最后一批替换）**

按文件替换 `println!/eprintln!` 为 `tracing` 宏并设置对应 target，保持原文案。

**Step 4: 运行测试确认通过**

Run:

```bash
cd src-tauri && cargo test utils::logging -- --nocapture
cd src-tauri && cargo check
```

Expected: PASS / Finished

**Step 5: Commit**

```bash
git add src-tauri/src/vision/watcher.rs src-tauri/src/vision/server.rs src-tauri/src/imagegen/service.rs src-tauri/src/commands/imagegen.rs src-tauri/src/commands/pet.rs src-tauri/src/actions/registry.rs src-tauri/src/utils/logging.rs
git commit -m "refactor(logging): migrate remaining backend logs to tracing"
```

---

### Task 7: 全量验证（含跨平台）与最终提交

**Files:**
- Modify: （如验证发现问题再最小修复）
- Test: 现有 Rust 测试与编译链

**Step 1: 运行本地 Rust 质量门禁**

Run:

```bash
cd src-tauri && cargo fmt -- --check
cd src-tauri && cargo clippy -- -D warnings
cd src-tauri && cargo test
```

Expected: 全部通过

**Step 2: 运行应用进行人工烟测**

Run:

```bash
npm run tauri dev
```

Expected: 后端控制台出现 `[LEVEL][TARGET]` 结构，且错误日志明显高亮。

**Step 3: 验证无色降级（NO_COLOR）**

Run:

```bash
NO_COLOR=1 npm run tauri dev
```

Expected: 无 ANSI 色码，结构化文本可读。

**Step 4: 三平台编译验证（CI）**

Run:

```bash
git push
# 在 GitHub Actions 确认 Windows/macOS/Linux job 全绿
```

Expected: 三平台检查通过。

**Step 5: 最终 Commit（如 Step 1-4 有修复）**

```bash
git add src-tauri
git commit -m "test(logging): verify colored backend console output across platforms"
```

---

## 完成定义（Definition of Done）

- 后端控制台具备“级别 + 模块”双层视觉区分
- Rust 端高频日志完成 tracing 化（至少覆盖 lib/context/mcp/tts/stt/vision/imagegen/pet/tools）
- `NO_COLOR=1` 下输出仍结构化可读
- Rust 质量门禁与三平台 CI 通过
- 变更以小步提交形式完成
