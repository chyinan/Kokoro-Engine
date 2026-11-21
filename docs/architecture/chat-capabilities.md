# 聊天与桌面能力的依赖边界

## 调用关系

重构前，桌面 IPC、主动聊天及平台消息通过 `submit_chat(AppHandle, …)` 发起回合。准入后的工作任务和部分工具、TTS、回复后处理继续通过 `AppHandle` 查找运行状态或发送窗口事件。通用 Webhook 和其他 Bot 的运行实现位于 `commands/bot.rs`，Telegram 的轮询任务持有用于查找这些服务的桌面句柄。

```text
桌面 IPC / 主动聊天 ───────────────────────┐
Webhook / LINE / Discord / QQ / Telegram ──┼─ ChatService::submit
无窗口调用方 ──────────────────────────────┘      │
                                                  ├─ 准入与应用拥有的回合任务
                                                  ├─ 角色、会话和 Provider 快照
                                                  ├─ 模型轮次、工具和 SQLite 提交
                                                  └─ journal 与提交后工作
```

`lib.rs` 在启动时通过 `desktop_chat::install_services` 组装聊天及平台适配器。桌面 IPC 从注册状态取得 `ChatService`，平台监听任务持有 `BotChatServices`，其中的聊天服务、运行状态及 Provider 服务共享应用实例。Webhook、LINE、Discord、QQ 和 Telegram 在各自入口完成授权、角色绑定和媒体转换，通过 `ChatPolicy::External` 提交到同一应用任务。平台根据已提交的回复组装网络返回或后续发送。AI 初始化失败时，已启用的平台仍可启动并完成入站授权；需要聊天时适配器按平台规则报告服务不可用。

`ChatRequest` 和 `StreamChatResponse` 定义在 `chat/application.rs`，与聊天提交和回合执行共用。`chat/runtime.rs` 定义 `ChatRuntimeState`、`ChatTurnSnapshot` 与桌面恢复用的 `ChatViewSnapshot`。`chat/events.rs` 定义 `ChatEvent` 及现有事件名的映射，IPC 请求与响应继续使用现有序列化字段。

| 旧依赖 | 新归属 |
| --- | --- |
| 聊天通过 Tauri 状态查找与 `commands/tts.rs` 共用的 `TtsService`，并持桌面句柄调用语音输出 | `chat/application.rs` 接收同一 `TtsService`，`chat/runtime.rs` 把已捕获的语音快照交给它，播放事件经 `tts/desktop.rs` 发送。 |
| 提示词构建调用全局 Live2D 选中模型配置，模型库的文件操作和重命名后的选中状态修改位于 `commands/live2d.rs` | `live2d.rs` 和 `live2d/library.rs` 的 `Live2dStore` 接收显式目录。回合读取绑定角色的 cue map，再交给 `ai/context.rs` 构建提示词。 |
| QQ 和 Telegram 使用 `commands::bot` 的回复或音频转换函数 | `bot/mod.rs` 持有共享的 Bot 请求、媒体和传输逻辑。启动 IPC 负责注入服务。 |
| 聊天重生成测试调用 `commands::context::delete_last_messages_inner` | 测试与 IPC 降到 `ai/context/history.rs` 的尾部历史删除，使用同一份事务及窗口同步逻辑。 |
| `ActionContext.app` 给内置工具提供 Tauri 状态查找和桌面操作 | `actions/registry.rs` 的 `ActionServices` 显式提供状态与能力，`desktop_chat.rs` 装配截图和通知出口。 |

## 依赖与状态归属

| 依赖 | 职责 |
| --- | --- |
| `ChatService` 与 `ChatDependencies` | 向回合任务提供现有运行状态、LLM、激活协调器、工具配置、视觉数据和可选语音服务。共享服务的配置更新对后续回合生效。任务持有准入和完成所需的状态，IPC 或网络调用方可以单独结束等待。 |
| `ChatEvents` 与 `ChatDesktop` | 聊天使用 `ChatEvent` 标识事件。先更新 journal 和 MOD 队列，再通知桌面。桌面适配器处理窗口存在性、事件发布和播放出口。桌面句柄的所有权留在应用运行循环。 |
| `ActionServices` | 内置工具显式接收所需的视觉、记忆、图像、配置及桌面能力。桌面组装层把已有原生截图函数和通知回调注入工具。回合归属由 `ActionContext` 携带，直接工具调用和获批后的工具执行使用相同的分派规则。 |
| `TtsPlayback` | 同步输出播放开始、音频、浏览器委托、错误与结束。桌面适配器处理 Start 时选定 `main` 或 `pet` 并注册播放所有权，后续事件沿用同一窗口。Bot 使用回合捕获的 `TtsExecutionSnapshot` 获取音频字节。 |
| `BotEvents` | Bot 的 QQ 授权提示与图片通知交给外层适配器。平台认证、会话标识、媒体约束与发送结果留在各平台实现。 |
| `Live2dStore` | 使用显式目录读写模型库、模型配置与旧版选中模型状态。模型配置保存成功后由 IPC 广播更新。聊天构建提示词时直接接收已解析的 cue map。 |

