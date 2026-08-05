# `crates/domains/` — typed record and service domains

IronClaw Reborn's typed business records and the services that own them: sessions and threads, conversations, scheduled triggers, memory, skills, credentials, attachments, extraction, identity, projects, model providers, trace submissions, and outbound delivery state. A domain owns a record grammar and a service identity; it never owns an authority decision.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_attachments`](./ironclaw_attachments) | `substrates` | Channel-agnostic attachment landing for IronClaw Reborn: write attachment bytes through the scoped filesystem authority and return a ScopedPath storage key |
| [`ironclaw_auth`](./ironclaw_auth) | `substrates` | Product-facing Reborn auth contracts and fake services |
| [`ironclaw_conversations`](./ironclaw_conversations) | `substrates` | Conversation binding and session thread contracts |
| [`ironclaw_extractors`](./ironclaw_extractors) | `substrates` | Type-aware text extraction for IronClaw: turn a file's bytes (PDF, OOXML, legacy Office, RTF, text/code) into plain text, independent of where the file came from |
| [`ironclaw_identity`](./ironclaw_identity) | `substrates` | Canonical Reborn identity resolver: maps OAuth and external-channel actors to stable UserIds, and owns the Project entity + membership ACL (`projects` module) |
| [`ironclaw_llm`](./ironclaw_llm) | `substrates` | Multi-provider LLM integration with retry, failover, circuit breaker, and response caching |
| [`ironclaw_memory`](./ironclaw_memory) | `substrates` | Provider-neutral memory contract types |
| [`ironclaw_outbound`](./ironclaw_outbound) | `substrates` | Outbound egress and projection subscription policy storage |
| [`ironclaw_skills`](./ironclaw_skills) | `substrates` | Skill selection, scoring, and management |
| [`ironclaw_threads`](./ironclaw_threads) | `substrates` | Canonical session thread and transcript service contracts |
| [`ironclaw_trace_commons`](./ironclaw_trace_commons) | `substrates` | Trace Commons client: envelope, redaction, queue, credits |
| [`ironclaw_triggers`](./ironclaw_triggers) | `substrates` | Scheduled trigger domain and source-provider contracts |

✎ **`ironclaw_projects` is gone (2026-08-05, WS10).** This file used to carry a
"Not here, on purpose" note for it: §5 drew no row because §12.10 folds it into
`ironclaw_identity` as a `projects` module, and WS7 measured that merge and
skipped it rather than smuggling it into a move PR. The merge has now landed
exactly as measured — two consumer crates and five files repointed, one
`SAME_LAYER_EDGE_BASELINE` decrement (72 → 71, the `projects → filesystem` edge
absorbed by identity's own), and the `CRATE_LAYER_ORIGINS` row deleted under the
gate's stated removal path. The family is 12 crates and the workspace is 65
packages; the §5 delta and its `check-target-tree.py` exceptions row are both
closed. Project **records** live at `ironclaw_identity/src/projects/`; project
**authorization gating** is still `ironclaw_assistant::RebornProjectService`,
because its port is a `products`-layer declaration (PROPOSAL §6.4.11's
2026-08-04 correction — the hoist to `product_contracts` is the WS5 `product`
row's).

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/domains.md`](../../docs/reborn/target-architecture/families/domains.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
