# 聊天应用能力重构与验收记录

验证日期：2026-11-12（Asia/Tokyo）。提交范围为聊天生命周期重构、相关测试与复跑脚本、Clippy 修复及本文档。运行产生的日志、数据库、截图、浏览器配置和构建文件不纳入 Git。

## 结果与边界

聊天执行现由 Rust 应用任务持有。ChatPanel、桌宠和其他显示只提交意图并读取投影，卸载、隐藏、组件替换以及发起请求的主 WebView 被销毁均不会取消该任务。原有生成互斥、取消状态、工具执行、SQLite 持久化、记忆触发与角色激活事务继续复用。

当前检查：严格 Clippy 原命令通过，未使用 lint 豁免；Clippy 修复后重新运行 Rust 库测试，955 项通过。PR 准备时重新验证提交范围内的 JavaScript 86 个文件、820 项测试及 TypeScript 检查。前端构建、175 个 IPC 注册检查和真实 Tauri + WebView2 + 受控 localhost SSE 模型的 15 个场景在同日聊天重构验收中通过；Clippy 修复仅为 8 个 Rust trait 的 29 个 async 方法添加操作说明，未修改运行逻辑，因此未重复执行原生场景。

历史状态保留：修复前，Rust/Clippy 1.99.1 对 async_trait 宏展开报告 29 个 double_must_use；当时仅允许该 lint 的补充检查通过，严格检查失败。该状态已由下述属性修复和严格检查通过更新，不再是当前阻塞项。

未调用真实云端模型、外部 TTS/STT、MCP 服务或图像生成供应商。浏览器 TTS 验证了单输出窗口事件归属，未验证听感、声卡输出质量或外部供应商质量。缺失本地记忆嵌入模型时，真实原生聊天仍完成，日志明确记录语义检索不可用与关键词降级。MOD 通知路由完成迁移，未安装真实 MOD 消费端进行端到端计数。

## C01：前后职责和依赖

| 状态或行为 | 原实现 | 当前唯一权威/归属 |
| --- | --- | --- |
| 请求受理 | Panel busy、桌宠/interaction 握手、Rust turn lock | Rust 原有 turn lock 原子决定；不增加生成并行或请求队列 |
| 逻辑请求幂等 | 入口 ID + Panel refs/watchdog | ChatRuntimeState 按 client_request_id 与原始输入指纹保存唯一结果 |
| 执行生命周期 | Rust 流程 + 发起 WebView 的 IPC future | submit_chat 独立 spawn 原 execute_chat，IPC 只等待结果 |
| 流式内容与终态 | 事件、组件 refs、Promise fallback | Rust journal 完整快照与 revision；SQLite 是已保存历史的权威 |
| 取消 | Panel 当前 turn/ref 与握手反向事件 | Rust TurnCancellationState；request_id 未知 turn_id 时同样有效 |
| 历史修改 | Panel 内的删尾、重生成、清空编排 | ChatApplication 薄命令 + 后端原会话锁、生成锁、tail guard |
| 角色切换 | Panel 在 clearVisibleConversation 中以 character_switched 取消 | 原 serialized character activation owner 调用 prepareCharacterSwitch；先读权威快照，真切换才取消 |
| 保存、工具、记忆触发 | Rust 管线 | 仍是同一个原 Rust 管线 |
| 自动语音 | Panel finish/fallback | 后端执行任务只调度一次；main 或 pet 一个物理窗口负责播放 |
| MOD QuickJS 通知 | App 监听后转发 | 后端 journal 事件出口 try_lock/try_send；App 只保留 iframe 展示消息 |
| 展示缓存 | 每个 Panel 完整状态机 | 每个 WebView 的 ChatApplication 只作适配和投影；不是全应用权威 |
| 草稿、STT输入、键盘、动画、滚动 | Panel | 继续由显示/输入层负责，收尾不等待逐字动画 |

旧依赖：pet/interaction → Tauri 业务事件 → ChatPanel accepted → stream_chat；finish → Panel 保存投影和 TTS 收尾。

新依赖：输入适配 → ChatApplication → typed bridge → Rust submit_chat → 原 execute_chat；journal/read snapshot → 每窗口投影 → React 显示。主动聊天由 heartbeat/vision 直接进入 submit_chat，不需要任何 Panel。

核心模块在 src/lib/chat：application、projection、history、history-commands、turn-state、streaming-state。它们不 import UI、React、DOM，不读 localStorage。Node 环境测试直接创建应用适配器；运行不需要组件。旧 UI helper 路径保留兼容导出，业务实现没有复制成另一份引擎。

