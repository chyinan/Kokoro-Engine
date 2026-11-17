# 后端状态所有权与请求身份

本次工作最初从本地 `main`（`118c5fc75de7b038ec25d3f7f59fb1d80a5ada68`）建立 `codex/backend-state-ownership`。沿用 SQLx、已有聊天应用入口、原子保存、角色激活协调器和 `ChatRuntimeState`；没有引入状态管理框架或新的持久化系统。

## 不变量与身份路径

`Desktop / Pet / Mod / proactive / Telegram / Webhook / Discord → submit_chat → resolve_request_scope → TurnIdentity → execute_chat_with_pending → persistence / tools / post_commit / events`。

首次桌面请求允许兼容输入省略当前角色、会话或 request ID，但只在 `resolve_request_scope` 解析一次。角色必须存在；已有会话必须属于该角色；带选择世代的桌面输入必须仍属于其点击时的选择。平台请求必须先通过渠道授权并携带角色、会话及 request ID。核心要求已解析的 `ConversationTarget` 和非空 `TurnIdentity`，没有“缺失时读取当前选择”的补全路径。

空白桌面和 hidden 输入提前预留 UUID。预留不是插入数据库：首次用户保存失败仍原子回滚；hidden 无输出不创建空会话。内部可空的 `conversation_id` 表示会话是否已经物化，工具、审批、hook、事件和返回值使用不可变身份。请求 journal 从 admission 起保存预留 ID；前端只在原来的空白选择世代显示该轮次。会话存在后按角色与会话匹配，A→B→A 可以恢复 A 的流式内容、审批和 Stop。

身份解析在校验 SQL owner 的同一个短 selection 锁内捕获 `MemoryWriteScope`，其中包含角色/会话的运行期生命周期 token。删除或恢复先撤销旧 token；同 ID 重建得到新 token，旧任务仍然失效。该 scope 随原请求、保存恢复、工具、摘要和后台记忆工作传递，不在异步任务启动时重新按 ID 绑定。

相同 desktop request 的重投先校验原始输入指纹并恢复原结果，不能因选择已切换而重新解析。不同输入复用 request ID 仍返回冲突。外部持久化重投继续使用原用户行和既有提交/媒体恢复协议。

## 可变字段归属清单

