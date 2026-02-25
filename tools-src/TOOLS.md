# SRE Tools

WASM tools for the elastic-elk AI-SRE pod. Each tool lives in its own directory with a `Cargo.toml`, a `*.capabilities.json` declaring HTTP allowlists and credential requirements, and a `src/lib.rs` WASM implementation.

## Status

- [x] GitHub — issue, PR, and alert management (`github/`)
- [x] Slack — incident notifications and channel messaging (`slack/`)
- [x] Okta — identity and SSO (`okta/`)
- [ ] Elasticsearch — log and metric queries (`elasticsearch/`) ← Phase 1
- [ ] kubectl — Kubernetes cluster inspection and remediation (`kubectl/`) ← Phase 2
- [ ] AWS — CloudWatch alarms, EC2, ECS/EKS read-only (`aws/`) ← Phase 3

## Tool Structure

```
tools-src/<name>/
├── Cargo.toml
├── README.md
├── <name>-tool.capabilities.json
└── src/
    └── lib.rs
```

## Capabilities Schema

Each `*.capabilities.json` declares:
- `http.allowlist` — permitted outbound hosts, path prefixes, and HTTP methods
- `http.credentials` — secrets injected as headers (bearer, basic, or custom)
- `http.rate_limit` — requests per minute / hour
- `secrets.allowed_names` — secret keys this tool may access
- `auth` — optional OAuth or manual token setup instructions