## C03：入口与接口清单

| 入口 | 当前路径与约束 |
| --- | --- |
| 主面板文字/图片 | ChatPanel 输入适配 → application.send；携带 character_id、conversation_id、conversation_generation、图像意图 |
| STT 自动发送 | 保留录音开始时角色/会话/generation；改变则留原角色草稿，符合条件才调用同一 send |
| 桌宠 | usePetChat → application.send，pet_ ID；独立读取 journal 更新气泡 |
| 角色手势互动 | interaction-service → submitChat；保留既有单条合并手势重试策略 |
| 主动聊天 | heartbeat / vision watcher → submit_proactive → submit_chat |
| onboarding | UI-free OnboardingChat → application.send；dismiss 按逻辑 ID 取消，迟到成功不完成引导 |
| MOD UI send_message | App 只将已有 MOD UI 意图适配到 submitChat；不要求记忆模型可用 |
| 继续/重生成 | application.continueFrom → 原后端删尾保护；仅捕获的角色/会话/generation 仍相同才重发 |
| 编辑、清空、删除会话 | application 命令 → 原 Rust 历史操作；Sidebar 不凭旧 selection 在迟到删除后再次清空 |
| 工具批准/拒绝 | application.approveTool / rejectTool → 原 typed IPC |

生产源码中 streamChat 只在 application 的默认 transport 调用；bridge 只 invoke stream_chat；Rust stream_chat 只转交 submit_chat。旧 pet-chat-start/accepted/rejected 与 interaction-trigger/accepted/rejected 业务握手、Panel external-turn allowlist 和挂载 watchdog 均退出生产路径。旧生命周期 helper 的历史单元测试保留，不代表旧运行路径仍存在。

新增接口：get_chat_view（只读一致恢复）、chat-runtime-changed（完整 turn 快照）、finish_chat_playback（以当前窗口 + request_id 释放播放所有权）。stream_chat 返回 canonical text 与状态，事件全部丢失也能完成 onboarding 等调用者。

## ID、状态与竞态规则

- client_request_id 是逻辑请求键；同 ID 同原输入共享原结果，冲突输入明确拒绝。客户端重复点击共享 Promise；跨窗口幂等由 Rust 保证。
- turn_id 只在获得执行权后产生。受理之前/准备阶段/流中均能以 client_request_id 取消，复用 tombstone；取消命令等待 journal 终态（有界 10 秒），避免切换紧接着撞上仍在收尾的执行锁。
- character_id、conversation_id 与 conversation_generation 在受理流程校验并冻结。generation 防止 A→B→A 或清空后的旧请求重新写入。
- 状态是 pending → accepted → streaming → completed/cancelled/error/rejected；终态单次提交，重复终态和迟到旧事件不再改写。
- 用户消息写入 SQLite 时，在同一 conversation_switch_lock 内发布 chat-turn-bound，恢复不会在保存行旁边再加一条 optimistic 用户消息。
- 主面板在请求绑定已保存的用户消息后才清除已发送的草稿。窗口重开时按草稿的请求 ID 核对 SQLite，聊天运行时重启后也可识别已经写入的用户消息。受理失败时，文字和待发送图片仍可重试。
- 待审批工具事件先写入 journal，订阅者在工具结果入库前即可恢复批准和拒绝操作。
- 先订阅再读取；完整快照按 revision 合并，读被事件超越时重新校验。连接世代阻止退订前的旧读取覆盖新连接。
- 同一连接合并并行读取。普通 delta 直接替换流式投影，不对每个 token 读取全部历史；每 750ms 的补偿读取先核对聊天、历史和会话版本及忙碌状态。版本未变时不查历史、不复制 journal，也不刷新界面。历史版本由 SQLite 在消息写入、编辑或删除的同一事务内更新。
- 历史或会话选择操作完成后重新读取，操作前发起的旧读取不能覆盖结果，即使 journal revision 未变化。
- 终态快照不向历史追加行，删除/编辑后的旧 finish 不能复活消息。共享 turn_id 的 context 与 tool 信息不会被助手文字投影或逐字动画覆盖。
- 用户选择的临时图片按用户消息 ID 补入仍存在的历史行，供预览和重生成使用，仍受 journal 保留范围及临时上传文件的清理周期限制。自动摄像头帧只随本次模型请求发送，不进入聊天快照或重生成附件。
- 最近至多 256 个完整 turn 快照保留；淘汰后的请求指纹与原结果在本进程保留，避免旧 ID 再执行。该幂等表不跨应用重启，也不是无限持久化队列；长时间运行时结果表会随逻辑请求数量增长。

