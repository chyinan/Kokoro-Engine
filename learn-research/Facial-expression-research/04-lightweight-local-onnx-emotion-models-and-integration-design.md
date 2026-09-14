# 本地 ONNX 情感模型候选调查与高可用集成设计

> 状态：2026-09-14 再审后的设计基线，尚未实现或完成模型验收。本文取代原稿中的模拟批准、未经验证的性能排名和生产选型结论。
> 前置规范：[03 架构与统一契约](03-facial-expression-technical-selection-and-architecture-design.md)；基础算法：[05 L1 算法设计](05-l1-algorithm-driven-core-foundation-architecture-and-moat.md)。字段、时序与仲裁以 03 为准。

## 1. 最终选型结论

L2 是可选增强，默认不安装，当前没有获准生产分发的模型。先交付不依赖模型的 L1，再在目标硬件上试验 CPU 推理；模型缺失、加载中、不可用或超时不妨碍聊天、TTS 与基础表情。

模型识别的是输入文本表达的情绪线索，不能据此证明用户真实心理状态，也不能直接决定角色应作何回应。用户感知证据经回应策略生成角色目标；助手生成的情绪标签保留独立来源与时序。L2 不等待或阻止 L1，也不直接写 Live2D 参数。

## 2. 候选调查与证据边界

以下是模型发布者页面与文件清单调查，**不是本项目横向实测**。链接的 `main` 会变化；实施前必须固定完整 revision、确切文件及哈希。页面元数据中的许可证不替代对权重、基础模型、分词器、数据及分发文件的逐项许可记录。

| 候选 | 本轮核实的公开信息 | 裁决 |
| --- | --- | --- |
| `minuva/MiniLMv2-goemotions-v2-onnx` | 模型卡描述约 30M 参数、英语 GoEmotions，示例使用 sigmoid 多标签后处理；量化 ONNX 文件列表约 30.5 MB，tokenizer JSON 约 2.11 MB；页面标注 Apache-2.0。 | 英语实验候选；原稿 22.6 MB、22.7M、9.2 ms 和 36 MB 常驻内存均撤回。须验证标签顺序、输出张量与实际资源。 |
| `Johnson8187/Chinese-Emotion-Small` | 模型卡基于 mDeBERTa-v3-base 系列，页面约 0.3B 参数、MIT，列出 8 类中文语调；训练说明为约 4,000 条繁体中文样本。 | 中文质量对照候选，不视为 25M 微模型。原稿 Apache-2.0、24.8 MB ONNX 和 13.5 ms 无依据；导出、量化、简繁与口语泛化均待验证。 |
| `onnx-community/tanaos-emotion-detection-v1-ONNX` | 转换卡描述 Multilingual-MiniLM 骨干，语言栏目为 English；列表含 joy、anger、fear、sadness、surprise、disgust、excitement、neutral 共 8 类，标注 MIT。卡内还出现 12 类措辞，存在自相矛盾。 | 不承诺 50+ 语言有效覆盖。先固定并核对 config、输出形状及转换产物；原稿 7 类与精确性能数值撤回。 |
| `boltuix/NeuroFeel` | 发布者自述约 7M、13 类、约 25 MB；标注 Apache-2.0。 | 元数据与实际权重必须交叉验证。原稿 7 类、7.8 MB ONNX、3.8 ms 不可作为决策依据。 |
| `SamLowe/roberta-base-go_emotions-onnx` | 保留为英文对照调查条目。 | 本轮未完成固定产物复核，不沿用原稿参数、许可证、体积或速度断言；先完成产物审计再进入实验。 |
| SenseVoiceSmall | 官方仓库描述 ASR、语言识别、语音情绪与声音事件；发布检查点覆盖中、粤、英、日、韩，列举笑声、哭声、咳嗽等事件。 | 语音另立实验门槛；不承诺叹气识别或输入后瞬时输出。不把代码许可证自动套用到任何权重或 ONNX 转换包。 |
| emotion2vec 系列 | 官方项目以语音情绪表征及下游模型为研究对象。 | 原稿 `Alibaba-DAMO/emotion2vec_seed` 的确切检查点、ONNX、VA 输出与性能未建立证据链；暂不列入可安装选型。 |

