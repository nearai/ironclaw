# IronClaw Gotify WASM Tool

A WASM tool for IronClaw that sends notifications to Gotify.

## Build Instructions

**Recommended build method (OR use build.sh):**

```bash
cd /home/sun/ironclaw/ironclaw-gotify-tool
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/gotify_tool.wasm ~/.ironclaw/tools/gotify.wasm
```

## Installation

After building:
1. Copy `gotify.wasm` to `~/.ironclaw/tools/gotify.wasm`
2. Restart IronClaw to pick up the new tool

## Usage

The Gotify capabilities are integrated into `tools/gotify`. Configure your Gotify endpoint in IronClaw's config.

---

*Built for IronClaw agent infrastructure*

*Enhanced for latest Ironclaw Release*

*works great with ic_sm, my custom secret management scripts (work for both libsql and postgres ironclaw databases to dynamically list and insert new secrets*

