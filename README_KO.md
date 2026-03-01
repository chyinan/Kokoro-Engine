<div align="center">
  <a href="README.md">简体中文</a> | <a href="README_EN.md">English</a> | <a href="README_JA.md">日本語</a> | <a href="README_KO.md">한국어</a> | <a href="README_RU.md">Русский</a>
</div>

<p align="center">
  <h1 align="center">Kokoro Engine</h1>
  <p align="center">
    크로스 플랫폼 가상 캐릭터 몰입형 인터랙션 엔진<br/>
    <em>High freedom · Modular · Offline-first · Creator-friendly</em>
  </p>
</p>

---

> [!CAUTION]
> **⚠️ 얼리 액세스 공지 (Early Access Notice)**
>
> **Kokoro Engine**은 현재 **Alpha 개발 단계**에 있습니다.
>
> 이는 제한된 리소스(주 개발 장비는 RTX 4060 노트북 한 대)를 가진 개인 개발자가 구축한 오픈 소스 프로젝트입니다.
>
> - **🚧 즉시 사용 불가**: 기능이 자주 업데이트되며 구성에 약간의 기술적 배경이 필요할 수 있습니다.
> - **💸 자금 및 테스트 제한**: 저자의 자금 제한으로 인해 **LLM 대형 모델 인터페이스를 제외하고는 다른 온라인 서비스의 API 기능이 완전히 테스트되지 않았습니다**. 코드는 구현되었지만 모든 상용 API가 완벽하게 작동한다는 보장은 없습니다.
> - **🤝 지원 요청**: 이 프로젝트가 흥미롭거나 유용하다고 생각되면 **Star** ⭐️를 눌러주세요! API 디버깅 자금이나 하드웨어 업그레이드를 후원하고 싶은 "독지가"라면 [후원 방법](SPONSOR.md)을 확인해 주세요.

---

## ✨ 프로젝트 소개

**Kokoro Engine**은 누구나 데스크탑에서 "마음"이 있는 가상 파트너를 가질 수 있도록 설계된 크로스 플랫폼 가상 캐릭터 몰입형 인터랙션 엔진입니다(Neuro-sama에서 영감을 받음).
Live2D, LLM, TTS 및 STT와 같은 기술을 통합하여 고도로 모듈화되고 사용자 정의 가능한 인터랙션 시스템을 구축합니다.

## 📸 스크린샷 (Screenshots)

<div align="center">
  <img src="pictures/Homepage.png" alt="Homepage" width="800" />
  <p><em>홈 인터페이스 미리보기</em></p>
  <img src="pictures/Settings.png" alt="Settings" width="800" />
  <p><em>설정 인터페이스 미리보기</em></p>
</div>


## ✅ 구현된 기능 (Features)

현재 주로 로컬 환경(RTX 4060 Laptop)에서 검증된 기능:

### 🎭 핵심 인터랙션
- **Live2D 모델**: Live2D Cubism SDK를 완벽하게 지원하며 시선 추적 및 모션 트리거를 지원합니다.
- **모델 핫 스위칭**: 앱 내에서 다양한 Live2D 모델 및 관람 시점의 실시간 가져오기 및 전환을 지원합니다.
- **다국어 인터페이스**: 완벽한 국제화(I18n) 지원, 현재 **중국어 간체**, **영어**, **일본어**, **한국어**를 지원합니다.

### 🧠 AI 두뇌
- **멀티모달 대화**: **Ollama**(로컬) 및 **OpenAI 호환 인터페이스**(클라우드)를 대화의 핵심으로 지원합니다.
- **멀티모달 능력**: Vision 모델을 통합할 수 있으며, **스크린샷** 또는 **이미지 업로드**를 지원하여 캐릭터가 내용을 실시간으로 "보고" 설명할 수 있습니다.
- **텍스트-이미지 변환**: Stable Diffusion WebUI 또는 온라인 API 인터페이스를 통합할 수 있으며, 대화를 통한 이미지 생성 또는 대화 문맥에 따른 실시간 배경 이미지 생성을 지원합니다.
- **계층형 기억 시스템**: 3계층 기억 아키텍처를 탑재——계층형 기억(핵심 사실은 영구 보존, 임시 기억은 자연 감쇠), 시맨틱 + 키워드 하이브리드 검색(Embedding 코사인 유사도 + FTS5 BM25, RRF 랭크 융합), LLM 기반 자동 기억 통합(유사한 단편 기억을 클러스터링하여 병합). 대화에서 중요한 사실을 자동으로 추출하여 장기 저장(SQLite)하며, 실시간 문맥 회상 및 감정 지속성을 지원합니다.

