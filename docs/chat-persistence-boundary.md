# 聊天消息的持久化边界

聊天完成表示本轮需要保存的消息已经提交到 SQLite。模型流结束后可以显示生成的文字，草稿也可以随流式进度保存。只有最终写入事务提交后，才会发送 `chat-turn-finish` 的 `completed` 状态或向 Bot 调用方交付正常回复。无需保存助手文本的分支在完成各自应有的写入后可结束。

| 入口 | 生成前的提交 | 生成后的提交 | 完成反馈 |
| --- | --- | --- | --- |
| 桌面面板、桌宠、Mod 及主动聊天共用的 `submit_chat` | 普通发送将用户消息、选中的视觉上下文和新会话（如需）一起提交。重生成引用已提交的用户行。隐藏指令不写用户行。 | 在原角色和会话中写入最终助手文本，或将原草稿转成最终记录。应保留的原生工具调用及结果与最终消息同批提交。 | SQLite 提交之后才有 `completed` 事件、终态快照和 IPC 成功结果。 |
| 通用 Bot（Webhook、LINE、Discord、QQ） | 用角色和平台会话定位用户行。新会话与第一条用户消息一起提交。 | 使用同一会话提交助手文本。 | 回复对象、平台消息或 HTTP 成功响应在提交之后生成。通用 Webhook 返回失败 HTTP 状态和 `CHAT_PERSISTENCE_ERROR`。 |
| Telegram 文本、语音转录、照片说明 | 确定角色和会话后提交用户文本、转录结果或照片说明。 | 保存到本轮刚绑定的会话。 | 提交成功才发送正常 Telegram 回复。保存失败时返回错误并发送失败提示，同平台消息重投可恢复保存。 |

写入使用短 SQLx 事务，包括会话归属校验、消息写入及必要的会话标题和更新时间更新。会话选择、内存历史和计数在提交后更新。模型请求、工具操作、语音合成和播放、记忆提取、embedding 都在数据库事务外运行。文本已经提交后，语音错误单独报告。

后端用 `ChatPersistence` 表达持久化失败。桌面应用运行状态与错误事件还包含 `CHAT_PERSISTENCE_ERROR`。数据库不可写时，面板仍能显示保存失败，已流出的文本显示为未完成，不需要先写入诊断行。用户消息写入失败时不进入模型或工具流程，原输入留在编辑器中。

对于会写入用户消息的桌面请求，Mod 的 `BeforeUserMessage` 回调现在在用户行提交后、调用模型前触发。这样用户行写入失败时也不会提前触发 Mod 回调的副作用。

数据库保留已接受的用户原文和生成后的助手文本。上下文窗口可按预算裁剪模型看到的副本；未完成草稿不会进入模型历史或新生成的会话摘要。超过 256 KiB 的用户文本会收到校验错误。本次没有迁移已有消息内容。

SQL 写入对短期锁竞争采用有限次数重试。同一保存使用原有请求、会话与消息身份。桌面面板的“重试保存”复用已准备好的最终写入批次。通用 Webhook 使用同一 `request_id` 重投可重试保存，Telegram 使用平台消息 ID。重试最终写入不会重新生成或执行工具。会话内容发生改动时，重试会校验版本并拒绝提交过期的回复。等待重试的内容保存在进程内，受缓存容量限制，进程重启或导入备份会使其失效。

外部入口的待保存回复同时保留原生工具行、最终助手元数据、工具已生成图片的 base64 内容及输出提示，全部计入缓存容量。最终提交将媒体写入助手消息元数据；保存重试及后续同 ID 重投均可恢复，不依赖临时图片文件。新图片提示经共享工具策略生成，兼容旧记录中的 `[IMAGE_PROMPT]` 提示词。桌面从较早消息继续、删除后续提示时，不会从运行日志重新显示已删除的保存失败回复。

SQLite 保留完整助手文本；正常交付、保存恢复和重启重投共用显示格式处理，重投不会改变回复中的强调或换行表现。外部用户来源分类兼容旧保存身份、旧 `source` 元数据和新统一入口指纹；SQL 历史与内存回退使用相同规则。Telegram 显式共享桌面会话时，外部记忆保留其用户输入，桌面记忆仍排除外部轮次。

