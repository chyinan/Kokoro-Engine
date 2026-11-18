# 应用后台任务与退出顺序

桌面应用在 `lib.rs::run` 创建一个 `AppLifecycle`，为应用自己的 Tokio 任务、阻塞调用和非任务操作提供共同的准入与退出边界。业务服务仍负责何时取消、提交、重试和发布结果。这里没有第二套聊天执行器，也没有改变会话或消息持久化的提交点。

## 退出协议

1. 主窗口的 CloseRequested 保留原来的退出语义，由 Tauri 的 ExitRequested 启动退出；宠物和气泡窗口仍走原来的窗口关闭行为。首个退出请求在同步门闩内将 Running 改成 Stopping，以后同一进程的请求不会启动第二次排空。一般应用 IPC 在此后拒绝新调用；已接受任务的停止或终态确认仍可进入。
2. 关闭 Bot/Telegram 入口并向 Work scopes 发出取消信号。已登记的操作在自己的安全边界退出。仅持有有效 Work lease 的操作可以在关门后登记必要的收尾子任务；独立的后续工作仍须走正常准入门闩。
3. 排空 Work 后才向 Service scopes 发信号，并执行 Bot、Telegram、MCP、音频和快捷键的显式释放。Vision 本地文件服务、MCP、Mod 等依赖在此前仍供已接受任务使用。最后在数据库操作写锁下关闭 Orchestrator 和 MemoryManager 共享的 SQLite 池。任一依赖仍在使用或释放未确认时，跳过不安全的后续释放。
4. 所有步骤共用从第一次退出请求算起的 10 秒。Work 的正常等待至第 6 秒，允许强制取消的异步任务随后最多等待到第 7.5 秒确认结束。服务正常释放使用剩余时间至第 9 秒，异步确认和最终整理最迟至第 9.5 秒；剩余时间交给 Tauri 最终退出。发出取消或 abort 不等同于任务结束。原生调用、持久化和不可安全中断的释放若尚未完成，会作为具名 pending 留在报告；已发出的强制取消也单独记录。
5. 退出执行器不属于它正在等待的任务。只有排空获胜者能置 Finished 并放行最终 Tauri 退出码。Wry 即使收到非零的退出请求也可能返回零；桌面入口在 `run_return` 完成原生清理后使用另外保留的生命周期退出码。独立原生线程保持计时，在 Tokio 或主线程被阻塞时终止进程；另一个诊断线程尝试输出名称，日志写入不会拖延终止。Windows 使用 `TerminateProcess`，Unix 使用 `_exit`。强制终止不保证运行析构、补写数据或触发外部服务清理；退出码及日志区分该结果。

登记核心见 [lifecycle.rs](../src-tauri/src/lifecycle.rs) 的 `AppLifecycle::try_begin_shutdown`、`TaskScope`、`WorkLease::spawn_finalizer` 和 `Shutdown::finish_with_finalizer`；桌面桥接见 [app_shutdown.rs](../src-tauri/src/app_shutdown.rs)。每个任务在 Tokio 的实际 join 返回之后才从登记表移除。调用方丢掉接收句柄不会移除登记或掩盖任务 panic。

## 所有权清单

路径相对于 `src-tauri/src`。同一个入口内部被直接 await 的操作沿用其调用方的所有权。表中的 scope 既用于查找源码，也用于定位退出报告。

