<p align="center">
  <img src="ironclaw.png?v=2" alt="IronClaw" width="200"/>
</p>

<h1 align="center">IronClaw</h1>

<p align="center">
  <strong>언제나 당신 편인 안전한 개인 AI 어시스턴트</strong>
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
  <a href="https://t.me/ironclawAI"><img src="https://img.shields.io/badge/Telegram-%40ironclawAI-26A5E4?style=flat&logo=telegram&logoColor=white" alt="Telegram: @ironclawAI" /></a>
  <a href="https://www.reddit.com/r/ironclawAI/"><img src="https://img.shields.io/badge/Reddit-r%2FironclawAI-FF4500?style=flat&logo=reddit&logoColor=white" alt="Reddit: r/ironclawAI" /></a>
  <a href="https://gitcgr.com/nearai/ironclaw">
    <img src="https://gitcgr.com/badge/nearai/ironclaw.svg" alt="gitcgr" />
  </a>
</p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ru.md">Русский</a> |
  <a href="README.ja.md">日本語</a> |
  <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <a href="#철학">철학</a> •
  <a href="#기능">기능</a> •
  <a href="#설치">설치</a> •
  <a href="#설정">설정</a> •
  <a href="#보안">보안</a> •
  <a href="#아키텍처">아키텍처</a>
</p>

---

## 철학

IronClaw는 단순한 원칙 위에 만들어졌습니다: **AI 어시스턴트는 당신을 위해 일해야 하며, 당신을 거슬러서는 안 됩니다**.

AI 시스템이 데이터 처리에 대해 점점 더 불투명해지고 기업의 이익에 맞춰지는 세상에서, IronClaw는 다른 접근 방식을 취합니다:

- **데이터는 당신의 것** - 모든 정보는 로컬에 저장되고 암호화되며, 절대 당신의 통제를 벗어나지 않습니다
- **설계에 의한 투명성** - 오픈 소스, 감사 가능, 숨겨진 텔레메트리나 데이터 수집 없음
- **자가 확장 기능** - 공급업체의 업데이트를 기다리지 않고 즉석에서 새로운 도구를 만들 수 있습니다
- **심층 방어** - 프롬프트 인젝션 및 데이터 유출로부터 보호하는 다중 보안 계층

IronClaw는 개인적, 직업적 삶에서 실제로 신뢰할 수 있는 AI 어시스턴트입니다.

## 기능

### 보안 우선

- **WASM 샌드박스** - 신뢰할 수 없는 도구는 권한 기반의 격리된 WebAssembly 컨테이너에서 실행됩니다
- **자격 증명 보호** - 비밀은 도구에 노출되지 않고, 누출 감지와 함께 호스트 경계에서 주입됩니다
- **프롬프트 인젝션 방어** - 패턴 감지, 콘텐츠 정화, 정책 시행
- **엔드포인트 화이트리스트** - HTTP 요청은 명시적으로 승인된 호스트와 경로로만 전송됩니다

### 항상 사용 가능

- **다중 채널** - REPL, HTTP 웹훅, WASM 채널 (Telegram, Slack), 웹 게이트웨이
- **Docker 샌드박스** - 작업별 토큰과 오케스트레이터/워커 패턴을 사용한 격리된 컨테이너 실행
- **웹 게이트웨이** - 실시간 SSE/WebSocket 스트리밍이 있는 브라우저 UI
- **루틴** - 백그라운드 자동화를 위한 cron 일정, 이벤트 트리거, 웹훅 핸들러
- **하트비트 시스템** - 모니터링 및 유지 보수 작업을 위한 사전 백그라운드 실행
- **병렬 작업** - 격리된 컨텍스트로 여러 요청을 동시에 처리합니다
- **자가 복구** - 중단된 작업의 자동 감지 및 복구

### 자가 확장

- **동적 도구 빌드** - 필요한 것을 설명하면 IronClaw가 WASM 도구로 만들어 줍니다
- **MCP 프로토콜** - 추가 기능을 위해 Model Context Protocol 서버에 연결합니다
- **플러그인 아키텍처** - 재시작 없이 새로운 WASM 도구와 채널을 추가할 수 있습니다

### 영구 메모리

- **하이브리드 검색** - Reciprocal Rank Fusion을 사용한 전체 텍스트 + 벡터 검색
- **워크스페이스 파일시스템** - 노트, 로그, 컨텍스트를 위한 유연한 경로 기반 저장소
- **아이덴티티 파일** - 세션 간 일관된 성격과 선호도를 유지합니다

## 설치

### 사전 요구 사항

