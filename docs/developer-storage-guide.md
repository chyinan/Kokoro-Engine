# 开发者存储占用全生命周期闭环管理与极速编译指南

本文档为 Kokoro-Engine 开发者提供**全链路覆盖、完全自动且具备闭环自愈能力**的存储占用管理机制（Kokoro Storage Sentinel, KSS）及极速编译优化实践。

---

## 1. 存储膨胀机理与全链路架构

在频繁进行 `npm run tauri:dev`、`cargo test`、分支切换以及 IDE 语法分析时，`src-tauri/target/debug` 会迅速膨胀（实测可在数轮构建后达到 25GB 以上），根源包括：

1. **增量编译会话累积（`target/debug/incremental`）**：
   - `tauri dev`、`cargo test`、`cargo check` 带有不同的编译器标识，rustc 为每种模式分别创建独立的增量会话目录（单目录可达 4 GB）。rustc 本身不清理旧代际，分支切换后旧会话永久驻留。
2. **依赖产物历史哈希驻留（`target/debug/deps`）**：
   - 每次代码或配置变动生成新哈希单体库 `libtauri_appkokoro_engine_lib-*.rlib`（约 1.6 GB）与 `.pdb`，旧文件滞留不删。
3. **测试套件临时二进制膨胀**：
   - 每次 `cargo test` 都会为各模块集成测试生成独立的 `*-<hash>.exe` 与配对的巨型 PDB 符号文件，频繁测试后产生大量废弃二进制。

针对上述全链路场景，Kokoro-Engine 建立了覆盖开发全生命周期的自动化闭环：

```
首次开发 (postinstall 自动注册 Hooks / sccache 感知 / 符号优化)
      │
      ▼
分支切换 (Git post-checkout <1ms 极速打标，下次构建优先消解孤儿)
      │
      ▼
测试运行 (测试可执行文件 30 分钟时效保护窗 + 历史测试版本定时清理)
      │
      ▼
日常开发 (12GB 水位警戒 + 120min 冷却 + 编译器锁让步)
      │
      ▼
紧急防爆 (驱动器可用空间 < 5GB 自动打破冷却紧急熔断)
```

---

## 2. 全生命周期闭环机制与自动化规则

### 阶段一：首次开发环境自愈（First-time Setup & Onboarding）

1. **Git Hooks 安全就绪与兼容链式保障**：
   - 开发者运行 `npm install` 时，`postinstall` 自动进入安全就绪探测模式（`--safe`）；
   - 若检测到开发者已配置 Husky (`.husky`)、Lefthook 或企业自定义 Hooks，坚决**予以保留，绝不覆写**；若检测到 `.git/hooks` 下存在已有钩子（如 `pre-commit`），采用**安全前置注入/链式机制**，确保既有代码审查与安全检查不受任何影响；
   - 在纯净开发环境中自动将分支感知钩子绑定至 `.githooks`，克隆即生效。
2. **Profile 符号级瘦身保障**：
   - 在 `src-tauri/Cargo.toml` 中配置了 `[profile.dev.package."*"] debug = 1` 与 `[profile.test.package."*"] debug = 1`；
   - 第三方依赖在开发与测试中仅保留行号回溯，使 Windows 下 PDB 体积缩减 90%，而项目核心模块依然保留完整单步断点调试体验。
3. **全局编译缓存（sccache）感知**：
   - 运行体检命令自动探测 `sccache` 安装与运行状态。

### 阶段二：切换分支开发（Branch Switching）

1. **非阻塞极速打标（Execution < 1ms）**：
   - 在 Git 触发分支切换（`git checkout <branch>` 或 `git switch`）或合并（`git merge`）时，`.githooks/post-checkout` 仅写入微型时间戳标记 `target/.branch-switched`，绝不在 Git 执行期做耗时 I/O，Git 操作毫无延迟。
2. **构建前定向深度消解**：
   - 下次执行开发或测试构建时，看门狗检测到分支切换标记，优先执行深层孤儿会话消解（将增量保留收紧为最新 1 个，迅速释放 3~4 GB 跨分支垃圾），完成后自动清理标记。

### 阶段三：测试执行链路收敛（Test Ephemeral Retention）

1. **测试可执行文件精准识别与双层安全边界**：
   - 看门狗精准区分主程序 `tauri_appkokoro_engine`（开发期保留最新 2 代）与测试套件产物（如单元测试 `tauri_appkokoro_engine_lib-*.exe`，集成测试 `characters-*.exe`、`vision-*.exe` 等，仅保留最新 1 代）；
   - **双层安全白名单**：结合模块白名单（如 `characters`、`vision` 等已知模块）与 `deps/` 内部可执行目标动态反查机制。对非 Kokoro 命名的第三方依赖坚决不触碰其 `.rlib` 与 `.dll`，从根源确保第三方依赖库绝对安全。
2. **30 分钟保护窗 + 历史版本消解**：
   - 测试二进制及其配套的 PDB 符号文件仅保留最近 1 个活跃代际；
   - 对 30 分钟内新生成的测试产物（`.exe` 与 `.pdb`）提供时间窗口豁免保护，确保处于交互断点或测试中的产物不受干扰；
   - 超过时限的历史淘汰测试二进制与庞大 PDB 符号自动回收；在分支切换或驱动器空间紧急熔断时优先强制收敛。

