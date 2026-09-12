# Review 收口与修复记录

日期：2026-09-12。基线提交：`8ea9cf4377cbec642e57ec3b9b3498a7ae13d893`。报告覆盖 R01–R24；模块详情见同目录 `R01.md` 至 `R24.md`。

## 本轮修复

- ChatPanel 会话/generation 绑定；清空历史后清除旧 conversation ref。
- Settings persona 保存按角色和打开 revision 隔离；桌宠用 IPC 回包收敛 completed 状态。
- Dream proposal 记录源记忆版本，审批按角色/状态/指纹校验并事务化；自动合并同步 embedding/hash。
- Backup 配置递归脱敏，导出使用同目录临时文件和原子替换；Skip 导入拒绝危险 memory ID 关系。
- LLM 多字段快照读取加共享门；Codex turn-start 取消增加终止 guard。
- TTS/音频顺序播放、generation 丢弃迟到音频；STT/摄像头异步授权和播放生命周期增加取消代次，native wake-word stale start 增加强制补偿 stop。
- MCP 连接 generation、锁粒度、stderr、pending、SSE reader、失败 shutdown；MCP 工具默认进入保守权限策略，远程调用和工具目录刷新均在释放 manager 锁后等待 client；删除服务器后立即刷新 action registry。
- MOD iframe invoke/action 权限和 document generation 校验；未映射的高副作用 action fail closed，QuickJS unload 清理 listeners。
- Webhook 缺 token fail closed、会话 source 隔离、超限 JSON recover、Bot 音频 MIME 按字节识别。
- Vitest 排除旧工作树；Nix 版本与当前 0.4.0 对齐。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| `npm test` | 78 文件、766 测试通过（`evidence/final-frontend.log`） |
| `npm run build` | 通过；保留分包体积警告（`evidence/final-build.log`） |
| `npm run check:ipc` | 171 个命令名称已注册（`evidence/final-ipc.log`） |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 888 通过（`evidence/final-cargo.log`） |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings` | 退出码 0；仅显示已知 ONNX build warning（`evidence/final-clippy.log`） |
| Registry/清理脚本定向测试 | 137 通过 |
| Vision stress | 5 通过（显式 `--features stress`，`evidence/final-vision-stress.log`）。 |
| AstrBot 隔离插件 pytest | 8 通过（`evidence/final-astrbot.log`）；默认 pytest 被本机第三方插件版本冲突阻塞。 |
| Windows NSIS | release exe 编译完成；NSIS 工具 hash 重下后全局超时，安装包未生成 |

## 未关闭项

R06 损坏 embedding 模型重试、摘要孤立任务恢复及 MemoryPanel 专项回归、R07 provider detached stream 即时取消、R10 native mic 消费任务 join、R11 Vision producer 代次、R17 SQLite 一致快照/完整摘要恢复、R22 可访问性和真实设备/渠道联调仍需后续批次。MCP 真实进程树、MOD WebView2 和 Windows 安装版仍需 fixture 验证。它们在对应模块报告中标为待办或环境阻塞，不以 mock、构建或静态检查代替实测。

初始四个用户未提交文件及其字节哈希见 `baseline.json`，本轮未覆盖或提交这些改动。`dist`、`target` 和 pytest 字节码均不作为交付内容。

## 独立复核（2026-09-12）
独立复核确认 MCP registry refresh 已串行化，iframe 不再暴露 dispatch_mod_event；native wake-word owner 已贯穿前端、Rust worker、producer 和转录任务，旧 owner 的 listener 与迟到事件均按身份丢弃。LayoutRenderer 现在从已安装 MOD manifest 透传 host-approved permissions；剩余为真实设备、渠道、安装版和跨平台 fixture 验证项。
Web fallback 亦已纳入 capture generation/owner 校验，旧录音帧和迟到转录结果不会影响新 owner。
独立复核结论：未发现剩余 Critical 或 Important；延期项均为真实环境验证或明确的测试缺口。
