第十次复审：仍然 REQUEST_CHANGES
本次基于最新 head 50be2dcc9ea54731cc259f91777757de5538eb5f。上一轮的等待中 activation lock 取消问题已经通过 ActivationReservation 修复，但当前闸门仍有一个更严重的 in-flight cancellation 问题。

P1：activation 已拿到写锁后被取消，会跳过补偿并立即重新放行聊天
涉及：ActivationLockGuard 和 ActivationCoordinator::commit。

ActivationReservation 只解决了 write_owned().await 尚未拿到写锁时被取消的情况。写锁拿到后，commit() 仍有很多 await：打开/写入 SQLite transaction、backend.apply、提交 transaction、history sync 以及各类 rollback。此时如果 Tauri task 被 abort，Rust 会直接 drop 当前 future：transaction 可能自动回滚，但已经执行过的 backend.apply 不会自动回滚；如果 transaction 已提交而 history sync 尚未完成，数据库/runtime 也可能已经是新值而 history 仍是旧值。随后 ActivationLockGuard::drop() 无条件把 activating 设为 false 并释放写锁，新的 chat turn 就会进入这个半完成状态。

结果可能包括 backend 新 runtime + SQLite 旧 runtime、SQLite 新 runtime + 未同步 history，或者补偿执行到一半。当前新增的 cancellation 测试只在 write_owned() 等待阶段 abort，没有在拿到写锁后的 apply、commit、history sync 和 rollback 阶段注入 abort。

请保证 activation task 在拿到写锁后不可被外部取消，或实现可取消的 RAII compensation：取消时必须完成 rollback/recovery，并在状态未确认一致前保持 degraded/禁止聊天。ActivationLockGuard 不能仅靠 Drop 释放 gate；建议增加至少四个测试点：apply 后取消、SQLite commit 后取消、history sync 后取消、rollback 中取消，并断言 chat 仍被阻止或最终 backend/SQLite/history 三者一致。

当前验证
最新 head 本地验证：npm test 为 51 个测试文件、320 项通过；npm run build、npm run check:ipc、cargo fmt --check、git diff --check 通过。

第十次复审修复方案与验证结论：
- 采纳“Guard completed 终态防线”：ActivationLockGuard 引入 completed 标记与 runtime_degraded 防线，仅在 commit 与 recover_committed 成功达成一致终态时由后端显式调用 mark_activation_completed；若 task 在 apply、SQLite commit、history sync 或 rollback 中途被 abort，Drop 拒绝释放 gate（保持 activating == true 并置位 runtime_degraded），绝对阻止 chat 进入半完成撕裂状态。
- 新增全部 4 个专项测试（apply 后取消、SQLite commit 后取消、history sync 后取消、rollback 中取消），断言取消后 chat 保持阻止，且 recover_committed 后 backend/SQLite/history 强一致并安全放行。
- 本地完整回归验证：commands::characters::tests (36/36 passed), characters::activation_tests (35/35 passed), commands::chat::tests (61/61 passed), commands::bot::tests (20/20 passed), telegram::bot::tests (16/16 passed), npm test (320/320 passed), npm run build, npm run check:ipc, cargo clippy (--lib -- -D warnings), cargo fmt --check 全部通过。