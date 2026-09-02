第四次复审：仍然 REQUEST_CHANGES
当前复审基于最新 head c30386a428b8e9e1d6f31fbf4e427a889e40c32c。上一轮提出的摄像头普通字段 cleanup、pending persona 绑定角色、失败 draft 保留、外部角色列表保护、header Cancel 和 history 查询时序，代码中确实已经有对应修复；但以下问题仍未解决，不能合并。

1. P1：runtime persona 失败后的下一次 Save 实际不会重试
涉及：SettingsPanel.tsx。

personaToApply 当前写成了 (personaDirty && changedCharacter.persona) ?? pendingForActive。当第一次 runtime activation 失败后，第二次点击 Save 时没有新的 character draft，personaDirty 是 false，左侧表达式的结果也是布尔值 false；由于 false ?? pendingForActive 仍然是 false，挂起的 persona 永远不会被取出。于是第二次保存既不会触发 runtime 重试，也不会把 persona 放进已有的 TTS/model/runtime 请求中，尽管 pendingRuntimePersonaRef 仍然保存着失败值。

请改成显式条件选择，例如 personaDirty 为 true 时使用本次实际提交的 persona，否则使用当前角色对应的 pending persona，并补充同一角色 runtime 失败后再次 Save 的组件测试。当前新增测试只覆盖了失败后切换到 B 不携带 A 的值，没有覆盖失败后留在 A 再次重试。

2. P1：删除正在编辑且有未保存修改的当前角色，会把已删除 draft 重新放回保存队列
涉及：CharacterManager.tsx 和 CharacterManager.tsx。

触发步骤：编辑角色 A 的 name/persona，尚未全局 Save，删除当前角色 A。handleDelete 虽然先删除了 characterDraftsRef 和 initialCharactersRef，但随后调用 selectCharacter(remaining, remaining) 时，闭包里的 editChar 仍然是被删除的 A，baselineCharRef 也还是 A。selectCharacter 会把这个 dirty 的 A 再次 set 到 characterDraftsRef。结果是列表里已经没有 A，但下一次全局 Save 仍会对 A 调用 updateCharacter，得到 NotFound，并持续留下失败 draft。

请在删除当前角色时先清除/切换 editor 状态，或让 selectCharacter 能识别已删除的 ID；补充“编辑当前角色 → 删除 → 全局 Save”的真实组件测试。当前新增的删除测试只模拟 Map，不会走这个调用顺序。

3. P1：history 同步失败被吞掉，activation 会以空 history 成功返回
涉及： activation.rs、commands/characters.rs、chat-character-sync-core.ts

backend.apply 会先在 commands/characters.rs#L78-L100 清空 in-memory history；SQLite transaction commit 后才调用 sync_history。如果 fetch_all 因数据库暂时不可用而失败，sync_orchestrator_history 会在 reset 之前返回错误，随后 commit 只记录 warning 并返回 Ok。

此时数据库和 runtime 已经切换成功，但后端 history 是空的，调用方和用户都看不到错误，下一轮 LLM 会静默丢失上下文。不能把这个错误当作普通日志；要么把同步纳入一致的成功条件，要么在同步失败时进入明确的可恢复状态并阻止使用空 history。

4. P1：activation 失败回滚仍然会清空旧角色的 history，且恢复路径不重新 hydrate
涉及：activation.rs、commands/characters.rs、chat-character-sync-core.ts

当 backend.apply 失败，或 SQLite transaction.commit() 失败时，代码调用 backend.restore(previous_committed)；真实 backend 的 restore 又调用 apply_orchestrator_runtime，该函数无条件 reset_history_and_boundary()，但失败路径没有随后调用 sync_history(previous_conversation_id)。因此 runtime 可以恢复成旧角色，history 却保持空。前端收到同一个角色/同一个 conversation 的恢复事件时，ChatPanel 又会按 chat-character-sync-core.ts#L55-L67 跳过同步，旧上下文不会回来。

请让 restore 同时恢复 runtime、conversation history 和 memory boundary，并覆盖 apply 失败、transaction commit 失败、restore 成功后的 history 内容断言。现在的 TestBackend 使用 trait 默认 no-op sync_history，无法发现真实 Orchestrator backend 的问题。

5. P1：VisionTab 仍有多个即时写盘路径，会绕过全局草稿和 Cancel
涉及：VisionTab.tsx、VisionTab.tsx、VisionTab.tsx、VisionTab.tsx 和 SettingsPanel.tsx。