- Rust 1.85+
- [pgvector](https://github.com/pgvector/pgvector) 확장이 있는 PostgreSQL 15+
- NEAR AI 계정 (인증은 설정 마법사를 통해 처리됨)

## 다운로드 또는 빌드

[릴리스 페이지](https://github.com/nearai/ironclaw/releases/)를 방문하여 최신 업데이트를 확인하세요.

<details>
  <summary>Windows 인스톨러로 설치 (Windows)</summary>

[Windows 인스톨러](https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-x86_64-pc-windows-msvc.msi)를 다운로드하여 실행하세요.

</details>

<details>
  <summary>PowerShell 스크립트로 설치 (Windows)</summary>

```sh
irm https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.ps1 | iex
```

</details>

<details>
  <summary>셸 스크립트로 설치 (macOS, Linux, Windows/WSL)</summary>

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```
</details>

<details>
  <summary>Homebrew로 설치 (macOS/Linux)</summary>

```sh
brew install ironclaw
```

</details>

<details>
  <summary>소스 코드 컴파일 (Windows, Linux, macOS의 Cargo)</summary>

`cargo`로 설치하세요. 컴퓨터에 [Rust](https://rustup.rs)가 설치되어 있는지 확인하세요.

```bash
# 저장소 복제
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# 빌드
cargo build --release

# 테스트 실행
cargo test
```

**전체 릴리스**의 경우 (채널 소스를 수정한 후), `./scripts/build-all.sh`를 실행하여 채널을 먼저 다시 빌드하세요.

</details>

### 데이터베이스 설정

```bash
# 데이터베이스 생성
createdb ironclaw

# pgvector 활성화
psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

## 설정

설정 마법사를 실행하여 IronClaw를 구성하세요:

```bash
ironclaw onboard
```

마법사는 데이터베이스 연결, NEAR AI 인증 (브라우저 OAuth를 통해),
그리고 비밀 암호화 (시스템 키체인 사용)를 처리합니다. 설정은 연결된
데이터베이스에 저장됩니다. 부트스트랩 변수 (예: `DATABASE_URL`, `LLM_BACKEND`)는
데이터베이스가 연결되기 전에 사용할 수 있도록 `~/.ironclaw/.env`에 기록됩니다.

### 대체 LLM 공급자

IronClaw는 기본적으로 NEAR AI를 사용하지만 많은 LLM 공급자를 기본 지원합니다.
내장 공급자에는 **Anthropic**, **OpenAI**, **GitHub Copilot**, **Google Gemini**, **MiniMax**,
**Mistral**, **Ollama** (로컬)이 포함됩니다. **OpenRouter**
(300+ 모델), **Together AI**, **Fireworks AI**, 자체 호스팅 서버 (**vLLM**,
**LiteLLM**) 같은 OpenAI 호환 서비스도 지원됩니다.

마법사에서 공급자를 선택하거나 환경 변수를 직접 설정하세요:

```env
# 예: MiniMax (내장, 204K 컨텍스트)
LLM_BACKEND=minimax
MINIMAX_API_KEY=...

# 예: OpenAI 호환 엔드포인트
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

전체 공급자 가이드는 [docs/capabilities/llm-providers.md](docs/capabilities/llm-providers.md)를 참조하세요.

## 보안

IronClaw는 데이터를 보호하고 오용을 방지하기 위해 심층 방어를 구현합니다.

### WASM 샌드박스

신뢰할 수 없는 모든 도구는 격리된 WebAssembly 컨테이너에서 실행됩니다:

- **권한 기반 권한** - HTTP, 비밀, 도구 호출에 대한 명시적 옵트인
- **엔드포인트 화이트리스트** - HTTP 요청은 승인된 호스트/경로로만 전송됩니다
- **자격 증명 주입** - 비밀은 호스트 경계에서 주입되며, WASM 코드에 절대 노출되지 않습니다
- **누출 감지** - 비밀 유출 시도에 대해 요청과 응답을 스캔합니다
- **속도 제한** - 남용을 방지하기 위한 도구별 요청 제한
- **리소스 제한** - 메모리, CPU, 실행 시간 제약

```
WASM ──► 화이트리스트 ──► 누출 스캔 ──► 자격 증명 ──► 실행 ──► 누출 스캔 ──► WASM
         검증기            (요청)         주입기         요청       (응답)
```

### 프롬프트 인젝션 방어

외부 콘텐츠는 여러 보안 계층을 통과합니다:

- 인젝션 시도의 패턴 기반 감지
- 콘텐츠 정화 및 이스케이핑
- 심각도 수준이 있는 정책 규칙 (차단/경고/검토/정화)
- 안전한 LLM 컨텍스트 주입을 위한 도구 출력 래핑

### 데이터 보호

- 모든 데이터는 로컬 PostgreSQL 데이터베이스에 저장됩니다
- 비밀은 AES-256-GCM으로 암호화됩니다
- 텔레메트리, 분석, 데이터 공유 없음
- 모든 도구 실행에 대한 전체 감사 로그

## 아키텍처

```
┌────────────────────────────────────────────────────────────────┐
│                          채널                                  │
│  ┌──────┐  ┌──────┐   ┌─────────────┐  ┌─────────────┐         │
│  │ REPL │  │ HTTP │   │ WASM 채널   │  │ 웹 게이트웨이│         │
│  └──┬───┘  └──┬───┘   └──────┬──────┘  │ (SSE + WS)  │         │
│     │         │              │         └──────┬──────┘         │
│     └─────────┴──────────────┴────────────────┘                │
│                              │                                 │
│                    ┌─────────▼─────────┐                       │
│                    │   에이전트 루프   │  의도 라우팅         │
│                    └────┬──────────┬───┘                       │
│                         │          │                           │
│              ┌──────────▼────┐  ┌──▼───────────────┐           │
│              │  스케줄러     │  │  루틴 엔진       │           │
│              │ (병렬 작업)   │  │ (cron, 이벤트, wh)│          │
│              └──────┬────────┘  └────────┬─────────┘           │
│                     │                    │                     │
│       ┌─────────────┼────────────────────┘                     │
│       │             │                                          │
│   ┌───▼─────┐  ┌────▼────────────────┐                         │
│   │ 로컬    │  │   오케스트레이터    │                         │
│   │ 워커    │  │  ┌───────────────┐  │                         │
│   │(인프로세스)│ │ │Docker 샌드박스│ │                         │
│   └───┬─────┘  │  │   컨테이너    │  │                         │
│       │        │  │ ┌───────────┐ │  │                         │
│       │        │  │ │Worker / CC│ │  │                         │
│       │        │  │ └───────────┘ │  │                         │
│       │        │  └───────────────┘  │                         │
│       │        └─────────┬───────────┘                         │
│       └──────────────────┤                                     │
│                          │                                     │
│              ┌───────────▼──────────┐                          │
│              │   도구 레지스트리    │                          │
│              │  내장, MCP, WASM     │                          │
│              └──────────────────────┘                          │
└────────────────────────────────────────────────────────────────┘
```

### 핵심 구성 요소

| 구성 요소 | 목적 |
|-----------|---------|
| **에이전트 루프** | 주요 메시지 처리 및 작업 조정 |
| **라우터** | 사용자 의도 분류 (명령, 쿼리, 작업) |
| **스케줄러** | 우선순위가 있는 병렬 작업 실행 관리 |
| **워커** | LLM 추론과 도구 호출로 작업 실행 |
| **오케스트레이터** | 컨테이너 라이프사이클, LLM 프록시, 작업별 인증 |
| **웹 게이트웨이** | 채팅, 메모리, 작업, 로그, 확장, 루틴이 있는 브라우저 UI |
| **루틴 엔진** | 예약된 (cron) 및 반응형 (이벤트, 웹훅) 백그라운드 작업 |
| **워크스페이스** | 하이브리드 검색이 있는 영구 메모리 |
| **안전 계층** | 프롬프트 인젝션 방어 및 콘텐츠 정화 |

## 사용법

Engine v2는 현재 옵트인입니다. 기존 에이전트 루프 대신 새 엔진을 사용하려면 `ENGINE_V2=true`를 설정해서 IronClaw를 시작하세요.

```bash
# 첫 설정 (데이터베이스, 인증 등 구성)
ironclaw onboard

# 설치된 바이너리 시작
ironclaw

# Engine v2로 설치된 바이너리 시작
ENGINE_V2=true ironclaw

# 소스에서 대화형 REPL 시작
cargo run

# 소스에서 Engine v2 대화형 REPL 시작
ENGINE_V2=true cargo run

# Engine v2를 디버그 로깅과 함께 시작
ENGINE_V2=true RUST_LOG=ironclaw=debug cargo run
```

## 개발

```bash
# 코드 포맷
cargo fmt

# 린트
cargo clippy --all --benches --tests --examples --all-features

# 테스트 실행
createdb ironclaw_test
cargo test

# 특정 테스트 실행
cargo test test_name
```

- **채널**: Telegram, Discord 및 기타 채널 설정은 [docs/channels/overview.mdx](docs/channels/overview.mdx)를 참조하세요.
- **채널 소스 변경**: 업데이트된 WASM이 번들되도록 `cargo build` 전에 `./channels-src/telegram/build.sh`를 실행하세요.

## OpenClaw 역사

IronClaw는 [OpenClaw](https://github.com/openclaw/openclaw)에서 영감을 받은 Rust 재구현입니다. 전체 추적 매트릭스는 [FEATURE_PARITY.md](FEATURE_PARITY.md)를 참조하세요.

주요 차이점:

- **Rust vs TypeScript** - 네이티브 성능, 메모리 안전, 단일 바이너리
- **WASM 샌드박스 vs Docker** - 가벼운 권한 기반 보안
- **PostgreSQL vs SQLite** - 프로덕션 준비된 영속성
- **보안 우선 설계** - 다중 방어 계층, 자격 증명 보호

## 라이선스

다음 중 하나를 선택하여 라이선스가 부여됩니다:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

원하는 대로 선택할 수 있습니다.