回合准入仍在角色激活读保护内解析角色配置。显式 cue 文件或模型得到空映射时，空映射属于该角色。没有显式资产且角色仍为桌面当前角色时，才读取桌面选中模型。LLM 和 TTS 的 Provider 及执行参数在准入时捕获。凭据继续由应用级 Provider 配置管理。

## 完成与桌面恢复

最终文字、关联的工具行及会话归属以 SQLite 提交为准。保存失败时，重试使用保留的原批次和上下文。聊天完成后的记忆、背景与语音工作使用已捕获的角色和会话。自动语音开始和送出音频前检查播放范围与选择代次。语音失败记录在 `speech_error`，文字提交状态保留。

`get_chat_view` 使用 journal 和 SQLite 历史构建只读版本快照。窗口重新订阅时通过快照恢复回合和历史，生成和自动语音仍由原执行任务调度。尾部历史删除在 `ai::context::history` 中使用同一事务、版本检查和窗口同步；桌面 IPC 负责输入转换。

## 宿主边界

- `lib.rs` 和 `desktop_chat.rs` 持有应用启停和 Tauri 状态装配。原生截图与通知通过桌面能力注册，`tts/desktop.rs` 管理窗口播放事件。
- `vision/watcher.rs` 的自动采集启动和主动视觉通知使用宿主句柄。聊天和手动截图工具使用共享的视觉配置、已完成观察与 VLM 服务。
- `desktop_heartbeat.rs` 管理主动聊天与定期任务触发。自动备份仍由它调用 `commands::auto_backup::check_and_run`。该宿主路径解析 Tauri 数据目录，检查备份间隔并调用现有导出和清理逻辑；备份实现仍在既有命令模块中。
- `commands/live2d_protocol.rs` 使用 Tauri 资源路径和协议处理器为窗口提供 Live2D 文件。模型配置与库文件操作由 `Live2dStore` 负责。

## 验证

Rust 命令在 `src-tauri/` 执行，以加载该目录的 Cargo 配置。Rust 测试的 `KOKORO_TEST_DATA_ROOT` 指向系统临时目录下一个独立的绝对路径。前端与差异检查在仓库根目录执行。

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked --lib -j 4 -- --test-threads=1` | 1025 项通过，0 失败，0 忽略。测试执行耗时 126.08 秒。 |
| `cargo clippy --locked --lib -j 4 -- -D warnings` | 通过。 |
| `npm run build` | TypeScript 与 Vite 构建通过。 |
| `npm run check:ipc` | 177 个前端命令调用均有对应的已注册命令。 |
| `npm test -- src` | 89 个测试文件，846 项通过。 |
| `npm test` | 91 个测试文件通过，共 983 项。另有 `scripts/tauri.test.mjs` 在收集阶段失败。 |
| `npm test -- --exclude scripts/tauri.test.mjs` | 91 个测试文件，983 项通过。 |
| `git diff --check` | 通过。 |

Rust 验证覆盖无 Tauri 句柄的调用方脱离、服务取消、SQLite 保存重试与自动语音失败、角色配置和 cue 隔离、截图注入、Bot 退化时的入站与授权，以及 Windows 上用模拟宿主运行的通道集成。Windows 链接器报告 `LNK4098`。真实窗口人工操作、真实外部平台和外部模型未测试。

全量前端测试的收集失败可重现。`scripts/tauri.test.mjs` 第 5 行导入 `scripts/tauri.mjs`。两文件均通过 `node --check`，原模块可由 Node 直接导入。本地 Vite 7.3.5 的 SSR 转换把入口 shebang 留在输出的第 6 行，导致 `AsyncFunction` 抛出 `SyntaxError`。仅在内存中去掉首行的探针可正常加载。开工前已有改动的六个文件经 SHA-256 比对保持一致。
