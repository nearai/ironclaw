# `crates/events/` — evidence, derived views, transport streams

The system's record of what happened, kept structurally distinct from what a screen shows right now: the typed redacted event/audit vocabulary, the durable backends that persist it, the replay-derived read models, and the transport-neutral stream manager that ships it. Three separate contracts — evidence, derived view, transport — deliberately not one.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_event_log`](./ironclaw_event_log) | `substrates` | typed redacted event/audit vocabulary and log traits |
| [`ironclaw_event_projections`](./ironclaw_event_projections) | `substrates` | replay-derived read models |
| [`ironclaw_event_store`](./ironclaw_event_store) | `substrates` | Reborn-owned durable event and audit store backends |
| [`ironclaw_event_streams`](./ironclaw_event_streams) | `substrates` | Transport-neutral Reborn projection stream manager |

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/events.md`](../../docs/reborn/target-architecture/families/events.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
