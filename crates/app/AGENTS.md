# `crates/app/` — assembly and enforcement

The assembly root, the shipped artifact, the boot-configuration leaf, and the enforcement suite — four crates whose only shared trait is that nothing else in the workspace may depend on any of them. Composition wires; it does not behave.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_architecture_tests`](./ironclaw_architecture_tests) | `app` | the enforcement suite (test-only crate) |
| [`ironclaw_cli`](./ironclaw_cli) | `app` | the shipped binary: commands, `serve`, concrete-extension binding tables, registrars — the one directory whose name and package name differ (`ironclaw`, PROPOSAL §5.1) |
| [`ironclaw_composition`](./ironclaw_composition) | `app` | THE assembly root: deployment selection and service-graph wiring only |
| [`ironclaw_config`](./ironclaw_config) | `substrates` | boot configuration contracts (vendor sections removed) |

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/app.md`](../../docs/reborn/target-architecture/families/app.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
