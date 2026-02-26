<div align="center">
  <a href="README.md">简体中文</a> | <a href="README_EN.md">English</a> | <a href="README_JA.md">日本語</a> | <a href="README_KO.md">한국어</a> | <a href="README_RU.md">Русский</a>
</div>

<p align="center">
  <h1 align="center">Kokoro Engine</h1>
  <p align="center">
    跨平台虚拟角色沉浸交互引擎<br/>
    <em>High freedom · Modular · Offline-first · Creator-friendly</em>
  </p>
</p>

---

> [!CAUTION]
> **⚠️ 早期开发阶段通知 (Early Access Notice)**
>
> **Kokoro Engine** 目前处于 **Alpha 开发阶段**。
>
> 这是一个目前仅由个人开发者凭借有限资源（主力开发机仅为一台 RTX 4060 笔记本）构建的开源项目。
>
> - **🚧 非开箱即用**：功能仍在频繁迭代中，可能需要一定的技术背景进行配置。
> - **💸 资金与测试限制**：作者资金有限，**除了 LLM 大模型接口外，其他在线服务的 API 功能尚未经过全面测试**。虽然代码已实现，但无法保证所有商业 API 都能完美运行。
> - **🤝 寻求支持**：如果您觉得这个项目有趣或有用，欢迎 **Star** ⭐️ 支持！如果您是“大善人”并愿意资助项目的 API 调试资金或硬件升级，请查看赞助方式（TODO）。

---

## ✨ 项目简介

**Kokoro Engine** 是一个跨平台虚拟角色沉浸交互引擎，旨在让每个人都能在桌面上拥有一个“有心”的虚拟伙伴（灵感来源于 Neuro-sama）。
它整合了 Live2D、LLM、TTS 和 STT 等技术，构建了一个高度模块化、可定制的交互系统。

## 📸 截图 (Screenshots)

<div align="center">
  <img src="pictures/Homepage.png" alt="Homepage" width="800" />
  <p><em>主界面预览</em></p>
  <img src="pictures/Settings.png" alt="Settings" width="800" />
  <p><em>设置界面预览</em></p>
</div>


## ✅ 已完善功能 (Features)

目前主要在本地环境（RTX 4060 Laptop）下验证通过的功能：

### 🎭 核心交互
- **Live2D 模型**: 完整支持 Live2D Cubism SDK，支持视线追踪、动作触发。
- **模型热切换**: 支持在应用内实时导入和切换不同的 Live2D 模型和观赏视角。
- **多语言界面**: 完整的国际化支持 (I18n)，目前支持 **简体中文**、**English**、**日本語**、**한국어**。

### 🧠 AI 大脑
- **多模态对话**: 支持 **Ollama** (本地) 和 **OpenAI 兼容接口** (云端) 作为对话核心。
- **多模态能力**: 可接入 Vision 模型，支持 **屏幕截图** 或 **图片上传**，角色可以实时“看到”并描述内容。
- **文生图**: 可接入 Stable Diffusion WebUI 或在线API接口，支持通过对话让LLM生成图片或者根据对话场景生成实时的页面背景图片。
- **分层记忆系统**: 拥有三层记忆架构——分层记忆（核心事实永不遗忘，临时记忆自然衰减）、语义+关键词混合检索（Embedding 余弦相似度 + FTS5 BM25，RRF 融合排序）、以及 LLM 驱动的自动记忆整合（相似碎片记忆聚类合并）。支持从对话中自动提取关键事实并长期存储 (SQLite)，实时上下文回溯与情感持久化。

### 🗣️ 语音交互
- **语音合成 (TTS)**:
    - **GPT-SoVITS**: 情感表现力极佳，自定义角色声线，生态更加丰富。
    - **Browser TTS**: 浏览器原生轻量级 TTS。
- **声音转换 (RVC)**: 支持 RVC (Retrieval-based Voice Conversion) 变声器接口，用于实现角色演唱歌曲。
- **语音交互 (STT)**: Whisper/faster-whisper/whisper.cpp 模型初步支持。

### 🔌 扩展能力
- **MOD 系统**: 内置模块化 MOD 框架，支持通过 HTML/CSS/JS 替换聊天面板、设置面板等核心 UI 组件，支持自定义主题和 QuickJS 脚本沙箱。
- **MCP 协议支持**: 实现了 **Model Context Protocol (MCP)** 客户端。
    - 支持连接任意 MCP Server (通过 stdio 交互)。
    - 角色可以调用 MCP Server 提供的工具（如文件系统、Web 搜索、数据库等）来增强能力。
    - 支持通过 UI 来管理 MCP Server。

