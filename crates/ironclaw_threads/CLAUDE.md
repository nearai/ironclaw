# ironclaw_threads guardrails

- Own canonical Reborn `session_threads`, transcript message contracts, message ordering/status/redaction semantics, context-window reads, and in-memory/fake contract stores.
- Do not depend on v1 `Agent`, v1 `SessionManager`, product/channel adapters, raw runtime dispatchers, raw provider clients, capability execution internals, or workspace/memory services.
- Keep turn/run lifecycle authority out of this crate; store only stable turn/run references supplied by `TurnCoordinator`.
- Preserve message identity and per-thread sequence across redaction/deletion; do not infer status from nullable turn/run refs.
- Use policy-filtered read APIs for model-visible context; never expose raw secrets, host paths, raw runtime/tool payloads, or private backend diagnostics as ordinary transcript content.
