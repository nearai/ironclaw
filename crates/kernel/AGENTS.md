# `crates/kernel/` — the authority perimeter

A security perimeter, not a crate. Everything that decides whether an action is allowed, reserves the resources it consumes, mints the evidence that it was authorized, and owns the process and turn lifecycles that carry it. Authority originates here and nowhere else.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_approvals`](./ironclaw_approvals) | `kernel` | exact-invocation approval resolution and its policy stores |
| [`ironclaw_authorization`](./ironclaw_authorization) | `kernel` | grant matching and capability leases |
| [`ironclaw_capabilities`](./ironclaw_capabilities) | `kernel` | CapabilityHost workflows and the runtime dispatcher |
| [`ironclaw_host_runtime`](./ironclaw_host_runtime) | `kernel` | the kernel service graph: obligations, mediated egress and secret staging, lane executor, dispatch composition |
| [`ironclaw_processes`](./ironclaw_processes) | `kernel` | process lifecycle authority, the row-native journal, and the supervisor |
| [`ironclaw_resources`](./ironclaw_resources) | `kernel` | Resource reservation governor |
| [`ironclaw_runtime_policy`](./ironclaw_runtime_policy) | `kernel` | Runtime profile resolver |
| [`ironclaw_trust`](./ironclaw_trust) | `kernel` | Host-controlled trust-class policy engine |
| [`ironclaw_turns`](./ironclaw_turns) | `kernel` | Host-layer turn coordination contracts |

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/kernel.md`](../../docs/reborn/target-architecture/families/kernel.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
