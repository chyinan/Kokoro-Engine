# 开发者存储占用全生命周期闭环管理与极速编译指南

本文档为 Kokoro-Engine 开发者提供**全链路覆盖、完全自动且具备闭环自愈能力**的存储占用管理机制（Kokoro Storage Sentinel, KSS）及极速编译优化实践。

> 💡 **核心结论**：
> Kokoro Engine 当前已实现 **100% 全自动存储闭环看门狗管理**。无论是运行主桌面应用调试（`npm run tauri dev` / `npm run tauri:dev`）还是前端界面调试（`npm run dev`），看门狗均在前置生命周期中**自动、静默、无感**执行。日常开发**无需手动执行任何清理命令**，亦无需担忧 Rust `target` 目录膨胀或磁盘写满。

---

## 1. 存储膨胀机理与全链路架构

在频繁进行 `npm run tauri dev`、`cargo test`、分支切换以及 IDE 语法分析时，`src-tauri/target/debug` 会迅速膨胀（实测可在数轮构建后达到 25GB 以上），根源包括：

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
分支切换 (Git post-checkout <50ms 极速打标，下次构建优先消解孤儿)
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
   - 开发者运行 `npm install` 时，`postinstall` 自动进入安全就绪探测模式（`--setup-hooks --safe`）；
   - 若检测到开发者已配置 Husky (`.husky`)、Lefthook 或企业自定义 Hooks，坚决**予以保留，绝不覆写**；对仓库内受信任自定义钩子目录及 `.git/hooks` 下的 Shell 钩子采用**安全前置注入/链式机制**（保持既有钩子逻辑完好，且杜绝越界写与软链接劫持）；若遇外部路径或受限环境则显式报告 Sentinel 停用状态，确保既有代码审查与安全检查不受任何影响；
   - 在纯净开发环境中自动将分支感知钩子绑定至 `.githooks`，克隆即生效。
2. **Profile 符号级瘦身保障**：
   - 在 `src-tauri/Cargo.toml` 中配置了 `[profile.dev.package."*"] debug = 1` 与 `[profile.test.package."*"] debug = 1`；
   - 第三方依赖在开发与测试中仅保留行号回溯，使 Windows 下 PDB 体积缩减 90%，而项目核心模块依然保留完整单步断点调试体验。
3. **全局编译缓存（sccache）感知**：
   - 运行体检命令自动探测 `sccache` 安装与运行状态。

### 阶段二：切换分支开发（Branch Switching）

1. **非阻塞极速打标（Execution < 50ms）**：
   - 在 Git 触发分支切换（`git checkout <branch>` 或 `git switch`）或合并（`git merge`）时，Git Hook 启动轻量 Node 进程动态探测当前检出分支的脚本能力，仅在具备打标能力的版本中安全写入微型时间戳标记 `target/.branch-switched`（全程受符号链接防护与跨版本兼容自省保护，历史旧分支无副作用静默略过），绝不在 Git 执行期做重度 I/O，不增加开发者操作感知延迟。
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
2. **安全警戒水位与可回收预算（默认 12 GB）**：
   - 水位计算联动整个 `target` 目录总空间占用（含 debug、release、sherpa-onnx-prebuilt 等）并动态计算可回收 debug 预算；
   - 当 `target` 总足迹突破 12 GB 时，若非 debug 资产未占满阈值，安全回收 `target/debug` 下过时的增量编译代际与临时测试二进制，使目录健康回落至 9~10 GB；
   - **非 debug 资产保护与防抖动**：若 `target/release`、预编译模型或安装包单独已达到或超过 12 GB，看门狗自动停止对 debug 活跃编译缓存的徒劳驱逐，避免破坏日常增量构建效率，并通过 `--doctor` 引导使用 `npm run clean:target` 重置发行包；
   - 未超标或冷却期内 **100% 保持静音**；超标清理后仅输出单行高雅日志。
3. **宿主驱动器低空间紧急熔断（Drive-level Low-Disk Shield）**：
   - 自动检测所在驱动器（如 `D:` 盘）剩余可用空间。当可用空间 **< 5 GB** 时，自动**打破 120 分钟冷却限制**，立即强制修剪，并向终端发出显著警报，防止因磁盘写满引发系统或编译器崩溃。
4. **发行版与第三方模型资产绝对隔离**：
   - 自动物理删除操作严格限定于 `src-tauri/target/debug`，绝不触碰 `target/release`、`target/sherpa-onnx-prebuilt` 或打包安装包。
   - 检测到 `build`、`--release` 参数或 `CI=true` 时，0 毫秒瞬间退出。
5. **活跃编译器锁让步避让（Active Compiler Lock Yielding）**：
   - 自动检测系统中运行的 `cargo` / `rustc` 进程及 `target/.cargo-lock`、`s-*.lock` 等编译器锁；
   - 若检测到正在进行编译，自动看门狗立即安全让步退出，绝不与编译器竞争锁文件，绝不影响或打断当前构建任务。

---

## 3. 日常命令与存储体检看板

