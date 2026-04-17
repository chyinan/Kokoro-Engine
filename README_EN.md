<div align="center">
  <a href="README.md">简体中文</a> | <a href="README_EN.md">English</a> | <a href="README_JA.md">日本語</a> | <a href="README_KO.md">한국어</a> | <a href="README_RU.md">Русский</a>
</div>

<br/>

<p align="center">
  <img src="pictures/Poster_Girl.png" alt="Kokoro Engine banner" width="100%" />
</p>

<h1 align="center">Kokoro Engine</h1>
<p align="center"><strong>Open-source immersive character engine for desktop AI companions.</strong></p>
<p align="center">A cross-platform virtual character interaction engine for users who want a personalized AI companion.</p>

<p align="center">
  <a href="https://t.me/+U39dgiUspCo2NDNh"><img src="https://img.shields.io/badge/Telegram-Community-26A5E4?logo=telegram&logoColor=white" alt="Telegram community" /></a>
  <img src="https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri&logoColor=white" alt="Tauri v2" />
  <img src="https://img.shields.io/badge/React-18%2B-20232A?logo=react&logoColor=61DAFB" alt="React" />
  <img src="https://img.shields.io/badge/Rust-Stable-000000?logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/License-MIT-2EA44F" alt="MIT License" />
</p>

<p align="center">
  <a href="#-quick-start">Quick start</a> ·
  <a href="https://github.com/chyinan/Kokoro-Engine/releases">Download release</a> ·
  <a href="#-technical-architecture">Architecture</a> ·
  <a href="#-contributing">Contributing</a>
</p>

---

## What makes Kokoro Engine stand out

Kokoro Engine is not just a chat shell with a desktop pet skin. It is a complete desktop character runtime:

- **All-in-one**: Live2D, LLM, TTS, and STT are integrated into one runtime loop.
- **Built for extensibility**: a high-freedom MOD system and MCP protocol.
- **Local-first**: local memory storage, offline-first behavior, and a controllable data path.

## Overview

| Dimension | Details |
|---|---|
| Target users | virtual character creators, developers, and general users |
| Interaction modes | text, voice, image, vision input, multimodal dialogue |
| Extension model | MOD (HTML/CSS/JS + QuickJS), MCP servers |
| Tech stack | React + TypeScript + Rust + Tauri v2 + SQLite |
| Language support | 简体中文 / English / 日本語 / 한국어 / Русский |

## 📸 UI screenshots

<div align="center">
  <img src="pictures/Homepage.png" alt="Homepage" width="800" />
  <p><em>Main screen</em></p>
  <img src="pictures/Settings.png" alt="Settings" width="800" />
  <p><em>Settings screen</em></p>
</div>

## 🚀 Quick start

### Path 1: Download release (recommended)