### 🗣️ 음성 인터랙션
- **음성 합성 (TTS)**:
    - **GPT-SoVITS**: 뛰어난 감정 표현력, 사용자 정의 캐릭터 음성, 더 풍부한 생태계.
    - **Browser TTS**: 브라우저 네이티브 경량 TTS.
- **음성 변환 (RVC)**: 캐릭터 노래 등을 구현하기 위한 RVC(Retrieval-based Voice Conversion) 인터페이스를 지원합니다.
- **음성 인식 (STT)**: Whisper/faster-whisper/whisper.cpp 모델의 초기 지원.

### 🔌 확장 능력
- **MOD 시스템**: 내장 모듈식 MOD 프레임워크로 채팅 패널, 설정 패널 등 핵심 UI 컴포넌트를 HTML/CSS/JS로 커스텀 교체 가능. 커스텀 테마와 QuickJS 스크립트 샌드박스를 지원합니다.
- **MCP 프로토콜 지원**: **Model Context Protocol (MCP)** 클라이언트를 구현했습니다.
    - 모든 MCP Server에 연결 지원(stdio 상호 작용을 통해).
    - 캐릭터는 MCP Server에서 제공하는 도구(파일 시스템, 웹 검색, 데이터베이스 등)를 사용하여 능력을 확장할 수 있습니다.
    - UI를 통해 MCP Server 관리 지원.
- **Telegram Bot 원격 상호작용**: 내장 Telegram Bot 서비스로 공인 IP 없이 스마트폰에서 캐릭터와 대화 가능.
    - 텍스트, 음성, 사진 메시지를 지원하며 LLM/TTS/STT/ImageGen 파이프라인에 브리지.
    - Chat ID 화이트리스트 접근 제어, 세션 명령어 (`/new`, `/continue`, `/status`).
    - 데스크톱 채팅 UI에 Telegram 메시지 실시간 동기화.

### 🎮 공식 데모 MOD: 원신 스타일 UI

프로젝트에는 원신 게임의 비주얼 스타일로 채팅 및 설정 인터페이스를 재디자인한 완전한 공식 데모 MOD(`mods/genshin-theme`)가 포함되어 있습니다:

- 채팅 패널과 설정 패널을 완전히 교체하며 네이티브 기능과 완전히 동등
- 캐릭터 관리, LLM/TTS/STT/Vision/ImageGen 설정, MCP 관리, 배경 설정, 기억 관리 등 모든 설정 항목 포함
- 커뮤니티 개발자가 커스텀 UI MOD를 만들기 위한 참고 템플릿으로 활용 가능

## 📝 할 일 / 개발 중 (TODO)

다음 기능은 계획 중이거나 개발 중이거나 **장비 또는 자금 제한으로 인해 아직 테스트/검증되지 않은** 상태입니다.

- [ ] **온라인 서비스 심층 테스트**: LLM 외에 더 많은 상용 API(Azure TTS, Google STT 등) 검증.
- [ ] **모바일 지원**: iOS / Android 클라이언트 앱.
- [x] **계층형 기억 시스템**: 계층형 기억(core/ephemeral), 하이브리드 검색(시맨틱 + BM25 RRF 융합), LLM 기반 기억 통합.
- [x] **MOD 플러그인 시스템**: 커뮤니티 개발자가 MOD 모듈을 작성하여 기능을 확장할 수 있도록 허용(HTML/CSS/JS + QuickJS 스크립트 샌드박스).
- [x] **Live2D 모델 인터랙션**: Live2D 모델과의 실시간 상호 작용 피드백(시선 추적, 모션 트리거, 표정 동기화).
- [ ] **캐릭터 마켓/워크샵**: 캐릭터 프리셋 공유 및 다운로드 편의 제공.