| 状态/字段 | 所属范围与业务语义 | 权威来源、修改入口 | 生命周期与回收 |
|---|---|---|---|
| `AIOrchestrator.db`、`memory_manager`、`router` | 应用共享服务 | 初始化装配；SQLx 已提交记录为业务数据权威 | 应用退出释放；导入使用已有数据库操作世代 |
| `LlmService.providers/active_provider_id/config/state_snapshot` | 应用 Provider 配置及凭据 | `update_config` 在同一提交保护下更新；执行保留 Provider Arc 和参数 | 应用生命周期；旧轮次及其必要异步工作结束后释放，凭据不进入会话记录/事件 |
| `TtsService.providers/default_provider/committed_config/snapshot_gate/voice_registry/cache` | 应用语音服务、配置和缓存 | `reload_from_config` 先构建，再原子替换；`execution_snapshot` 同步读取 provider 与角色语音参数 | 旧 provider 由在途快照及最多 256 个近期 journal outcome 保留；进入 retired 墓碑时去掉语音快照；缓存沿用已有容量/TTL |
| `TtsService.requests`、播放 owner | 应用唯一音频输出设备协调；每个合成请求有独立取消信号 | `register_request/cancel_request/publish_playback_start`，chat journal 保存播放请求归属 | 合成结束释放；取消 tombstone 有容量限制 |
| `system_prompt/character_name/character_id/response_language` | 当前已提交角色配置的应用执行投影；不是会话身份权威 | 角色激活协调器 `prepare/apply/commit`；非当前角色从该角色 SQL 配置构造同规则快照 | 激活替换投影；在途快照不变；无语言覆盖时为 auto，不继承上一角色语言 |
| `user_name/user_language/jailbreak_prompt` | 应用用户资料、显示语言和全局提示词设置 | 保留的应用设置 IPC 持 activation 写保护；用户资料先持久化，执行读取快照 | 应用设置生命周期；每轮固定读取结果 |
| `proactive_enabled` | 已激活角色的主动行为设置投影 | 角色配置提交及现有设置入口 | 激活切换；原轮次请求不受之后的选择影响 |
| `context_strategy/max_message_chars/vision_context_history_mode/max_history_tokens` | 应用上下文策略与窗口预算 | 设置服务；单轮 prompt/历史快照固定使用读取值 | 应用生命周期；无跨会话历史对象 |
| `memory_enabled`、ExternalMemory `epoch/database_generation` | 应用即时记忆授权与撤销世代 | `set_memory_enabled`；每次切换（包括关后再开）递增 epoch，恢复数据库更换 generation | 每次记忆实际 SQL 提交复查，旧任务不能因重新开启恢复写权限 |
| `runtime_degraded/activation_gate` | 应用已提交角色视图的一致性协调 | 激活/恢复 owner；失败进入 degraded，成功完整提交才清除 | 不持 activation 锁等待模型、工具、媒体或语音 |
| `chat_turn_lock/chat_admission` | 所有渠道共同的执行许可与麦克风准入 | `submit_chat` 统一获取；取消/异常靠 RAII 释放 | 当前实现保留全应用单轮串行策略，同一会话绝不能从另一个渠道绕过；没有强制不同会话并行 |
| `DesktopSelection.current_conversation_id/conversation_generation/conversation_switch_lock` | 界面选择 | load/create/clear/activation 等边界；generation 只用于陈旧入口与空白视图 | 切换不重置会话进度，不取消已启动轮次 |
| `DesktopSelection.history/memory_history_boundary/persist_conversation_selection` | 可丢弃桌面历史投影及旧读取兼容缓存 | load/同步从 SQL 重建；本会话成功提交才更新；edit/delete/restore 重建或失效 | 最多 20 条且限制单条字符；切换可丢弃；执行历史和记忆窗口不以此为权威 |
| `RuntimeOwners` owner 的 `alive` token | 角色和会话对象本次运行生命周期 | 准入取得；`forget_character/forget_conversation/reset_runtime_owners` 撤销后移除 | Arc 可被在途 scope 保留，但旧 token 永不复活；相同 ID 重建使用新 token |
| `RuntimeOwners.characters[character].last_activity` | 角色活动计时 | 明确 owner 的用户活动/提交与主动触发更新 | 切换保留；删除角色/数据库恢复移除；进程重启重建 |
| `CharacterBehavior.conversation_count` | 角色累计已提交用户互动数（关系深度近似值） | 仅 `save_messages_for_origin_locked` 对新提交 user 行增长；所有渠道共享该角色关系 | hidden/主动触发/失败保存/重投不增长；进程内计数，重启从零开始 |
| `CharacterBehavior.curiosity/initiative/idle_behaviors/last_proactive/last_prune/last_dream_day` | 角色好奇、主动行为、动画去重和周期任务冷却 | heartbeat 每次先绑定角色与选择，再修改该 owner；日 Dream 额外查询数据库当天任务 | 角色切换不重置；角色删除/恢复回收；冷却是运行期状态，不承诺重启恢复计时 |
| `RuntimeOwners.conversations[(character,conversation)].message_count/memory_count/memory_epoch/first_memory_message` | 会话桌面用户提交次数、记忆窗口进度 | 保存成功且 `row.changed` 才增长；epoch 改变后下次提交重置记忆窗口，不重置总计数 | 切换/临时引用不丢失；会话/角色删除与恢复清理；应用退出释放 |
| `ExternalMemory.conversations` 的 `completed/first_message_id/through_message_id` | 会话内外部渠道入口的记忆节奏 | 外部 assistant 原子提交后更新 ticket；桌面与外部入口窗口分开，但共同以该会话 SQL 为依据 | /new 使用新 key；toggle/restore 清空；角色/会话删除定向清理 |
| `memory_event_cooldowns` | 按角色、会话和事件类型的记忆节流 | `should_trigger_memory_event` 唯一写入口 | 清理过期 key，最多 2048 项；角色/会话删除移除对应项 |
| 消息、conversation title/topic/pinned state/history revision | 会话 | 已提交 SQL；统一 `persistence::save`，预期 revision 防止过期重试 | 删除外键级联；迟到保存重新校验 owner 和会话存在性，禁止创建已删已有会话 |
| 摘要与 summary task 状态 | 会话 | SQL summaries 表；任务捕获 owner、request、Provider、语言与记忆世代 | 阈值/版本沿用原规则；模型返回后 scope 检查；会话删除 FK 级联，迟到 UPDATE 不能重建 |
| 记忆、候选、Dream proposal/job | 角色数据；聊天触发任务同时受发起会话存活约束 | `MemoryWriteScope` 在 embedding/model 之后、SQL 之前检查；role daily Dream 明确使用角色 scope | 删除沿用 `CHARACTER_OWNED_TABLES`；关闭再开启不能允许旧任务写入 |
| `external_replies` | 应用协调的会话/request 回复提交恢复缓存 | 保留已提交 user 锚点、pending batch、原语音/记忆 Provider、语言与生命周期 scope；只有原子 commit 后发布结果 | 最多 32 项且 payload 总量 16 MiB；成功、删除会话/角色、数据库恢复时回收；SQL 不存 Provider/凭据 |
| `ChatRuntimeState.entries/retired/abandoned_drafts/revision` | 应用保存的单轮 journal、幂等墓碑与视图修订 | 应用执行 owner 更新，界面只订阅 | 256 个近期完整条目；pending payload 总量 16 MiB；旧 outcome/指纹墓碑保持进程内幂等；数据库恢复整体失效 |
| `ChatRequest/PreparedChatTurn/PromptSnapshot/TurnIdentity` | 单轮输入、不可变配置和已绑定身份 | admission 建立；core 不改角色/会话/request 身份 | 结束释放；保存失败只保留重试所需有界数据 |
| `TurnCancellationState`、审批、工具轮次、delta/草稿/媒体 | 单轮 | request/turn ID 定位，工具分派前检查原 owner 生命周期，审批后重新检查权限；tool `ActionContext` 包含角色/会话/request/turn ID | 终态清理取消注册与审批；独立请求的取消不会串线；迟到事件由终态规则拒绝 |
| `DesktopPostCommit/MemoryWriteScope/TtsExecutionSnapshot` | 单轮派生异步工作 | 捕获 Provider Arc、语言、语音与原身份；记忆即时授权在提交时检查 | UI 切换不丢弃工作；删除/撤销/数据库世代使写入失败或跳过；快照不持久化凭据；语音快照随近期 journal 有界保留，淘汰后不再为旧结果重新合成语音 |
| Telegram `Sessions.modes/deliveries` | 平台来源、chat、forum thread、角色绑定及回复交付进度 | `/new` 轮换对应绑定；`/continue` 一次绑定桌面明确角色+会话；实际 `submit_message` 使用绑定 | 运行期映射；重启按平台+chat+thread+角色确定性重建私有默认映射，绝不读取桌面默认；已提交重投按原 request SQL 恢复。`/new`、`/continue` 的临时覆盖不会跨进程恢复 |

