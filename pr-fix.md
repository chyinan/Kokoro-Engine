第六次复审：仍然 REQUEST_CHANGES
本次基于最新 head a3df039d896a5b727293dab842cc59390305acd1。上一轮指出的 5 项问题已经有对应修复：同角色点击保留 draft、activation 补偿事务化、TTS refresh 使用独立 revision、Background 失败向上抛错等均已落地，且前端新增测试覆盖了正常故障路径。本轮只保留下面两条仍属于本 PR recovery 机制的边界问题。

1. P1：activation recovery 失败时没有设置 runtime_degraded，清空 history 后仍允许继续聊天
涉及：commands/characters.rs、activation.rs 和 context.rs。

本 PR 新增了 runtime_degraded 保护，但 apply_orchestrator_runtime 在 history 同步之前就无条件调用 clear_runtime_degraded()。当 activation 的 history 查询失败、回滚 restore 也失败时，recover_committed_backend 会尝试清空 history 后返回 Ok；它没有设置 degraded。此时 runtime 可能已经应用、history 可能为空，compose_prompt/stream_chat 也不会被 degraded gate 拦截，用户仍可继续发送消息，正好违反了新增保护的目标。

启动路径在 lib.rs 会单独设置 degraded，但普通 activation failure/recovery path 没有同等处理。请保证所有 recovery 失败路径要么完整恢复 history，要么设置 runtime_degraded 并阻止聊天；不能只清空 history 后继续运行。请补充“非启动 activation recovery 失败后，compose_prompt/stream_chat 必须被拒绝”的测试。

2. P1：recover_committed 忽略 restore 和 fail-safe clear 的错误，错误信息可能与实际状态不符
涉及：activation.rs。

history sync 失败时，代码忽略了 backend.restore(&initial_snapshot) 和 backend.sync_history(None) 的结果，然后返回包含“rolled back runtime / cleared dirty history”语义的错误。若 restore 失败，backend 仍可能停留在 committed runtime；若 clear 也失败，history 仍可能是旧内容，但调用方只拿到一个没有补偿结果的错误。

这段 recover_committed 同时被启动恢复和公开的 get_committed_character_runtime IPC 使用。启动方虽然会设置 degraded，但普通 IPC 调用不会自动设置 degraded；因此调用方可能把前端恢复到旧 snapshot，而 backend 仍是新 runtime，或者 backend 与 history 状态未知。请对 restore/clear 结果逐一处理并返回明确的 degraded 状态，或把补偿封装成一个保证一致性的函数；不要用 let _ = 隐藏失败。

当前验证
最新 head 本地验证：npm test 为 51 个测试文件、320 项通过；npm run build、npm run check:ipc、cargo fmt --check、git diff --check 通过。cargo test --manifest-path src-tauri/Cargo.toml activation 仍因环境下载 sherpa-onnx 和 ONNX Runtime 失败，不能算 Rust 测试通过。

请修复以上两条 PR recovery 路径后再提交，并在提交前再次检查所有 restore、history sync 和 degraded 分支。