## 🛠️ 기술 스택

| 계층 | 기술 |
|---|---|
| **프론트엔드** | React + TypeScript + Tailwind CSS + shadcn/ui |
| **백엔드** | Rust (Tauri v2) |
| **렌더링** | PixiJS + Live2D Cubism SDK |
| **데이터** | SQLite (로컬 스토리지) |

> **🚀 왜 Rust인가요?**
>
> Rust 언어의 놀라운 성능 덕분에 Kokoro Engine은 **매우 낮은 메모리 사용량**과 **매우 높은 실행 효율성**을 자랑합니다.
> 백그라운드에서 24/7 실행되더라도 시스템 속도를 늦추지 않으며 진정한 "경량" 파트너를 실현합니다.

## 🚀 빠른 시작

### 전제 조건

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://www.rust-lang.org/tools/install) (stable)

### 설치 및 실행

```bash
# 저장소 복제
git clone https://github.com/chyinan/kokoro-engine.git
cd kokoro-engine

# 의존성 설치
npm install

# 개발 서버 시작 (프론트엔드 + Tauri)
npm run tauri dev
```

### 배포용 빌드

```bash
npm run tauri build
```

## 🤝 기여 (Contributing)

**Kokoro Engine**은 커뮤니티의 기여를 적극 환영합니다!
저자의 에너지와 리소스가 제한되어 있으므로 프로젝트의 발전은 개발자의 지원 없이는 진행될 수 없습니다. 이 프로젝트에 관심이 있으시면 다음을 환영합니다.

1. **Pull Requests**: 버그 수정이나 새로운 기능을 추가하는 코드를 직접 제출하세요.
2. **Issues**: 발견한 문제를 보고하거나 개선 제안을 제출하세요.
3. **Discussions**: 토론 영역에서 아이디어를 공유하세요.
4. **로고 디자인**: 디자인에 능숙하다면 Kokoro Engine의 로고를 디자인해 보세요! 현재 로고는 임시용입니다.

모든 기여(오타 수정이라도)는 Kokoro Engine을 더 좋게 만듭니다! 최고의 데스크탑 가상 파트너를 함께 만들어 봅시다.

## ❤️ 후원

Kokoro Engine이 도움이 된다고 느끼시면 프로젝트의 지속적인 개발을 지원하는 후원을 고려해 주세요.

👉 [**후원 방법 보기**](SPONSOR.md)

## 📄 라이선스 (License)

이 프로젝트의 핵심 코드는 **MIT 라이선스**에 따라 오픈 소스입니다.

### ⚠️ Live2D Cubism SDK 면책 조항

이 프로젝트는 Live2D Inc. 소유의 **Live2D Cubism SDK**를 사용합니다.
이 프로젝트를 사용(컴파일, 배포 또는 수정 포함)할 때 Live2D의 라이선스 계약에 동의해야 합니다.

- **Live2D Proprietary Software License Agreement**: [https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html](https://www.live2d.com/eula/live2d-proprietary-software-license-agreement_en.html)
- **Live2D Open Software License Agreement**: [https://www.live2d.com/eula/live2d-open-software-license-agreement_en.html](https://www.live2d.com/eula/live2d-open-software-license-agreement_en.html)

> 이 오픈 소스 프로젝트는 비영리 또는 소규모 및 개인 사용을 위한 "개인/소규모 기업" 범주에 속합니다.
> 연간 매출이 1,000만 엔을 초과하는 중대형 기업인 경우 이 프로젝트를 사용하려면 Live2D Inc.와 별도의 상용 라이선스 계약이 필요할 수 있습니다.

---

**Kokoro Engine** is an open-source project.
The specific Live2D libraries and models included or downloaded are subject to the **Live2D Proprietary Software License Agreement**.
Live2D is a registered trademark of Live2D Inc.
