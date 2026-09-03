本轮对 Kokoro Engine 的对话框模块进行了全方位、深度的源码级审计。为杜绝上下文污染与主观判断偏差，本次审查采用 「主审 Agent（全景静态与运行时行为审计）」+「独立复审 Sub-Agent（对抗性验证与盲区扫描）」 的双视角交叉复审机制。

审查范围与模块索引

核心交互视图：ChatPanel.tsx、ChatMessage.tsx、ConversationSidebar.tsx

状态与流水线：turn-state.ts、chat-streaming-state.ts、chat-character-sync.ts、chat-scroll-state.ts

输入与草稿：use-character-draft.ts、chat-input-layout.ts

前后端契约与 IPC：chat.rs、context.rs、kokoro-bridge.ts

第一部分：初审报告（全景缺陷与体验问题诊断）

一、 核心功能缺陷与逻辑漏洞 (Functional Defects)

1. 【P0 严重缺陷】重新生成 (onRegenerate) 引发数据库重复插入与上下文污染

代码位置：ChatPanel.tsx:L1335-L1379 与 chat.rs:L1206-L1215

缺陷分析： 在 onRegenerate(globalIndex) 中，计算 messagesToDelete = msgs.length - globalIndex（即只删除该助理气泡及之后的条目），调用 deleteLastMessages 从 SQLite 中移除了这几条助理消息。 紧接着前端调用 streamChat({ message: userMsg.text, ... })。但在后端 commands/chat.rs 的第 1206 行：

rust
if !request.hidden {
    state.add_message_with_metadata("user".to_string(), request.message.clone(), ...).await;
}

由于 hidden 默认为 false，后端将 userMsg.text 再次作为新的一条用户消息插入了数据库！

严重后果：前端此时 UI 只有 1 条用户消息，但数据库中已经有了 2 条完全相同的用户消息。当下一次重新加载会话、切换角色或重启应用时，用户的历史记录中会出现连续重复的用户提问，且发送给大模型的历史 Prompt 上下文也会携带重复问题。

2. 【P0 逻辑死锁】提前点击“停止生成”导致 UI 永久进入不可恢复的 Busy/Stopping 锁死

代码位置：ChatPanel.tsx:L481-L494、L836-L840、L938-L942

缺陷分析： 当用户发送消息后、大模型首字返回前（isThinking === true 期间），currentTurnRef.current 尚为 null。若此时用户点击停止按钮 handleStopGeneration：

cancelRequestedRef.current = true; setIsStopping(true);

后续后端事件 onChatTurnStart({ turn_id }) 到达，执行：

ts
if (cancelRequestedRef.current) {
    void requestTurnCancellation(turn_id);
    return; // 此时 currentTurnRef.current 从未被初始化赋值，直接退出了！
}

后端执行取消并触发 onChatTurnFinish({ turn_id, status: "cancelled" })。

unDone 监听器判断：

ts
const turn = currentTurnRef.current;
if (!turn || turn.turnId !== turn_id) return; // 因为 turn 是 null，直接忽略并返回！

严重后果：endTurnActivity() 永远不会被调用！isBusy 和 isStopping 将永久保持为 true，输入框、麦克风和发送按钮被永远置灰禁用，用户无法再发送任何消息，必须刷新或重启应用。

3. 【P1 数据孤岛】编辑消息 (onEdit) 纯前端内存修改，刷新/切换后失效且脱离 LLM 记忆

代码位置：ChatPanel.tsx:L1327-L1333与ChatMessage.tsx:L272-L305

缺陷分析：onEdit 仅执行了 setMessages(prev => ...) 变更了 React 本地 State。后端没有对应的持久化接口，既没有更新 SQLite 中的 conversation_messages，也没有在后续生成中反映修改后的内容。一旦用户切换角色、重新打开会话侧边栏或重启应用，编辑的内容彻底丢失。

4. 【P1 毁灭性操作风险】面板头部清空历史 (Trash2) 无二次确认弹窗

代码位置：ChatPanel.tsx:L1306-L1315、L1686-L1692

缺陷分析： 在会话侧边栏删除单个会话时有 confirm(t("chat.history.confirmDelete")) 防误触机制；但在聊天面板顶栏右上角的核心工具区，点击垃圾桶图标会立刻无条件调用 clearHistory() 并清空当前会话全部消息。在桌面端误触率极高，造成不可逆的数据丢失。

5. 【P1 交互阻断】工具调用待审批项 (pending_approval) 默认折叠，导致用户无感知卡住

代码位置：ChatMessage.tsx:L181、L395-L410

缺陷分析： 当助理触发高危工具并进入 pending_approval（等待用户 Approve/Reject）时，toolsExpanded 状态默认为 false。所有的“Approve”和“Reject”按钮全被隐藏在折叠项内部，界面呈现“停止输出且无响应”的假死状态，用户根本不知道模型正等待自己点击批准。

二、 核心使用体验与交互改进点 (UX & UI Optimization)

缺失 Markdown 格式化与代码高亮/复制：

