<p align="center">
  <img src="ironclaw.png?v=2" alt="IronClaw" width="200"/>
</p>

<h1 align="center">IronClaw</h1>

<p align="center">
  <strong>あなたの味方になる、安全なパーソナルAIアシスタント</strong>
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
  <a href="README.ja.md">日本語</a> |
  <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <a href="#フィロソフィー">フィロソフィー</a> •
  <a href="#機能">機能</a> •
  <a href="#インストール">インストール</a> •
  <a href="#設定">設定</a> •
  <a href="#セキュリティ">セキュリティ</a> •
  <a href="#アーキテクチャ">アーキテクチャ</a>
</p>

---

## フィロソフィー

IronClawはシンプルな原則に基づいて構築されています：**あなたのAIアシスタントは、あなたのために働くべきであり、あなたに不利益をもたらすべきではありません。**

AIシステムがデータの取り扱いについて不透明になり、企業の利益に沿って調整されることが増えている世界で、IronClawは異なるアプローチを取ります：

- **あなたのデータはあなたのもの** - すべての情報はローカルに保存・暗号化され、あなたの管理下から離れることはありません
- **設計段階からの透明性** - オープンソース、監査可能、隠れたテレメトリやデータ収集なし
- **自己拡張する能力** - ベンダーのアップデートを待たずに、新しいツールをその場で構築
- **多層防御** - 複数のセキュリティレイヤーがプロンプトインジェクションやデータ流出から保護

IronClawは、個人生活にも仕事にも本当に信頼できるAIアシスタントです。

## 機能

### セキュリティファースト

- **WASMサンドボックス** - 信頼されていないツールは、機能ベースの権限を持つ隔離されたWebAssemblyコンテナで実行
- **認証情報の保護** - シークレットはツールに公開されず、リーク検出付きでホスト境界で注入
- **プロンプトインジェクション防御** - パターン検出、コンテンツサニタイズ、ポリシー適用
- **エンドポイントの許可リスト** - HTTPリクエストは明示的に許可されたホストとパスのみに制限

### 常時利用可能

- **マルチチャネル** - REPL、HTTPウェブフック、WASMチャネル（Telegram、Slack）、Webゲートウェイ
- **Dockerサンドボックス** - ジョブごとのトークンとオーケストレーター/ワーカーパターンによる隔離されたコンテナ実行
- **Webゲートウェイ** - リアルタイムSSE/WebSocketストリーミング対応のブラウザUI
- **ルーティン** - cronスケジュール、イベントトリガー、ウェブフックハンドラーによるバックグラウンド自動化
- **ハートビートシステム** - 監視・保守タスクのためのプロアクティブなバックグラウンド実行
- **並列ジョブ** - 隔離されたコンテキストで複数のリクエストを同時に処理
- **自己修復** - スタックした操作の自動検出と復旧

### 自己拡張

- **動的ツール構築** - 必要なものを説明すると、IronClawがWASMツールとして構築
- **MCPプロトコル** - Model Context Protocolサーバーに接続して追加機能を利用
- **プラグインアーキテクチャ** - 再起動なしで新しいWASMツールやチャネルを追加

### 永続メモリ

- **ハイブリッド検索** - Reciprocal Rank Fusionを使用した全文検索+ベクトル検索
- **ワークスペースファイルシステム** - メモ、ログ、コンテキストのための柔軟なパスベースストレージ
- **アイデンティティファイル** - セッション間で一貫した人格と設定を維持

## インストール

### 前提条件

