# 验证基线

基线提交及初始四文件字节哈希见 baseline.json。以下结果均来自本次执行的原始日志，不引用历史通过记录。

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| 默认 npm test | 失败：143 文件，1485 通过/16 失败；失败来自 .worktrees/pr20-audio-lifecycle | evidence/baseline-frontend.log |
| 当前 src/scripts，排除 .worktrees | 72 文件、753 测试通过 | evidence/baseline-frontend-scoped.log |
| 前端构建 | 通过；存在分包体积和动静态混合导入警告 | evidence/baseline-build.log |
| IPC 名称注册 | 171 个调用名称通过；不证明类型与事件契约一致 | evidence/baseline-ipc.log |
| Rust 测试发现 | 874 项 | evidence/baseline-rust-list.log |
| Rust 全量 | 874 通过，0 失败 | evidence/baseline-rust.log |
| Clippy --lib -D warnings | 通过 | evidence/baseline-clippy.log |
| Vision stress | 显式启用 stress，5 项通过 | evidence/baseline-vision-stress.log |
| 默认 AstrBot pytest | 未进入测试：自动加载 langsmith 插件，其全局 pydantic-core 版本不兼容 | evidence/baseline-astrbot.log |
| AstrBot 隔离插件 | PYTEST_DISABLE_PLUGIN_AUTOLOAD=1，仅加载 pytest_asyncio.plugin，8 项通过 | evidence/astrbot-isolated-plugins.log |

## 发现性与证据边界

- 当前 Vite 配置没有限定测试发现范围，导致旧工作树测试和当前源码 alias 混用。修复阶段限定项目测试入口，保留默认失败证据。
- 自主行为 tests_autonomous 文件是否接入模块树列入 R08。源文件存在不代表实际执行。
- 真实 embedding 路径在部分 Rust 测试中被 deterministic test_embedding 替代，不将此类通过解释为真实模型运行通过。
- Windows Sandbox 可执行文件未找到；已有 NSIS 产物为 0.2.7–0.2.9，不能作为当前 0.4.0 验收产物。当前没有正在运行的 Kokoro 窗口。
- 源码同时使用硬编码 dirs_next 数据目录与 Tauri app 数据目录；仅改变进程 APPDATA 或 Tauri identifier 不能证明隔离了全部用户数据。安装/桌面破坏性场景需独立隔离运行环境；不会直接使用真实用户库。

具体命令、起止时间、退出码在对应 evidence/*.json。新增缺陷回归的失败属于审查产物，与上表初始基线区分。

## 修复后收口证据

- `npm test`：78 个文件、766 个测试通过；`npm run build`、`npm run check:ipc`（171 个命令）和 `git diff --check` 通过。
- `cargo test --manifest-path src-tauri/Cargo.toml`：888 个测试通过；`--features stress vision::tests::concurrency`：5 个测试通过；Clippy `--lib -- -D warnings` 退出码 0。
- R06/R17/R12/R18/R01 定向新增回归分别通过 4、3、2、3、1 项；前端生命周期回归 19 项通过。
- AstrBot 在禁用自动加载的外部 pytest 插件后 8 项通过；默认进程的 pydantic-core 版本冲突仍是环境问题。
- Windows installer 的 release 编译完成，NSIS 生成阶段因官方工具下载超时仍未完成；不将旧 0.2.x 安装器当作本轮证据。