## C02/C05/C06/C07：真实原生验证

脚本连接实际 Tauri debug 进程的 WebView2 CDP，不是浏览器 mock，也没有隐藏 ChatPanel。配置最初仅创建 pet，main create=false。第一条消息通过真正的桌宠输入框填入并按 Enter 发送；主界面发送也走真实 textarea。事件/历史检查使用真实 IPC 和 SQLite。模型为 scripts/chat-lifecycle-model-fixture.mjs 的受控 OpenAI 兼容 SSE 服务，不能称为真实模型质量测试。

最终 native-report.json 的 15 项均 passed：

1. main WebView 从未存在时，桌宠 UI 完整流式发送、终态、保存。
2. 创建 main 后恢复桌宠历史，不重新调用模型。
3. 真实主输入开始生成；注册表替换 ChatPanel 为 null，确认 textarea 不存在；生成中重新挂载恢复。
4. main 与 pet 同时提交，1 个完成、1 个 chat_turn_busy，只有 1 次生成模型请求。
5. 两个独立 WebView 投递同 ID 同输入，结果一致，只有 1 次生成模型请求。
6. 请求受理前取消：逻辑 ID tombstone 生效，没有模型请求，没有永久 busy。
7. 流式取消产生终态，下一条真实桌宠输入成功。
8. 受控供应商失败产生终态，随后真实桌宠重试成功。
9. 真实 builtin get_time 一次工具执行，2 次模型轮次（工具请求/结果跟进），1 条工具落库记录。
10. 生成中实际 edit/delete/create-conversation IPC 被 busy 拒绝；应用切换会话取消旧请求；旧内容不进入目标；编辑、重生成、删除旧会话、清空恢复均正确。
11. 隐藏 main 后桌宠发送成功。
12. 既有 serialized character activation owner 先读取权威 active turn 并取消；旧请求 cancelled，新角色请求只写入新角色，随后切回。
13. main 发起 slow 请求后，在已有 delta 时 destroy main WebView；同请求仍 completed 并持久化；之后桌宠 UI 再发送成功。
14. main/pet 同时存在及销毁切换期间，12 次被观测自动语音 start 对应 12 个唯一 ID，只有一个物理输出窗口消费每次请求。
15. 受控模型调用计数核对：并发、重复、恢复、隐藏/销毁场景均不重复生成；受理前取消为 0 调用；工具场景恰好 2 模型轮次。

原生验证发现并修复两个真实问题：结构化 KokoroError 被 String 转成 [object Object]（桌宠改用桥接错误提取）；Tauri 默认 Any listener 会接收其他窗口的定向事件（5 种播放事件均改为当前窗口 target，未靠按 ID 去重掩盖双播放）。

首次干净 profile 的开发 StrictMode 还暴露既有模板首次创建竞争和无 committed 角色时的默认选择变化。最终夹具在 main 从未创建前，通过公开后端命令串行建立模板实例与专用手动角色、完成 prepare/commit，再运行聊天检查，模拟已经配置的安装；未隐藏错误 toast、未修改产品首次初始化逻辑。前两次尝试的失败报告和调用记录仍保留。

## C04/C08：自动回归与旧测试迁移

| 旧测试风险 | 新负责位置/仍保留的验证 |
| --- | --- |
| chat-external-turns：外部allowlist/未知请求 | 同文件现为 Node 应用测试；所有后端认可的入口可投影，显示层无权取消未知 sender |
| chat-finish-loss：丢 finish、Promise fallback、迟到 finish | 同文件 + lib/chat/application.test：丢全部事件/迟到终态、预绑定用户 ID、读事件竞态、重连、单次读取与 context/tool 保留 |
| chat-pet-proactive-concurrency：握手拒绝/watchdog/tombstone | 同文件：独立窗口适配器、权威 busy 拒绝、慢准备不被显示层超时取消、按逻辑 ID 停止、旧结果不清新请求；真实 Rust/原生补原子性 |
| chat-edit-session-switch：迟到编辑/删尾/清空 | 同文件：捕获归属、generation、tail guard、mutation busy、迟到删除不清新会话；原生历史场景补真实 IPC |
| STT session-switch | 保留组件测试，覆盖角色/会话切换、录音/唤醒录音、draft 保留、历史修改中不自动发送 |
| Pet/interaction | 入口薄适配测试替换旧握手模拟；重复点击、终态恢复、旧事件隔离、busy/error、卸载不取消仍覆盖 |
| OnboardingChat | 新直接测试：无视图正常完成、dismiss-before-turnId、迟到旧成功不影响新尝试、空/取消/失败不得完成 |
| TTS | 中断/cleanup/替换释放 owner；物理窗口 target；晚到音频不复活；播放 drain 与后台完成分离 |
| 字符显示 | 纯显示 hook 恢复逐字效果；恢复全文即显，后续 delta 动画；context/tool 原样保留 |