## 修改入口与配置一致性

手动逐字段复制的 `fork_with_isolated_history` 已删除。生产代码不克隆 `AIOrchestrator` 来隔离执行；外部请求不写桌面的角色或会话选择。测试中的共享服务 clone 使用 Rust 派生 clone，验证共享服务引用，不再模拟会话对象。

无生产调用的 `set_persona/set_character_name/set_active_character_id/set_response_language` 部分角色 setter 保留 IPC 名称作明确错误兼容，桥接标记 deprecated；调用方须迁移角色覆盖项与激活流程后才可移除。这避免只改角色 ID 而沿用另一角色 persona/TTS。`set_user_name/set_user_language/set_jailbreak_prompt` 是应用设置，使用 activation 写保护避免快照撕裂。

提示词、模型、工具循环、Live2D cue 白名单和交付语音使用同一轮保留的配置快照；后续配置修改影响下一轮。Browser 同样捕获 Provider/voice/speed/pitch，无 Rust 注册时固定使用 BrowserDelegate；已配置但不可用的 Provider 也记录为本轮不可用快照，后来注册不会使旧轮次改用新 Provider，文本仍提交并报告 speech error。聊天完成阶段不再重新路由当前全局语音服务，独立手动 TTS 入口保留现有路由。需要即时生效的工具授权仍在实际执行/审批后复查，记忆授权在最终写入复查。凭据由应用服务管理，只有内存 Arc 被保留。外部回复保存失败后恢复仍使用首次执行的语音和记忆快照；跨进程或缓存淘汰后，已完成请求从 SQL 重投只返回原文本/媒体，不重新触发语音或记忆工作。

