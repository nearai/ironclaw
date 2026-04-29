# ironclaw_wasm

Owns the Reborn WASM component runtime lane.

## Responsibilities

- Load, compile, validate, meter, and execute already-selected WASM components for Reborn.
- Use the canonical WIT/component-model ABI from `wit/tool.wit` and later `wit/channel.wit`.
- Provide thin host-import adapters for workspace, time, logging, secret-existence checks, tool invocation, and HTTP egress.
- Fail closed by default for host capabilities that are not explicitly wired by the Reborn composition root.

## Non-responsibilities

- Do not decide which tools/channels are exposed to the LLM.
- Do not own authorization, approvals, trust policy, dispatcher routing, run-state, or `CapabilityHost` orchestration.
- Do not perform direct production HTTP or secret retrieval; route those through injected host seams. Production HTTP egress belongs to the shared runtime egress service tracked by #3085.
- Do not modify or depend on V1 `src/tools/wasm/*` or `src/channels/wasm/*`; those are compatibility references only.

## Safety rules

- No JSON pointer/length ABI (`invoke_json`, `alloc`, `output_ptr`, `output_len`) in Reborn WASM.
- Instantiate fresh component instances per call.
- Preserve fuel, epoch timeout, memory, and table/instance limits.
- `ResourceUsage.network_egress_bytes` counts outbound request body bytes only; response-size limits are separate.
