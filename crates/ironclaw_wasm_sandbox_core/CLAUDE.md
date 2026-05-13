# ironclaw_wasm_sandbox_core guardrails

Owns shared v1-style WASM sandbox primitives for IronClaw runtimes.

- Keep this crate domain-free: no ProductAdapter, tool, channel, workflow, dispatcher, secret, network, filesystem, host-runtime, or app composition dependencies.
- Provide only Wasmtime/WASI sandbox kernel pieces: component-engine setup, epoch ticker, minimal WASI p2 linker, resource limiter, limits, and store-core helpers.
- Preserve v1-style minimal WASI semantics: clock/random allowed; env, args, stdio inheritance, preopened directories, inherited network, and DNS lookup disabled.
- Do not add custom host imports here. Runtime-specific crates own their WIT bindings and host trait implementations.
- Do not perform HTTP, DNS/private-IP checks, secret injection, leak scanning, redaction, workspace reads, tool invocation, product workflow calls, or channel lifecycle logic here.

Tests:

- Unit tests cover limiter behavior and minimal WASI defaults when mechanically observable.
- `ironclaw_architecture` pins workspace independence and ProductAdapter usage.
