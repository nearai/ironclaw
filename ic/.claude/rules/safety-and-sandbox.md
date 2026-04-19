---
paths:
  - "src/safety/**"
  - "src/sandbox/**"
  - "src/secrets/**"
  - "src/tools/wasm/**"
---
# Safety Layer & Sandbox Rules

## Safety Layer

All external tool output passes through `SafetyLayer`:
1. **Sanitizer** - Detects injection patterns, escapes dangerous content
2. **Validator** - Checks length, encoding, forbidden patterns
3. **Policy** - Rules with severity (Critical/High/Medium/Low) and actions (Block/Warn/Review/Sanitize)
4. **Leak Detector** - Scans for 15+ secret patterns at two points: tool output before LLM, and LLM responses before user

Tool outputs are wrapped in `<tool_output>` XML before reaching the LLM.

## Shell Environment Scrubbing

The shell tool scrubs sensitive env vars before executing commands. The sanitizer detects command injection patterns (chained commands, subshells, path traversal).

## Sandbox Policies

| Policy | Filesystem | Network |
|--------|-----------|---------|
| ReadOnly | Read-only workspace | Allowlisted domains |
| WorkspaceWrite | Read-write workspace | Allowlisted domains |
| FullAccess | Full filesystem | Unrestricted |

## Zero-Exposure Credential Model

Secrets are stored encrypted on the host and injected into HTTP requests by the proxy at transit time. Container processes never see raw credential values.
