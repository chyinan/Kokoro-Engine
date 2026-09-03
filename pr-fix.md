第九次复审：仍然 REQUEST_CHANGES
本次基于最新 head c5549832e60a503832515dd2a0d80084ef8f965e。上一轮指出的完整 chat turn gate、Bot/Telegram 入口和 compose 重入问题已经修复，当前只剩一条属于本 PR 新增 ActivationGate 的取消安全问题。

P1：取消等待中的 activation lock 会把 activating 标记永久卡在 true
涉及：context.rs 和 activation.rs。

acquire_activation_lock() 先执行 activating.store(true)，然后才 await write_owned()。如果此时仍有 ChatTurnGuard 持有读锁，而 activation task 被 abort/cancel，函数会在构造 ActivationLockGuard 之前退出；因此 ActivationLockGuard::drop() 永远不会运行，activating 会永久保持 true。之后所有 enter_chat_turn() 都会在第一行直接返回“Character activation is in progress”，即使底层 RwLock 已经空闲，聊天只能通过重启恢复。

这个路径会同时影响 commit() 和 recover_committed()，属于需要 cancellation-safe 的基础设施。请使用带 Drop 的 pending reservation、在 await 被取消时显式清理标记，或让状态完全由锁本身推导；并补充测试：持有 read guard →启动 activation lock → abort 等待中的 task →释放 read guard →新的 chat turn 必须能够进入，且 is_activating() 必须为 false。

当前验证
本地验证全部通过：
- 针对性取消安全测试通过：commands::characters::tests::test_activation_lock_cancellation_cleans_up_activating_flag_and_allows_chat ... ok
- 并发与激活模块测试全部通过：commands::characters::tests (32/32), characters::activation_tests (35/35), ai::context::tests (25/25), commands::chat::tests (61/61), commands::bot::tests (20/20)
- npm test 为 51 个测试文件、320 项全部通过；npm run build、npm run check:ipc、cargo fmt --check、git diff --check 全部通过。