Telegram 默认按角色和聊天 ID 使用独立会话，`/new` 轮换该聊天的新会话身份，首次消息提交时落库。`/continue` 显式固定当时桌面角色和会话；之后桌面切换不会改变该绑定。后续文本、照片说明和语音转录共用新会话，其他聊天及桌面选择不受影响；旧平台消息重投仍绑定原会话。

Telegram 语音和照片重投先按角色、聊天 ID 和平台消息 ID 读取已提交的用户原文。已有用户行时复用首次转录或照片说明，不再依赖 STT、音频下载或图片下载，随后恢复待保存回复或重发已提交回复；语音在同一个角色激活保护范围内完成身份解析、输入恢复和后续聊天处理。新请求仍须先成功下载或转录才写用户行，因此首次媒体处理失败不会占用保存身份。真实 SQLite 与媒体替身回归覆盖转录标点/空白漂移、STT/下载不可用、助手保存失败后恢复、重复投递、`/new` 后原会话归属及角色/聊天/消息隔离；真实 Telegram 网络和 STT 服务未在这些回归中调用。

通用 Webhook 音频、LINE 与 Discord 媒体入口也在下载或转录前绑定角色、会话和请求身份，并持有角色激活及数据库操作保护。已提交输入直接复用，普通文字请求仍校验完整原文；新媒体用户行还保留调用方原始文字，拒绝同 ID 改文字。兼容旧记录时只接受空原始文字、与已存原文一致的文字或原转录前缀。首次媒体准备失败不占用请求身份；Discord 不再提交附件准备失败后的残缺输入。同请求恢复不依赖媒体服务再次成功。

Telegram 正常回复和首次保存恢复共用交付流程：提交成功后发送文字及翻译，再按配置尝试语音和已保存的图片。已成功请求同 ID 重投只重发保存的文字，不重跑模型、工具或已尝试的媒体生成。当前轮询实例最多保留 32 个请求的交付进度，文字发送失败后的重投可继续尚未开始的媒体步骤。媒体仍为尽力交付，每个步骤在调用前标记为已尝试，避免外部结果不确定时重复生成；媒体失败不会回滚文字，重投不会自动重试该媒体。进程/轮询重启或进度淘汰后，已有 SQLite 回复仅重发文字。

外部 Bot 的周期记忆按角色和会话累计已提交的完整轮次，每 5 轮提取、每 20 轮整合，不依赖临时请求实例或桌面计数。用户或助手写入失败不计入完成轮次；助手保存重试首次成功后计入一次，后续同 ID 重投不重复触发。计数保留在当前进程内，新会话独立起算，切换记忆开关会重置计数与提取历史边界。提取器读取该会话已提交的数据库历史，并排除开关边界之前的消息及其晚到的助手回复。

桌面与外部入口使用独立的周期计数和提取历史。共享保存边界显式区分写入来源；外部用户行不会推进桌面的计数，即使 Telegram 经 `/continue` 显式共享桌面会话，或同角色 LINE/Discord 使用桌面的 orchestrator。桌面周期计数仍由已提交的桌面用户行推进，并记录当前选择世代下的首条消息作为提取边界；记忆开关与历史重载会重置该边界。双方都在数据库窗口裁剪前过滤自身入口的消息，避免另一入口的大量消息挤掉尚待提取的轮次。数据库不可用时，桌面提取可退回现有的有界内存历史，普通聊天不依赖记忆提取成功。共享 Telegram 会话的模型上下文和界面历史仍包含双方消息。真实 SQLite 回归覆盖单一 orchestrator 中的“桌面 4 轮→外部 1 轮→桌面第 5 轮”、双方 5/20 周期、保存失败/重投、记忆开关边界，以及超过共享内存窗口的双向稀疏交错。

桌面最终保存重试与正常提交共用提交后处理，首次成功提交后调度事件记忆、5 轮提取、20 轮整合及原本允许的背景图片分析。重试保留原系统模型、语言、历史快照、计数及会话/数据库/记忆开关世代；切换角色或会话、关闭后重开记忆不会让旧任务借用新状态。重复重试不重复调度，也不重跑原聊天生成或工具。各记忆任务与背景分析独立异步执行。最终待保存批次及其提交后上下文合计最多 16 MiB，并受既有 256 个运行记录的淘汰限制；超限内容不再提供保存重试。该恢复保证限于尚未调度的任务，已经开始的记忆模型调用沿用现有生命周期。

