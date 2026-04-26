# ironclaw_host_runtime guardrails

- Own composition wiring only: shared handles into dispatcher, runtime adapters, capability host, approval resolver, and process host.
- Do not implement authorization, approval, run-state, obligation, process lifecycle, runtime execution, or product workflow semantics here; adapter wrappers and handler pass-throughs should delegate to owning crates.
- Keep this crate logic-light; if behavior grows, move it to the owning service crate.
- Use `AuditSink` for control-plane audit and `EventSink` for runtime/process events.
- Preserve shared service handles so capability/process/approval paths do not split stores by accident.