| 启动位置 | 所有者与停止 / 等待边界 | 依赖、失败或超时 |
| --- | --- | --- |
| `lib.rs::spawn_startup`；延迟启动 MCP、Vision、Telegram、Bot、桌面宠物 | Work `startup/*`，定时器遇关门取消；已经进入启动过程的任务等待完成，各服务另做原子登记 | 可用性检查或正在启动的服务可具名等待；不允许计时结束后绕过准入 |
| `desktop_heartbeat.rs::heartbeat_loop`，`commands/auto_backup.rs::check_and_run` | Work `heartbeat/loop`；停止下一次轮询。剪枝、日 Dream 有各自耐久登记；计划备份仅在到期时取得 `backup/scheduled-auto-backup` | DB、LLM、备份路径到当前操作完成才可释放；备份临时文件、提交、旧归档清理用同一 lease |
| `chat/application.rs::ChatService`；桌面、Mod、主动消息、Bot 外部提交及保存重试 | Work `chat` 登记实际执行；可取消阶段响应退出，原有必要保存阶段耐久等待。订阅生命周期不等于执行生命周期 | 不早于既有提交点发布；已提交结果保留原有恢复语义，退出不制造新的成功事件 |
| `chat/post_commit.rs`、`chat/follow_up.rs`、`ai/context.rs`、`commands/context.rs::end_session`；后处理、记忆、摘要 | Work 子任务；独立后续工作关门后不再进入。模型等待与必要写入分别在任务内处理 | 保留已有角色、会话和 DB 世代校验；较慢的 native embedding 可能在截止时保留任务名称 |
| `chat/runtime.rs` 自动播放和 `tts/manager.rs` 合成请求 | Work 播放任务和请求取消；等候已登记请求、终态和 SDK 资源 | 前端音频反馈可终止已存在的播放，不触发新合成 |
| `llm/provider.rs`、`llm/anthropic.rs`、`llm/responses.rs` 流 | 流对象直接拥有 transport 的 pending futures；消费或 drop 负责结束，沿用上层 turn 的取消和等待 | 这些 HTTP/SSE 解析器没有独立 pump 需要等待 |
| `llm/codex_runtime.rs` 的 Codex 模型工作和 app-server | Work 模型轮次、必要收尾和阻塞 binary 探测；Service 子进程持有 stdio、事件处理和退出等待 | Work 排空前保留连接；Service 阶段关闭子进程并等待，阻塞探测或未退出进程有具名诊断 |
| `bot/lifecycle.rs`、`bot/mod.rs`、`qqbot/runtime.rs`、`telegram/{mod,bot}.rs` | Work 接受请求或更新；Service 托管监听、gateway、心跳或 polling。入口关门拒绝新处理，已接受处理显式保留至回复结束 | Bot 工作结束前保留发送端、模型、MCP、语音及 DB；并发连接子任务由协议 owner 等待 |
| `mcp/bridge.rs::start_enabled`、连接/重连、catalog、`mcp/client.rs`、`mcp/transport.rs` | Service 连接 worker 拥有 SDK session 和 transport；显式 shutdown 关闭并等候 session / 管道 / 子进程，撤销 action 注册 | 已接受工具执行结束后才停止；客户端协议和依赖库内部线程通过拥有它们的 session 退出结果确认 |
| `vision/watcher.rs::VisionWatcher::{start,stop,capture}`、`commands/vision.rs` | Work 中的一次 watcher run、VLM 分析链、原生屏幕捕获和枚举各自登记。手动停止最多等候两秒，若未结束仍保留旧 run，并拒绝重启；应用退出使用全局预算 | generation 阻止过期结果。Native 捕获若不可中断保留诊断；VLM 请求可取消，Provider panic 作为失败完成该帧并继续处理最新排队帧 |
| `vision/server.rs::VisionServer::start` | Service `vision-http/http-listener` 与 `vision-http/upload-ttl-cleanup`；Warp graceful shutdown 等连接，清理任务响应定时器取消 | 已接受聊天仍可读取图像 URL；已开始的文件系统操作等候完成 |
| `stt/native_worker.rs`、`stt/{mic,wake_word,sensevoice_local}.rs` 和 `commands/stt.rs` | Work 结束采集/拒绝新命令，Service 设备 owner 和显式 shutdown 负责确认退出；推理/模型提取等阻塞操作单独登记 | CPAL、Sherpa 或模型本地执行不承诺可被抢占；逾时报告具体 native 工作 |
| `tts/openai_realtime.rs`、`tts/omnivoice.rs` | Realtime 写端由请求流本人持有，沿用其取消；OmniVoice Python 进程由具名 Service 持有并确认回收 | Python 已开始执行时不能仅以取消 request future 判定它已停止 |
| `mods/manager.rs::ModManager::init` 和 `mods/script_queue.rs` | Service supervisor 等待 QuickJS runtime 与事件转发，也拥有初始化健康定时器。命令/转发在退出信号或通道关闭结束；脚本有既有执行上限 | 外部事件由 Work lease 保留，在途聊天可继续使用必要的 Mod 服务；卡在原生工作时保留任务名称 |
| `commands/pet.rs::register_pet_shortcut`、`show_pet_window`、`show_bubble_window` | Work 维护快捷键触发和初次气泡内容定时器，显式注销快捷键在 Service release | 被拒绝的晚到快捷键不创建窗口；已关闭气泡的发送错误不保留窗口 |
| `commands/backup.rs`、`commands/registry.rs` 和 `commands/live2d.rs` | Work lease 跨整个有副作用的导入、导出、验证、清理、文件/软件包提交或移除 | 截止前不因 bridge 请求消失而视为结束；保留 staging、原子提交与验证边界 |
| `commands/{characters,conversation,memory,database,context,actions,imagegen}.rs` 及 `commands/chat.rs` 的数据库视图 | 依赖共享数据库或有长期资源操作的 IPC 在 handler 取得 Work lease，随返回/出错释放 | 全局 IPC 预检只拒绝晚到调用，不能观测 Tauri 内部命令 future；生命周期由操作内的 lease 保持，不依赖 JS 等待回包 |

