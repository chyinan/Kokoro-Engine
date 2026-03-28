# Emotion System

这份文档说明当前项目里的情绪识别、情绪状态更新、持久化和消费链路。

## 1. 情绪识别识别的是什么文本

当前情绪识别吃的是 **本轮 assistant 的最终输出文本**，不是用户输入。

入口在：
- [src-tauri/src/commands/chat.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/commands/chat.rs)

关键逻辑分两段：

1. 先正常生成完整回复
2. 回复结束后，再把最终 assistant 正文裁剪成适合分类的片段，送给情绪分类器

也就是说，分类器输入是：
- `full_response` 的清洗/裁剪版本

不是：
- `request.message`
- 完整历史拼接文本
- system prompt
- tool 结果

这样 emotion 表示的是：
- “角色这一轮实际表现出来的情绪”

而不是：
- “用户这句话的情绪”

### 1.1 长回复怎么处理

本地模型本身有输入长度限制：
- [src-tauri/src/ai/emotion_classifier.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/emotion_classifier.rs)
- `MAX_SEQUENCE_LEN = 128`

所以当前不会把超长 assistant 输出整段原样送进去，而是：
- 先清洗标签和空白
- 如果文本较长，只取：
  - 前 160 个字符
  - 后 160 个字符
  - 中间用 `...` 连接

这样做的目的是：
- 避免只看开头导致后半段情绪丢失
- 同时不把超长全文硬塞给本地分类器

## 2. 当前情绪分类器

文件：
- [src-tauri/src/ai/emotion_classifier.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/emotion_classifier.rs)

### 2.1 本地模型

默认使用本地 ONNX 模型：
- 仓库：`AdamCodd/tinybert-emotion-balanced`
- 默认模型文件：`onnx/model_int8.onnx`

首次缺失时会自动下载到本地缓存目录：
- `~/.local/share/com.chyin.kokoro/models/models--AdamCodd--tinybert-emotion-balanced`

辅助文件：
- `tokenizer.json`
- `config.json`

### 2.2 模型标签

当前内部使用的标签体系是：
- `sadness`
- `joy`
- `love`
- `anger`
- `fear`
- `surprise`
- `neutral`

注意：
- ONNX 模型本身通常只输出前 6 个标签
- `neutral` 是项目内部保留的基线状态，用于默认/衰减回落

## 3. raw_mood 是怎么来的

分类器输出是：
- `label`
- `score`
- `raw_mood`

`raw_mood` 不是模型直接给的，而是由标签和置信度推出来的：

文件：
- [src-tauri/src/ai/emotion_classifier.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/emotion_classifier.rs)

规则：
- `joy / love / surprise -> 0.5 + score * 0.45`
- `sadness / anger / fear -> 0.5 - score * 0.45`
- 其他 -> `0.5`

所以：
- 正向标签把 mood 往 `1.0` 拉
- 负向标签把 mood 往 `0.0` 拉
- `neutral` 留在 `0.5`

## 4. 情绪状态机怎么更新

文件：
- [src-tauri/src/ai/emotion.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/emotion.rs)
- [src-tauri/src/ai/context.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/context.rs)

真正的状态更新入口是：
- `AIOrchestrator::update_emotion(raw_emotion, raw_mood)`

内部调用：
- `EmotionState::update(raw_emotion, raw_mood)`

### 4.1 状态包含什么

`EmotionState` 主要包含：
- `current_emotion`
- `mood`
- `accumulated_inertia`
- `personality`
- `history`

### 4.2 更新规则

`EmotionState::update()` 的逻辑核心：

1. 根据角色 personality 计算有效惯性
- `effective_inertia = personality.inertia + accumulated_inertia * 0.1`

2. 先平滑 mood
- 新 mood 不会直接跳到 `raw_mood`
- 而是旧值和新值按惯性混合

3. 再决定 emotion label 是否切换
- 如果 `raw_emotion == current_emotion`
  - 累积惯性继续上升
- 如果不同
  - 只有当 mood 差值足够大，或当前累积惯性很低时，才会切换
  - 否则会抵抗切换，维持旧 emotion

4. 再用 `expressiveness` 调整外显强度
- 内部 mood 和对外表现的 mood 不是完全相同

一句话概括：
- 它不是“分类器输出什么就立刻变成什么”
- 中间有一层带惯性的状态机

## 5. 角色 personality 怎么影响情绪

文件：
- [src-tauri/src/ai/emotion.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/emotion.rs)

`EmotionPersonality` 三个关键参数：
- `inertia`
- `expressiveness`
- `default_mood`

来源：
- 从角色 persona 文本里按关键词解析

影响：
- `inertia`
  - 决定情绪是否容易被扳动
- `expressiveness`
  - 决定表情/TTS/外显 mood 强度
- `default_mood`
  - 决定长期回落到什么基线

## 6. 什么时候会恢复或衰减

### 6.1 聊天时更新

