# ironclaw_wasm guardrails

- Own loading, validating, metering, and executing an already-selected WASM capability.
- Keep host imports fail-closed: filesystem, network, resources, and other privileged effects must cross explicit host-provided interfaces.
- Do not decide LLM tool exposure, authorization, approval resolution, dispatcher routing, run-state, CapabilityHost behavior, or product workflow policy.
- Do not import authorization, approvals, dispatcher, capabilities, host-runtime, run-state, processes, secrets, or product workflow crates.
- Preserve sandbox boundaries: no ambient filesystem, network, process, environment, or credential authority should be added here.
- Keep guest-controlled strings, paths, URLs, schemas, and output bounded before host interaction or publication.
