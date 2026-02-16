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
- **多语言界面**: 完整的国际化均支持 (I18n)，界面语言可自由切换。

### 🧠 AI 大脑
- **多模态对话**: 支持 **Ollama** (本地) 和 **OpenAI 兼容接口** (云端) 作为对话核心。
- **视觉能力**: 可接入 Vision 模型，角色可以实时“看到”屏幕内容并描述屏幕内容或图片。
- **文生图**: 可接入 Stable Diffusion WebUI 或在线API接口，支持通过对话让LLM生成图片或者根据对话场景生成实时的页面背景图片。

### 🗣️ 语音交互
- **语音合成 (TTS)**:
    - **GPT-SoVITS**: 情感表现力极佳，自定义角色声线，生态更加丰富。
    - **Browser TTS**: 浏览器原生轻量级 TTS。
- **声音转换 (RVC)**: 支持 RVC (Retrieval-based Voice Conversion) 变声器接口，用于实现角色演唱歌曲。

### 🔌 扩展能力
- **MCP 协议支持**: 实现了 **Model Context Protocol (MCP)** 客户端。
    - 支持连接任意 MCP Server (通过 stdio 交互)。
    - 角色可以调用 MCP Server 提供的工具（如文件系统、Web 搜索、数据库等）来增强能力。
    - 支持通过 UI 来管理 MCP Server。

## 📝 待办事项 / 开发中 (TODO)

以下功能处于计划中、开发中或**因设备或资金限制尚未经过测试或验证**的状态：

- [ ] **在线服务深度测试**: 验证除 LLM 外的更多商业 API（如 Azure TTS, Google STT 等）。
- [ ] **移动端支持**: iOS / Android 客户端应用。
- [ ] **高级记忆系统**: 基于向量数据库 (Vector DB) 和 RAG 的长期记忆检索。
- [ ] **更强的插件系统**: 允许社区开发者编写MOD模块来扩展功能。
- [ ] **角色市场/工坊**: 方便分享和下载角色预设。
- [ ] **STT语音识别**: Whisper/faster-whisper/whisper.cpp 模型支持与验证。
- [ ] **Live2D模型交互**: 与Live2D模型的实时交互反馈功能。

## 🛠️ 技术栈

| 层级 | 技术 |
|---|---|
| **前端** | React + TypeScript + Tailwind CSS + shadcn/ui |
| **后端** | Rust (Tauri v2) — ⚡ 极致性能 & 🔒 内存安全 |
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
- [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)

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