### 🎮 官方示范 MOD：原神风格 UI

项目内置了一个完整的官方示范 MOD（`mods/genshin-theme`），以原神游戏的视觉风格重新设计了聊天和设置界面：

- 完整替换聊天面板和设置面板，与原生功能完全对等
- 包含角色管理、LLM/TTS/STT/Vision/ImageGen 配置、MCP 管理、背景设置、记忆管理等全部设置项
- 可作为社区开发者创建自定义 UI MOD 的参考模板

## 📝 待办事项 / 开发中 (TODO)

以下功能处于计划中、开发中或**因设备或资金限制尚未经过测试或验证**的状态：

- [ ] **在线服务深度测试**: 验证除 LLM 外的更多商业 API（如 Azure TTS, Google STT 等）。
- [ ] **移动端支持**: iOS / Android 客户端应用。
- [x] **分层记忆系统**: 分层记忆（core/ephemeral）、混合检索（语义 + BM25 RRF 融合）、LLM 驱动的记忆整合。
- [ ] **更强的插件系统**: 允许社区开发者编写MOD模块来扩展功能。
- [ ] **角色市场/工坊**: 方便分享和下载角色预设。
- [ ] **Live2D模型交互**: 与Live2D模型的实时交互反馈功能。

## 🛠️ 技术栈

| 层级 | 技术 |
|---|---|
| **前端** | React + TypeScript + Tailwind CSS + shadcn/ui |
| **后端** | Rust (Tauri v2) |
| **渲染** | PixiJS + Live2D Cubism SDK |
| **数据** | SQLite (本地存储) |

> **🚀 为什么选择 Rust?**
>
> 得益于 Rust 语言的惊人性能，Kokoro Engine 拥有**极低的内存占用**和**极高的运行效率**。
> 即使在后台全天候运行，也不会拖慢您的系统，真正做到“轻量级”陪伴。

## 🚀 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://www.rust-lang.org/tools/install) (stable)

### 安装与运行

```bash
# 克隆仓库
git clone https://github.com/chyinan/kokoro-engine.git
cd kokoro-engine

# 安装依赖
npm install

# 启动开发服务器 (前端 + Tauri)
npm run tauri dev
```

### 构建发行版

```bash
npm run tauri build
```

## 🤝 参与贡献 (Contributing)
 
**Kokoro Engine** 极其欢迎社区的贡献！
由于作者精力和资源有限，项目的发展离不开开发者的支持。如果您对这个项目感兴趣，欢迎：
 
1. **Pull Requests**: 直接提交代码修复 Bug 或添加新功能。
2. **Issues**: 报告您发现的问题或提出改进建议。
3. **Discussions**: 在讨论区分享您的想法。
4. **Logo Design**: 如果您擅长设计，欢迎为 Kokoro Engine 设计一个 Logo！目前的 Logo 只是临时的。

 
您的每一份贡献（哪怕只是修改一个错别字）都能让 Kokoro Engine 变得更好！让我们一起打造最棒的桌面虚拟伙伴。
 
## 📄 License (许可协议)

本项目核心代码遵循 **MIT 协议** 开源。

### ⚠️ Live2D Cubism SDK 声明

本项目使用了 **Live2D Cubism SDK**，该部分归 Live2D Inc. 所有。
使用本项目（包括编译、分发或修改）时，您必须同意 Live2D 的许可协议：

- **Live2D Proprietary Software License Agreement**: [https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html](https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html)
- **Live2D Open Software License Agreement**: [https://www.live2d.com/eula/live2d-open-software-license-agreement_en.html](https://www.live2d.com/eula/live2d-open-software-license-agreement_en.html)

> 本开源项目属于“个人/小规模企业”范畴的非营利性或小规模使用。
> 如果您是年营业额超过 1000 万日元的中大型企业，使用本项目可能需要单独与 Live2D Inc. 签署商业授权协议。

---

**Kokoro Engine** is an open-source project.
The specific Live2D libraries and models included or downloaded are subject to the **Live2D Proprietary Software License Agreement**.
Live2D is a registered trademark of Live2D Inc.