## 进程与框架边界

- 未启动持久任务的简单配置读写、选择状态读取、只读远端探测和内嵌资源 URI 响应由同步代码或 Tauri 命令 future 直接完成，没有自己的后台调度器。共享 SQLite 读写和可能跨 await 的提交入口通过上表登记；SQLite 的 worker 线程由池关闭统一回收。
- 浏览器相机和语音资源在 `src/features/camera/CameraWatcher.tsx`、`src/ui/widgets/settings/VisionTab.tsx`、`src/ui/hooks/{useVoiceInput,useWakeWord}.ts`、`src/core/services/voice-interrupt-service.ts` 管理，显式停止 media tracks、定时器、节点与 AudioContext。UI 订阅、动画、轮询的 owner 是各 webview/组件；示例为 `src/lib/chat/application.ts::disconnect`、`src/ui/widgets/settings/{BotTab,McpTab}.tsx` 和 `src/windows/{PetWindow,BubbleWindow}.tsx` 的卸载清理。它们不是 Rust 后台任务，整个 webview 随最终退出关闭。
- 脚本、Warp/Hyper、Tauri、SQLx 和设备/模型 SDK 内部活动按所属资源的 stop/join 边界处理。直接运行的远端模型服务、用户手动启动的进程及独立 AstrBot 部署不属于此进程，关闭客户端不承诺停止这些外部服务。
- MCP stdio、Codex app-server 和 OmniVoice 的正常关闭等待本地子进程退出。Windows 通过启用 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 Job Object 管理子进程和组内后代，强制结束主进程时由系统关闭 Job 句柄并发出终止。Unix 上由应用向自有进程组发出停止信号。如果主进程在这一步前执行 `_exit`，子进程可能留下，退出报告保留未确认的 owner。MCP Streamable HTTP 等待本地 SDK 关闭并尝试删除远端会话，远端拒绝或没有确认时无法据此判断会话是否已释放。
- Tauri 2.10.3 的 `ExitRequestApi::prevent_exit` 无法阻止框架内置的 `RESTART_EXIT_CODE`。仓库的 process restart 权限目前没有产品调用方，保留原权限；调用该底层重启或在应用收到退出事件之前被操作系统终止，不执行异步排空协议。
- 全局计时从 Tauri 实际投递首个退出事件时开始。启动期间 `setup` 的同步装配、配置恢复和数据库迁移仍由 Tauri 的 Ready 回调执行，尚未收到退出事件时不能进入排空；启动完成后才投递的延迟任务均已纳入登记。
- `cargo test` 的 mock listener、合成进程、等待任务和线程不属于生产后台；例如 `config.rs`、`actions/executor.rs`、`chat/*test.rs`、`mcp/tests.rs` 与 Live2D 的 Windows reparse 测试。
- `tts/queue.rs::TtsQueue::enqueue` 是没有生产调用方的辅助 API；`TtsService` 当前只构造队列配置，实际合成走其请求 owner。

## 验证入口

