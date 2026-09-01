# 进度追踪：接入 Codex Runtime Provider

> 创建时间：2026-09-01 | 状态：实现完成，等待桌面端重启后的人工验证

## 目标
审计并扩展 Kokoro-Engine 的 provider 架构，通过本地 `codex app-server --listen stdio://` 复用用户现有 Codex runtime、provider、auth 和模型配置。

## 成功标准
- 保留现有 API Key、OpenAI-compatible、流式、工具调用和视觉能力。
- Codex runtime 负责 authentication、token refresh、`config.toml`、custom endpoint 和 backend 变化。
- Kokoro 只负责检测/启动 app-server、JSON-RPC stdio、thread/turn、流式事件、取消和失败恢复。
- 前端明确标记 `Codex Runtime — Experimental`，不显示或持有 Codex token。
- 现有 API-key/native provider、Memory/RAG/MCP/tool 系统保持不变。
- 增加针对进程生命周期、协议转换、streaming、失败和兼容性的测试。
- 更新文档并记录 app-server 协议、权限与版本限制。

## 已读文件
- `AGENTS.md` — 项目结构、构建/测试命令和跨域契约。
- `C:\Users\chyinan\.codex\RTK.md` — shell 命令使用 `rtk` 前缀。
- `C:\Users\chyinan\.agents\skills\focused-problem-solver\SKILL.md` — 检查点、进度和 Summary 工作流。
- `summary/llm-provider-integration.md` — 当前 LLM 配置、provider、IPC 和前端设置边界；现有 `openai` 保持 Chat Completions 语义。
- `src-tauri/src/llm/llm_config.rs` — LLM provider 配置直接包含可序列化的 `api_key`，扩展字段为 `extra`。
- `src-tauri/src/llm/provider.rs` — `LlmProvider` 暴露 provider-neutral chat/stream/tool API；OpenAI provider 使用 Bearer API key。
- `src-tauri/src/llm/service.rs` — provider map 按 `provider_type` 构建，当前支持 OpenAI/Responses/Anthropic/Ollama/llama.cpp；配置更新会重建并写入磁盘。
- `src-tauri/src/commands/llm.rs` — 仅有 get/save/test 配置及模型发现命令，没有账户认证命令。
- `src-tauri/Cargo.toml` — 有 reqwest、tokio、tauri-plugin-opener 和 async-openai，无 keyring/OAuth 依赖。
- `src-tauri/src/config.rs` — 通用 JSON 配置读写和环境变量 API key 解析；LLM 配置没有独立安全存储。
- `src-tauri/src/lib.rs` — 启动时从 app data 读取 `llm_config.json` 并注册 LLM Tauri 命令。
- `src/lib/kokoro-bridge.ts` — 前端 bridge 暴露可含 `api_key` 的 LLM 配置和 get/save/test 调用。
- `src/ui/widgets/settings/ApiTab.tsx` — 单一 API-key-centric provider 编辑界面，已存在 Responses provider 选择和连接测试。
- `src/ui/widgets/settings/ApiTab.provider-types.test.ts` — 仅覆盖 provider 类型默认值和 Responses provider 创建。
- `src/ui/locales/*.json` — 设置文案包含 API key/provider 类型，尚无账户登录状态文案。
- `docs/architecture.md`、`docs/API specification.md`、`docs/quick-start.md`、`docs/openai-responses.md` — 文档将 LLM 配置描述为 JSON/provider API key，明确备份不包含 provider credentials。
- `src/features/onboarding/provider-setup.ts` — 前端 provider setup 通过 bridge 直接发现/保存/测试 provider，OpenAI-compatible discovery 使用 API key。
- `src/features/onboarding/provider-setup-core.ts` — provider 类型和默认值的纯逻辑；现有类型集合为 openai/openai_responses/anthropic/ollama/llama_cpp。
- `src-tauri/src/llm/codex_runtime_protocol.rs` — Codex app-server JSONL/RPC 请求、thread/turn 输入、dynamic tools、事件和 model/list 的纯转换。
- `src-tauri/src/llm/codex_runtime.rs` — Codex app-server 进程、初始化、重启、超时、流式事件、工具请求和 Windows binary detection 的 imperative shell。
- `src/ui/locales/en.json`、`src/ui/locales/zh.json` — Codex Runtime Experimental 与模型列表可见 UI 文案。
- `docs/codex-runtime.md`、`summary/codex-runtime-provider.md` — 使用说明、协议假设、权限边界和实验性限制。
- `src/features/onboarding/provider-setup.test.ts` — 覆盖 provider setup 标准化、模型发现和保存/测试 bridge 适配。
- `src-tauri/capabilities/default.json`、`src/ui/widgets/settings/AboutTab.tsx` — 现有 opener 权限仅用于打开外部链接，没有 OAuth callback 能力。
- `src-tauri/src/commands/backup.rs` — `llm_config.json` 会被备份/恢复，但 inspection 标记 provider credentials 不包含在专门凭据范围内；当前文件本身可含明文 API key。
- `C:\Users\chyinan\.codex\skills\.system\openai-docs\SKILL.md` — OpenAI 资料要求以官方文档为主，并限制来源域名。
- `C:\Users\chyinan\.agents\skills\agent-reach\SKILL.md`、`C:\Users\chyinan\.agents\skills\agent-reach\references\dev.md` — 互联网/GitHub 研究路由；本机 `agent-reach` 命令不可用。
- `C:\Users\chyinan\.agents\skills\researching-on-the-internet\SKILL.md` — 外部 API 设计需以当前官方资料交叉验证。