首次用户行明确写入失败时，IPC 调用方可用同一请求 ID 调用 `retry_chat_persistence`。确认选择和会话版本仍匹配后，会复用已准备好的用户消息，提交成功才第一次调用模型。面板也可手动重新发送保留的原稿。

Windows 原生故障测试可通过 `scripts/run-chat-lifecycle-native.ps1 -Persistence -PlaywrightModule <Playwright 模块路径>` 运行。它使用独立的应用数据、WebView2 配置和本地模型替身，向数据层注入失败并从 IPC、HTTP 和 SQLite 三处核对结果。测试证据位于 `output/playwright/chat-lifecycle/persistence/`。

该入口还运行 `chat-persistence-recovery-check.mjs`：通过实际 Webhook HTTP 入口与本地 STT/模型替身检查音频重投、转录漂移、服务失败、改文字和首次失败重试；通过原生 IPC 检查桌面第 5/20 轮、事件记忆、记忆关闭再开启、会话切换、重复/并发重试及背景分析次数。整合以真实 SQLite 的周期记录及执行路径验证，模型替身返回空记忆，未验证真实记忆内容质量或图片生成。LINE/Discord 复用准备边界的回归在真实 SQLite 上运行，尚未覆盖其实际网络处理器。运行器支持 `CARGO_TARGET_DIR`，重跑保留上次报告与失败日志。

这组测试覆盖用户与助手 INSERT、流式草稿最终 UPDATE、会话元数据更新、延迟外键导致的 COMMIT 失败、选择变更、重试去重、工具记录回滚和 TTS 失败。Telegram 使用 Rust 和真实 SQLite 验证进入模型前的写入失败，以及同一平台消息的最终保存重投；外部 Telegram 投递、语音转录和照片下载不在这些测试的范围内。

隐藏桌面回合的工具交换在最终提交前只保留于本轮内存，保持原 `None` 选择范围供流式输出、审批和 Stop 使用。最终事务或保存重试创建会话时，在同一个选择锁内更新 journal 的会话 ID 与世代；失败和取消不创建空壳会话。

## 外部请求归属查询的执行计划

`validate_external_request_owner` 依次检查当前请求 ID 和可信入口推导的旧 ID。每轮以 `(<id>:user, source, <id>)` 绑定同一条查询，同时匹配当时已写入的 `chat_save_id` 和历史 `source + client_request_id`。对方角色的命中会拒绝请求，同角色的候选也必须有与当前请求一致的保存身份才允许重放。媒体准备前和核心写入前各自执行对应入口的检查。

基线查询可从 `git show b3256e03243e467aad39c3bc67a6ab16756053c9:src-tauri/src/chat/persistence.rs` 的 `validate_external_request_owner` 提取。当前查询是 [EXTERNAL_REQUEST_OWNER_SQL](../src-tauri/src/chat/persistence.rs)，通过 [0015](../src-tauri/migrations/0015_chat_user_save_id.sql) 的部分表达式索引按保存身份查找，旧身份按 [0013](../src-tauri/migrations/0013_chat_user_request_id.sql) 的请求 ID 索引定位，再核对平台来源。

用项目 SQLx 所链接的 SQLite 3.46.0（启用 STAT4）对同一份由历史迁移 0001–0014 建立的库执行 `EXPLAIN QUERY PLAN`。库有 702 个会话和 56,005 条消息，其中用户 22,403 条、助手 22,401 条、工具 11,201 条。用应用的 `database_migrations::run` 将迁移版本从 14 升到 15，再运行当前查询。两个阶段均采用无 `ANALYZE` 统计的默认状态。五组绑定覆盖当前 ID 命中、它的旧别名未命中、旧 ID 命中，以及另一请求的当前和旧 ID 未命中。

| 版本 | 五组绑定的消息查询计划 |
| --- | --- |
| 0001–0014 的旧查询 | `SCAN m` |
| 应用 0015 后的当前查询 | 两支 `SEARCH conversation_messages USING INDEX … (<expr>=?)`，分别使用 `idx_chat_user_save_id` 和 `idx_chat_user_request_id` |