- Windows 上 `cargo test --manifest-path src-tauri/Cargo.toml --quiet` 的 1065 个 Rust 测试全部通过；`cargo check --manifest-path src-tauri/Cargo.toml --tests` 和 `cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings` 均通过。
- `lifecycle::tests` 用暂停的 Tokio 时钟和显式屏障测试关门、必要子任务、panic、确认取消、阻塞任务、服务和资源释放顺序。
- `app_shutdown::tests::watchdog_stops_process_even_when_main_is_blocked` 对停在阻塞等待且没有 Tokio runtime 的子进程运行实际原生 watchdog。单独运行该子进程时，600 ms 预算下的墙钟时间（含启动）约为 621 ms，退出码为 1，并记录 `watchdog-fixture/blocked native work`。
- `vision::tests::correctness::vision_http_keeps_serving_until_admitted_work_drains` 通过本地回环 HTTP 验证在途工作结束前图像可读取、随后 listener 已退出。
- `vision::watcher::lifecycle_tests` 检查手动停止超时不会丢弃旧 run，以及 VLM Provider panic 后还会处理最新排队帧。
- 聊天取消和持久化、Bot/SDK 退出及 native 待处理结果分别由 domain 测试覆盖。SDK worker、GUI 和异平台进程行为应结合实际平台验证；单元测试通过不等于验证了真实模型或设备。
- 前端 `npm run check:ipc` 的 locked Rust 导出校验及前端传输检查均通过；`npm run build` 通过。原样运行 `npm test` 时 92 个文件、992 个测试通过，基线已有的 `scripts/tauri.test.mjs` 在收集时报 `SyntaxError: Invalid or unexpected token`；两个脚本及测试配置与基线相同，Node 直接语法检查通过。用临时 Vite transform 只去掉被导入的 `scripts/tauri.mjs` 首行 hashbang 后，该文件的 6 个测试单独通过。探针已删除，未修改启动脚本或测试配置。
- Windows 的测试、桌面程序和 IPC 导出器链接均成功，MSVC 报 LNK4098 链接警告。

### Windows 桌面复现

从仓库根目录编译 debug 桌面程序并运行隔离用例：

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --bin tauri-appkokoro-engine
node scripts/native-background-shutdown-check.mjs src-tauri/target/debug/tauri-appkokoro-engine.exe '<Playwright模块目录>' '<新建或空的输出目录>'
```

需要 Node、带 `sqlite3` 的 Python（可用 `KOKORO_FIXTURE_PYTHON` 指定）、WebView2、Playwright Node 模块，以及空闲的本机端口 1420、19334、19335。脚本为每个实例准备独立的应用数据目录和本机模型，保存 `native-shutdown-report.json` 与进程日志。检查宠物关闭/重开、重复主窗口关闭、模型流取消、SQLite 写锁及时释放后的保存，以及写锁一直占用时的有界退出。最后一项是应用驱动的强制结果（预期退出码 1、报告具名 `chat/turn`、未完成助手行不提交），与子进程测试触发的原生 watchdog 终止不同。

最终 Windows Tauri/WebView2 运行 `branch-final-20261118-171333` 通过 4 个用例，二进制 SHA-256 为 `46d827aa5699d1e046d3e1940b0969dd705ae4d1e5545825d53de09e47516559`。以下用时从发出关闭请求算起，仅用于这次受控环境：

| 用例 | 退出码 / 用时 | 观测结果 |
| --- | --- | --- |
| `window-events` | 0 / 249 ms | 宠物窗口先关闭并可重开；三个主窗口关闭请求只触发一次正常排空 |
| `shutdown-during-generation` | 0 / 208 ms | localhost 模型流中止；已保存的用户行仍存在，未提交助手行 |
| `shutdown-drains-held-save` | 0 / 508 ms | 保持进程等待写锁，释放后提交助手行；`PRAGMA integrity_check` 为 `ok` |
| `shutdown-forces-held-save` | 1 / 7590 ms | 写锁长期持有，报告具名 pending `chat/turn`；已提交旧助手行保留，未完成行不落盘，`integrity_check` 为 `ok` |

本次桌面测试使用本地受控模型和临时数据；没有连接真实 Bot、外部 MCP 或云端模型，也没有验证物理麦克风、真实 VLM 或 macOS/Linux GUI。
