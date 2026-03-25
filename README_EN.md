<div align="center">
  <a href="README.md">简体中文</a> | <a href="README_EN.md">English</a> | <a href="README_JA.md">日本語</a> | <a href="README_KO.md">한국어</a> | <a href="README_RU.md">Русский</a>
</div>

<p align="center">
  <h1 align="center">Kokoro Engine</h1>
  <p align="center">
    Cross-platform Virtual Character Immersive Interaction Engine<br/>
    <em>High freedom · Modular · Offline-first · Creator-friendly</em>
  </p>
</p>

---

## ✨ Introduction

**Kokoro Engine** is a cross-platform virtual character immersive interaction engine designed to let everyone have a "heartfelt" virtual companion on their desktop (inspired by Neuro-sama).
It integrates technologies such as Live2D, LLM, TTS, and STT to build a highly modular and customizable interaction system.

## 📸 Screenshots

<div align="center">
  <img src="pictures/Homepage.png" alt="Homepage" width="800" />
  <p><em>Home Interface Preview</em></p>
  <img src="pictures/Settings.png" alt="Settings" width="800" />
  <p><em>Settings Interface Preview</em></p>
</div>


## ✅ Features

Features currently verified mainly in a local environment (RTX 4060 Laptop):

### 🎭 Core Interaction
- **Live2D Models**: Full support for Live2D Cubism SDK, supporting eye tracking and motion triggering.
- **Model Hot-Switching**: Supports real-time import and switching of different Live2D models and viewing angles within the app.
- **Multi-language Interface**: Full Internationalization (I18n) support, currently supporting **Simplified Chinese**, **English**, **Japanese**, and **Korean**.
- **Desktop Float Mode**: Display the Live2D model as a transparent floating window on your desktop — always on top, borderless, and fully transparent.
  - Right-click drag to move the window; right-click menu to enter resize mode and drag edges to resize freely.
  - Ctrl + scroll wheel to fine-tune model scale.
  - Global shortcut (customizable via key recording) to summon the chat input; AI replies appear in a separate bubble window above the model.
  - Position, window size, and model scale are automatically restored after restart.

### 🧠 AI Brain
- **Multimodal Conversation**: Supports **Ollama** (Local) and **OpenAI Compatible Interfaces** (Cloud) as the conversation core.
- **Multimodal Capabilities**: Can integrate Vision models, supporting **screen capture**, **image upload**, or **real-time webcam vision**, allowing the character to "see" and describe the content. Webcam vision supports multi-device selection with a live preview, and automatically attaches the latest camera frame to every chat message.
- **Text-to-Image**: Can integrate Stable Diffusion WebUI or online API interfaces, supporting image generation via dialogue or generating real-time background images based on conversation context.
- **Tiered Memory System**: Features a three-layer memory architecture — tiered memory (core facts never forgotten, ephemeral memories naturally decay), hybrid semantic + keyword retrieval (embedding cosine similarity + FTS5 BM25 with RRF rank fusion), and LLM-driven automatic memory consolidation (similar fragmented memories are clustered and merged). Automatically extracts key facts from conversations for long-term storage (SQLite), with real-time context recall and emotion persistence.

### 🗣️ Voice Interaction
- **Text-to-Speech (TTS)**:
    - **GPT-SoVITS**: Excellent emotional expressiveness, custom character voices, and a richer ecosystem.
    - **VITS**: Compatible with local VITS inference servers such as vits-simple-api.
    - **OpenAI TTS**: Supports OpenAI-compatible cloud speech synthesis APIs.
    - **Azure TTS**: Microsoft Azure Cognitive Services speech synthesis.
    - **ElevenLabs**: High-quality AI speech synthesis with voice cloning support.
    - **Browser TTS**: Lightweight browser-native TTS.
- **Voice Conversion (RVC)**: Supports RVC (Retrieval-based Voice Conversion) interface for implementing character singing.
- **Voice Interaction (STT)**: Multi-engine support for OpenAI Whisper/faster-whisper/whisper.cpp/SenseVoice, with built-in wake word detection and VAD auto-stop.

### 🔌 Extended Capabilities
- **MOD System**: Built-in modular MOD framework that allows replacing core UI components (chat panel, settings panel, etc.) with custom HTML/CSS/JS, with support for custom themes and a QuickJS script sandbox.
- **MCP Protocol Support**: Implements **Model Context Protocol (MCP)** Client.
    - Supports connection to any MCP Server (via stdio interaction).
    - Characters can use tools provided by MCP Servers (such as file system, Web search, databases, etc.) to enhance capabilities.
    - Supports managing MCP Servers via UI.
- **Telegram Bot Remote Interaction**: Built-in Telegram Bot service for chatting with your character from your phone — no public IP required.
    - Supports text, voice, and photo messages, bridging to the full LLM/TTS/STT/ImageGen pipeline.
    - Chat ID whitelist access control, session commands (`/new`, `/continue`, `/status`).
    - Desktop chat UI syncs Telegram messages in real-time.

