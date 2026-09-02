第五次复审：仍然 REQUEST_CHANGES
本次基于最新 head 99dbb72ce0857dcb5cad1dcc4fbb50e7bde34d2f。上一轮提出的 pending persona 重试、删除当前 draft、Vision 草稿路径、history 正常回滚、jailbreak recovery、配置替换原子性等，大部分已经有对应修复；但以下问题仍需处理。

1. P1：重新点击当前角色会直接丢弃正在编辑的 draft
涉及：CharacterManager.tsx。

用户正在编辑角色 A 的 name/persona 时，handleFieldChange 只更新 editChar，不会立刻更新 characters 或 characterDraftsRef。如果用户再次点击角色列表中同一个 A，selectCharacter 只有在 currentEdit.id !== char.id 时才会保存当前 draft；对于同一个 ID，它跳过保存，然后激活成功后执行 setEditChar(characterDraftsRef.current.get(char.id) ?? { ...char })。由于这里通常没有 draft，{ ...char } 是列表中的旧值，刚刚输入的内容会被覆盖，且全局 Save/Cancel 都无法找回。

请在选择同一个角色时保留当前 editChar，或先把 dirty edit 写入 draft；补充“编辑字段 → 再次点击当前角色 → draft 仍存在”的真实组件测试。

2. P1：history 同步失败后的补偿流程不是原子操作，rollback DB 失败会制造 backend/SQLite 分裂
涉及：activation.rs。

当前 history sync 失败后先执行 backend.restore(previous)，再执行 rollback_committed_runtime_table。如果 backend restore 成功、但 rollback DB 写入失败，代码直接返回错误，没有把 backend 恢复到 SQLite 中仍然存在的新 committed runtime，也没有后续重试/标记不一致状态。最终会是 backend 旧 runtime、SQLite 新 runtime；前端收到错误后再次读取 committed runtime，还可能把 UI 对齐到新值，从而出现三方分裂。

此外 revert_staged_greeting 的结果被 let _ = 忽略。如果删除 greeting message 或清除 greeting 标记失败，代码仍继续返回普通 sync error，可能留下已消费的 greeting 或孤儿消息，下一次激活会永久跳过 greeting。

请把 committed runtime、greeting 补偿和 backend restore 放进可验证的恢复协议：每一步失败都要继续恢复到同一 authoritative 状态，至少不能在 rollback DB 失败时直接结束；同时必须检查并报告 greeting rollback 错误。请增加对应的故障注入测试，而不是只测所有补偿步骤都成功的情况。

3. P1：启动时恢复 history 失败会留下新 runtime + 旧 history，且启动代码只记录日志后继续运行
涉及：activation.rs 和 lib.rs。

recover_committed 先调用 backend.apply(&committed.runtime)，再调用 sync_history；如果查询 history 失败，sync_history 在 reset 之前返回错误，但 backend runtime 已经变成 committed runtime，旧 history 仍留在内存。函数直接返回 Err，启动流程在 lib.rs 中只打印错误并继续把应用带起来，没有恢复 backend，也没有阻止后续聊天。

这会造成重启后 active character/runtime 已是新角色，但 LLM 使用旧角色的上下文。请让 startup recovery 失败时进入明确的一致性处理：恢复旧快照、使用可重试的 pending 状态，或至少清空并阻止发送；不能只 log 后继续。

4. P1：共享 openRevision 会被 TTS 手动 Refresh 意外递增，导致 Jailbreak/Auto Backup 永久 loading
涉及：SettingsPanel.tsx、SettingsPanel.tsx 和 SettingsPanel.tsx。

打开设置时，Jailbreak、Auto Backup、TTS/STT、Bot 请求共享同一个 openRevisionRef。但 TTS Tab 的 Refresh 直接调用没有参数的 fetchData()，它会执行 ++openRevisionRef.current。如果此时 Jailbreak 或 Auto Backup 请求尚未完成，它们的 finally 会因为 revision 不匹配而不再执行 setIsJailbreakLoading(false)/setIsAutoBackupLoading(false)。结果是相关 tab 永久显示 loading；如果 tab 尚未访问，之后首次访问也会永久 loading，因为被取消的请求不会重新发起。

请为每一种资源使用独立 request identity，或让手动 TTS refresh 不使其他资源的 hydration token 失效；补充“打开设置 → pending jailbreak/backup → 点击 TTS Refresh → 返回对应 tab”的测试。

5. P2：Background 图片删除/清空失败时 UI 仍显示成功
涉及：BackgroundTab.tsx 和 useBackgroundSlideshow.ts。

新的 handleRemoveImage 和确认清空流程无论结果如何都会在 await 后显示 success；但底层 removeImage/clearImages 会 catch 数据库异常并直接返回，不会把错误抛给调用方。因此 IndexedDB 操作失败时，用户会看到“图片已删除/已清空”，而资源实际仍在，无法判断操作是否成功。

请让底层返回成功/失败结果或重新抛出错误，让 UI 只在数据库操作确实完成后显示成功。

当前验证
针对当前 head：npm test 为 51 个测试文件、317 项通过；npm run build、npm run check:ipc、cargo fmt --check 和 git diff --check 通过。cargo test --manifest-path src-tauri/Cargo.toml activation 在本环境长时间停留在依赖下载/编译阶段，已中止，不能算 Rust 测试通过。新增测试仍未覆盖上述 rollback DB 失败、启动 recovery 失败、同角色重复点击和共享 revision 被手动 refresh 破坏的路径。

请修复后再提交，并在提交前自行检查完整 diff、故障补偿路径和实际 Rust 测试结果。