- Rust 1.85+
- PostgreSQL 15+ ([pgvector](https://github.com/pgvector/pgvector)拡張機能を含む)
- NEAR AIアカウント（セットアップウィザードで認証を処理）

## ダウンロードまたはビルド

最新のアップデートは[リリースページ](https://github.com/nearai/ironclaw/releases/)をご覧ください。

<details>
  <summary>Windowsインストーラーでインストール（Windows）</summary>

[Windowsインストーラー](https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-x86_64-pc-windows-msvc.msi)をダウンロードして実行してください。

</details>

<details>
  <summary>PowerShellスクリプトでインストール（Windows）</summary>

```sh
irm https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.ps1 | iex
```

</details>

<details>
  <summary>シェルスクリプトでインストール（macOS、Linux、Windows/WSL）</summary>

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```
</details>

<details>
  <summary>Homebrewでインストール（macOS/Linux）</summary>

```sh
brew install ironclaw
```

</details>

<details>
  <summary>ソースコードからコンパイル（Windows、Linux、macOSでCargo）</summary>

`cargo`でインストールします。コンピューターに[Rust](https://rustup.rs)がインストールされていることを確認してください。

```bash
# リポジトリをクローン
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# ビルド
cargo build --release

# テストを実行
cargo test
```

**フルリリース**（チャネルソースを変更した後）の場合、まず`./scripts/build-all.sh`を実行してチャネルを再ビルドしてください。

</details>

### データベースのセットアップ

```bash
# データベースを作成
createdb ironclaw

# pgvectorを有効化
psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

## 設定

セットアップウィザードを実行してIronClawを設定します：

```bash
ironclaw onboard
```

ウィザードは、データベース接続、NEAR AI認証（ブラウザOAuth経由）、シークレットの暗号化（システムキーチェーンを使用）を処理します。設定は接続されたデータベースに永続化されます。ブートストラップ変数（例：`DATABASE_URL`、`LLM_BACKEND`）は、データベース接続前に利用できるよう`~/.ironclaw/.env`に書き込まれます。

### 代替LLMプロバイダー

IronClawはデフォルトでNEAR AIを使用しますが、多くのLLMプロバイダーをすぐに利用できます。組み込みプロバイダーには**Anthropic**、**OpenAI**、**Google Gemini**、**MiniMax**、**Mistral**、**Ollama**（ローカル）が含まれます。**OpenRouter**（300以上のモデル）、**Together AI**、**Fireworks AI**、セルフホストサーバー（**vLLM**、**LiteLLM**）などのOpenAI互換サービスもサポートされています。

ウィザードでプロバイダーを選択するか、環境変数を直接設定してください：

```env
# 例：MiniMax（組み込み、204Kコンテキスト）
LLM_BACKEND=minimax
MINIMAX_API_KEY=...

# 例：OpenAI互換エンドポイント
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

完全なプロバイダーガイドは[docs/capabilities/llm-providers.md](docs/capabilities/llm-providers.md)をご覧ください。

## セキュリティ

IronClawは、データを保護し悪用を防ぐために多層防御を実装しています。

### WASMサンドボックス

すべての信頼されていないツールは、隔離されたWebAssemblyコンテナで実行されます：

- **機能ベースの権限** - HTTP、シークレット、ツール呼び出しの明示的なオプトイン
- **エンドポイントの許可リスト** - 許可されたホスト/パスへのHTTPリクエストのみ
- **認証情報の注入** - シークレットはホスト境界で注入され、WASMコードに公開されない
- **リーク検出** - リクエストとレスポンスのシークレット流出試行をスキャン
- **レート制限** - 悪用防止のためのツールごとのリクエスト制限
- **リソース制限** - メモリ、CPU、実行時間の制約

```
WASM ──► 許可リスト ──► リーク    ──► 認証情報 ──► リクエスト ──► リーク    ──► WASM
         バリデーター    スキャン       注入        実行          スキャン
                       (リクエスト)                             (レスポンス)
```

### プロンプトインジェクション防御

外部コンテンツは複数のセキュリティレイヤーを通過します：

- パターンベースのインジェクション試行検出
- コンテンツのサニタイズとエスケープ
- 重要度レベル付きポリシールール（ブロック/警告/レビュー/サニタイズ）
- 安全なLLMコンテキスト注入のためのツール出力ラッピング

### データ保護

- すべてのデータはローカルのPostgreSQLデータベースに保存
- AES-256-GCMでシークレットを暗号化
- テレメトリ、分析、データ共有なし
- すべてのツール実行の完全な監査ログ

## アーキテクチャ

```
┌────────────────────────────────────────────────────────────────┐
│                          チャネル                               │
│  ┌──────┐  ┌──────┐   ┌─────────────┐  ┌─────────────┐         │
│  │ REPL │  │ HTTP │   │WASMチャネル │  │ Web         │         │
│  └──┬───┘  └──┬───┘   └──────┬──────┘  │ ゲートウェイ│         │
│     │         │              │         │(SSE + WS)   │         │
│     │         │              │         └──────┬──────┘         │
│     └─────────┴──────────────┴────────────────┘                │
│                              │                                 │
│                    ┌─────────▼─────────┐                       │
│                    │  エージェントループ │  インテントルーティング│
│                    └────┬──────────┬───┘                       │
│                         │          │                           │
│              ┌──────────▼────┐  ┌──▼───────────────┐           │
│              │ スケジューラー │  │ ルーティン       │           │
│              │ (並列ジョブ)  │  │ エンジン         │           │
│              └──────┬────────┘  │(cron,event,wh)   │           │
│                     │           └────────┬─────────┘           │
│       ┌─────────────┼────────────────────┘                     │
│       │             │                                          │
│   ┌───▼─────┐  ┌────▼────────────────┐                         │
│   │ ローカル │  │  オーケストレーター  │                         │
│   │ ワーカー │  │  ┌───────────────┐  │                         │
│   │(プロセス │  │  │ Docker        │  │                         │
│   │ 内)     │  │  │ サンドボックス│  │                         │
│   └───┬─────┘  │  │ コンテナ      │  │                         │
│       │        │  │ ┌───────────┐ │  │                         │
│       │        │  │ │Worker / CC│ │  │                         │
│       │        │  │ └───────────┘ │  │                         │
│       │        │  └───────────────┘  │                         │
│       │        └─────────┬───────────┘                         │
│       └──────────────────┤                                     │
│                          │                                     │
│              ┌───────────▼──────────┐                          │
│              │   ツールレジストリ    │                          │
│              │ 組み込み, MCP, WASM  │                          │
│              └──────────────────────┘                          │
└────────────────────────────────────────────────────────────────┘
```

### コアコンポーネント

| コンポーネント | 目的 |
|---------------|------|
| **エージェントループ** | メインのメッセージ処理とジョブの調整 |
| **ルーター** | ユーザーの意図を分類（コマンド、クエリ、タスク） |
| **スケジューラー** | 優先度付きの並列ジョブ実行を管理 |
| **ワーカー** | LLM推論とツール呼び出しでジョブを実行 |
| **オーケストレーター** | コンテナのライフサイクル、LLMプロキシ、ジョブごとの認証 |
| **Webゲートウェイ** | チャット、メモリ、ジョブ、ログ、拡張機能、ルーティンのブラウザUI |
| **ルーティンエンジン** | スケジュール（cron）とリアクティブ（イベント、ウェブフック）のバックグラウンドタスク |
| **ワークスペース** | ハイブリッド検索付き永続メモリ |
| **セーフティレイヤー** | プロンプトインジェクション防御とコンテンツサニタイズ |

## 使い方

```bash
# 初回セットアップ（データベース、認証などを設定）
ironclaw onboard

# インタラクティブREPLを起動
cargo run

# デバッグログ付き
RUST_LOG=ironclaw=debug cargo run
```

## 開発

```bash
# コードフォーマット
cargo fmt

# リント
cargo clippy --all --benches --tests --examples --all-features

# テスト実行
createdb ironclaw_test
cargo test

# 特定のテストを実行
cargo test test_name
```

- **チャネル**: Telegram、Discord、その他のチャネルの設定は[docs/channels/overview.mdx](docs/channels/overview.mdx)を参照してください。
- **チャネルソースの変更**: `cargo build`の前に`./channels-src/telegram/build.sh`を実行して、更新されたWASMをバンドルしてください。

## OpenClawの系譜

IronClawは[OpenClaw](https://github.com/openclaw/openclaw)にインスパイアされたRust再実装です。完全な対応表は[FEATURE_PARITY.md](FEATURE_PARITY.md)をご覧ください。

主な違い：

- **Rust vs TypeScript** - ネイティブパフォーマンス、メモリ安全性、シングルバイナリ
- **WASMサンドボックス vs Docker** - 軽量、機能ベースのセキュリティ
- **PostgreSQL vs SQLite** - 本番環境対応の永続化
- **セキュリティファースト設計** - 複数の防御レイヤー、認証情報の保護

## ライセンス

以下のいずれかのライセンスの下で提供されています：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

お好みに応じて選択してください。
