---
paths:
  - "src/safety/**"
  - "src/docker.rs"
  - "src/secrets/**"
  - "src/tools/wasm/**"
---
# Safety Layer & Docker Rules

## Safety Layer

All external tool output passes through `SafetyLayer`:
1. **Sanitizer** - Detects injection patterns, escapes dangerous content
2. **Validator** - Checks length, encoding, forbidden patterns
3. **Policy** - Rules with severity (Critical/High/Medium/Low) and actions (Block/Warn/Review/Sanitize)
4. **Leak Detector** - Scans for 15+ secret patterns at two points: tool output before LLM, and LLM responses before user

Tool outputs are wrapped in `<tool_output>` XML before reaching the LLM.

## Shell Environment Scrubbing

The shell tool scrubs sensitive env vars before executing commands. The sanitizer detects command injection patterns (chained commands, subshells, path traversal).

## Container Isolation

When IronClaw runs as a managed agent container (via LobsterPool), container-level isolation is provided by the orchestrator:
- Memory/CPU limits, network egress filtering, workspace bind mounts
- Per-agent PostgreSQL database isolation
- LLM proxy with HMAC-signed auth

The `src/docker.rs` module provides Docker detection and connection utilities used by the orchestrator's container job management system.

## Zero-Exposure Credential Model

Secrets are stored encrypted on the host and injected into HTTP requests by the proxy at transit time. Container processes never see raw credential values.