## 当前进度
已完成最小 Codex Runtime Provider/CLI Bridge 和设置页模型发现接入；OAuth 方向保持停止。

## 下一步
重启 Tauri 桌面端，确认设置页从 `Not installed` 变为可检测状态并能显示模型按钮；随后用用户当前 Codex 登录状态验证正常聊天、streaming、工具调用、logout/失效 credential 的 graceful failure。Kokoro 不负责 logout Codex 账号，因此“logout”仅适用于切换/停用 Kokoro provider；Codex 账号退出仍由 Codex CLI 管理。

## 发现的关键信息
- 现有 provider 已有 provider-neutral `LlmProvider` 和 `providers[].extra` 扩展点。
- `provider_type: "openai"` 必须保持现有 Chat Completions 兼容语义。
- provider 协议细节当前已隔离在 Rust LLM 层，聊天命令消费中性流事件。
- 现有 API key 是普通 `llm_config.json` 的明文可序列化字段；备份逻辑不打包该文件，但本地配置仍是明文。
- 没有现成 OAuth 回调、keyring、深链或本地回调服务器；只有 `tauri-plugin-opener` 可打开外部 URL。
- 官方 OpenAI 文档确认 Platform API 使用 Bearer API key，但没有找到 ChatGPT/Codex 订阅 transport 的公开稳定 API 文档；该部分必须保持实验性并隔离。
- 聊天命令仅依赖 `LlmProvider` 的中性 stream/tool 事件，因此账户 provider 可在 LLM 层隔离，不需要 Agent/Memory/RAG/MCP/TTS/STT 改写。
- 模型发现目前有前端直连 `/models` 的 API-key 路径；账户 provider 不能复用这条路径，应由受信任 Rust backend 处理。
- 要实现真实 ChatGPT 登录，必须确定 OAuth client/redirect 和订阅 transport 的来源；官方文档未提供可验证契约。
- 用户已明确改为优先复用官方 Codex app-server，不由 Kokoro 管理 OAuth，也不读取 `~/.codex/auth.json`。
- 官方 app-server 当前通过双向 JSONL RPC 暴露 `initialize`、`thread/start`、`thread/inject_items`、`turn/start`、`turn/interrupt`、`model/list` 以及流式 item/turn 事件；当前本机 `codex-cli 0.151.0` 的实测 `model/list` 返回 7 个可用模型。
- bridge 使用临时、只读、无 environment access 的 thread，并以空 base/developer instructions、禁用 Codex MCP/skill/plugin/shell/app 等配置尽量接近 inference-only；Kokoro 工具通过 experimental `dynamicTools` 注入，`item/tool/call` 返回 host-handled 并由 Kokoro `chat.rs` 执行。
- Rust `cargo check`、`cargo clippy --lib -- -D warnings`、`cargo test --lib --no-run` 均通过；运行测试二进制在当前 Windows 环境被既有 `onnxruntime.dll` 触发 `STATUS_ENTRYPOINT_NOT_FOUND`，不是 Codex bridge 断言失败。
- 前端 `npm test`（provider 目标测试）、`npm run build` 和 IPC command registry check 均通过；模型列表此前仅写入隐藏 `datalist`，现改为 Codex Runtime 卡片下的可见模型按钮。
- 用户补充确认本机 Codex CLI 的 WebSocket→HTTPS fallback 是代理/VPN 环境中的正常路径；因此此前 app-server turn 的 30 秒统一 RPC timeout 不能作为 protocol deadlock 证据。已将 `turn/start` ack、首输出、stream idle 和 total turn 分开，并记录非敏感 transport 状态。
- 使用与 Codex CLI 配置等价的只读临时 app-server 参数、五分钟以上等待窗口重跑最小 text turn：`thread/start` 与 `turn/start` ack 成功，约 126 秒后收到 `turn/completed: completed`。这证明此前失败主要是过早的外层等待/错误事件处理，不能归因于 JSON-RPC deadlock；本次观测未输出原始 warning 或凭据。
- 用户实际重启后发现发送消息瞬间收到 `-32600: dynamic tool name is reserved: mcp__time__convert_time`；根因为 MCP 工具原名直接传给 Codex dynamicTools。已改为 `kokoro_tool_<index>_<safe-name>` 别名，并在返回事件进入 `chat.rs` 前恢复原始 Kokoro/MCP 工具名。
- 当前 `codex-cli 0.151.0` 已实测接受 `kokoro_tool_0_mcp_time_convert_time` dynamic tool alias，`thread/start` 返回成功；原始 `mcp__time__convert_time` 不再发送给 Codex。
- 用户随后发现发送消息瞬间出现 `-32600: items[0] is not a valid response item: missing field type`；根因为 `thread/inject_items` 收到缺少 Responses `type` 的 `{role, content}` 历史项。当前 bridge 已将普通历史消息转换为 `{type: "message", role, content: [{type: "input_text", text}]}`，并保持已有 `function_call`/`function_call_output`/`reasoning` item 不变；当前 Codex CLI 实测该注入形状被接受。
- 用户发现每次点击全局保存都会重复追加角色名/用户称呼；根因为前端把 resolved system prompt 当成原始角色 persona 保存。已删除这条回写路径，角色卡 persona 只保存原文，运行时字段在激活时组合；新增 `src/ui/widgets/character-persona.ts` 对历史污染做一次可识别片段清理，并在当前角色被清理后刷新运行时。
<!-- 解决问题过程中发现的重要线索、根因、相关逻辑 -->