依据：[minuva 模型卡](https://huggingface.co/minuva/MiniLMv2-goemotions-v2-onnx)、[minuva 文件列表](https://huggingface.co/minuva/MiniLMv2-goemotions-v2-onnx/tree/main)、[中文模型卡](https://huggingface.co/Johnson8187/Chinese-Emotion-Small)、[tanaos 转换卡](https://huggingface.co/onnx-community/tanaos-emotion-detection-v1-ONNX)、[NeuroFeel 模型卡](https://huggingface.co/boltuix/NeuroFeel)、[SenseVoice 官方项目](https://github.com/QwenAudio/SenseVoice)、[emotion2vec 官方项目](https://github.com/ddlBoJack/emotion2vec)。

没有资料证明这些候选在本项目机器上满足 30 MB 下载、40 MB 常驻、10–25 ms 延迟、反讽准确识别或 60 FPS。模型大小、应用内存增量、峰值工作集和端到端响应必须分开测量。中文候选过大时，保持 L1 可用并继续调查，不为凑预算虚构量化结果。

## 3. 领域划分

建议划分职责而非把一切塞进 `emotion_onnx.rs`：

- 生命周期 manager：安装/启用意图、串行状态变更、运行代次、健康状态和请求接纳。
- inference runtime：独占 Session 与 Tokenizer、后台加载和预热、有界执行与结果校验。
- package store：可信 manifest、下载暂存、文件边界、版本目录、提交与可重试清理。
- InferenceOutputAdapter：输入张量、标签顺序、单标签或多标签后处理、校准及证据归一化。

命令层只做参数验证与领域调用，前端桥接遵循 03 的统一契约。不要把模型、驱动参数、AU、角色性格、下载协议一起暴露给调用方；业务层不持有 Session 或文件句柄。

## 4. 执行、超时与负载边界

采用专属 worker 独占 Session，进程内全局最多 **1 个在途推理 + 1 个最新待处理请求**，包括不同窗口和角色。新请求替换旧 pending；停用或切换使旧代次结果失效。工作未开始时也要检查有效期，过期请求不进入推理。

**50 ms 是结果接受预算的初始实验值，不是物理执行时限。** 遵循 03，从用户提交开始计量端到端接受预算，包含 IPC、排队、分词与执行；后端只按剩余预算使用自己的单调时钟限时，接收端最终拒收过期结果，不比较跨进程绝对时钟。超过预算即返回可区分的不可用原因，调用方继续 L1；即便时钟调度有抖动，也不得接受过期结果。执行槽仅在真实任务退出后释放，不能超时后不断新建推理任务。

Tokio 明确说明已经开始的 `spawn_blocking` 任务无法被 abort；`timeout` 放弃等待不能终止原生 ONNX 计算。若在 async future 内直接执行阻塞推理，连超时检测本身也可能推迟。因此不能承诺“50 ms 强杀”。[Tokio 官方说明](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)

持续超时进入冷却，停止新派发并丢弃 pending；半开恢复须等待原任务确实退出，再按有限次数、退避策略发送健康探测。疑似永久挂起时将该 runtime 标为不可用，L1 继续；不尝试释放仍在使用的 Session。进程内线程也不能隔离 native 崩溃或保证应用退出时限。若产品要求故障后自动恢复、硬停止或崩溃隔离，必须另行验收独立进程 worker、终止后确认退出、子进程资源限制和重启策略，才能作相应承诺。

CPU 实验先限制算子内/算子间线程数为 1，记录 execution mode、图优化、实际 ORT 版本及 provider。线程数限制可减少争抢，但不等于绑核、专属 CPU、低优先级或保证音频无欠载。共享 STT/TTS/embedding 的负载也必须纳入测量。[ONNX Runtime 线程管理](https://onnxruntime.ai/docs/performance/tune-performance/threading.html)

## 5. 输入、输出与校准

在 normalization、分词及复制大字符串之前应用字节边界，并按 03 规定的输入范围裁剪。候选模型的 token 上限需包括特殊 token，校验实际输入维度、类型和模型允许范围。统一采用 03 的规范化前 2048 UTF-8 bytes、NFC 后 512 Unicode scalars 有界前缀，L2 再以最多 64 tokens（含特殊 token）作为待验证上限。撤回原稿末尾 128 字符方案；截断丢失否定或引用上下文时弃权，不维持多套并行默认值。

加载后用有代表性的有效输入后台预热；预热完成前报告 loading/unavailable，保持 L1。空字符串预热不能证明所有输入路径已预热，也不能承诺首次推理额外开销为零。

每个 adapter 依据固定 config 定义 sigmoid 多标签或 softmax 单标签，禁止把两者混作和为 1 的分布。输出检查标签数量、NaN/Infinity、范围与 schema；VA/VAD 若由标签映射生成，应标明为设计映射，不能宣称模型直接回归了这些维度。

Emoji/颜文字可提供符号证据，但不同 tokenizer 不一定产生 `[UNK]`，也不能保证符号总表达字面情绪。符号与模型的融合须在一致空间校准；概率加权应称概率融合。固定 `T=1.2`、混合系数 0.7 和置信阈值 0.60 均撤回为未验证候选；温度缩放不改变单标签 logit 排序，不能修复反讽方向错误。独立校准集拟合后，再在保留盲测集验证准确度、覆盖率与拒判效果。

## 6. 安装、信任与卸载

通过 manager 的单一串行所有者协调安装、启停、替换和卸载；状态区分用户启用意图、包是否存在、runtime 是否 ready、清理是否完成。加载或下载失败不能把“已安装且启用”误报为可推理。

安装前校验 manifest 的来源身份、完整 revision、兼容版本、文件清单、大小和 SHA-256。哈希只证明与 manifest 一致，不能授予官方身份。官方标签必须维持仓库既有的 source-bound trust；自定义来源保留社区/不受信任语义与明确确认流程。

权重、external data、tokenizer、config、标签映射与校准文件作为同一不可变版本提交。禁止清单外路径、绝对路径、`..` 逃逸、symlink/reparse 绕过及不受控 external-data 引用；对单文件、总大小、解压大小、文件数设置上限。包不能带任意 DLL、自定义算子或自动执行的 remote code。ONNX 是不可信解析输入，不因 SHA-256 正确就获得进程安全保证。

下载进入受控 staging，完整校验后原子提升到版本目录，再提交 active 指针；失败保留旧可用版本或无模型状态。恢复逻辑处理断网、断电、磁盘不足、重复请求与进程重启。用户数据、对话、记忆、角色覆盖及应用凭据不属于模型卸载范围。

卸载先禁止新请求、使代次失效并清空 pending，再等待 runtime 明确释放所持资源；不得靠 sleep 30 ms 猜测句柄已排空。安全删除或重命名仍可能失败，保持逻辑停用、记录 `pending_cleanup` 和可重试原因，不谎报磁盘已清理，也不阻止基础聊天。Windows 的共享删除权限会影响删除及重命名，不能保证锁定文件“总能 rename”。[Windows CreateFileW 共享访问说明](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)

不宣称 ORT 在所有版本默认 mmap 权重，不要求 Session 释放后进程内存或显存归零。清理验证以句柄释放、目录操作结果及重复循环后的稳定内存趋势为准，并区分 runtime allocator 缓存与泄漏。

## 7. 模型准入与实现验收

| 门槛 | 必备证据 |
| --- | --- |
| 产物 | 固定 revision/哈希、实际字节数、许可证链与归属、export/quantize 命令、opset、tokenizer/config 配对、目标 ORT/provider 兼容性。 |
| 质量 | 与 L1 对照，按语言、标签、反讽、否定、混合语言、符号、引用文本分层；独立人工盲测与校准集隔离；报告 macro-F1、各类召回、拒判覆盖率和有害方向误触率。 |
| 性能 | OS/CPU/内存、电源模式、线程、输入分布、冷/热启动、端到端 p50/p95/p99、峰值与稳态内存；并发音频/渲染负载下的欠载与帧时间。所有数值在实验前均为待定。 |
| 可靠性 | 排队饱和、超时后槽位不泄露、跨代次晚到、加载失败、停用/卸载竞态、连续失败恢复、worker 挂起及清理失败；真 native 运行与伪造 sleep 测试分开。 |
| 存储 | 损坏清单、越界 external data、超限包、重解析路径、更新中断、重复安装、Windows 文件占用及重启恢复。 |
| 兼容性 | Rust 注册、bridge 类型与消费者一起检查；`check:ipc` 只覆盖命令注册，不证明事件字段或运行语义正确。 |

质量或资源未通过时不默认启用模型；允许实验选型结论为“暂无适合生产候选”。本文完成的是设计约束收敛，不代表已交付上述测试或高可用保证。