Go to the [Releases page](https://github.com/chyinan/Kokoro-Engine/releases), download the installer for your platform, and run it.

### Path 2: Build from source

#### Requirements

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://www.rust-lang.org/tools/install) (stable)

#### Install and run

```bash
git clone https://github.com/chyinan/kokoro-engine.git
cd kokoro-engine
npm install
npm run tauri dev
```

#### Build release

```bash
npm run tauri build
```

### Path 3: Nix / Flakes (Linux only)

```bash
nix develop
npm install
npm run tauri dev
```

For more Nix usage, see [docs/nix.md](docs/nix.md).

## ✨ Core capabilities

### Character runtime

- Live2D rendering, eye tracking, motion triggers, desktop floating mode.
- Model hot-switching and interaction state recovery.

### AI brain

- Supports Ollama and OpenAI-compatible protocol API interfaces.
- Supports multimodal input, context recall, long-term memory, and emotional state continuity.

### Voice stack

- TTS (text-to-speech): GPT-SoVITS, VITS, OpenAI, Azure, ElevenLabs, Edge TTS, Browser TTS.
- STT (speech-to-text): Whisper / faster-whisper / whisper.cpp / SenseVoice.
- Supports VAD auto-stop and wake-word flow.

### Extensibility

- MOD framework: HTML/CSS/JS UI replacement + QuickJS script sandbox.
- MCP support: connect MCP servers and call external tools.
- Built-in official demo MOD: `mods/genshin-theme`.

### Remote interaction

- Built-in Telegram Bot service.
- Bridges text, voice, and image messages to the full AI pipeline.

## 🏗️ Technical architecture

```mermaid
flowchart LR
  subgraph FE["Frontend (React + TypeScript)"]
    FE_UI["UI Layout Engine"]
    FE_REG["Component Registry"]
    FE_THEME["Theme & MOD UI"]
    FE_BRIDGE["kokoro-bridge.ts"]
    FE_UI --> FE_REG
    FE_REG --> FE_THEME
    FE_THEME --> FE_BRIDGE
  end

  subgraph IPC["Tauri Typed IPC"]
    IPC_INVOKE["invoke / events"]
  end

  subgraph BE["Backend (Rust / Tauri v2)"]
    BE_CMD["Commands Layer"]
    BE_ORCH["AI Orchestrator"]
    BE_MULTI["LLM / TTS / STT / Vision / ImageGen / MCP"]
    BE_MOD["MOD Runtime (QuickJS)"]
    BE_TG["Telegram Bridge"]
    BE_CMD --> BE_ORCH
    BE_ORCH --> BE_MULTI
    BE_MOD --> BE_CMD
    BE_TG --> BE_CMD
  end

  subgraph DATA["Data & Runtime Config"]
    DB[("SQLite: memories / summaries / conversations / characters")]
    CFG["Config Files: llm/tts/stt/vision/imagegen/mcp/telegram"]
  end

  subgraph EXT["External Services"]
    EXT_LLM["OpenAI-Compatible / Ollama"]
    EXT_TTS["TTS Providers"]
    EXT_MCP["MCP Servers"]
    EXT_TG["Telegram"]
  end

  FE_BRIDGE <--> IPC_INVOKE
  IPC_INVOKE <--> BE_CMD

  BE_MULTI <--> EXT_LLM
  BE_MULTI <--> EXT_TTS
  BE_MULTI <--> EXT_MCP
  BE_TG <--> EXT_TG

  BE_ORCH <--> DB
  BE_CMD <--> CFG
```

- Frontend: declarative layout, component registry, theme system, MOD UI injection.
- Backend: command modules + AI orchestration (LLM/TTS/STT/Vision/ImageGen/MCP).
- Data layer: a local-first memory layer built on SQLite, persistently storing characters, conversations, summaries, and long-term memory, and using `embedding + FTS5 BM25 + RRF` hybrid retrieval to provide stable long-term context for dialogue.

See [docs/architecture.md](docs/architecture.md) for details.

## 🗺️ Roadmap

### Current

- Cross-platform stability and compatibility validation (Windows / Linux / macOS).
- Deep testing of online service pipelines.
- Ongoing optimization of memory and multimodal experience.

### Next

- Character marketplace / workshop.
- Mobile support exploration (iOS / Android).
- Stronger developer extension ecosystem.

## 🤝 Contributing

You can contribute in these ways:

1. **Pull requests**: fix issues or add features.
2. **Issues**: report bugs and propose improvements.
3. **Discussions**: share ideas and practical experience.
4. **Design contributions**: logo and visual assets are welcome.

## 💬 Community

👉 [**Kokoro Engine official Telegram group**](https://t.me/+U39dgiUspCo2NDNh)

## ❤️ Sponsor

👉 [**Sponsorship options / Sponsor**](SPONSOR.md)

## 🎉 Special Thanks

Special thanks to all contributors for their contributions to Kokoro Engine.

<table align="center">
  <tr>
    <td align="center">
      <a href="https://github.com/aegbirou">
        <img src="https://github.com/aegbirou.png?size=120" alt="@aegbirou" width="88" height="88" />
      </a>
      <br />
      <sub>@aegbirou</sub>
    </td>
    <td align="center">
      <a href="https://github.com/Initsnow">
        <img src="https://avatars.githubusercontent.com/u/79002121?s=96&v=4" alt="@Initsnow" width="88" height="88" />
      </a>
      <br />
      <sub>@Initsnow</sub>
    </td>
  </tr>
</table>


## 📄 License

Core project code is licensed under **MIT License**.

### ⚠️ Live2D Cubism SDK notice

This project uses **Live2D Cubism SDK**, and related parts are owned by Live2D Inc. If you compile, distribute, or modify this project, you must comply with Live2D license terms:

- [Live2D Proprietary Software License Agreement](https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html)
- [Live2D Open Software License Agreement](https://www.live2d.com/eula/live2d-open-software-license-agreement_en.html)

> Organizations with annual revenue above JPY 10 million may need a separate commercial license agreement with Live2D Inc.

---

**Kokoro Engine** is an open-source project.
Live2D is a registered trademark of Live2D Inc.
