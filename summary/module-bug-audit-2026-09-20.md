# Kokoro Engine 全模块 Bug 审计

> 审计日期：2026-09-20
> 范围：当前工作树的前端、Tauri IPC、Rust 服务、AstrBot 适配器，以及 `docs/reviews/module-review-2026-09-12` 的 R01–R24 历史覆盖矩阵。

## 结论

当前没有复现 Critical/Important 或 P1 级问题。构建、IPC 注册、前端测试、Rust 测试、Clippy 和 AstrBot 离线测试均通过。

当前审计确认的 2 个问题已完成修复，并有回归测试覆盖：

### P2：MOD 脚本执行没有 deadline 或取消预算（已修复）

- 位置：`src-tauri/src/mods/manager.rs:230-235`、`:492-520`。
- MOD 脚本在专用 QuickJS 线程中同步 `ctx.eval`；调用方发送 `ScriptCommand::Eval` 后直接等待 `reply_rx.await`。
- 触发：MOD 脚本包含死循环、同步高耗时计算或永不返回的逻辑。
- 影响：QuickJS 线程无法继续处理 `DispatchEvent`、`ResetListeners`、后续脚本或卸载命令，MOD runtime 会表现为永久卡住；主进程未必退出，但该 MOD 和相关宿主操作不可恢复。
- 修复：使用 `rquickjs::Runtime::set_interrupt_handler` 和 5 秒执行预算；超时返回明确错误，QuickJS 线程继续处理后续命令。
- 回归：`script_execution_budget_interrupts_an_infinite_loop`、`script_execution_budget_allows_a_short_script` 通过。

### P3：通用下载失败时不清理临时文件（已修复）

- 位置：`src-tauri/src/utils/download.rs:39-96`。
- `temporary_download_path` 创建唯一 `.download` 路径；网络流、写盘、进度回调等任一步失败时，`download_file_with_progress` 通过 `?` 直接返回。
- 临时文件只有成功走到 `atomic_replace_file` 时才被替换/移除；失败产生的部分文件没有 RAII 清理守卫。
- 影响：模型下载中断、磁盘错误或进度回调失败重复发生时，会在目标目录积累孤立的部分下载文件，增加磁盘占用，并可能影响后续诊断。
- 修复：新增 `TemporaryDownloadGuard`，成功原子替换后提交守卫；网络、写盘、进度回调和替换失败均自动清理临时文件。
- 回归：`failed_download_removes_partial_temporary_file` 通过；下载相关 8 项测试通过。

## 已复核且当前未计入 Bug 的历史问题

- 角色目录错误消息、导入反馈、内容包变更后的目录/运行时同步和 SemVer 模板选择已有当前代码保护。
- MemoryPanel 角色切换请求使用 epoch；损坏 embedding 模型会重新验证并允许重试；摘要任务会在启动时恢复；梦境/记忆写入已有角色与事务约束。
- LLM 配置快照、HTTP SSE stream drop-cancel、Codex turn-start 清理已接入当前代码。
- Vision watcher 使用 producer generation；native mic 的帧处理线程有 cancel + join；背景资源和 TTS 播放生命周期已有代次/清理保护。
- 备份使用 `VACUUM INTO` 一致快照，并已恢复 conversation/session summary 相关表。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| `npm run build` | 通过；保留 chunk/dynamic-import 警告 |
| `npm run check:ipc` | 通过；171 个调用命令已注册 |
| `npm test` | 通过；81 个测试文件、782 个测试 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 通过 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 通过；908 个测试、3 个测试套件 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings` | 通过；0 errors、1 个已知 warning |
| `integrations/astrbot-kokoro: pytest -q` | 通过；8 项 |
| `git diff --check` | 通过 |
| 定向 MOD 预算测试 | 2 项通过 |
| 定向下载测试 | 8 项通过 |
| 本次修改文件 `rustfmt --check` | 通过 |

## 未验证边界

真实 LLM/TTS/STT/Vision provider、真实麦克风/摄像头、WebView2 恶意 MOD fixture、Tauri 多窗口、Windows 安装包、远程注册表和真实 Bot 渠道仍未做桌面/外部环境联调。这些是验证缺口，不直接等同于当前 Bug。MOD 的真实 WebView2 加载/卸载仍建议后续做桌面验收。