提交范围内的测试未使用 skip 或禁用套件取得通过。退役的是产品已移除的 UI 握手/看门狗断言；对应风险迁往实际权威边界并补原生验证。工作区曾包含未纳入本 PR 的独立 Cargo 启动器测试（scripts/tauri.test.mjs，6 项）；此前 87 文件／826 项的工作区记录包含它。PR 准备时显式排除该未提交文件，提交范围的结果为 86 文件／820 项。

| 命令 | 结果 | 日志 |
| --- | --- | --- |
| npm test -- --exclude scripts/tauri.test.mjs | 提交范围：86 文件 / 820 项通过；排除项是未纳入 Git 的独立启动器测试 | .chat-pr-tests.log |
| npx tsc --noEmit | PR 准备时通过 | .chat-pr-tsc.log |
| cargo check --manifest-path src-tauri/Cargo.toml | PR 准备时通过 | .chat-pr-cargo-check.log |
| npm run build | 通过；既有大 chunk 提示 | .chat-recovery-build.log |
| npm run check:ipc | 175 个 invoke 命令均注册 | .chat-recovery-ipc.log；独立复核 .chat-master-ipc.log |
| cargo test --manifest-path src-tauri/Cargo.toml --lib | Clippy 修复后重新运行，955 通过，无忽略 | .chat-clippy-fixed-rust-tests.log；聊天重构验收 .chat-rust-all.log |
| cargo build --manifest-path src-tauri/Cargo.toml（隔离 TAURI_CONFIG） | 通过；既有 LNK4098 warning | .chat-native-build-final.log |
| cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings | Clippy 修复后通过，exit 0，无 lint 豁免 | .chat-clippy-fixed-strict.log |
| 修复前同一严格 Clippy 命令 | 历史失败：29 个 double_must_use | .chat-recovery-clippy.log |
| 修复前严格命令另加 -A clippy::double_must_use | 历史诊断检查，仅用于隔离当时的 lint；不作为当前通过依据 | .chat-recovery-clippy-baseline-exception.log |
| python scripts/audit-chat-lifecycle-db.py | integrity=ok；foreign_key_check 空；指定场景/角色每种消息恰好 1 行 | database-audit.json |

clippy-baseline.json 是 Clippy 修复前的本地历史快照：当时 8 文件的工作区 blob 与基线提交一致，toolchain 为 rustc/clippy 1.99.1。聊天重构中发现的 3 处 needless_borrow 已先行修复；后续 Clippy 补丁修复余下 29 个 trait 方法诊断。该历史快照不代表提交补丁后的文件内容。

### 严格 Clippy 后续修复（2026-11-12）

本地 async-trait 0.1.89 的 src/expand.rs:69 对每个 trait async 方法无条件追加裸 must_use。Clippy 修复前先以相同宏版本和工具链构建独立最小 fixture，证实在声明处添加带说明的 must_use 后，追加属性组合能通过严格 Clippy，且不存在重复属性诊断；再以 deny(unused_must_use) 故意丢弃 Future，确认编译失败并显示具体操作提醒。该预期失败是诊断契约测试，不是项目回归失败。

随后只为 actions/registry、characters/activation、hooks/runtime、imagegen/interface、llm/provider、mcp/transport、stt/interface、tts/interface 中的 29 个 async trait 方法增加属性，例如“await or poll this future to execute the action”。这些说明分别指出未轮询 Future 时不会执行的动作、运行状态变更、请求、转写或合成操作。未加 allow、未放宽命令、未更新依赖，也未改函数签名或函数体；逐文件去除这 29 行后与修改前内容一致。

