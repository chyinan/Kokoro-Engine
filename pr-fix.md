上一轮指出的多数问题已经修复：角色 draft purge、Vision 全局保存同步、Jailbreak hydration revision、history boundary 和配置错误聚合均已改进。但当前 head 仍有以下问题，暂时不能合并。

1. P1：Vision 普通字段修改会停止摄像头预览
涉及：VisionTab.tsx、VisionTab.tsx、SettingsPanel.tsx。

普通字段修改时 update() 调用 onConfigChange(next)，SettingsPanel 随即更新传给 VisionTab 的 initialConfig。这会触发 useEffect([initialConfig]) 的 cleanup，而 cleanup 无条件执行 stopPreview()。

由于 config.camera_enabled、selectedDeviceId 和设备列表没有变化，后续启动预览的 effect 不会重新执行。因此只修改 VLM URL、模型、间隔等字段，也可能把当前摄像头预览停掉，并且一直不恢复，直到重新开关 camera 或重挂载组件。

请不要让 draft prop 变化触发“组件卸载时”的 stopPreview()；或者在配置变化后显式、可靠地重启预览。

2. P1：runtime persona 失败重试值没有绑定 character ID
涉及：SettingsPanel.tsx、SettingsPanel.tsx、SettingsPanel.tsx。

pendingRuntimePersonaRef 只保存 persona 字符串，不保存它属于哪个角色。触发方式：

active character A 的 persona 保存到 SQLite；
runtime activation 失败，pending persona 保留等待重试；
用户切换到 character B；
再次点击 Save 或保存 B 的其它设置。
此时 onCharacterRuntimeChange 会以 B 为 active character，却携带 A 的 pending persona，可能直接覆盖 B。点击右上角关闭按钮也不会清除 pending 值，因为按钮仍然直接调用 onClose：SettingsPanel.tsx，只有底部 Cancel 才清除。

请把 pending 状态绑定到 character ID，并在 active character 变化、关闭和重新打开时正确清理或迁移。

3. P1：activation 已提交后 history 同步失败会造成 backend/frontend 分裂
涉及：activation.rs、commands/characters.rs、character-activation.ts。

ActivationCoordinator::commit() 在 SQLite transaction 已经 commit 后才调用 backend.sync_history()。如果 history 查询失败，它直接返回错误，但此时：

SQLite 中的新角色 runtime 已提交；
backend runtime 已经应用新值；
history 可能已被清空；
frontend activation service 只会恢复 frontend snapshot，不会恢复 backend/database。
另外 sync_orchestrator_history() 是先 reset_history_and_boundary()，再查询数据库；查询失败时旧 history 也已经丢失。

请设计成可回滚或幂等的事务边界：要么 history 同步失败不把已经提交的 activation 报告为失败，要么在错误路径明确恢复 backend、history 和 frontend；不能让调用方收到失败但数据库和 backend 已经进入新状态。

4. P1：角色 draft 的数据库写入失败会被吞掉并清除，非 active draft 无法重试
涉及：CharacterManager.tsx。

保存多个 draft 时，每个 updateCharacter() 的异常只打印日志，失败的 draft 不会加入 committedMap，但循环结束后仍然执行 characterDraftsRef.current.clear()。SettingsPanel 也收不到失败信息，因此如果没有其它错误，最后仍可能显示 Saved。

对于当前 active draft，当前编辑值可能在下一次 Save 时重新入 map；但非 active draft 已经从 map 和 UI 中丢失，用户无法可靠重试。请保留失败 draft、把失败 ID/错误返回给父组件，并让全局 Save 明确显示失败。

用户 profile 的 setUserName/setUserPersona 也在 baseline 更新后吞掉异常：CharacterManager.tsx。这同样会出现“本地状态/界面认为已保存，但后端没有保存且下次不重试”。

5. P1：外部角色列表刷新仍会丢弃正在编辑的 draft
涉及：CharacterManager.tsx、CharacterManager.tsx。

这次代码只保护了 initialCharactersRef，但没有保护 editChar：

用户正在编辑角色 A，当前 editChar 有未保存修改；
父级因为 runtime 更新、其它窗口或外部操作刷新 charactersProp；
effect 先 setCharacters(next)；
依赖 characters 的 active/characterToEdit effect 再从刷新后的角色记录执行 setEditChar(selected)。
用户刚输入的字段因此被旧列表覆盖。请在当前 editor 有 dirty draft 时保留 editChar，或从 characterDraftsRef 合并回列表，不能只保留 baseline。

6. P2：所谓 atomic replace 仍有崩溃窗口，且没有恢复遗留 backup
涉及：config.rs。

当前顺序是：

写 temporary；
把旧 target rename 到 backup；
把 temporary rename 到 target；
删除 backup。
第 2 步和第 3 步之间 target 不存在；如果进程在此时退出，磁盘只剩 backup，下一次读取不会自动恢复它。并发保存也可能互相覆盖 backup/target。真正的原子替换应在同一目录直接 rename 临时文件覆盖 target，或实现明确的启动恢复协议；请不要只靠正常路径测试来宣称 atomic。

7. P2：Vision preview/controlled-config 的新增测试仍没有覆盖真实组件生命周期
新增测试主要验证纯 Map/变量模拟，未真正 render CharacterManager、SettingsPanel 或 VisionTab，因此无法发现上面的 effect cleanup、pending persona 和父子状态竞态。

请补充至少一组组件/集成测试，实际触发“修改 → 父级 prop 更新 → cleanup/effect → Save/Cancel”的路径，而不是只复制实现逻辑后断言 Map 结果。

提交前必须自行验证
请在提交前至少验证：

Vision 修改非 camera 字段时预览不会停止；
runtime 失败后切换 character，再次 Save 不会携带旧 persona；
history 查询失败、activation commit 失败、backend restore 失败时，SQLite/backend/frontend 状态一致；
非 active 多角色 draft 中一个写入失败时，失败 draft 仍可重试；
外部角色列表刷新不会覆盖当前正在编辑的字段；
配置写入过程中退出后 target/backup 可恢复；
新增组件测试真正覆盖以上生命周期，而不是只测辅助变量。
当前本地验证：前端 npm test 48 个测试文件、297 项通过；npm run build、npm run check:ipc、cargo fmt --check 通过。Rust cargo test 仍因环境无法下载 sherpa-onnx/ONNX Runtime 依赖而无法完成，不能算作通过。