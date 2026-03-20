<p align="center">
  <img src="ironclaw.png?v=2" alt="IronClaw" width="200"/>
</p>

<h1 align="center">IronClaw</h1>

<p align="center">
  <strong>安全可靠的个人 AI 助手，始终站在你这边</strong>
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
  <a href="https://t.me/ironclawAI"><img src="https://img.shields.io/badge/Telegram-%40ironclawAI-26A5E4?style=flat&logo=telegram&logoColor=white" alt="Telegram: @ironclawAI" /></a>
  <a href="https://www.reddit.com/r/ironclawAI/"><img src="https://img.shields.io/badge/Reddit-r%2FironclawAI-FF4500?style=flat&logo=reddit&logoColor=white" alt="Reddit: r/ironclawAI" /></a>
</p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ru.md">Русский</a> |
  <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <a href="#设计理念">设计理念</a> •
  <a href="#功能特性">功能特性</a> •
  <a href="#安装">安装</a> •
  <a href="#配置">配置</a> •
  <a href="#安全机制">安全机制</a> •
  <a href="#系统架构">系统架构</a>
</p>

---

## 设计理念

IronClaw 基于一个简单的原则：**你的 AI 助手应该为你服务，而不是与你为敌。**

在 AI 系统对数据处理日益不透明、与企业利益捆绑的今天，IronClaw 选择了一条不同的路：

- **数据归你所有** — 所有信息存储在本地，加密保护，始终在你掌控之下
- **透明至上** — 完全开源，可审计，没有隐藏的遥测或数据收集
- **自主扩展** — 随时构建新工具，无需等待供应商更新
- **纵深防御** — 多层安全机制抵御提示注入和数据泄露

IronClaw 是一个你真正可以信赖的 AI 助手，无论是个人生活还是工作。

## 功能特性

### 安全优先

- **WASM 沙箱** — 不受信任的工具在隔离的 WebAssembly 容器中运行，采用基于能力的权限模型
- **凭据保护** — 密钥永远不会暴露给工具；在宿主边界注入并进行泄露检测
- **提示注入防御** — 模式检测、内容清理和策略执行
- **端点白名单** — HTTP 请求仅限于明确批准的主机和路径

### 随时可用

- **多渠道接入** — REPL、HTTP webhook、WASM 渠道（Telegram、Slack）和 Web 网关
- **Docker 沙箱** — 隔离的容器执行，支持每任务令牌和编排器/工作器模式
- **Web 网关** — 浏览器 UI，支持实时 SSE/WebSocket 流式传输
- **定时任务** — Cron 调度、事件触发器、Webhook 处理器，实现后台自动化
- **心跳系统** — 主动后台执行，用于监控和维护任务
- **并行任务** — 使用隔离上下文同时处理多个请求
- **自修复** — 自动检测并恢复卡住的操作

### 自主扩展

- **动态工具构建** — 描述你的需求，IronClaw 会将其构建为 WASM 工具
- **MCP 协议** — 连接模型上下文协议（Model Context Protocol）服务器以获取额外能力
- **插件架构** — 无需重启即可加载新的 WASM 工具和渠道

### 持久记忆

- **混合搜索** — 全文搜索 + 向量搜索，采用倒数排名融合（Reciprocal Rank Fusion）
- **工作空间文件系统** — 灵活的基于路径的存储，用于笔记、日志和上下文
- **身份文件** — 跨会话保持一致的个性和偏好设置

## 安装

### 前置要求

