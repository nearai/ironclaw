# ironclaw_extensions guardrails

- Own extension manifest parsing, typed package metadata, capability descriptors, runtime declarations, and the in-memory extension registry.
- Keep this crate declarative. Do not execute tools, resolve authorization, perform network I/O, read secrets, spawn processes, or inspect WASM/script/MCP payloads here.
- Depend only on neutral substrate crates such as `ironclaw_host_api`; runtime, host composition, authorization, approvals, networking, and persistence live in their owning crates.
- Preserve manifest validation as fail-closed and stable: unknown/invalid capability ids, provider mismatches, malformed paths, duplicate capabilities, or unsupported runtime shapes should not be papered over.
- Keep package roots virtual-path based. Do not introduce raw host paths or product-specific workspace assumptions.
- Registry lookups should remain deterministic and side-effect free; callers own trust, visibility filtering, and execution policy.
