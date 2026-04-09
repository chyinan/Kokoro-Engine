<div align="center">
  <a href="README.md">简体中文</a> | <a href="README_EN.md">English</a> | <a href="README_JA.md">日本語</a> | <a href="README_KO.md">한국어</a> | <a href="README_RU.md">Русский</a>
</div>

<br/>

<p align="center">
  <img src="pictures/Poster_Girl.png" alt="Kokoro Engine banner" width="100%" />
</p>

<h1 align="center">Kokoro Engine</h1>
<p align="center"><strong>Open-source immersive character engine for desktop AI companions.</strong></p>
<p align="center">Кроссплатформенный движок взаимодействия с виртуальным персонажем для пользователей, которым нужен персональный AI-компаньон.</p>

<p align="center">
  <a href="https://t.me/+U39dgiUspCo2NDNh"><img src="https://img.shields.io/badge/Telegram-Community-26A5E4?logo=telegram&logoColor=white" alt="Telegram community" /></a>
  <img src="https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri&logoColor=white" alt="Tauri v2" />
  <img src="https://img.shields.io/badge/React-18%2B-20232A?logo=react&logoColor=61DAFB" alt="React" />
  <img src="https://img.shields.io/badge/Rust-Stable-000000?logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/License-MIT-2EA44F" alt="MIT License" />
</p>

<p align="center">
  <a href="#-быстрый-старт">Быстрый старт</a> ·
  <a href="https://github.com/chyinan/Kokoro-Engine/releases">Скачать релиз</a> ·
  <a href="#-техническая-архитектура">Архитектура</a> ·
  <a href="#-вклад-в-проект">Вклад</a>
</p>

---

## Уникальность Kokoro Engine

Kokoro Engine — это не просто чат-оболочка с внешним видом desktop pet. Это полноценный десктопный runtime персонажа:

- **All-in-one**: Live2D, LLM, TTS и STT объединены в единый runtime-контур.
- **Built for extensibility**: высокогибкая MOD-система + MCP-протокол.
- **Local-first**: локальная память, офлайн-приоритет и контролируемый путь данных.

## Кратко

| Измерение | Содержание |
|---|---|
| Целевая аудитория | создатели виртуальных персонажей, разработчики, обычные пользователи |
| Форматы взаимодействия | текст, голос, изображения, vision-вход, мультимодальный диалог |
| Модель расширения | MOD (HTML/CSS/JS + QuickJS), MCP Servers |
| Технологии | React + TypeScript + Rust + Tauri v2 + SQLite |
| Поддержка языков | 简体中文 / English / 日本語 / 한국어 / Русский |

## 📸 Скриншоты интерфейса

<div align="center">
  <img src="pictures/Homepage.png" alt="Homepage" width="800" />
  <p><em>Главный экран</em></p>
  <img src="pictures/Settings.png" alt="Settings" width="800" />
  <p><em>Экран настроек</em></p>
</div>

## 🚀 Быстрый старт

### Путь 1: скачать релиз (рекомендуется)

Перейдите на [страницу Releases](https://github.com/chyinan/Kokoro-Engine/releases), скачайте установщик под вашу платформу и запустите его.

### Путь 2: собрать из исходников

#### Требования

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://www.rust-lang.org/tools/install) (stable)

#### Установка и запуск

```bash
git clone https://github.com/chyinan/kokoro-engine.git
cd kokoro-engine
npm install
npm run tauri dev
```

#### Сборка релиза

```bash
npm run tauri build
```

### Путь 3: Nix / Flakes (только Linux)

```bash
nix develop
npm install
npm run tauri dev
```

Подробнее по Nix: [docs/nix.md](docs/nix.md).

## ✨ Ключевые возможности

### Рантайм персонажа

- Рендеринг Live2D, отслеживание взгляда, триггеры движений, плавающий режим на рабочем столе.
- Горячее переключение моделей и восстановление состояния взаимодействия.

### AI-мозг

- Поддержка Ollama и OpenAI-совместимых провайдеров.
- Мультимодальный вход, возврат контекста, долгосрочная память и непрерывность эмоционального состояния.

### Голосовой стек

- TTS (текст в речь): GPT-SoVITS, VITS, OpenAI, Azure, ElevenLabs, Edge TTS, Browser TTS.
- STT (речь в текст): Whisper / faster-whisper / whisper.cpp / SenseVoice.
- Поддержка VAD-автоостановки и wake-word цепочки.

### Расширяемость

- MOD-фреймворк: замена UI на HTML/CSS/JS + песочница скриптов QuickJS.
- MCP-поддержка: подключение MCP Server и вызов внешних инструментов.
- Встроенный официальный демо-MOD: `mods/genshin-theme`.

### Удалённое взаимодействие

- Встроенный Telegram Bot сервис.
- Мост для текстовых, голосовых и image-сообщений в полный AI-пайплайн.

## 🏗️ Техническая архитектура

```text
Frontend (React + TypeScript)
      <-> Typed IPC Bridge (kokoro-bridge.ts)
Backend (Rust / Tauri v2)
```

- Фронтенд: декларативный layout, реестр компонентов, тема, MOD UI injection.
- Бэкенд: командные модули + AI orchestration (LLM/TTS/STT/Vision/ImageGen/MCP).
- Слой данных: SQLite + локальный векторный retrieval.

Подробности: [docs/architecture.md](docs/architecture.md).

## 🗺️ Дорожная карта

### Сейчас

- Проверка стабильности и совместимости на Windows / Linux / macOS.
- Глубокое тестирование онлайн-сервисных цепочек.
- Постоянная оптимизация памяти и мультимодального опыта.

### Далее

- Маркет/мастерская персонажей.
- Исследование мобильной поддержки (iOS / Android).
- Усиление экосистемы расширений для разработчиков.

## 🤝 Вклад в проект

Вы можете участвовать так:

1. **Pull requests**: исправления и новые функции.
2. **Issues**: баг-репорты и предложения по улучшению.
3. **Discussions**: обмен идеями и практикой.
4. **Design contributions**: логотип и визуальные материалы.

## 💬 Сообщество

👉 [**Официальная Telegram-группа Kokoro Engine**](https://t.me/+U39dgiUspCo2NDNh)

## ❤️ Поддержка

👉 [**Способы поддержки / Sponsor**](SPONSOR.md)

## 📄 Лицензия

Основной код проекта распространяется по **MIT License**.

### ⚠️ Уведомление о Live2D Cubism SDK

Проект использует **Live2D Cubism SDK**, а связанные части принадлежат Live2D Inc. При компиляции, распространении и модификации проекта необходимо соблюдать лицензии Live2D:

- [Live2D Proprietary Software License Agreement](https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html)
- [Live2D Open Software License Agreement](https://www.live2d.com/eula/live2d-open-software-license-agreement_en.html)

> Для компаний с годовой выручкой выше 10 млн иен может потребоваться отдельное коммерческое лицензионное соглашение с Live2D Inc.

---

**Kokoro Engine** is an open-source project.
Live2D is a registered trademark of Live2D Inc.