### 3.1 日常开发（每次启动前自动检查与修剪，无需任何手动指令）

开发者只需像往常一样启动开发命令，存储看门狗会在每次启动前自动完成水位检查与无感修剪（在开发进程运行期间不驻留后台，无常驻内存与文件锁开销）：

```bash
# 启动桌面完整应用开发（看门狗已通过 pretauri / tauri:dev 前置钩子全自动执行）
npm run tauri dev
# 或
npm run tauri:dev

# 启动纯前端界面开发（看门狗已通过 predev 前置钩子全自动执行）
npm run dev
```

> 💡 **提示**：自动检查挂载于 `npm run tauri dev` / `npm run dev` 脚本的前置钩子中；若习惯直接在终端运行原生 `cargo` 命令，可在构建前按需手动执行 `npm run prune:target`，或利用 Git 分支切换自动打标唤醒清理。

### 3.2 可选维护与体检看板（按需使用）

以下命令仅供开发者查看存储分布、排查异常或进行环境自检时使用：

```bash
# 1. 一键全面体检（查看驱动器余量、target 各目录分布、hooks 状态及优化建议）
npm run storage:doctor

# 2. 手动立即执行无损修剪（耗时 < 1 秒）
npm run prune:target

# 3. 预览将被清理的文件与预计释放的空间（Dry-run 演练模式，不实际删除）
npm run prune:target:dry

# 4. 手动重新校验并注册 Git 钩子
npm run storage:setup-hooks

# 5. 全量清空构建缓存（仅在遇到极端罕见的编译器损坏时使用）
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
  - [DAILY]  Watchdog automatically maintains target under 12 GB during 'npm run tauri dev' / 'npm run dev'.
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

## 5. 首次开发编译极速优化：启用 `sccache`（可选全局加速）

> ❓ **为什么 `sccache` 没有被强制全自动安装与写死配置？**
>
> 1. **外部系统级原生工具，非 npm 包**：`sccache` 是 Mozilla 开发的跨项目 Rust 编译缓存工具（系统二进制 CLI），需通过系统包管理器（如 `scoop`、`brew`、`apt`）或 `cargo install` 安装，不属于 npm 依赖包或 Rust 官方默认内置组件。
> 2. **避免引发编译致命报错（Hard-fail 风险）**：Cargo 不支持容错式的 wrapper 配置。如果项目在 `.cargo/config.toml` 中硬编码 `rustc-wrapper = "sccache"`，任何未安装 `sccache` 的开发者运行 `npm run tauri dev` 或 `cargo build` 时都会**立即致命崩溃**（`error: failed to run compiler: program not found: 'sccache'`）。
> 3. **保护 `npm install` 极速轻量体验**：若在 `postinstall` 强行后台静默执行 `cargo install sccache`，克隆后首次安装将额外耗费 3~5 分钟进行本地源码编译，并容易因宿主机网络受限或缺失特定平台的 C++ 运行时而导致整个 `npm install` 失败。
>
> 💡 **项目已内置且 100% 全自动生效的编译优化**：
>
> - **依赖包 Profile 符号级瘦身（已 100% 自动生效，零配置）**：已在 `src-tauri/Cargo.toml` 中内置 `[profile.dev.package."*"] debug = 1`，克隆即享，**无需安装任何额外工具**即可缩减 90% PDB 符号体积与编译耗时。
> - **自动感知与健康自检**：运行 `npm run storage:doctor` 会全自动探测当前环境是否具备 `sccache`，并给出精准加速建议。

### 如何按需启用 `sccache` 享受跨工程秒级构建？

对于需要在不同分支或多个 Rust 项目间频繁切换的开发者，推荐安装 `sccache`，实现外部依赖跨工程全局共享：

1. **安装 sccache**：

   ```bash
   # Windows (推荐使用系统内置 winget，免编译秒级安装):
   winget install Mozilla.sccache

   # 或通过 Scoop:
   # scoop install sccache

   # 或通过 Cargo 源码编译安装:
   cargo install sccache --locked

   # macOS:
   brew install sccache

   # Linux:
   sudo apt install sccache  # 或对应发行版包管理器
   ```

2. **推荐配置方式（用户全局一次性配置，所有 Rust 项目通用）**：
   在用户主目录下的全局 Cargo 配置 `~/.cargo/config.toml` 中写入（文件不存在则创建）：

   ```toml
   [build]
   rustc-wrapper = "sccache"
   ```

   或者在当前终端会话临时指定（PowerShell）：

   ```powershell
   $env:RUSTC_WRAPPER = "sccache"
   ```

3. **检验生效**：
   配置完成后（Windows 下使用 winget 安装后需重启 IDE 或重开终端以加载新 PATH），运行存储体检命令：

   ```bash
   npm run storage:doctor
   ```

   看门狗将自动感知并显示：

   ```text
   [Build Cache]     Sccache: ✅ Installed (sccache x.x.x)
   ```

   首次编译后，后续分支切换或 clean 构建将由全局缓存接管，依赖编译耗时从几分钟缩短至几秒。
