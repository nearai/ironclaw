# `crates/substrates/` — privileged mechanism substrates

The durable, reusable mechanisms the kernel mediates: storage fabric, database-connection admission, secret storage, network policy and transport, safety scanning, and cross-cutting tracing. A substrate does real privileged work — but it never decides whether that work was allowed; that decision belongs to `kernel/`.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_filesystem`](./ironclaw_filesystem) | `substrates` | Scoped filesystem service |
| [`ironclaw_libsql_runtime`](./ironclaw_libsql_runtime) | `substrates` | Shared libSQL connection admission runtime |
| [`ironclaw_network`](./ironclaw_network) | `substrates` | network policy and hardened egress transport |
| [`ironclaw_observability`](./ironclaw_observability) | `substrates` | Low-level observability helpers |
| [`ironclaw_safety`](./ironclaw_safety) | `substrates` | Prompt injection defense, input validation, secret leak detection, and safety policy enforcement |
| [`ironclaw_secrets`](./ironclaw_secrets) | `substrates` | encrypted secret store, leases, one-shot consumption |

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/substrates.md`](../../docs/reborn/target-architecture/families/substrates.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