## 并发与锁顺序

1. 入口取得数据库操作读许可，统一聊天执行许可；备份恢复通过既有数据库操作排它机制协调。
2. 身份解析/配置读取短暂使用 activation read → desktop selection；角色激活持 activation write，经 prepare/apply/commit 发布一致视图。
3. 保存持短暂 selection lock → SQL 事务 → 单会话/角色进度锁。模型、工具、下载、TTS 不在 selection/SQL 事务/activation 写锁内执行。
4. 记忆提交取得数据库操作许可 → `memory_commit` read → 最终 SQL；删除或 toggle 使用 `memory_commit` write，删除角色或会话随后取得 selection。记忆服务不在持 read guard 时等待模型或 embedding。
5. 运行期 maps 的 std Mutex 只用于查找/删除 Arc，不跨 await。许可覆盖整轮是有意串行策略，渠道和 clone 都共享同一许可。

## 行为证据与验收追踪

`chat/ownership_test.rs` 在已有真实 Tauri 测试 app 上调用 production command/submit 入口；ControlledProvider 的 `Notify` 控制模型暂停。切换通过真实 `load_conversation` 和角色 `prepare/commit`；第二次工具模型请求验证原 persona/provider，工具 ActionContext 与实际 emit payload 验证原角色/会话/request/turn。

| 成功条件 | 状态 | 关键行为证据 |
|---|---|---|
| 1 状态归属 | 通过（本地验证范围） | 上表；`verify_state_ownership` 检查真实提交后的角色/会话计数 |
| 2 必需身份 | 通过（本地验证范围） | `desktop_webhook_and_telegram_execute_the_same_bound_chat`：边界拒绝、hidden 预留、失败原子性、replay |
| 3 稳定身份 | 通过（本地验证范围） | `verify_state_ownership`：第二轮工具请求及实际 delta/tool/typing/finish；`verify_cue_snapshot` |
| 4 选择分离 | 通过（本地验证范围） | `application.test.ts` 的 A→B→A/默认 Stop；两个 chat-edit switch 文件；真实 native 选择场景 |
| 5 修改入口 | 通过（本地验证范围） | `persistence` 原子保存/提交计数测试；SQL 重建；废弃分片角色 setter 明确报错 |
| 6 删除 fork | 通过（本地验证范围） | 无生产 fork API；`managed_external_turns_preserve_desktop_and_external_memory_cadence_and_transcripts` |
| 7 配置视图 | 通过（本地验证范围） | `verify_pending_reply_snapshots` 实际旧 Provider 记忆/语音；TTS 同 ID 替换；`verify_cue_snapshot` 及实际前端 cue listener |
| 8 平台绑定 | 通过（本地验证范围） | `verify_bound_session_submission`；`topic_delivery_payloads_keep_the_origin_thread`；Telegram new/continue tests |
| 9 并发范围 | 通过（本地验证范围） | `verify_state_ownership` 的 Desktop+Telegram 同会话 busy、两个 Bot 交错、取消隔离；`verify_deleted_approval` 的删除后批准无副作用与记忆关闭时普通工具正向控制 |
| 10 删除清理 | 通过（本地验证范围） | `memory_commit_rechecks_ownership_and_revocation_after_provider_response`、`queued_scope_keeps_revoked_owner_before_worker_starts`；真实入口同 ID 重建后普通/流错误/工具结果全部零消息写入；external pending 清理 |
| 11 行为测试 | 通过（本地验证范围） | 上述 Notify 暂停点、SQL/事件/实际合成断言；native 继续覆盖完整 IPC/WebView/提交恢复 |
| 12 隐式状态减少 | 通过（本地验证范围） | 本文状态表、身份路径、兼容条件；删除 fork、全局业务计数和执行期当前选择补全 |

