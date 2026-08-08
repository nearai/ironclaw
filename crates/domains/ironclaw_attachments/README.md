# ironclaw_attachments

The single channel-agnostic routine that lands inbound attachment bytes into
agent-accessible storage, together with the ports every channel and protocol
adapter calls (`InboundAttachmentLander` / `InboundAttachmentReader`), their
filesystem-backed default implementation, and the size-ceiling constants every
caller shares. One home for a landing path that several independent callers
used to reimplement across three crates.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_attachments` · **Manifest:** `crates/domains/ironclaw_attachments/Cargo.toml`
- **Use this when:** landing normalized inbound attachments for a channel or
  protocol surface, or reading the advertised attachment budgets/ceilings
  (`AttachmentCapabilities`, `DEFAULT_ATTACHMENT_BUDGETS`).
- **Don't use this when:** decoding a channel's payload — the adapter
  normalizes its own payload *before* calling this crate; delivering or
  sending outbound attachments → `ironclaw_outbound` + the product tier;
  extracting text from the landed bytes → `ironclaw_extractors`.

## Public surface

- `land_inbound_attachments` / `land_attachment` — the landing routine, with
  `AttachmentLanding`, `AttachmentLandingError`, and the scoped-path helpers
  (`attachment_scoped_path`, `attachment_batch_scoped_path`,
  `ATTACHMENTS_DIR`, `WORKSPACE_ALIAS`).
- `InboundAttachmentLander` / `InboundAttachmentReader` ports +
  `AttachmentCleanupReport`, with `ProjectScopedAttachmentLander` as the
  default implementation over `ScopedFilesystem`.
- Budgets: `AttachmentBudgets`, `AttachmentCapabilities`,
  `attachment_capabilities()`, `DEFAULT_MAX_ATTACHMENT_BYTES` — WebUI reads
  its advertised ceilings from the crate that enforces them.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_common`, `ironclaw_extractors`
  (bytes→text on landing), `ironclaw_filesystem`, `ironclaw_host_api`,
  `ironclaw_product_contracts` (the ports error with `ProductSurfaceError`; a
  recorded `[decision]` — narrowing it would move the WebUI 404/403 mapping),
  `ironclaw_threads` (`ThreadScope` keys the ports).
- **Consumed by (8):** `ironclaw_assistant`, `ironclaw_composition`,
  `ironclaw_extension_host`, `ironclaw_host_runtime`, `ironclaw_outbound`
  (reuses `DEFAULT_ATTACHMENT_BUDGETS`), `ironclaw_slack_extension`,
  `ironclaw_telegram_extension`, `ironclaw_webui`.

## Invariants

- The ports and the size ceilings live **here and only here** — pinned by
  `the_attachment_ports_and_size_ceilings_live_in_the_attachments_crate`
  (`reborn_conversations_threads_attachments.rs`).
- **Known carve-out:** `ProjectScopedAttachmentReader` stays in
  `ironclaw_assistant` (`src/scoped_fs/attachment_reader.rs`) because it also
  implements `ironclaw_loop_host::LoopAttachmentReadPort`, a `loops`-layer
  trait no `substrates` crate may name. The same gate pins it there **and**
  asserts this crate never acquires a `loop_host` dependency (tracked in
  #7010).
- Writes go through the project-scoped filesystem authority — the same one the
  agent's file tools resolve through — so landing still requires an explicit
  mount grant even though this crate makes no authorization decision.
- All three in-family edges (`→ extractors`, `→ threads`, and inbound
  `outbound → attachments`) are inventoried in
  `reborn_same_layer_edge_inventory.rs`; a new same-layer edge fails the gate.

## Tests

```bash
cargo test -p ironclaw_attachments
```

(Unit tests live in `src/`; the port behavior is additionally exercised by the
consumers' suites and the WebUI attachment routes.)

## See also

- Family boundary: [`../AGENTS.md`](../AGENTS.md) (this crate has no separate
  working-rules file; this README is its crate guidance).
- Design record: `families/domains.md`, PROPOSAL §6.4.9 (the WS5 widening and
  the carve-out history).
