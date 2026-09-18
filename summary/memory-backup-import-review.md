# 设置页「导出数据 / 导入数据」（含记忆）缺陷审查与修复

审查时间：2026-09-14
审查范围：`src/ui/widgets/settings/BackupTab.tsx`、`src-tauri/src/commands/backup.rs`、`src-tauri/src/commands/auto_backup.rs`、`src-tauri/migrations/*`
验证方式：源码走查 + SQLite 复现脚本（已随修复删除，见下方回归测试）+ Rust/Vitest 全量测试

状态：**P1/P2 已修复，P3 部分修复**（详见每节「修复」）

---

## 0. 结论：老版本备份的兼容性

记忆数据本身（`memories` 表）在 overwrite 模式下**可以**从很老的备份恢复：`import_data` 会先探测 `import_db.memories` 的列，把 0008 迁移新增的 12 列用 `ALTER TABLE` 补齐并回填（`backup.rs` 的列归一化段），`memory_dream_proposals.source_memory_versions` 也会补齐。列投影用显式列名，不依赖列顺序。

| 备份来源 | memories 列 | dream/审计表 | 修复后 overwrite | 修复后 skip |
|---|---|---|---|---|
| v0.1.0-beta ~ v0.2.0（迁移 ≤0004） | 9 列 → 自动补齐 ✔ | 无 | 记忆/对话恢复，且不再残留旧 dream 行 | 正常导入，冲突关系被丢弃而非整单失败 |
| v0.2.5（≤0007） | 9 列 → 自动补齐 ✔ | 缺 5 张 dream 表 | 同上 | 同上 |
| v0.2.8（≤0010） | 21 列 ✔ | 有（proposals 17 列 → 自动补齐 ✔） | 正常 | 正常（此前必失败） |
| 当前（0011） | 21 列 ✔ | 18 列 ✔ | 正常 | 正常（此前必失败） |

---

## P1-1（高）「跳过已有」策略整单失败 —— 已修复

**原问题**：`import_data` 的 skip 分支在写入前调用两个守卫，只要发现“导入行引用了本地已存在的 id”就 `return Err` 整单拒绝。恢复自己的备份时该条件必然成立（会话 id 与本地完全相同 → 消息全部命中；v0.2.8 起每条记忆都写 `memory_operations`，id 重叠即命中），所以「跳过已有」实际上不可用。

**修复**：
* `reject_unsafe_skip_memory_conflicts` → `resolve_skip_memory_conflicts`：只有“同 id 且内容/归属确实不同”才算冲突（`content` / `character_id` 无法比较时退回严格判定）。
* 删除 `reject_unsafe_skip_conversation_conflicts`，改为结构化过滤：
  * `prepare_skip_conversation_scope` 记录“本地不存在的新会话”和消息 id 偏移量；
  * `insert_skip_conversation_messages` 只导入新会话的消息，且 id 统一上移到本地最大 id 之上 —— 不可能与本地会话或消息发生错挂。
* 冲突关系改为**丢弃而不是整单失败**：`insert_skip_memory_relations` 过滤掉指向冲突 id 的 `memory_candidates` / `memory_evidence` / `memory_operations` / `memory_dream_proposals`；`MEMORY_INSERT_SKIP_SQL` 会把指向冲突 id 的 `supersedes` 链接清空。
* `INSERT OR IGNORE` 因本地约束丢弃行时不再静默：新增 `skipped_memories` 计数并写日志。

## P1-2（高）overwrite 导入老备份后 dream/审计表错挂 —— 已修复

**原问题**：`memories` 无条件清空重建，但 5 张 dream/审计表只有“备份里存在”时才清空。≤v0.2.5 的备份没有这些表，本地旧行原样保留，而它们持有指向 `memories.id` 的整数外键，此时这些 id 已属于备份里的记忆。

**修复**：抽出 `restore_memory_aux_tables_overwrite`。overwrite 时，只要持有 `memories.id` 引用的表（`MEMORY_REFERENCING_TABLES` = `memory_candidates` / `memory_evidence` / `memory_dream_proposals` / `memory_operations`）一律清空，无论备份里有没有这些表；`memory_dream_jobs` 无整数外键，仍保持“备份有才动”。
`session_summaries` 等按 TEXT 归属的表维持原语义（既有测试 `optional_backup_overwrite_keeps_local_rows_when_import_table_is_missing` 继续通过）。

## P2-3（中）skip 跨机器导入静默丢光对话消息 —— 已修复

**原问题**：`INSERT OR IGNORE INTO conversation_messages (id, ...)` 复用备份里的自增 id，目标机消息 id 同样从 1 开始 → 全部被忽略且不计入统计。

