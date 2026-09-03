第八次复审：仍然 REQUEST_CHANGES
本次基于最新 head f91eebbeb73bbb28f9043e1613f656e38e75c6de。上一轮指出的 activation 并发窗口已经新增 ActivationGate 和对应测试，但闸门目前没有覆盖所有实际聊天入口，仍有一条 P1。

P1：并发闸门只保护主 stream_chat 的部分流程，Bot/LINE/Discord/QQ 仍可在 activation 期间污染会话
涉及：AIOrchestrator::ActivationGate、commands/chat.rs、commands/bot.rs 和 commands/bot.rs。

有两个相互独立的问题：

stream_chat 在入口先持有 _chat_turn_guard，但之后调用的 compose_prompt 又重新调用 enter_chat_turn。如果 activation 在入口 guard 已取得后开始，activating flag 会变成 true，二次 enter_chat_turn 立即失败；此时用户消息已经在 commands/chat.rs#L1199-L1215 写入 history/SQLite，最终得到“用户消息已保存但本轮失败”的半个 turn。已经持有有效 ChatTurnGuard 的调用不应再次按新请求检查 gate，建议把 compose 的纯逻辑与 gate 分开，或让 guard 可重入/传递。

generate_bot_reply 在 commands/bot.rs#L893-L895 先 add_message，但没有持有 ChatTurnGuard；它直到 compose_prompt 内部才短暂尝试读锁，且读锁在 compose 返回后就释放。LINE、Discord、QQ 等调用方传入 None，没有像 webhook 那样的 activation 检查。这样 activation 期间可以先写入 user message 后在 compose 处失败；更危险的是 activation 可以发生在 compose 完成到 assistant add_message 之间，随后共享的 current_conversation_id 已经属于新角色，旧角色的 assistant reply 可能被写进新角色会话。

请让每个完整 chat turn（包括 user message、compose、LLM/tool loop、assistant persistence）持有同一个 read guard，并避免在已持有 guard 的路径中重复做非重入的 try-read；或者统一把所有 bot/Telegram/QQ/Discord/Webhook 入口路由到一个带 gate 的 turn runner。请增加至少一个非 webhook bot 的并发测试，以及主 stream_chat 在 activation flag 变为 true 后仍能完成已开始 turn 的测试。

当前验证
本地验证全部通过：
- 针对性并发测试通过：test_active_stream_chat_turn_completes_when_activation_starts_concurrently, test_non_webhook_bot_turn_during_activation_is_blocked_at_entry_without_message_pollution
- 核心模块测试全部通过：commands::characters::tests (31/31), characters::activation_tests (35/35), ai::context::tests (25/25), commands::chat::tests (61/61), commands::bot::tests (20/20)
- npm test 为 51 个测试文件、320 项全部通过；npm run build、npm run check:ipc、cargo fmt --check、git diff --check 全部通过。