首次验收记录（2026-11-17，旧基线 `118c5fc`；主代理独立验收通过，后续 rebase 验证见文末）：

- `npm test`：88 文件、953/953；`output/state-ownership/npm-snapshot-final.log`。
- `npm run build`：通过；`build-snapshot-final.log`，主代理独立 `master-build-ownership-final.log`。保留已有 bundle 体积/动态导入提示。
- `npm run check:ipc`：177 个注册命令检查通过；主代理 `master-ipc-ownership-final.log`。
- `cargo clippy --offline --manifest-path src-tauri/Cargo.toml --lib -- -D warnings`：通过；`clippy-final-5.log`。
- `cargo test --offline --manifest-path src-tauri/Cargo.toml`：1000/1000，bin/doc 0；`rust-final-5.log`，无后台 panic。
- `pwsh -NoProfile -File scripts/run-chat-lifecycle-native.ps1 -PlaywrightModule "C:\Users\19254\AppData\Local\npm-cache\_npx\31e32ef8478fbf80\node_modules\playwright"`：主代理从最终源码重建后验收 **39/39**；`master-native-lifecycle-final.log`、`output/state-ownership/pre-rebase-native/lifecycle-report.json`。
- `pwsh -NoProfile -File scripts/run-chat-lifecycle-native.ps1 -PlaywrightModule "C:\Users\19254\AppData\Local\npm-cache\_npx\31e32ef8478fbf80\node_modules\playwright" -Persistence -SkipBuild`：同一最终二进制 **17/17**，恢复检查 `status=passed`；`master-native-persistence-final.log`、`output/state-ownership/pre-rebase-native/persistence-report.json`、同目录 `recovery-report.json`。覆盖真实 WebView/IPC/SQLite 的提交故障、hidden 保存恢复、媒体恢复、第五/二十轮记忆与巩固、撤销 epoch、切换后原 owner 后台任务。

Rust 使用 `ORT_SKIP_DOWNLOAD=1` 与已有本地 ONNX DLL，以及 `CARGO_TARGET_DIR=E:\Kokoro-Engine\src-tauri\target` 缓存；源码始终来自本工作树。未纳入 DLL、node_modules junction 或 output 产物。MSVC 链接器可能输出已有 LNK4098 警告，与严格 lib clippy 检查分开记录。

首次验收原生二进制 SHA-256：`3E58AC86CC3A0F23B1D423D29BC4E9352EC550D28BFD09F53A7800E3302704EB`，记录在 `output/state-ownership/pre-rebase-native/lifecycle-executable.json`；主代理另存 `output/state-ownership/pre-rebase-native/source-manifest.json`，最终复算 47 个源文件 changed=0。两个原生套件均 exit 0，端口 1420/19333/19334/19335/19336 无监听残留。阶段/失败日志留在 `output/state-ownership/` 供诊断，不作为最终通过依据。`git diff --check` exit 0；依赖、DLL、运行数据和测试产物均未纳入交付。


## 未验证边界与有意保留