**修复**：新会话的消息用 `id + offset`（offset = 本地 `MAX(conversation_messages.id)`）插入，保证唯一且保持顺序；`conversation_summaries` 只跟随新会话导入，并把 `start_message_id` / `end_message_id` 一起偏移，摘要区间继续指向自己的会话。已存在会话的历史保持本地版本（skip 语义）。

## P2-4（中）孤立记忆不可见 —— 已按「不展示、静默清理」处理

**原问题**：前端从不传 `target_character_id`（`BackupTab.tsx` 里 `firstPass` 恒为空的整段 Phase 1 是死代码），导入 v0.1.x 备份或跨机器 skip 恢复时，记忆可能归属到本地不存在的角色，而记忆面板强制按 `character_id` 过滤 → 完全不可见。

**无主记忆的来源**：

1. 删除角色（此前只删 `characters` 行，记忆保留）；
2. `character_id` 兜底 `"default"`（`chat.rs` / `actions.rs` / `bot.rs` / `telegram`），而 `characters` 表从来没有该行；
3. 导入 v0.1.x 备份（那时角色在 IndexedDB，`kokoro.db` 没有 `characters` 表）；
4. skip 模式跨机器导入时沿用备份的 character_id。

**处理**：不再向用户展示计数。

- 写入侧（来源 2）：`MemoryManager::memory_owner_exists` 在 `add_memory_with_importance_unlocked` 与抽取入口校验归属，无主写入直接跳过并记 warn；`remember` 工具改为返回明确错误；`test_vector_store` 诊断改用真实角色。
- 删除侧（来源 1）：见下方「删除角色级联清理」。
- 导入侧（来源 3/4）：`purge_orphaned_memories` 在导入事务内静默删除无主记忆及其关系行，只写日志与 `debug_log`。

## P2-5（中）删除角色级联清理 + 确认弹窗 —— 已实现

**行为变更**：`delete_character_with_resources_in_pool` 现在在同一事务里调用 `purge_character_owned_rows`，删除该角色的对话（含消息）、记忆、dream/审计表、会话摘要与可观测性日志（`CHARACTER_OWNED_TABLES`）。此前只删 `characters` 一行，等于把记忆和对话「藏起来」而不是删掉，与删除的意图相悖。

**UI**：`CharacterManager` 的确认层从单行提示改为带换行的警示块，文案为「删除『X』会同时删除该角色的所有记忆和对话，确认吗？」（六语言同步）。

**文档**：AGENTS.md 的跨域约定拆成两条——卸载内容包保留用户数据；删除角色实例是破坏性操作，连同对话与记忆一起删除。

## P2-6（功能）跨机器恢复：预览中选择角色归属 —— 已实现

**背景**：角色实例 id 由前端 `crypto.randomUUID()` 生成，跨机器必然不同；而恢复是 **character_id 权威**的，全程没有按名字匹配的逻辑。跨机器导入因此会得到两个同名角色（overwrite 下本地那个还会被整库清空成空壳）。`cross_machine_overwrite_restore_keeps_the_backup_character_identity` 测试固化了这一事实。

**实现**（用户点选，不做自动按名合并）：

- `ImportPreview.characters`：预览阶段列出备份中的角色（`id` / `name` / 记忆数 / 对话数），由 `read_backup_characters` 读取；v0.1.x 备份没有 `characters` 表则返回空列表。
- `ImportOptions.character_merges`（`{ imported_id, target_id }[]`）替换了废弃的 `target_character_id`：列出的角色，其记忆/对话/摘要/dream/日志行在拷贝前被改写到本地目标角色（`apply_import_character_merges` + `IMPORT_CHARACTER_SCOPED_TABLES`），且不再作为新角色插入；未列出的按新角色导入。
- `ImportOptions.ignored_characters`（角色 id 列表）：被忽略的角色在拷贝前由 `remove_imported_character` 从备份库里整体剔除（角色行、记忆、对话、消息、摘要、dream 与可观测行），既不导入也不参与孤儿清理；`ImportResult.ignored_characters` 上报数量。
- `validate_character_merges` 在事务外先校验：备份里必须存在 `imported_id`、本地必须存在 `target_id`、不能自映射、不能重复映射、忽略项不能重复、**同一个角色不能既合并又忽略**、备份没有 `characters` 表时不允许映射或忽略。
- `ImportResult.merged_characters` 上报合并数量；合并后把用户选定的本地角色持久化为 active character。
- 前端：`BackupTab` 预览中每个备份角色一个下拉框（`backup-character-mapping.ts` 的 `defaultCharacterTargets` / `buildCharacterPlan`），三个选项为「作为新角色导入 / 合并到「X」/ 忽略（不导入）」，控件复用 `@/components/ui/select`（与 Bot 页角色选择同款），不是原生 `<select>`。默认建议仅在「同名本地角色恰好一个」时预选合并，其余一律「作为新角色导入」，**默认永不预选忽略**；选中忽略时面板出现一行警示（覆盖模式下被忽略角色的记忆与对话不会恢复）。
- MOD 动作 `confirm_import_backup` 同步支持可选的 `character_merges`；`mods/genshin-theme` 的备份面板也补上了同一套「角色归属」选择（默认建议规则与主界面一致，仅在同名本地角色恰好一个时预选合并），未传 `character_merges` 的调用方维持原行为。