- Rust 1.85+
- PostgreSQL 15+，需安装 [pgvector](https://github.com/pgvector/pgvector) 扩展
- NEAR AI 账户（通过设置向导进行身份验证）

## 下载或编译

访问 [Releases 页面](https://github.com/nearai/ironclaw/releases/) 查看最新版本。

<details>
  <summary>通过 Windows 安装程序安装 (Windows)</summary>

下载 [Windows 安装程序](https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-x86_64-pc-windows-msvc.msi) 并运行。

</details>

<details>
  <summary>通过 PowerShell 脚本安装 (Windows)</summary>

```sh
irm https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.ps1 | iex
```

</details>

<details>
  <summary>通过 Shell 脚本安装 (macOS、Linux、Windows/WSL)</summary>

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```
</details>

<details>
  <summary>通过 Homebrew 安装 (macOS/Linux)</summary>

```sh
brew install ironclaw
```

</details>

<details>
  <summary>从源码编译 (Windows、Linux、macOS 上使用 Cargo)</summary>

确保你已安装 [Rust](https://rustup.rs)。

```bash
# 克隆仓库
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# 编译
cargo build --release

# 运行测试
cargo test
```

如需进行**完整发布构建**（修改了渠道源码后），先运行 `./scripts/build-all.sh` 重新编译渠道。

</details>

### 数据库设置

```bash
# 创建数据库
createdb ironclaw

# 启用 pgvector 扩展
psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

## 配置

运行设置向导来配置 IronClaw：

```bash
ironclaw onboard
```

向导将引导你完成数据库连接、NEAR AI 身份验证（通过浏览器 OAuth）和密钥加密（使用系统钥匙串）。设置会保存在数据库中；引导变量（如 `DATABASE_URL`、`LLM_BACKEND`）写入 `~/.ironclaw/.env`，以便在数据库连接前可用。

### 替代 LLM 提供商

IronClaw 默认使用 NEAR AI，但开箱即用地支持多种 LLM 提供商。
内置提供商包括 **Anthropic**、**OpenAI**、**Google Gemini**、**MiniMax**、**Mistral** 和 **Ollama**（本地部署）。同时也支持 OpenAI 兼容服务，如 **OpenRouter**（300+ 模型）、**Together AI**、**Fireworks AI** 以及自托管服务器（**vLLM**、**LiteLLM**）。

在向导中选择你的提供商，或直接设置环境变量：

```env
# 示例：MiniMax（内置，204K 上下文）
LLM_BACKEND=minimax
MINIMAX_API_KEY=...

# 示例：OpenAI 兼容端点
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

详见 [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md) 获取完整的提供商指南。

## 安全机制

IronClaw 实现了纵深防御策略来保护你的数据并防止滥用。

### WASM 沙箱

所有不受信任的工具都在隔离的 WebAssembly 容器中运行：

- **基于能力的权限** — 明确授权 HTTP、密钥、工具调用等能力
- **端点白名单** — HTTP 请求仅限已批准的主机和路径
- **凭据注入** — 密钥在宿主边界注入，永远不会暴露给 WASM 代码
- **泄露检测** — 扫描请求和响应以防止密钥外泄
- **速率限制** — 每个工具独立的请求限制，防止滥用
- **资源限制** — 内存、CPU 和执行时间约束

```
WASM ──► 白名单  ──► 泄露扫描 ──► 凭据  ──► 执行  ──► 泄露扫描 ──► WASM
         验证器     (请求)      注入器    请求     (响应)
```

### 提示注入防御

外部内容需通过多个安全层：

- 基于模式的注入尝试检测
- 内容清理和转义
- 带严重级别的策略规则（阻止/警告/审核/清理）
- 工具输出包装，确保安全的 LLM 上下文注入

### 数据保护

- 所有数据存储在本地 PostgreSQL 数据库中
- 密钥使用 AES-256-GCM 加密
- 无遥测、无分析、无数据共享
- 所有工具执行的完整审计日志

## 系统架构

```
┌────────────────────────────────────────────────────────────────┐
│                            渠道                                 │
│  ┌──────┐  ┌──────┐   ┌─────────────┐  ┌─────────────┐         │
│  │ REPL │  │ HTTP │   │ WASM 渠道   │  │  Web 网关   │         │
│  └──┬───┘  └──┬───┘   └──────┬──────┘  │ (SSE + WS)  │         │
│     │         │              │         └──────┬──────┘         │
│     └─────────┴──────────────┴────────────────┘                │
│                              │                                 │
│                    ┌─────────▼─────────┐                       │
│                    │    代理循环       │  意图路由              │
│                    └────┬──────────┬───┘                       │
│                         │          │                           │
│              ┌──────────▼────┐  ┌──▼───────────────┐           │
│              │    调度器      │  │   定时任务引擎    │           │
│              │  (并行任务)    │  │(cron, 事件, Webhook)│          │
│              └──────┬────────┘  └────────┬─────────┘           │
│                     │                    │                     │
│       ┌─────────────┼────────────────────┘                     │
│       │             │                                          │
│   ┌───▼─────┐  ┌────▼────────────────┐                         │
│   │  本地   │  │      编排器          │                         │
│   │ 工作器  │  │  ┌───────────────┐  │                         │
│   │(进程内) │  │  │ Docker 沙箱   │  │                         │
│   └───┬─────┘  │  │     容器      │  │                         │
│       │        │  │ ┌───────────┐ │  │                         │
│       │        │  │ │工作器/CC  │ │  │                         │
│       │        │  │ └───────────┘ │  │                         │
│       │        │  └───────────────┘  │                         │
│       │        └─────────┬───────────┘                         │
│       └──────────────────┤                                     │
│                          │                                     │
│              ┌───────────▼──────────┐                          │
│              │      工具注册表       │                          │
│              │ 内置、MCP、WASM      │                          │
│              └──────────────────────┘                          │
└────────────────────────────────────────────────────────────────┘
```

### 核心组件

| 组件 | 用途 |
|------|------|
| **代理循环** | 主消息处理和任务协调 |
| **路由器** | 分类用户意图（命令、查询、任务） |
| **调度器** | 管理带优先级的并行任务执行 |
| **工作器** | 执行包含 LLM 推理和工具调用的任务 |
| **编排器** | 容器生命周期、LLM 代理、每任务认证 |
| **Web 网关** | 浏览器 UI，含聊天、记忆、任务、日志、扩展、定时任务 |
| **定时任务引擎** | 定时（cron）和响应式（事件、webhook）后台任务 |
| **工作空间** | 带混合搜索的持久记忆 |
| **安全层** | 提示注入防御和内容清理 |

## 使用方式

```bash
# 首次设置（配置数据库、认证等）
ironclaw onboard

# 启动交互式 REPL
cargo run

# 启用调试日志
RUST_LOG=ironclaw=debug cargo run
```

## 开发

```bash
# 格式化代码
cargo fmt

# 代码检查
cargo clippy --all --benches --tests --examples --all-features

# 运行测试
createdb ironclaw_test
cargo test

# 运行指定测试
cargo test test_name
```

- **Telegram 渠道**：参见 [docs/TELEGRAM_SETUP.md](docs/TELEGRAM_SETUP.md) 了解设置和私信配对。
- **修改渠道源码**：在 `cargo build` 之前运行 `./channels-src/telegram/build.sh` 以便打包更新后的 WASM。

## OpenClaw 传承

IronClaw 是受 [OpenClaw](https://github.com/openclaw/openclaw) 启发的 Rust 重新实现。参见 [FEATURE_PARITY.md](FEATURE_PARITY.md) 了解完整的功能追踪矩阵。

主要差异：

- **Rust vs TypeScript** — 原生性能、内存安全、单一二进制文件
- **WASM 沙箱 vs Docker** — 轻量级、基于能力的安全机制
- **PostgreSQL vs SQLite** — 生产级持久化存储
- **安全优先设计** — 多层防御、凭据保护

## 许可证

可选择以下任一许可证：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))