### 阶段四：日常看门狗、低盘熔断与体检看板

1. **时间戳冷却保护（Cooldown = 120 分钟）**：
   - 在正常开发周期中，每次检查后进入 120 分钟冷却期。在冷却期内前置耗时 **< 2 毫秒**，重启服务零延迟。
2. **安全警戒水位（默认 12 GB）**：
   - 水位计算覆盖整个 `target` 目录总空间占用（含 debug、release、sherpa-onnx-prebuilt 等）；
   - 当 `target` 总足迹突破 12 GB 时触发自动修剪，安全回收 `target/debug` 下过时的增量编译代际与临时测试二进制，使目录健康回落至 9~10 GB。未超标或冷却期内 **100% 保持静音**；超标清理后仅输出单行高雅日志。
3. **宿主驱动器低空间紧急熔断（Drive-level Low-Disk Shield）**：
   - 自动检测所在驱动器（如 `D:` 盘）剩余可用空间。当可用空间 **< 5 GB** 时，自动**打破 120 分钟冷却限制**，立即强制修剪，并向终端发出显著警报，防止因磁盘写满引发系统或编译器崩溃。
4. **发行版与第三方模型资产绝对隔离**：
   - 自动物理删除操作严格限定于 `src-tauri/target/debug`，绝不触碰 `target/release`、`target/sherpa-onnx-prebuilt` 或打包安装包。
   - 检测到 `build`、`--release` 参数或 `CI=true` 时，0 毫秒瞬间退出。

---

## 3. 日常命令与存储体检看板

```bash
# 1. 启动桌面应用开发（前置挂载自动化无感看门狗）
npm run tauri:dev

# 2. 一键全面体检（查看驱动器余量、target 各目录分布、hooks 状态及优化建议）
npm run storage:doctor

# 3. 手动立即执行无损修剪（耗时 < 1 秒）
npm run prune:target

# 4. 预览将被清理的文件与预计释放的空间（不实际删除）
npm run prune:target:dry

# 5. 手动重新注册 Git 钩子
npm run storage:setup-hooks

# 6. 全量清空构建缓存（仅在遇到极端罕见的编译器损坏时使用）
npm run clean:target
```

### 一键体检看板（`npm run storage:doctor`）效果展示

```text
================================================================================
             Kokoro Storage Sentinel - Health & Diagnostics Dashboard
================================================================================

[Drive Storage]   Available: xxx GB / xxx GB (xx% used) [✅ HEALTHY]
[Target Footprint] Total size: 10.80 GB (10.80 GB)
  └─ target/debug                 : 9.86 GB
       ├─ debug/deps            : 6.36 GB
       ├─ debug/incremental     : 2.31 GB
       ├─ debug/build           : 1.02 GB
  └─ target/sherpa-onnx-prebuilt  : 942.7 MB

[Git Lifecycle]   Hooks Status: ✅ Active (.githooks registered)
[Build Cache]     Sccache: ✅ Installed (sccache 0.8.2)

--------------------------------------------------------------------------------
Recommendations:
  - [DAILY]  Watchdog automatically maintains target under 12 GB during 'npm run dev'.
================================================================================
```

---

## 4. 环境变量自定义（可选）

如需微调参数，可在环境变量中指定：

- `KOKORO_PRUNE_THRESHOLD_GB`：修改触发清理的警戒水位（默认为 `12`，单位 GB）。
- `KOKORO_PRUNE_COOLDOWN_MINUTES`：修改两次检查之间的最小冷却间隔（默认为 `120`，单位分钟）。
- `KOKORO_EMERGENCY_FREE_GB`：修改驱动器低可用空间紧急熔断阈值（默认为 `5.0`，单位 GB）。
- `KOKORO_SKIP_HOOKS`：设为 `1` 时完全跳过 Git Hooks 的探测与配置（适合特定的 CI 管道或自定义构建容器）。

---

## 5. 首次开发编译极速优化：启用 `sccache`

当首次拉取项目、新建分支或必须执行 `cargo clean` 时，外部 crate 编译占用大量时间。建议配置 `sccache` 实现跨工程/跨分支全局共享缓存。

1. **安装 sccache**：

   ```bash
   cargo install sccache --locked
   # 或使用 Scoop (Windows):
   # scoop install sccache
   ```

2. **配置环境变量或 Cargo**：
   在 PowerShell 中临时启用（或加入用户环境变量）：

   ```powershell
   $env:RUSTC_WRAPPER = "sccache"
   ```

   或者在用户全局 `~/.cargo/config.toml` 中配置：

   ```toml
   [build]
   rustc-wrapper = "sccache"
   ```

3. **效果**：
   - 依赖产物全量存入全局缓存（默认 `~/.cache/sccache`）；
   - 切换分支或新建 worktree 时，所有公共依赖全部命中缓存，编译时间从几分钟缩短至几秒。
