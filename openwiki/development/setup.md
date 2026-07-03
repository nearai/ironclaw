# Development Setup

This page covers setting up your local development environment for IronClaw.

## Quick Start (5 minutes)

```bash
# Clone the repo
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# Run the setup script (installs dependencies, sets up hooks, etc.)
./scripts/dev-setup.sh

# Verify the setup
cargo test --lib 2>&1 | head -20

# Run the CLI
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- --help
```

If all commands succeed, you're ready to start developing!

## Prerequisites

### Required
- **Rust 1.96+** — Install from [rustup.rs](https://rustup.rs/)
- **Cargo** — Comes with Rust
- **Git** — For cloning and hooks
- **Node.js 18+** — Required for WebUI (only if building with `webui-v2-beta` feature)
- **npm** — Comes with Node.js

### Optional
- **PostgreSQL 14+** — For testing dual-backend scenarios (tests use libSQL by default)
- **Docker** — For container builds
- **Docker Compose** — For PostgreSQL in CI-like environments

### System-Specific

**macOS:**
```bash
brew install rust node
```

**Ubuntu/Debian:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
sudo apt-get install build-essential pkg-config libssl-dev nodejs npm
```

**Windows:**
- Install Rust from [rustup.rs](https://rustup.rs/)
- Install Node.js from [nodejs.org](https://nodejs.org/)
- Use Git Bash or WSL for shell commands

## Installation

### 1. Clone the Repository

```bash
git clone https://github.com/nearai/ironclaw.git
cd ironclaw
```

### 2. Run the Setup Script

```bash
./scripts/dev-setup.sh
```

This script:
- Installs Rust components (clippy, rustfmt, wasm32)
- Sets up git hooks (pre-commit, commit-msg, pre-push)
- Installs Cargo tools (cargo-nextest, cargo-deny, cargo-watch)
- Builds initial WASM artifacts
- Validates the environment

### 3. Verify Installation

```bash
# Run unit tests (should pass)
cargo test --lib -- --test-threads 1

# Check Rust formatting
cargo fmt -- --check

# Check clippy (zero warnings)
cargo clippy -- -D warnings

# Run a simple command
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- config path
```

## Useful Commands

### Building

```bash
# Build the main binary (debug)
cargo build -p ironclaw_reborn_cli --bin ironclaw-reborn

# Build for release
cargo build --release -p ironclaw_reborn_cli --bin ironclaw-reborn

# Build with WebUI (requires Node.js)
cargo build -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn

# Build with Slack support
cargo build -p ironclaw_reborn_cli --features slack-v2-host-beta --bin ironclaw-reborn

# Build with all features
cargo build --all-features -p ironclaw_reborn_cli
```

### Testing

```bash
# Run all unit tests
cargo test --lib

# Run integration tests (requires PostgreSQL for some)
cargo test --test '*'

# Run a specific test file
cargo test --test executor_happy_paths

# Run tests with output visible
cargo test -- --nocapture

# Run tests in a specific crate
cargo test -p ironclaw_agent_loop

# Run tests with nightly features (if available)
cargo +nightly test
```

### Formatting and Linting

```bash
# Check formatting
cargo fmt -- --check

# Auto-fix formatting
cargo fmt

# Run clippy (lint checker)
cargo clippy -- -D warnings

# Run clippy with all features
cargo clippy --all-features -- -D warnings

# Check dependencies for vulnerabilities
cargo deny check

# Check for unused dependencies
cargo deny check advisories
```

### Development Workflow

```bash
# Watch for file changes and re-run tests
cargo watch -x test

# Build and run the CLI
cargo run -p ironclaw_reborn_cli --bin ironclaw-reborn -- run --message "hello"

# Start an interactive REPL
cargo run -p ironclaw_reborn_cli --bin ironclaw-reborn -- repl

# Start the WebUI service (requires webui-v2-beta feature)
cargo run -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve

# Get help for any command
cargo run -p ironclaw_reborn_cli --bin ironclaw-reborn -- --help
```

## Environment Setup

### Configure LLM (Required to Run)

Before running the CLI, you need to configure an LLM provider:

```bash
# Set up OpenAI (example)
export OPENAI_API_KEY="sk-..."
export IRONCLAW_REBORN_HOME="$PWD/.reborn-home"

# Configure the model route
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- \
  models set-provider openai --model gpt-4

# Verify the configuration
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models status

# Run a simple message
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- \
  run --message "hello"
```

### Supported LLM Providers

| Provider | Env Variable | Example | Optional |
|----------|--------------|---------|----------|
| OpenAI | `OPENAI_API_KEY` | `sk-...` | `OPENAI_MODEL`, `OPENAI_BASE_URL` |
| Anthropic | `ANTHROPIC_API_KEY` | `sk-ant-...` | `ANTHROPIC_MODEL`, `ANTHROPIC_BASE_URL` |
| OpenRouter | `OPENROUTER_API_KEY` | `sk-or-...` | `OPENROUTER_MODEL` |
| Ollama | (none, use `--llm-backend ollama`) | Local | `OLLAMA_BASE_URL`, `OLLAMA_MODEL` |
| OpenAI-compatible | `LLM_BASE_URL` | `http://...` | `LLM_API_KEY`, `LLM_MODEL` |

### Configure WebUI (Optional)

```bash
export IRONCLAW_REBORN_HOME="$PWD/.reborn-home"
export OPENAI_API_KEY="sk-..."
export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
export IRONCLAW_REBORN_WEBUI_USER_ID="dev-user"

# Build and serve (requires webui-v2-beta feature)
cargo run -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve

# Visit http://127.0.0.1:3000 in your browser
```

### Configure Secrets (Security)

For local development, secrets are stored unencrypted in the state root (`~/.ironclaw/reborn`):

```bash
# Set or get a secret
export IRONCLAW_REBORN_HOME="/var/lib/ironclaw-reborn"

# In production, use encrypted storage with a master key:
export IRONCLAW_REBORN_SECRET_MASTER_KEY="$(openssl rand -hex 32)"
```

**Never commit secrets to git.** Use env vars or .env files (add to .gitignore).

## Git Hooks

The setup script installs git hooks that enforce code quality:

### pre-commit
Runs on `git commit`:
- Format check (`cargo fmt`)
- Clippy lint (`cargo clippy -- -D warnings`)
- Dependency check (`cargo deny`)
- Detects hardcoded `/tmp` paths
- Validates UTF-8 in committed files

### commit-msg
Validates commit message format:
- Must match conventional commits: `type(scope): message`
- Types: `fix`, `feat`, `docs`, `style`, `refactor`, `test`, `chore`
- Example: `fix(agent-loop): handle timeout in capability request`

### pre-push
Runs on `git push`:
- All pre-commit checks
- Regression tests (tests marked with `#[ignore]`)
- Architecture boundary tests

If a hook fails, fix the issue and try again:
```bash
# If clippy fails, fix warnings
cargo clippy --fix

# If format fails, auto-fix
cargo fmt

# Skip hooks (last resort)
git commit --no-verify
```

## IDE Setup

### VS Code

```bash
# Install recommended extensions
code --install-extension rust-lang.rust-analyzer
code --install-extension serayuzgur.crates
code --install-extension fill-labs.dependi
```

Create `.vscode/settings.json`:
```json
{
  "[rust]": {
    "editor.formatOnSave": true,
    "editor.defaultFormatter": "rust-lang.rust-analyzer"
  },
  "rust-analyzer.check.command": "clippy",
  "rust-analyzer.check.extraArgs": ["--", "-D", "warnings"]
}
```

### IntelliJ IDEA / RustRover

- Install the Rust plugin
- Open the project root
- Let IDE index the codebase (~1 minute)
- Configure run configurations for cargo commands

### Neovim / Vim

Use rust.vim and rust-tools.nvim:
```bash
# Install neovim plugins (example using vim-plug)
Plug 'rust-lang/rust.vim'
Plug 'mrcjkb/rustaceanvim'
```

## Docker Setup

### Development Container

```bash
# Build the dev container (includes all dependencies)
docker build -f Dockerfile -t ironclaw:dev .

# Run tests in the container
docker run --rm ironclaw:dev cargo test --lib

# Interactive development shell
docker run -it --rm -v "$(pwd):/workspace" ironclaw:dev bash
```

### PostgreSQL for Testing

```bash
# Start PostgreSQL in Docker
docker-compose up -d postgres

# Run tests against PostgreSQL
IRONCLAW_HOOKS_POSTGRES_URL="postgres://ironclaw:ironclaw@127.0.0.1:5432/ironclaw" \
  cargo test --features postgres

# Stop PostgreSQL
docker-compose down
```

See `docker-compose.yml` for PostgreSQL configuration.

## Troubleshooting

### Problem: "Rust version too old"
```bash
rustup update
rustc --version  # Should be 1.96+
```

### Problem: "Missing WASM target"
```bash
rustup target add wasm32-unknown-unknown
```

### Problem: "Node.js not found" (when building WebUI)
```bash
node --version  # Should be 18+
npm --version   # Should be 9+
```

If not installed:
```bash
# macOS
brew install node

# Ubuntu/Debian
curl -sL https://deb.nodesource.com/setup_18.x | sudo -E bash -
sudo apt-get install -y nodejs
```

### Problem: "PostgreSQL connection refused"
```bash
# Check if PostgreSQL is running
psql --version

# If not, start it
docker-compose up -d postgres

# Or install locally:
# macOS: brew install postgresql
# Ubuntu: sudo apt-get install postgresql
```

### Problem: Clippy warnings won't go away
```bash
# Update Rust
rustup update

# Clean and rebuild
cargo clean
cargo build
```

### Problem: Tests failing with "permission denied"
```bash
# Make setup script executable
chmod +x scripts/dev-setup.sh

# Run it again
./scripts/dev-setup.sh
```

## Next Steps

- **Run tests:** `cargo test --lib` (should pass)
- **Review code style:** Read [AGENTS.md](/AGENTS.md#repo-wide-coding-rules)
- **Set up your IDE:** Follow IDE-specific instructions above
- **Configure your editor:** Enable clippy and fmt on save
- **Read the testing guide:** [Testing Guide](testing.md)
- **Start coding:** Pick an issue or feature from the tracker

---

## See Also

- **[Testing Guide](testing.md)** — How to write and run tests
- **[Development Workflows](workflows.md)** — How to fix bugs, add features, code review
- **[Architecture Overview](/openwiki/architecture/overview.md)** — System design
- **[README.md](/README.md)** — Deployment and production setup

---

**Last updated:** Auto-generated by OpenWiki. For setup issues, file an issue on GitHub.