### 🎮 Official Demo MOD: Genshin Impact Style UI

The project includes a complete official demo MOD (`mods/genshin-theme`) that redesigns the chat and settings interfaces in the visual style of Genshin Impact:

- Fully replaces the chat and settings panels with complete feature parity
- Includes character management, LLM/TTS/STT/Vision/ImageGen configuration, MCP management, background settings, memory management, and all other settings
- Serves as a reference template for community developers creating custom UI MODs

## 📝 TODO / In Development

The following features are planned, in development, or **not yet tested/verified due to device or funding limitations**:

- [ ] **Linux and macOS Support**: Currently only thoroughly tested on Windows. Full feature verification and optimization needed on Linux and macOS.
- [ ] **Deep Testing of Online Services**: Verify more commercial APIs besides LLM (e.g., Azure TTS, Google STT, etc.).
- [ ] **Mobile Support**: iOS / Android client applications.
- [x] **Tiered Memory System**: Tiered memory (core/ephemeral), hybrid retrieval (semantic + BM25 RRF fusion), and LLM-driven memory consolidation.
- [x] **MOD Plugin System**: Allow community developers to write MOD modules to extend functionality (HTML/CSS/JS + QuickJS script sandbox).
- [x] **Live2D Model Interaction**: Real-time interaction feedback with Live2D models (eye tracking, motion triggers, expression sync).
- [ ] **Character Market/Workshop**: Facilitate sharing and downloading character presets.

## 🛠️ Tech Stack

| Layer | Technology |
|---|---|
| **Frontend** | React + TypeScript + Tailwind CSS + shadcn/ui |
| **Backend** | Rust (Tauri v2) |
| **Rendering** | PixiJS + Live2D Cubism SDK |
| **Data** | SQLite (Local Storage) |

> **🚀 Why Rust?**
>
> Thanks to the amazing performance of the Rust language, Kokoro Engine has **extremely low memory usage** and **extremely high execution efficiency**.
> Even running 24/7 in the background, it won't slow down your system, truly achieving "lightweight" companionship.

## 🚀 Quick Start

### Option 1: Download Pre-built Release (Recommended)

Visit the [Releases page](https://github.com/chyinan/Kokoro-Engine/releases) to download the installer for your platform and run it directly.

### Option 2: Build from Source

#### Prerequisites

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://www.rust-lang.org/tools/install) (stable)

#### Installation & Running

```bash
# Clone the repository
git clone https://github.com/chyinan/kokoro-engine.git
cd kokoro-engine

# Install dependencies
npm install

# Start development server (Frontend + Tauri)
npm run tauri dev
```

#### Build for Distribution

```bash
npm run tauri build
```

## 🤝 Contributing

**Kokoro Engine** extremely welcomes community contributions!
Due to the limited energy and resources of the author, the development of the project cannot proceed without the support of developers. If you are interested in this project, welcome to:

1. **Pull Requests**: Submit code fixes for bugs or add new features directly.
2. **Issues**: Report problems you find or propose improvements.
3. **Discussions**: Share your ideas in the discussions area.
4. **Logo Design**: If you are good at design, welcome to design a Logo for Kokoro Engine! The current Logo is just temporary.

Every contribution (even if it's just fixing a typo) makes Kokoro Engine better! Let's build the best desktop virtual companion together.

## 💬 Community

Join our official Telegram group to connect with other users, share tips, and stay up to date:

👉 [**Kokoro Engine Official Discussion Group**](https://t.me/+U39dgiUspCo2NDNh)

## ❤️ Sponsor

If you find Kokoro Engine helpful, consider sponsoring to support the ongoing development of the project.

👉 [**View Sponsorship Options**](SPONSOR.md)

## 📄 License

The core code of this project is open-source under the **MIT License**.

### ⚠️ Live2D Cubism SDK Disclaimer

This project uses the **Live2D Cubism SDK**, which belongs to Live2D Inc.
When using this project (including compiling, distributing, or modifying), you must agree to Live2D's license agreement:

- **Live2D Proprietary Software License Agreement**: [https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html](https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html)
- **Live2D Open Software License Agreement**: [https://www.live2d.com/eula/live2d-open-software-license-agreement_en.html](https://www.live2d.com/eula/live2d-open-software-license-agreement_en.html)

> This open-source project falls under the category of "Individual/Small-Scale Enterprise" for non-profit or small-scale use.
> If you are a medium-to-large enterprise with an annual turnover exceeding 10 million JPY, using this project may require a separate commercial license agreement with Live2D Inc.

---

**Kokoro Engine** is an open-source project.
The specific Live2D libraries and models included or downloaded are subject to the **Live2D Proprietary Software License Agreement**.
Live2D is a registered trademark of Live2D Inc.
