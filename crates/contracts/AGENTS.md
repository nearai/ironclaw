# `crates/contracts/` — neutral vocabulary and ports

The vocabulary tier: the one family every other family depends on, and which depends on nothing itself. A type earns a home here when it names a concept crossing an authority, host, or product boundary, is neutral with respect to vendor/runtime/storage/deployment, is needed by two or more consumers that must not import one another, and carries no execution, persistence, policy engine, or workflow. Nothing here runs, stores, or decides; it only names.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_common`](./ironclaw_common) | `contracts` | Shared types, paths, and platform helpers used across the IronClaw workspace |
| [`ironclaw_extension_contracts`](./ironclaw_extension_contracts) | `contracts` | The extension-tier contract for IronClaw Reborn: what an installable extension declares and exposes — manifest surfaces, auth recipes, lifecycle states, and the vendor-implemented preference-target port |
| [`ironclaw_host_api`](./ironclaw_host_api) | `contracts` | Shared host API contracts |
| [`ironclaw_loop_contracts`](./ironclaw_loop_contracts) | `contracts` | The loop-tier contract for IronClaw Reborn: the Loop*Port set, driver/host vocabulary, run-profile shapes, and the LoopExit claim |
| [`ironclaw_product_contracts`](./ironclaw_product_contracts) | `contracts` | The product-tier contract for IronClaw Reborn: the ProductSurface membrane, its caller/descriptor types, the product wire DTOs every transport speaks, and the product-side ports whose implementations sit beside product |
| [`ironclaw_prompt_envelope`](./ironclaw_prompt_envelope) | `contracts` | the untrusted-snippet envelope (leaf) |

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/contracts.md`](../../docs/reborn/target-architecture/families/contracts.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