两次都用 `conversations` 主键关联角色。当前查询以 `UNION` 汇总去重后的候选 ID，再按主键读取消息和归属。执行计划的 `SCAN candidate_ids` 只遍历命中集合，临时 B-tree 负责候选去重和按原消息顺序排序。五组查询结果一致，角色、会话和消息表逐字段相同，SQLite 完整性和外键检查通过。

可在临时 SQLite 库中按下列规则复现同规模样本。按顺序应用 0001–0014，插入 `c-alpha`、`c-beta` 和 700 个噪声会话，为噪声会话轮换分配 20 个角色。每个噪声会话按 `user, assistant, user, assistant, tool` 插入 80 条消息，以会话序号和消息序号生成唯一键 `noise-{i:04}-{j:03}`。按 `(i + j // 5) % 8` 分配元数据，依次是两份 SQL NULL、一份非法 JSON、Discord 旧标识、Webhook 旧标识、保存标识、Telegram 新旧字段、Webhook 新旧字段。保存标识追加 `:<role>`，旧标识使用唯一键。再加入以下五条记录。

- `c-alpha` 的 `user` 仅写当前 `chat_save_id`，由 `external_identity("webhook", &["fixture-peer", "current-message"])` 追加 `:user` 得到。
- `c-beta` 的 `user` 仅写旧 `source` 和 `client_request_id`，旧 ID 用 `legacy_external_identity("webhook", "fixture-peer", "legacy-message")` 得到。再加一条绑定相同旧 ID、来源为 `telegram` 的用户干扰行。
- 加入同保存标识的 `assistant` 和同旧标识的 `tool`。旧别名未命中使用 `current-message` 的旧推导，另外两组未命中使用 `missing-message` 的新旧推导。

另外检查三组偏斜样本。单会话按序写入 25,000 条只有 `chat_save_id = request-<序号>:user` 的用户行，再加一条 `source = webhook, client_request_id = legacy` 的旧用户行。删除这条旧行得到纯当前样本。保留它并将前 25,000 条的元数据换成 `source = webhook, client_request_id = legacy-<序号>`，得到纯历史样本。对 `request-5150`、`legacy`、`absent` 分别绑定两支查询，纯历史样本另查 `legacy-5150`。四组样本共 15 组绑定，在项目 SQLite 3.46.0 无统计、完整 STAT4 与 Python SQLite 3.50.4 的 STAT1 状态下，均以 `(<expr>=?)` 定位两类身份。完整统计使用 `PRAGMA analysis_limit=0` 后执行 `ANALYZE`。

对旧查询执行 `EXPLAIN QUERY PLAN` 及查询本身，再对同一库应用 0015，用相同参数运行当前查询。迁移后在新连接上采集计划，避免把其他连接缓存的旧 schema 当成新计划。自动回归通过 SQLx 对磁盘库应用正式迁移并检查索引定位、历史行与重开后的归属判断，也覆盖存在和缺少旧标识时 `ANALYZE` 之后的执行计划。另有 Webhook 和 Telegram 的重开、回滚补交与模型/工具去重回归。

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib external_owner_checks
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib external_reply_commit
```

### 外部请求的已保存输入查询

入口解析、聊天核心和媒体准备都按所属角色与当前请求的 `<id>:user` 查找 `role = 'user'` 的已保存输入。四处查询都使用 [0015 的部分表达式索引](../src-tauri/migrations/0015_chat_user_save_id.sql)，其查询条件显式包含 `m.metadata IS NOT NULL`。归属校验另行以 `(<id>:user, source, <id>)` 查询当前 ID 和可信入口推导的旧 ID，返回角色和保存身份。查询结果与调用点对应如下。

| 路径 | 查询结果 |
| --- | --- |
| 外部请求入口 `resolve_request_scope` 恢复会话 | `m.conversation_id` |
| 聊天核心执行前查找已提交的用户输入 | `m.content, m.metadata` |
| 通用 Bot `prepare_bot_media_input` 复用已保存媒体输入 | `m.content, m.metadata`，与聊天核心共用查询 |
| Telegram 媒体准备经 `ExternalTurn::saved_user_message` 复用消息 | `m.conversation_id, m.content` |

四条基线语句取自 `b3256e03243e467aad39c3bc67a6ab16756053c9` 的 `src-tauri/src/chat/application.rs`、`src-tauri/src/bot/mod.rs` 和 `src-tauri/src/chat/persistence.rs`。更新后的三个查询及归属校验对照语句来自 [persistence.rs](../src-tauri/src/chat/persistence.rs)。两版在同一份已应用 0001–0015 的样本上比较，均为 8 个角色、256 个会话和 32,771 条消息，其中用户 13,314 条、助手 6,657 条、工具与系统各 6,400 条。以 `character-0`、`plan-hit:user` 查询得到已保存转录，以 `plan-miss:user` 查询得到空结果。归属校验以 `webhook` 为来源，并额外绑定不带后缀的 `plan-hit` 或 `plan-miss`。

项目所链接的 SQLite 3.46.0（启用 STAT4）和 Python SQLite 3.50.4 得到相同的 `EXPLAIN QUERY PLAN` 详情。在无统计及完整 `ANALYZE` 后，命中与未命中各自执行，结果与对应的基线语句一致。

| 语句 | 无统计，命中与未命中 | 完整 `ANALYZE` 后，命中与未命中 |
| --- | --- | --- |
| 四条基线语句 | A | B |
| 四条更新后的调用路径 | C | C |
| 两次实验的归属校验对照 | D | D |

```text
A
SCAN m
SEARCH c USING INDEX sqlite_autoindex_conversations_1 (id=?)