- 真实 Telegram Bot API、Discord/QQ/AstrBot 网络、云模型/图像/TTS 服务未连接。本地测试证明适配器绑定、请求参数、可控模型交错及 SQLite/事件归属，不能冒充远程平台端到端测试。
- 此次 Windows Rust/Tauri/WebView2 验收不能代表 Linux/macOS 的窗口与音频行为。
- 保留全应用单轮执行许可。因此两个会话交错的可观察策略是第二个入口明确 busy，第一轮结束后可重试；同一会话从多渠道也遵守同一规则。没有声称不同会话已经并行。
- 平台 `/new`、`/continue` 的会话覆盖是当前进程内状态，重启回到确定性私有映射；原会话/消息仍在 SQLite，可显式再次 continue。不会随机采用桌面会话。
- 角色关系计数、活动冷却、会话记忆节奏为进程内业务运行态，保留跨选择/跨请求连续性，重启重建。摘要和消息进度以 SQLite 为准，持久化行为沿用现有模型。
- `ChatRuntimeState.retired` 是既有进程内幂等墓碑，不是新的持久化事件系统；新增语音 Provider 快照仅保留在最多 256 个近期 outcome，淘汰时释放，墓碑不保留该 Arc。需跨重启的外部重试继续依赖数据库已提交用户/回复行。
- 已移除旧 `#[cfg(test)] ensure_conversation_created_for_hidden_turn` 及 7 个只测试该镜像构造器的测试。它们曾断言“切换取消旧任务”，与新要求冲突且并非生产路径。真实 Tauri channels 测试继续覆盖 hidden 提前预留、审批、取消、no-op、提交失败恢复和重复投递。

原生脚本同步了本次选择语义，并修正了已有验收假设：工具已派发后的失败仍保持原有不可重放规则；另以没有派发工具的 partial SSE 错误验证可重试提示、部分回复清理和用户锚点。准备暂停持有 selection lock 时，使用发送前订阅的 `chat-runtime-changed` 记录 accepted 身份，直接查 SQLite 验证预留未物化，不用会等待该锁的 snapshot IPC 代替同步信号。生产工具重放策略没有改变。


## Rebase 到最新 fork/main（2026-11-17）

原验收改动已单独提交为 `f8edaaff960bda310bf778d4bb6f5196bd060fa2`，精确包含 49 个任务文件，提交前 47 个源码文件与首次验收清单相符。随后 `git fetch fork main`，将分支从旧基线 `118c5fc75de7b038ec25d3f7f59fb1d80a5ada68` rebase 到 `e3180e860bbee1cb07d417df1ed68146fceb649b`。重放后的实现提交为 `d7add8039f91676665bb1bdcfa0791ea3489b86c`，其父提交和 merge-base 均为新 fork/main。

冲突与兼容处理：

- `chat/application.rs`：保留首次回复的语音/记忆快照、生命周期 scope 与同一保存提交入口，同时保留新基线 assistant 查询的 `CASE WHEN json_valid(metadata)` 保护、外部错误来源元数据和 `require_saved_external_reply` 防止淘汰后的旧请求重新生成。
- `src/lib/chat/projection.ts`：保留新基线 hidden 保存失败的按钮与可见回复，筛选继续使用稳定角色/会话身份；仅空白预留会话依赖 admission generation。对应测试保留指令不泄露、保存不重发、不同会话隔离等断言。
- 自动合并后的 `chat/runtime.rs` 按新 `RetiredOutcome` 显式处理桌面终态，拒绝跨渠道复用；外部已完成回复淘汰后只保留指纹并受 SQL 重放保护，保留首次 busy 拒绝与导入失效规则；旧语音快照仍在淘汰时释放。
- Telegram 保留新基线入口 allowlist 校验、精确带符号 chat ID、逐消息配置更新及全部拒绝路径测试，迁移测试中的旧 fork/history 访问；平台 thread/session 绑定和回复 thread_id 继续生效。
- 上游新增 `verify_external_failure_memory_origin` 保留外部诊断来源、不可重放、300 条诊断在 SQL LIMIT 前过滤的断言。其最后一段改为验证“人工污染 UI 历史投影不覆盖已提交 SQL 记忆窗口”，与本次权威来源约束一致。

新基线检查：

