# 用户体验 Bug 审查

> 审查日期：2026-09-11
> 范围：当前工作树的 React/TypeScript、Tauri/Rust、IPC、角色/内容包、聊天、语音输入和 TTS 关键路径

## 结论

当前代码中确认存在 3 个高影响用户体验问题、3 个中影响问题，以及 1 个测试基础设施问题。此次审查没有修改产品代码。

## 确认问题

### P1：麦克风启动失败后无法重试

- 证据：`src/ui/hooks/useVoiceInput.ts:278-279` 只允许 `VoiceState.Idle` 启动；启动异常在 `:309-314` 设置为 `VoiceState.Error`；`stop` 在 `:317-318` 因 `isRunning=false` 直接返回。
- 触发：首次点击麦克风时权限被拒绝、Native/WebRTC 初始化失败或设备不可用。
- 用户影响：按钮停留在错误态，再次点击只调用无效的 stop，用户必须重载窗口才能恢复语音输入。
- 建议：错误收敛时回到 Idle 并保留一次性错误提示，或让 Error 状态的点击执行完整重置后重试；补充 `useVoiceInput` 组件级回归测试。

### P1：TTS 合成失败被吞掉，前端收到“正常结束”

- 证据：`src-tauri/src/tts/manager.rs:303` 将 provider 错误变成流错误；`src-tauri/src/tts/manager.rs:356-358` 只记录错误，不向上返回；随后 `:362-364` 仍发送 `tts:end` 并返回成功。
- 触发：TTS provider 不可用、单句合成失败、流式 provider 返回错误。
- 用户影响：用户只听不到声音，没有可见错误或重试反馈；自动朗读的 `synthesize(...).catch(...)` 也不会触发。
- 建议：保留成功/失败状态，至少在全部句子失败时返回错误并发出带状态的结束事件；部分句失败应向用户显示可理解的降级提示。

### P1：内容包删除/安装后主界面运行时和目录可能不同步

- 证据：`src/ui/widgets/ContentLibrary.tsx:63` 删除角色包时调用 `removeCharacterPackage`，`:157-162` 忽略返回值；后端 `src-tauri/src/commands/registry.rs:474-572` 会计算并返回 `active_fallback`。Settings 仅在 `src/ui/widgets/SettingsPanel.tsx:1286` 挂载 `<ContentLibrary />`，没有传入 App 刷新或运行时同步回调。App 的目录加载主要发生在 `src/App.tsx:801-814` 和启动流程 `:1391-1400`。
- 触发：从 Settings 的内容库安装、更新或删除角色包，尤其是删除当前活动角色包。
- 用户影响：内容库显示操作成功，但主界面角色目录、活动角色 ID、Live2D/对话上下文可能仍保留旧状态，直到重载；删除活动包后可能出现目录点击可见但激活失败或运行时显示旧角色。
- 建议：内容包操作完成后统一触发 catalog/runtime reconciliation；删除活动包必须消费 `active_fallback` 并更新前端状态；补充安装/删除后的 App→Settings 集成测试。

### P2：聊天失败提示可能串入新会话

- 证据：`src/ui/widgets/ChatPanel.tsx:2360-2370` 在失败回调中延迟 500ms 追加错误气泡，但定时器回调没有校验 `conversationGenerationRef` 或 `clientRequestId`；同文件 `:1116-1119` 对主异步结果已有代次/请求守卫，说明此处缺少一致的保护。
- 触发：请求失败后 500ms 内切换会话、角色或新建会话。
- 用户影响：旧会话的“连接错误”可能出现在新会话消息列表中，造成用户误以为新会话也失败。
- 建议：将错误气泡绑定到请求代次/会话 ID，或在超时回调执行同一组 stale-request 守卫。

### P2：工具审批失败显示 `[object Object]`

- 证据：`src/ui/widgets/chat/turn-state.ts:142-143` 对非 `Error` 直接 `String(error)`；`src/ui/widgets/ChatPanel.tsx:2837-2840` 和 `:2853-2856` 用它展示审批失败。结构化 Tauri/Kokoro 错误对象会被转换成 `[object Object]`。
- 触发：后端拒绝 approve/reject tool approval，返回结构化错误。
- 用户影响：用户看不到失败原因，无法判断是请求过期、权限问题还是后端错误。
- 建议：统一使用 `getKokoroErrorMessage`；同类直接 `String(e)` 的 Settings 错误提示也应统一处理。

### P2：Settings 角色激活/创建/删除失败对用户静默

- 证据：`src/ui/widgets/CharacterManager.tsx:548-586` 的激活异常、`:589-621` 的创建异常、`:724-750` 的删除异常都只 `console.error`，没有设置可见错误或重试状态。
- 用户影响：用户点击后界面可能不变或状态不完整，但没有原因；容易重复点击或误以为数据已保存。
- 建议：复用 Settings 的保存错误区域，展示结构化错误并保留草稿/重试入口。

## 验证结果

- `npm run build`：通过。
- `npm run check:ipc`：通过，171 个调用命令已注册。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过。
- 定向前端测试：251 个通过。
- `npm test`：失败，Vitest 扫描了 `.worktrees/pr20-audio-lifecycle` 旧工作树；共 17 个失败，其中 16 个来自该工作树，另有 `scripts/prune-target.test.mjs` 超时。该问题主要影响测试可信度/开发流程，不直接等同于用户运行时 Bug。

## 审查限制

本轮没有启动桌面应用进行人工点击或真实 provider 网络测试，因此 TTS、内容包同步和会话污染的结论基于当前代码路径与已有测试结构；它们应在修复时补充端到端或最小集成复现。

## 修复状态（2026-09-11）

- 已修复上述 6 类用户体验问题，并为每类补充回归覆盖：结构化审批错误、聊天 stale 错误、麦克风重试、内容包目录/运行时同步、TTS 错误返回与可见反馈、Settings 角色操作反馈。
- 前端修复定向回归：129 个通过（排除已有的 `CharacterCatalog` outside-click 测试污染用例）。
- 当前源码全量回归（排除 `.worktrees`，关闭文件并行）：72 个测试文件中 71 个通过，752/753 个测试通过；唯一失败是已有的 `CharacterCatalog.test.ts` outside-click 用例，单独与相关测试组合时可通过，但嵌入全套测试时受 jsdom 全局状态影响。
- `npm run build`、`npm run check:ipc`（171 个命令）和 `cargo check --manifest-path src-tauri/Cargo.toml` 通过；TTS 专项 Rust 测试通过。
- 默认 `npm test` 仍会发现 `.worktrees/pr20-audio-lifecycle` 的旧测试；该隔离工作树及全局 DOM 测试并行污染属于测试基础设施遗留，不影响生产代码构建。
