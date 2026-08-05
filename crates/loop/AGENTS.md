# `crates/loop/` — the loop-hosting tier

Replaceable agent behavior and the adapters that connect it to the kernel: the canonical executor and its sealed families/planner, the host-port implementations over kernel services, the agent-turn executor and driver registry, and the trust-tiered hook framework. Userland strategy above the perimeter, never authority.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_agent_loop`](./ironclaw_agent_loop) | `loops` | Agent-loop framework state and strategy contracts |
| [`ironclaw_hooks`](./ironclaw_hooks) | `loops` | trust-tiered hook framework, the WASM hook engine, and Loop*Port middleware |
| [`ironclaw_loop_host`](./ironclaw_loop_host) | `loops` | Loop host adapters for IronClaw Reborn AgentLoopHost implementations |
| [`ironclaw_turn_runner`](./ironclaw_turn_runner) | `loops` | Reborn runner control plane and loop-driver adapters |

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/loop.md`](../../docs/reborn/target-architecture/families/loop.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