- `cargo check --offline --manifest-path src-tauri/Cargo.toml --tests`：通过；`output/state-ownership/rebase-check-1.log`。
- `npm test`：89 文件、960/960；`rebase-npm-test-1.log`。
- `npm run build`：通过；`rebase-build-1.log`，仅已有 bundle/import 提示。
- `npm run check:ipc`：177 个命令通过；`rebase-ipc-1.log`。
- `cargo test --offline --manifest-path src-tauri/Cargo.toml`：1007/1007，bin/doc 0，无后台 panic；`rebase-rust-all-2.log`。
- `cargo clippy --offline --manifest-path src-tauri/Cargo.toml --lib -- -D warnings`：通过；`rebase-clippy-1.log`。
- 主代理重新运行上述 lifecycle 命令，从提交 `d7add80` 重建后 **39/39**，进程 exit 0；`master-rebase-native-lifecycle.log`、`output/playwright/chat-lifecycle/native-report.json`。
- 主代理对同一新二进制运行上述 `-Persistence -SkipBuild` 命令：**17/17**，恢复检查 `status=passed`，进程 exit 0；`master-rebase-native-persistence.log`、`output/playwright/chat-lifecycle/persistence/persistence-report.json`、同目录 `recovery-report.json`。
- 两个套件的二进制 SHA-256 均为 `DA322CBFD54B3387CCE7FE168EBB780F1DF57D36396DF7572CEEA57207FEFF8E`；`output/state-ownership/source-manifest-rebase.json` 中 47 个源文件最终复算 changed=0。端口 1420/19333/19334/19335/19336 无监听残留。

本次 rebase 后的全量测试、严格检查和原生验收全部通过，表中 12 项本地行为保证在新基线继续成立。仅提交本任务源码和文档，`git diff --check` 通过；运行产物、依赖、数据库和本机配置未纳入提交。

首次验收报告与二进制信息已复制到 `output/state-ownership/pre-rebase-native/`，绑定旧源码清单和原二进制；不能用它们代替本次 rebase 后的原生验收。真实平台网络、云 Provider 和非 Windows 平台的未验证范围继续适用。


## PR #4 修复：保留空白视图的准入代次（2026-11-17）

隐藏/主动轮次在空白视图准入后，若用户在快照准备、模型响应或保存重试前点击“新建会话”，后续 bound/scope/保存恢复曾重新采用最新桌面选择代次，使旧轮次投影到新的空白视图。现在保留原准入代次，只有本轮成功提交并实际选中其新会话时才发布更新代次；没有切换的首次可见、隐藏回复及隐藏保存恢复继续正确选中自己的已提交会话。

`verify_admission_selection_generation` 通过已有 prompt mutex 和 Provider Notify 控制真实入口的执行顺序，并用 SQLite trigger 注入回复保存失败。7 个场景检查实际 bound/start/finish 事件、journal 代次、SQL 会话所属角色与消息行、最终选择和保存恢复不重复调用模型。新增回归在修复前稳定失败：原准入代次 1 被 bound 改为 2；红灯日志为 `output/state-ownership/pr4-generation-red.log`。

测试 fixture 显式关闭桌面选择文件持久化；本轮修复后 Rust 命令还设置 `KOKORO_TEST_DATA_ROOT` 到本工作树忽略的 `output/state-ownership/pr4-isolated-data`，不会写入用户桌面选择文件。

本轮验证（日志均在 `output/state-ownership/`）：

- 定向 `cargo test --offline --manifest-path src-tauri/Cargo.toml chat::channels_test::desktop_webhook_and_telegram_execute_the_same_bound_chat -- --nocapture`：通过，包含新增 7 场景；`pr4-generation-green.log`。
- `npm exec -- vitest run src/lib/chat/application.test.ts`：61/61；`pr4-projection.log`。前端代码未修改。
- `cargo test --offline --manifest-path src-tauri/Cargo.toml`：1007/1007，bin/doc 0，无后台 panic；`pr4-rust-all.log`。
- `cargo clippy --offline --manifest-path src-tauri/Cargo.toml --lib -- -D warnings`：通过；`pr4-clippy.log`。
- 三个变更 Rust 文件的 `rustfmt --check --edition 2021` 与 `git diff --check`：通过。

本轮未重跑原生 WebView 套件；上文 rebase 原生结果仍绑定原实现提交和二进制，不作为本轮补丁的原生验证。