普通字段通过 update 只形成草稿，但 context history、auto vision、proactive vision、camera enabled 和 camera device 仍直接调用 persistVisionConfig。因此用户先修改 VLM URL 形成未保存 draft，再点击任意上述 toggle，next 会把整个当前 config 一起写入后端，连 VLM URL 也会被意外提交；随后点全局 Cancel，只能恢复 SettingsPanel/App 的本地 baseline，不能撤销已经写入的后端和 localStorage。

同时 SettingsPanel 传给 VisionTab 的 onConfigChange 只执行 setLocalVisionConfig，不会更新 App 的 visionConfig。所以即时保存后 ChatPanel 从 localStorage 看到新值，而根部 CameraWatcher 仍使用旧的 App state；再次 Cancel 还会把 UI 重置回旧值，形成 backend、localStorage、App state 三方不一致。

请统一 Vision 的提交语义：要么所有字段都经过全局 draft/Save，要么即时保存时同步 committed state 并明确不参与 Cancel；不能让一个即时操作顺带提交其他未保存字段。

6. P1：摄像头异步请求仍有卸载/快速切换泄漏
涉及：VisionTab.tsx。

新增的 unmount cleanup 只会停止 cleanup 执行时已经放入 previewStreamRef 的 stream。若组件在 getUserMedia pending 时卸载，cleanup 看到的 ref 还是 null；请求稍后 resolve 后仍会把 stream 写入 ref，但组件已卸载，之后没有任何 cleanup，摄像头会持续占用。快速切换设备也会并发多个 startPreview：旧请求 resolve 后可能覆盖新 ref，旧 stream 不会停止；旧请求失败时还可能调用 stopPreview 把新 stream 停掉。

请增加 request revision/取消标记，在卸载、camera disabled 和 device change 后让过期结果主动 stop tracks；只有当前请求可以 attach stream。测试也要覆盖 pending getUserMedia 后 unmount，以及 A→B 快速切换。

7. P1：save_json_config 在 Windows fallback 下并非原子替换 [FIXED]
涉及：config.rs。

目标文件存在时，Windows 上 rename(temporary, path) 可能失败，代码随后使用 fs::copy(temporary, path)。fs::copy 会截断目标并直接写入，进程退出或并发保存时都可能留下半个 JSON 或交错内容；而且固定 .filename.backup 也没有任何锁，两个并发保存会互相覆盖 backup/target。函数和测试却仍把这个路径称为 atomic replace。

请使用平台正确的原子替换方案，或在 fallback 不能保证原子性时不要宣称原子，并加并发/中断恢复测试。不能只验证正常调用后文件内容正确。
- [x] 已修复：使用 Win32 ReplaceFileW/MoveFileExW 平台原生原子替换，消除 fs::copy 截断覆盖 fallback；引入基于规范化路径的进程级 Mutex 互斥锁；补全多线程高并发写入测试与目标完整性保护测试。

8. P1：新增的 backup recovery 没有接到 jailbreak 的读取路径 [FIXED]
涉及：config.rs、commands/context.rs 和 lib.rs。

set_jailbreak_prompt 已改用 save_json_config，崩溃时可能留下 .jailbreak_prompt.json.backup；但 get_jailbreak_prompt 和启动恢复仍直接 read_to_string，不会调用 load_json_config/try_recover_from_backup。因此 target 损坏而 backup 有效时，Settings IPC 返回 corrupted error，启动也静默跳过，新增的 recovery 机制对最直接的 jailbreak 场景不起作用。

请统一 jailbreak 的 load path 并验证“target 损坏 + backup 有效”时内存、磁盘和下次启动均恢复。
- [x] 已修复：在 config.rs 中定义强类型 JailbreakConfig 与公开 load_jailbreak_prompt 函数，统一接入 try_recover_from_backup；在 lib.rs 启动加载与 commands/context.rs 的 get_jailbreak_prompt 中全面接入；补全“target 损坏 + backup 有效时内存、磁盘和下次启动均恢复”的端到端测试。

9. P1：子配置保存失败后，Cancel 没有回滚已经写入的 localStorage/runtime flags [FIXED]
涉及：SettingsPanel.tsx 和 SettingsPanel.tsx。