**MOD 面板说明**：`mods/genshin-theme/components/settings.html` 是自带的官方示范主题，仓库里没有任何自动化测试覆盖它，所以这次用「抽出内联函数跑同一组用例」的方式核对：默认建议的四种情形（唯一同名 / 无同名 / 多个同名 / 前后空格与大小写）与载荷形状，结果与 `backup-character-mapping.ts` 的 Vitest 用例完全一致。另外注意 `registry/packages/genshin-theme-1.0.0.zip` 本来就落后于 `mods/` 源码（上次改动也没重建），按仓库既有流程由发布提交统一重建，本次未动 registry 制品。

**文档**：`docs/API specification.md` 的 `ImportPreview` / `ImportOptions` / `ImportResult` 已更新。


---

## P3（低）已处理项

| # | 处理 |
|---|---|
| P3-1 调试信息外泄 | 导入成功提示不再拼接 `debug_log`，只保留统计与提示 |
| P3-2 manifest 版本 | 手动导出与自动备份统一用 `BACKUP_FORMAT_VERSION = "2"` |
| P3-3 统计时机 | `create_consistent_database_snapshot` 返回 `{ bytes, stats }`，统计来自快照本身，与归档字节一致 |
| P3-5 memories 表预检 | 导入前检查 `import_db.memories` 是否存在，缺失时给出明确错误而不是深层的列归一化失败 |
| P3-4 未知配置名 | **保持拒绝**（`config_import_rejects_nested_unknown_and_database_names` 明确了该契约），未改动 |

---

## 回归测试

新增/改写的测试：

* `overwrite_import_clears_memory_relations_the_backup_does_not_carry` —— P1-2
* `import_purges_memories_without_a_local_character` —— P2-4 静默清理（含关系行与 per-character 行）
* `skip_import_keeps_local_rows_and_drops_only_stale_relations` —— P1-1 组合行为（相同行不算冲突、新记忆带入、冲突关系与 supersedes 链接被丢弃）
* `review_r17_skip_accepts_an_unchanged_backup` —— 原「重导入自己的备份必失败」回归
* `review_r17_skip_flags_rows_that_really_are_different_memories` —— 真实冲突仍会被识别并过滤
* `skip_import_never_attaches_messages_to_a_local_conversation` —— 本地会话历史不被覆盖
* `skip_import_ignores_orphan_messages_that_reference_local_conversations` / `skip_import_ignores_unsupported_import_message_shapes`
* `skip_import_moves_new_conversation_history_above_local_message_ids` —— P2-3
* `memory_writes_are_skipped_for_an_unknown_character`（`ai/memory.rs`）—— P2-4 写入侧守卫
* `delete_removes_the_instance_with_its_conversations_and_memories`（`commands/characters_tests.rs`，替换原 `delete_removes_only_the_instance_row`）—— P2-5
* `cross_machine_overwrite_restore_keeps_the_backup_character_identity` / `cross_machine_overwrite_restore_can_merge_into_a_local_character` / `cross_machine_overwrite_restore_can_ignore_a_backup_character` —— P2-6 三种选择的端到端结果
* `character_merge_rewrites_import_rows_without_touching_live_rows` / `character_merge_validation_rejects_unknown_ids`（含忽略项的重复、未知、与合并冲突三种非法输入）/ `preview_lists_backup_characters_with_their_data_counts` —— P2-6
* `backup-character-mapping.test.ts`（Vitest）—— 默认建议与 IPC 载荷（含「默认永不预选忽略」）
* `BackupTab.character-mapping.test.tsx`（Vitest + jsdom）—— 渲染层：确认不再出现原生 `<select>`、默认选中项正确、改选合并/忽略后提交的 `character_merges` 与 `ignored_characters` 正确、忽略时的警示行出现与消失

验证命令与结果：

```
cargo test --manifest-path src-tauri/Cargo.toml          # 907 passed, 0 failed
cargo test --manifest-path src-tauri/Cargo.toml backup   # 71 passed
cargo clippy --manifest-path src-tauri/Cargo.toml --lib  # 无新增告警
rustfmt --edition 2021 --check src-tauri/src/commands/backup.rs   # clean
npx tsc --noEmit                                          # clean
npx vitest run                                            # 81 files / 781 tests passed
npm run check:ipc                                         # 171 commands registered
```

> 注：`cargo fmt -- --check` 在整个仓库仍有既有未格式化文件（`actions/executor.rs` 等），与本次改动无关。
