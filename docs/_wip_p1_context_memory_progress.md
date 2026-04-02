# P1 分层上下文压缩与长期记忆优先注入进度

## 当前目标
按已批准的 P1 计划落地分层上下文系统，优先保证长期记忆数据库在 prompt 中高于会话摘要，并把摘要改成非破坏式后台派生物。

## 本轮实施顺序
1. 数据库 migration：`conversations.topic` / `pinned_state` + `conversation_summaries`
2. `memory.rs`：conversation summary 状态机 API
3. `context.rs`：非破坏式摘要触发 + prompt 分层重排
4. `conversation.rs` / `kokoro-bridge.ts`：贯通会话状态字段
5. `ConversationSidebar.tsx`：最小展示 topic / pinned 标记
6. `backup.rs`：修复扩列后的导入兼容
7. 编译与类型验证

## 已确认现状
- 当前 `add_message_with_metadata(...)` 仍会在 `summary` 模式下先从内存窗口弹出旧消息，再异步生成 `session_summaries`
- 当前 `compose_prompt(...)` 注入顺序是 rules → character → memory/session summaries → tools → language → recent history
- 当前 `conversations` 仍只有 `id/character_id/title/created_at/updated_at`
- 当前 `backup.rs` 仍有 `INSERT INTO conversations SELECT * FROM import_db.conversations`

## 当前进展
- [x] 建立本进度文件
- [x] 复核 P1 计划与关键现状代码
- [x] 新增 migration 与 conversation summary 状态表
- [x] 改造 memory/context 主链路
- [x] 贯通会话状态到前后端
- [x] 修复备份导入兼容
- [x] 运行验证命令

## 本轮已落地
- 新增 migration：`src-tauri/migrations/0005_add_conversation_state_and_summaries.sql`
- `memory.rs` 新增 conversation summary 状态机、冷却熔断、非破坏式待摘要区间选择
- `context.rs` 改为 recent window 仅做内存窗口裁剪，summary 从数据库消息区间派生，不再先删再总结
- `compose_prompt(...)` 重排为：rules → character → long_term_memory → conversation_state → conversation_summary → tools/live2d/language → recent raw messages
- `conversation.rs` / `kokoro-bridge.ts` / `ConversationSidebar.tsx` 已贯通 `topic` / `pinned_state`
- `ConversationSidebar.tsx` 已补最小编辑入口：可编辑并保存当前会话的 `topic` 与 `pinned_state`
- `backup.rs` 已改为显式列导入 `conversations`，兼容旧备份缺少新字段的情况
- 已通过 `npx tsc --noEmit`、`cargo check`、`cargo test --lib --no-run`、`cargo check --tests`

## 备注
- 长期记忆数据库中的关键事件是角色持续性的核心，优先级高于 conversation summary。
- summary 失败时不能导致历史上下文丢失；主聊天链路必须继续工作。