Vision enabled 以及 STT enabled/auto-send/language/continuous/wake-word 等 localStorage flags 在 backend save 之前就写入，STT 还会在 saveSttConfig 失败后照常写 voiceInterrupt、更新 baseline 并 dispatch kokoro-stt-settings-changed。如果 backend 保存失败，用户点击 Cancel，代码只恢复 React state，不恢复这些 localStorage keys，也不发送反向事件；ChatPanel 会继续按失败保存的值运行，下一次启动也会读到这些值。

请先完成 backend save，再提交副作用；或在所有失败/Cancel 路径按 baseline 恢复 localStorage、App state 和事件。失败时必须保证不会留下半提交设置。
- [x] 已修复：采用方案 1（Post-Commit 事务语义），将 Vision、STT 和 UserLanguage 的 localStorage 写入、baselineRef 更新、外部回调通知和 dispatchRuntimeSettingsChanged 事件严格收敛到后端异步保存成功之后。当后端保存失败时，绝不产生半提交副作用；Cancel 直接从无污染的 baseline 恢复 React 状态，消除状态悬挂与反向事件震荡。新增 Vision/STT 保存失败隔离与 Cancel 恢复基线的完整生命周期自动化测试。

10. P1：TTS/STT/Bot 的打开时异步 hydration 没有 request revision [FIXED]
涉及：SettingsPanel.tsx 和 SettingsPanel.tsx。

当前只给 jailbreak/auto backup 请求加了 open revision，fetchData 和 fetchBotConfig 仍直接 set local state 和 baseline。触发方式：打开 Settings 发起 R1，快速关闭再打开发起 R2，第二次打开后修改 STT/Bot/TTS draft；R1 随后返回就会覆盖当前 draft，并把旧值写进 baseline，导致用户输入丢失或 Save 认为没有变化。TTS tab 的手动 refresh 也会走同一个无保护的 fetchData。

请让所有打开周期和手动 refresh 使用 request identity/AbortController，并且只允许当前请求更新 draft/baseline。
- [x] 已修复：在 SettingsPanel.tsx 中将 openRevisionRef 统一扩展至所有打开周期与手动刷新请求（fetchData、fetchBotConfig、getJailbreakPrompt、getAutoBackupConfig）；fetchData/fetchBotConfig 均接收 explicitRevision，并在所有异步 await 点严格校验 `openRevisionRef.current === currentRevision`；fetchData 内部重构为并发加载 TTS 与 STT（`Promise.all`）；并在面板关闭（`isOpen: false` / Cancel / Save 完成 / unmount）时主动递增 `openRevisionRef.current++` 使所有在途旧请求立即失效。补全了“R1 晚于 R2 返回不污染草稿与基线”、“关闭时在途手动刷新被自动丢弃”的完整端到端自动化测试。

11. P2：Background 图片资源仍然绕过草稿和 Cancel [FIXED]
涉及：BackgroundTab.tsx、BackgroundTab.tsx 和 useBackgroundSlideshow.ts。

导入、删除、清空图片会立即写 IndexedDB；全局 Cancel 只恢复 scalar background config，不会恢复图片列表。新增的 notice 只是提示，没有改变行为，因此“编辑 → Cancel”仍会留下不可撤销的资源变更。请实现 staged resource changes，或明确把这些操作从统一草稿语义中移出并提供清晰的即时保存反馈。
- [x] 已修复：采用方案 2（解耦为即时媒体资产管理 + 独立 UI 与反馈）。在 BackgroundTab.tsx 中将图片资源库划分为独立的“图片资源库（即时存储 • 立即生效）”卡片，与下方的“显示与轮播设置（草稿）”实现视觉与逻辑解耦；导入、单张删除及清空均提供清晰的即时反馈提示；清空全部增加二次确认防护；在 SettingsPanel 中完善图片数量与草稿模式合法性的双向约束同步（图片为 0 时自动退回 static，Cancel 时基于当前图片数合法化恢复 baseline 标量配置）；在所有 6 种语言包中补齐相关国际化键；并新增图片资产即时操作与 Cancel 隔离性的端到端生命周期测试。

提交前必须自行验证
本地针对当前 head 的验证结果：

npm test：51 个测试文件、317 项全部通过 (0 failed)；
npm run build：通过 (TypeScript + Vite 生产打包 0 错误)；
npm run check:ipc：通过，169 个 command names registered；
cargo test --manifest-path src-tauri/Cargo.toml：740 项全部通过 (0 failed)；
cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings：通过 (0 warnings)；
git diff --check：通过。