严格原命令通过后，重跑全部 Rust 库测试，覆盖动作、角色激活、hook、LLM、MCP、STT、TTS 等相关模块。属性调整不改变前端、IPC 契约或运行逻辑；原生场景结论仍来自同日聊天重构验收。PR 准备阶段另外重跑提交范围的前端测试与 TypeScript 检查。最小 fixture 日志为 .chat-clippy-must-use-probe.log 与 .chat-clippy-must-use-diagnostic.log；Clippy 修改、命令结果和哈希保存在本地 clippy-fix-evidence.json。

## C01—C08 判定

| 条目 | 判定 | 证据/限制 |
| --- | --- | --- |
| C01 | 通过 | 职责表、UI-free 模块、Node 测试；App/Panel没有新执行引擎 |
| C02 | 受控环境通过 | 真实 Tauri 15 场景中的无main、替换/重挂载、隐藏、发送方销毁；01—04截图 |
| C03 | 通过（入口代码与相关回归） | 上述入口清单；生产 streamChat 唯一路径；外部 MOD/MCP 服务未配置 |
| C04 | 自动化通过 | revision/终态/重连/丢失/竞态测试及 journal 测试 |
| C05 | 自动化与原生通过 | 并发仅1次模型调用、跨窗口同ID1次、取消各边界/恢复 |
| C06 | 自动化与原生通过 | 历史与角色原生检查、STT组件测试、原Rust history/activation回归；未使用真实麦克风输入 |
| C07 | 已配置能力通过；外部依赖未具备 | 真实SQLite/工具/浏览器TTS路由计数、17次语义模型不可用降级；真实云模型/外部语音/MCP/图像/MOD消费端未验证 |
| C08 | 通过 | 严格Clippy与Rust经修复后验证，提交范围前端测试与类型检查在PR准备时重跑；build/IPC与原生证据来自同日聊天验收；没有lint豁免 |

## 隔离、复跑和证据

scripts/chat-lifecycle.tauri.conf.json 使用独立 identifier com.chyin.kokoro.chat-validation。debug-only KOKORO_TEST_DATA_ROOT 覆盖应用原 dirs_next::data_dir 入口：Windows Known Folder API 不随 APPDATA 环境覆盖，因此需要显式隔离；release 编译仍直接使用原 data_dir 语义。测试用 WebView profile 放在 TEMP，避免 Vite 监视工作区内浏览器缓存导致 EBUSY。未读写用户真实配置、令牌或数据库。

复跑环境为 Windows、PowerShell 7、Node.js、Rust/Tauri 开发依赖及 WebView2。准备可用的 Playwright Node 模块（可使用已安装 @playwright/cli 附带模块）；19333、19334 端口需空闲，1420 端口需空闲或运行本仓库 Vite。随后在 PowerShell 7 运行：

```powershell
./scripts/run-chat-lifecycle-native.ps1 -PlaywrightModule '<绝对路径>/node_modules/playwright'
python scripts/audit-chat-lifecycle-db.py
```

launcher 默认重新编译隔离 debug app、创建全新数据根/profile、启动受控模型和必要的 Vite，执行后关闭自建进程；-SkipBuild 只在已有同隔离配置构建时使用；-KeepRunning 留给人工复核。脚本只终止 PID 文件对应且路径/命令匹配的验证进程。所有生成数据和截图位于忽略目录 output/playwright/chat-lifecycle。

以下是本地验收生成的证据，不纳入 Git。launcher 与数据库审计脚本可重新生成 native-report.json、controlled-model-calls.json、database-audit.json、run-info.json、executable.json、截图和原生日志。summary.json、Clippy 历史/修复记录及哈希清单是验收时额外整理的记录，不是 launcher 的自动输出：

- native-report.json：15 项结果、实际 request/turn/character/conversation ID、语音窗口事件、取消错误与最终结果。
- controlled-model-calls.json：全部受控模型请求（streaming 与系统辅助请求分列）。
- database-audit.json：只读 SQLite 完整性、外键、case/角色分组与消息行。
- summary.json：15 passed；12 个 TTS start / 12 个唯一 ID；17 次 semantic unavailable 日志。
- run-info.json / executable.json / evidence-sha256.json：隔离路径、验证进程、二进制与证据哈希。
- 01-pet-only.png、02-main-restored.png、03-remounted.png、04-no-main.png。
- app.stdout.log、native-check.log 与根目录 .chat-* 检查日志；clippy-baseline.json 保留修复前状态，clippy-fix-evidence.json 记录当前严格通过及 29 行属性修改。
- docs-before-clippy-fix.md、evidence-sha256-before-clippy-fix.json 保留上一轮文档和证据清单；当前 evidence-sha256.json 包含更新后的报告与本轮证据。
