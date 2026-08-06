# `crates/product/` — first-party userland above the kernel

The supported first-party experience: the single place that turns validated channel and HTTP traffic into admitted, idempotent, durably bound turns, and turns kernel and domain state back into redacted, product-safe views and deliveries. Product calls the kernel; it never reaches around it.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_assistant`](./ironclaw_assistant) | `products` | Product-facing workflow service for IronClaw Reborn (#3280) |
| [`ironclaw_host_ingress`](./ironclaw_host_ingress) | `substrates` | Host HTTP ingress route mount carriers |
| [`ironclaw_openai_compat`](./ironclaw_openai_compat) | `products` | Reborn-native OpenAI-compatible API contract surface |
| [`ironclaw_operator`](./ironclaw_operator) | `products` | Host/operator control-plane services |
| [`ironclaw_webui`](./ironclaw_webui) | `products` | Host-owned listener binding + serve loop for the Reborn WebChat v2 HTTP gateway |

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/product.md`](../../docs/reborn/target-architecture/families/product.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
