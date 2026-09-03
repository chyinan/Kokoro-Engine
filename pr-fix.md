第十一次复审：仍然 REQUEST_CHANGES
本次基于最新 head d896d34e1ceeb38f7afa99170eb35a1ba7351c10。上一轮的 in-flight cancellation 保护已经新增完成 barrier 和 apply/commit/history/rollback abort 测试，但 Guard 的错误退出路径仍有一个 P1。

P1：拿到 activation 写锁后，早期 ? 返回会错误地把系统永久留在 activating/degraded 状态
涉及：activation.rs 和 context.rs。

commit() 在第 343 行拿到 ActivationLockGuard 后，后续仍有多个没有显式补偿/mark_activation_completed() 的 ?：pool.begin()、读取并校验 character、ensure_owned_conversation、stage_greeting、创建 activation table、序列化和写入 committed runtime。比如 token 准备后角色被另一个路径删除，live_updated_at 查询返回 None，ok_or_else 直接返回；又比如 stale updated_at、数据库暂时不可用或 greeting staging 失败，都会走同一条路径。

这些错误发生在 runtime apply 之前，或者 transaction 还没有 commit，本身不应该把系统判定为未完成 activation。但由于 guard 没有被标记 completed，ActivationLockGuard::drop() 会把 activating 保持为 true，并尝试设置 runtime_degraded。之后新的 chat turn 都会被拒绝；如果前端的自动 recovery 看到数据库没有 committed runtime，也可能无法获得一个会清理这个 gate 的恢复机会，只能再触发一次 activation 才能解除。

请把写锁获取放到所有不会修改 runtime 的校验之后，或为所有 early-return 路径统一使用 scope guard/显式 mark_completed()；只有真正进入不可逆状态且取消后需要 fail-closed 时才保留 activating/degraded。请补充“character 在 prepare 后被删除”“updated_at 变旧”“stage greeting/transaction 初始化失败”后，gate 能正常释放且 chat 可以重新进入的测试。

当前验证
最新 head 本地验证：npm test 为 51 个测试文件、320 项通过；npm run build、npm run check:ipc、cargo fmt --check、git diff --check 通过。

第十一次复审修复方案与验证结论：
- 采纳“方案二：Guard 明确 Pre-mutation 与 Post-mutation 状态屏障”：ActivationLockGuard 引入 fail_closed 标记（默认为 false），在进入不可逆的 backend.apply 之前显式调用 backend.arm_activation_mutation()；在 mutation 之前的早期校验（如 character 不存在、stale token、数据库不可用、greeting staging 失败）发生 ? 早期退出或 abort 时，Guard drop 自动安全释放 gate（activating = false）且绝不置位 runtime_degraded。
- 新增 3 个 early-return 专项测试（prepare 后角色被删除、stale token、db 事务初始化失败），全部断言早期错误发生后 gate 正常释放，runtime 未降级，且后续 chat turn 均可正常进入。
- 完整回归验证：commands::characters::tests (39/39 passed), characters::activation_tests (35/35 passed), commands::chat::tests (61/61 passed), npm test (320/320 passed), npm run build, npm run check:ipc, cargo clippy (--lib -- -D warnings), cargo fmt --check 全部通过。