当前在 ChatMessage.tsx:L307-L309 中直接使用 <div className="whitespace-pre-wrap break-words">{msg.text}</div>。大模型输出的代码块（```ts）、加粗、列表、表格均以丑陋的纯文本字符显示，且无法一键复制整段代码块。

历史消息缺失“复制全文”与“语音重播 (TTS)”入口：

消息悬停区仅有“从这里继续”、“编辑”、“重试”，缺少基础的“复制到剪贴板”按钮，用户必须手动框选文字；缺少“朗读当前消息”喇叭按钮，关掉自动朗读或想重听时无法触发。

向上滚动加载分页 (visibleCount) 缺少滚动锚定 (Scroll Anchoring)：

在 ChatPanel.tsx:L791-L793，当用户向上滚动到 < 100px 时，visibleCount 增加 20 条。旧元素前置插入后，未计算高度差并修正 scrollTop，导致可视区域剧烈跳动，甚至可能连续误触发二次加载。

缺失“回到底部”快捷悬浮按钮与新消息通知：

当用户向上翻阅历史消息时，模型若有新回复流式输出，用户既无法直观感知最新进度，也必须费力手动滑到底部。

语音识别 (STT) 粗暴覆盖用户已有草稿：

在 ChatPanel.tsx:L681-L697，sttPartialText 直接执行 setInput(sttPartialText)，若用户此前在输入框中已键入部分文本，会被语音转写直接抹去。

多模态相机帧（Camera Frame）“静默发送”，UI 气泡未展示：

在 ChatPanel.tsx:L1180-L1182，附带的实时相机画面仅追加到传给后端的请求体中，但用户消息气泡却只展示了普通上传图片，导致用户看不到当前发给 AI 的究竟是哪一张抓拍帧。

图片缺乏全屏查看/大图预览 (Lightbox) 与拖拽上传支持：

气泡里的图片被限制在 180×120 的 object-cover 内，无法点击放大；且当前输入区域仅支持点击按钮选图或剪贴板粘贴，不支持外部文件拖拽放入（Drag & Drop）。

会话侧边栏已有“Pin”状态展示，但缺失置顶操作功能：

ConversationSidebar.tsx:L223-L228
 虽有 Pin 已固定 的展示逻辑，但列表右侧只提供了重命名与删除按钮，用户无法在界面上手动置顶/取消置顶会话。

第二部分：独立复审 Sub-Agent 评估报告（对抗性验证）
复审机制声明：由独立视角重新审查上述初审结论，验证是否存在误报、已知架构约束或非真实问题，并挖掘初审未关注到的深层系统盲区。

一、 对初审结论的独立复核与校准
序号	初审缺陷项	复审验证结论	证据与校准说明
1	onRegenerate 消息重复插入	完全属实 (Confirmed)	验证了 Rust 端 stream_chat 逻辑，非 hidden 消息一定会写入 conversation_messages 表。前端只删除了旧助理回复，未删除上一条用户消息，重复插入 100% 发生，且会在下一次会话激活时污染上下文。
2	提前取消导致的 UI 永久挂死	完全属实 (Confirmed)	验证了 unTurnStart 的 early return 使得 currentTurnRef.current 保持为 null，而 unDone 与 unFailure 都把 turnId !== currentTurnRef.current?.turnId 作为门禁，导致 endTurnActivity() 彻底失联。严重等级确认为 P0。
3	onEdit 纯本地临时状态	属实但需场景界定 (Refined)	确认无后端同步。复审指出：AI 对话场景下“修改用户消息”通常伴随“重新由此分支生成”，单纯支持原地文本保存并不符合大模型多轮历史逻辑；因此最佳整改应为“修改并作为新分支重新发送”或“同步更新上下文”。
4	头部清空无二次确认	完全属实 (Confirmed)	与侧边栏的防护等级不一致，属于高危交互缺陷。
5	待审批工具默认折叠	完全属实 (Confirmed)	验证了 toolsExpanded 初始为 false，导致等待批准状态在 UI 上呈现静默断流。建议当且仅当存在 approvalStatus === "requested" 时强制自动展开。
二、 复审挖掘出的 4 个深层盲区 (Sub-Agent Blind Spots)
盲区 A：TTS 播放与新轮次对话的并发打断竞态 (TTS Audio Collision)：
在 

ChatPanel.tsx:L1171
 发送新消息时，代码并没有主动调用 audioPlayer.stop() 或打断正在进行的上一轮 TTS 合成。如果上一轮的长语音还在播放，用户发送了新问题并得到了新回复，两段语音或新回复的流式文本与旧语音会严重冲突重叠。
盲区 B：无障碍（a11y）与键盘导航严重不可达：
消息气泡上的操作按钮（复制、重试、从这里继续、编辑）在 

ChatMessage.tsx:L326
 中使用了 opacity-0 group-hover:opacity-100。对于使用键盘 Tab 键导航的无障碍用户，这些关键按钮永远不可见且无法获取焦点（缺少 focus-within:opacity-100）。
盲区 C：输入框高度缺乏动态弹性 (Fixed Height Card vs Auto-grow)：
目前输入框高度由全局配置 inputHeight（默认 88px）强行写死，即使只输入一个词也是大块空白；输入长篇提示词时不会随着文本行数自适应弹性微调（需要手动拉伸）。应引入最大/最小限制下的 textarea auto-grow 体验。
盲区 D：空白对话状态（Empty State）的体验缺失：
当会话没有任何历史消息时，整个中间区域完全是一片漆黑死寂，缺少当前角色的招呼语、人设特征介绍、或推荐的快捷提问气泡（Prompt Suggestions），新会话上手引导偏弱。
