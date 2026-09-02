第七次复审：仍然 REQUEST_CHANGES
本次基于最新 head 6fa871242334202df03baed1753ff87dcf2db616。上一轮的两条 recovery 问题已经修复：degraded 标记、restore/clear 错误诊断和故障测试均已补上。本轮只发现一条属于 PR 新增 activation 一致性协议的并发问题。

P1：activation 的多步 apply → SQLite commit → history sync 期间没有阻止并发聊天
涉及：activation.rs、commands/characters.rs 和 commands/chat.rs。

ActivationCoordinator 的 mutex 只串行 prepare/commit，不会阻止 stream_chat、主动消息、Webhook 或 Telegram 同时进入。commit 在 backend.apply 中先逐项修改 system prompt、character id、current conversation，再提交 SQLite，最后才读取并替换 in-memory history；而 stream_chat 只检查 runtime_degraded，正常 activation 进行中并没有 degraded/busy 状态。

因此并发请求可以落在这个窗口内：它可能使用新角色的 system prompt + 旧角色的 history，或看到新 conversation id 但 history 尚未同步。sync_orchestrator_history 随后 reset history 还可能把该并发请求刚加入的 in-memory 消息清掉，虽然数据库消息已经写入。主动消息和辅助窗口不受设置弹窗遮挡，实际存在触发机会。

请增加 activation-in-progress 的共享闸门，或让聊天读取不可变的已提交 runtime/history snapshot；至少要在 apply 开始到 history sync 完成期间阻止新 chat turn，并在失败/回滚/成功路径释放闸门。请补充并发测试，验证 activation 期间 chat、proactive、webhook 请求不会读取跨角色混合上下文，也不会被后续 history reset 丢失。