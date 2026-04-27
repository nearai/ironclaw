# ironclaw_host_runtime guardrails

- Own composition wiring only: shared handles into dispatcher, runtime adapters, capability host, approval resolver, obligation handlers, and process host.
- Do not implement authorization, approval, run-state, process lifecycle, runtime execution, or product workflow semantics here; adapter wrappers and handler pass-throughs should delegate to owning crates.
- Built-in obligation handling here is limited to metadata-only `AuditBefore`/`AuditAfter`, `ApplyNetworkPolicy` preflight and WASM host-HTTP policy/egress handoff, direct-handle `InjectSecretOnce` lease/consume into `RuntimeSecretInjectionStore` when a scoped `SecretStore` is configured, and immediate dispatch/resume `RedactOutput`/`EnforceOutputLimit`. Spawned/background process output handling, `InjectCredentialOnce`, account selection, OAuth repair, and generic runtime environment injection remain out of scope. Already-resolved `RuntimeHttpCredential` values may be injected only in the hardened WASM egress adapter after request leak scanning and before response leak scanning.
- Keep this crate logic-light; hardened HTTP/DNS behavior belongs in `ironclaw_network`, while this crate only adapts it to runtime host imports.
- Use `AuditSink` for control-plane audit and `EventSink` for runtime/process events.
- Preserve shared service handles so capability/process/approval paths do not split stores by accident.
