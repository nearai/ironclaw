# M01 Tool

Local IronClaw WASM tool for selected read-only M01 JSON APIs.

## Prerequisites

- `cargo-component` installed
- `wasm32-wasip2` target installed
- IronClaw runtime environment variable `M01_TOOL_HOST` set to the host part of your M01 URL
- M01 API key stored as the `m01_api_key` secret or exposed through `M01_API_KEY`

Example:

```bash
export M01_TOOL_HOST="m01.example.com"
ironclaw secret set m01_api_key "<your-api-key>"
```

## Build

```bash
cd /Users/kpas/SynologyDrive/Github/ironclaw/tools-src/m01
cargo component build --release
```

构建后会生成：

- `target/wasm32-wasip1/release/m01_tool.wasm`
- `target/wasm32-wasip1/release/m01_tool.capabilities.json`

`m01_tool.capabilities.json` 会由 `build.rs` 自动复制到构建产物目录，供 `ironclaw tool install` 自动发现。

## Install

```bash
ironclaw tool install /Users/kpas/SynologyDrive/Github/ironclaw/tools-src/m01/target/wasm32-wasip1/release/m01_tool.wasm
```

## Supported actions

- `investigation_mail_list`
- `investigation_mail_tags`
- `investigation_mail_file_types`
- `investigation_mail_related_emails_detection`
- `workflow_queue_stats`
- `intelligence_local_stats`
- `safe_admin_firewall_zone_list`
- `system_admin_monitor_status`