每次 `stream_chat` 完成一轮最终输出后：
- 先拿 `full_response` 做清洗和裁剪
- 再调用本地情绪分类器
- 分类成功时才调用 `update_emotion()`

如果本地模型不可用、下载失败或推理失败：
- 这轮不会更新 emotion
- 系统保留上一状态，继续靠 heartbeat 衰减

### 6.2 heartbeat 时衰减

文件：
- [src-tauri/src/ai/heartbeat.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/heartbeat.rs)

heartbeat 每 10 秒做一次：
- `emotion.decay_toward_default()`

所以如果后续没有新的刺激：
- mood 会慢慢回到 `default_mood`
- 当 mood 足够接近默认值时，`current_emotion` 会慢慢回到 `neutral`

## 7. 情绪状态在哪里被持久化

文件：
- [src-tauri/src/ai/context.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/context.rs)

落盘文件：
- `~/.local/share/com.chyin.kokoro/emotion_state.json`

规则：
- 只有 `memory_enabled=true` 时才保存和恢复
- 关闭 memory 时不会继续持久化 emotion state
- 关闭 memory 时还会删除已有的 `emotion_state.json`

此外 heartbeat 还会在 memory 开启时把 snapshot 存进 memory 系统：
- `emotion_snapshots`

## 8. 情绪状态现在被谁使用

### 8.1 prompt 注入

当前聊天 prompt **不再注入 emotion 状态**。

也就是说，这些内容已经不再进入聊天主 prompt：
- `emotion.describe()`
- emotion event 的 system instruction

emotion 现在主要作为：
- 后台状态
- 演出/行为输入

而不是对主聊天模型的前置硬约束

### 8.2 打字速度

文件：
- [src-tauri/src/ai/typing_sim.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/typing_sim.rs)

当前使用：
- `emotion.current_emotion()`
- `expressiveness`
- 用户消息长度
- 是否问句

例如：
- `surprise` 更快
- `sadness` / `fear` 更慢

### 8.3 Live2D 微表情帧

文件：
- [src-tauri/src/ai/expression_driver.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/expression_driver.rs)

heartbeat 每次会基于：
- `current_emotion`
- `mood`
- `trend`
- `expressiveness`

生成：
- `expression-frame`

用于驱动 Live2D 微表情参数。

### 8.4 情绪事件

文件：
- [src-tauri/src/ai/emotion_events.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/emotion_events.rs)
- [src-tauri/src/ai/heartbeat.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/heartbeat.rs)

heartbeat 会根据：
- 当前 `mood`
- 最近 `mood_history`

触发：
- `emotion:ecstatic`
- `emotion:very_happy`
- `emotion:very_sad`
- `emotion:sulking`
- `emotion:mood_swing`

这些事件会：
- 发前端 `emotion-event`
- 再通过 semantic cue map 映射到 Live2D cue

### 8.5 Idle 行为

文件：
- [src-tauri/src/ai/idle_behaviors.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/idle_behaviors.rs)

当前按 `mood` 决定 idle 倾向：
- 低 mood：更容易 `Sigh`
- 高 mood：更容易 `Hum` / `Stretch`
- 中间：`LookAround` / `Fidget`

### 8.6 主动说话概率

文件：
- [src-tauri/src/ai/initiative.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/ai/initiative.rs)

当前用 `mood` 调整主动触发概率：
- 心情越好，越容易主动开口
- 心情越差，越偏安静

### 8.7 TTS 调制

文件：
- [src-tauri/src/tts/emotion_tts.rs](/home/initsnow/projects/Kokoro/Kokoro-Engine-live2d-compat/src-tauri/src/tts/emotion_tts.rs)

当前基于：
- `emotion`
- `mood`

调节：
- speed
- pitch

例如：
- `joy` 更快、更高
- `sadness` 更慢、更低

## 9. 目前这套 emotion 的边界

现在的输入源是：
- 本轮 assistant 最终输出的裁剪文本

所以它的特点是：
- 更符合“角色状态”语义
- token 成本为 0
- 不依赖额外 LLM
- 不会把用户输入直接当成角色情绪

但它也有边界：
- 不会直接看完整历史语义
- 长回复仍然是裁剪后分类，不是完整全文理解
- 本地模型不可用时，这轮 emotion 不会更新

如果以后想增强，可以考虑：
- 多轮上下文情绪聚合
- 在 tool / interaction / heartbeat 中引入二次修正

## 10. 当前最准确的一句话总结

当前 emotion 系统的真实工作流是：

1. 先生成 assistant 最终回复
2. 用本地 ONNX 模型对回复裁剪片段做情绪分类
3. 产出 `label + raw_mood`
4. 用 `EmotionState::update()` 经过惯性和平滑更新内部状态
5. 把这个状态用于：
- heartbeat 衰减
- emotion-event
- expression-frame
- typing delay
- idle behavior
- initiative
- TTS 调制

它现在更接近“角色输出后的后置情绪状态机”，而不是“生成前的提示驱动器”。
