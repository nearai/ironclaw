# `crates/extensions/` — everything "installable package"

Every concern that follows from "an extension is an installable package", short of the vocabulary that concern is expressed in (that lives in `contracts/`): manifests and the registry, the generic lifecycle/binding host with its ingress router and egress transports, the product-side management surface, the shared native executors, and `packages/` — one self-contained directory per installable package.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_extension_host`](./ironclaw_extension_host) | `loops` | Generic extension lifecycle host, active snapshot, loaders, and resolvers |
| [`ironclaw_extension_manager`](./ironclaw_extension_manager) | `products` | Product face of extensions: lifecycle commands/capabilities, the lifecycle product service, admin/operator capability handlers, and the extension hub |
| [`ironclaw_extension_registry`](./ironclaw_extension_registry) | `substrates` | Extension manifest and registry contracts |
| [`ironclaw_extension_support`](./ironclaw_extension_support) | `runtimes` | First-party userland extension implementations |
| [`packages/mem0`](./packages/mem0) | `substrates` | mem0-backed memory provider adapter for IronClaw Reborn (third-party provider lane for issue #3537 / #5264) |
| [`packages/memory-native`](./packages/memory-native) | `substrates` | Memory document service adapters |
| [`packages/slack`](./packages/slack) | `products` | Slack channel extension for IronClaw Reborn (#3857) |
| [`packages/telegram`](./packages/telegram) | `products` | Telegram channel extension for IronClaw Reborn: Bot API protocol engine + ChannelAdapter (#3285) |

`packages/` also holds the **data-only** packages (`github/`, `gmail/`,
`google-*/`, `web-access/`, `notion-mcp/`, `nearai-mcp/`, …): manifest, prompts,
schemas, committed `wasm/`, and an excluded `wasm-src/` guest. A package gets
its own crate only when it needs one — a channel adapter, a provider surface
such as `[memory]`, or a heavy native dependency (PROPOSAL §5).

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/extensions.md`](../../docs/reborn/target-architecture/families/extensions.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