B
SCAN m
BLOOM FILTER ON c (id=?)
SEARCH c USING INDEX sqlite_autoindex_conversations_1 (id=?)

C
SEARCH m USING INDEX idx_chat_user_save_id (<expr>=?)
SEARCH c USING INDEX sqlite_autoindex_conversations_1 (id=?)

D
CO-ROUTINE candidate_ids
COMPOUND QUERY
LEFT-MOST SUBQUERY
SEARCH conversation_messages USING INDEX idx_chat_user_save_id (<expr>=?)
UNION USING TEMP B-TREE
SEARCH conversation_messages USING INDEX idx_chat_user_request_id (<expr>=?)
SCAN candidate_ids
SEARCH m USING INTEGER PRIMARY KEY (rowid=?)
SEARCH c USING INDEX sqlite_autoindex_conversations_1 (id=?)
USE TEMP B-TREE FOR ORDER BY
```

[回归测试](../src-tauri/src/chat/persistence/external_request_lookup_tests.rs) 创建上述样本，直接检查应用运行所用的查询常量，并验证其他角色、非用户消息和仅带旧元数据的记录不会匹配当前保存身份。下列命令输出两种统计状态下的命中与未命中计划。

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib external_request_lookups -- --nocapture
```

### 保存与重放中的 ID 查询

外部请求通过角色归属校验后，`ExternalTurn::begin_with_input_metadata` 恢复已绑定会话中的用户元数据，再调用 `write_message`。用户行已存在时读取已提交的助手回复。下表的参数顺序与当前 [persistence.rs](../src-tauri/src/chat/persistence.rs) 一致。

| 查询 | 绑定参数 | 结果与用途 |
| --- | --- | --- |
| 读取历史输入元数据 | `(conversation_id, <request_id>:user)`，消息类型固定为 `user` | 复用已保存 `metadata`，保留历史来源、原始输入和工具派发标记 |
| `write_message`，存在 `save_id` 时 | 用户写入绑定 `(conversation_id, save_id)`，其他写入绑定 `(conversation_id, role, save_id)`。旧查询统一使用三参数 | 读取 `id, content, metadata`。同一身份且内容和元数据一致时复用该行，冲突时回滚 |
| 重投时读取已保存的回复 | `(conversation_id, 'assistant', <request_id>:assistant)`。旧查询在 SQL 中固定 `assistant`，仅绑定会话和保存 ID | 读取 `id, content, metadata`，恢复文本、译文和输出媒体 |

这些查询在已绑定会话内按消息类型查找。来源与可信旧别名仍由上游归属校验处理。用户元数据和用户写入使用 0015 的索引。助手与工具等写入，以及重复请求的助手读取，共用一个新增的非用户部分索引，以 `(conversation_id, role, 保存 ID 表达式)` 定位。表达式先验证 JSON，只为有保存 ID 的行创建索引条目。基线三条查询可从 `b3256e03243e467aad39c3bc67a6ab16756053c9` 的同一文件提取。[迁移 0016](../src-tauri/migrations/0016_chat_non_user_save_id.sql)

以项目链接的 SQLite 3.46.0（启用 STAT4）比较实际 SQL。基线库应用 0001–0015，更新库在同一数据上应用 0016。两个混合样本均有 8 个角色、256 个会话。以 `i=1` 开始插入 32,766 或 65,534 条历史消息，会话由 `i%256` 选择，角色由会话序号对 8 取余确定，消息类型由 `(i/256)%4` 在用户、助手、工具与系统之间轮换。按 `i%13` 的前七个值依次写 SQL NULL、非法 JSON、空字符串、空对象、JSON null、值为 null 的保存 ID、仅有旧 ID，其余写唯一保存 ID。最后三行是同一会话下带 `plan-hit` 保存 ID 的用户、工具和助手行，未命中使用 `plan-miss`。用 `sqlite3_stmt_status(SQLITE_STMTSTATUS_VM_STEP)` 计数。命中执行到第一行，与 SQLx `fetch_optional` 一致，未命中执行到结束。无统计和 `PRAGMA analysis_limit=0; ANALYZE;` 后分别测量，结果行、计划和步数在两种统计状态下相同。

| 查询 | 32,769 行基线，命中 / 未命中 | 65,537 行基线，命中 / 未命中 | 更新后两种规模，命中 / 未命中 |
| --- | ---: | ---: | ---: |
| 用户元数据 | 98,777 / 98,787 | 197,547 / 197,557 | 14 / 10 |
| 用户写入查重 | 98,779 / 98,787 | 197,549 / 197,557 | 16 / 10 |
| 工具写入查重 | 98,791 / 98,794 | 197,559 / 197,562 | 22 / 18 |
| 助手写入查重及重放 | 98,796 / 98,794 | 197,564 / 197,562 | 22 / 18 |

```text
基线（命中与未命中）
SCAN conversation_messages

更新后，用户元数据和写入（命中与未命中）
SEARCH conversation_messages USING INDEX idx_chat_user_save_id (<expr>=?)

更新后，工具、助手写入和助手重放（命中与未命中）
SEARCH conversation_messages USING INDEX idx_chat_non_user_save_id (conversation_id=? AND role=? AND <expr>=?)
```

另用 32,769 行测试两种偏斜。当每 256 条历史行只有一条保留原元数据，以及将七成以上的历史行集中到同一会话并按消息类型重复相同的保存 ID 时，`plan-hit` 与 `plan-miss` 得到相同的更新后计划和步数。对集中会话中的重复键查找也保持索引定位。在其他会话查找相同的重复键，非用户路径仍为 18 步。0015 以保存 ID 读取等值候选，再核对会话，因而同一个用户 ID 人为复制 7,168 次的样本需要 35,850 步，和其他保存 ID 的历史数量无关。正常外部请求对已归属的身份复用原会话，并在事务内查重。

迁移前后，上一节四个入口调用点与归属校验的命中和未命中计划一致。`save_reply_locked` 校验原始用户行时绑定 `(user_message_id, conversation_id, <request_id>:user)`，SQLite 按 `INTEGER PRIMARY KEY (rowid=?)` 定位。会话归属及版本核对按会话主键，草稿变更和工具派发标记按消息主键。`saved_history_for` 仍按会话读取上下文窗口，消息行 ID 用于限定范围。上述路径中其余按单个 ID 查找、校验或更新的语句均已按索引或主键定位。[持久化查询](../src-tauri/src/chat/persistence.rs) [历史窗口](../src-tauri/src/ai/context.rs)

回归用正式迁移升级包含非法与空元数据的旧库，然后直接检查生产 SQL 的计划与返回值。另有测试覆盖相同身份的写入冲突、可信旧别名、其他会话或消息类型的同 ID，以及已提交回复的重投。[查询计划测试](../src-tauri/src/chat/persistence/external_request_lookup_tests.rs) [持久化测试](../src-tauri/src/chat/persistence.rs)

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib chat::persistence -- --nocapture
```
