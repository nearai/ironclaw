# ironclaw_threads

The canonical transcript service for IronClaw Reborn: one contract
(`SessionThreadService`) that every reader and writer of thread and message
history goes through, with a durable filesystem-backed implementation and an
in-memory one for deterministic tests. It exists as its own crate because one
contract serves many independent consumers with two production-shaped
implementations — folding it into a neighbor would make that neighbor a
dumping ground.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_threads` · **Manifest:** `crates/domains/ironclaw_threads/Cargo.toml`
- **Use this when:** you need to read or write canonical thread/message
  history, tool-result records, summaries, or transcript-derived display
  projections.
- **Don't use this when:** you are binding an external conversation to a
  thread or accepting an inbound message idempotently → `ironclaw_conversations`;
  deciding turn lifecycle → the kernel (`ironclaw_turns`); deciding delivery →
  `ironclaw_outbound`.

## Public surface

- `SessionThreadService` — the transcript trait; implemented by
  `FilesystemSessionThreadService` (durable, over `ScopedFilesystem`) and
  `InMemorySessionThreadService` (tests).
- The `contract` vocabulary: `ThreadScope`, `SessionThreadRecord`,
  `ThreadMessageRecord`, `MessageContent`/`MessageKind`/`MessageStatus`,
  context-window reads (`LoadContextWindowRequest`, `ContextMessages`),
  tool-result records/references, summary artifacts, goal statements.
- `SessionThreadError`; `ThreadMessageId` / `SummaryArtifactId`.
- Re-exported attachment vocabulary (`AttachmentKind`, `AttachmentRef` from
  `ironclaw_common`) so transcript consumers need no extra dependency.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_common`, `ironclaw_filesystem`,
  `ironclaw_host_api`, `ironclaw_safety` (validates provider-originated replay
  metadata before persisting — the boundary rule deliberately permits it).
- **Consumed by (7 workspace crates):** `ironclaw_assistant`,
  `ironclaw_attachments`, `ironclaw_composition`, `ironclaw_extension_host`,
  `ironclaw_loop_host`, `ironclaw_stress`, `ironclaw_turn_runner`.
  Re-measure: `cargo metadata --no-deps --format-version 1 | jq -r
  '.packages[] | select(.dependencies[]?.name == "ironclaw_threads") | .name'`.

## Invariants

- Never a turn/run lifecycle authority — it stores references supplied by the
  coordinator, and the `BoundaryRule { crate_name: "ironclaw_threads" }` in
  `reborn_dependency_boundaries.rs` forbids the kernel/runtime crates.
- Shares no declared type name with `ironclaw_conversations` — pinned by
  `conversations_and_threads_declare_no_name_in_common`
  (`reborn_conversations_threads_attachments.rs`).
- Message identity and per-thread sequence survive redaction/deletion;
  model-visible reads go through policy-filtered APIs — pinned by the contract
  suites below (see `AGENTS.md` for the full working rules).
- Inbound routing metadata that affects turn submission is committed atomically
  when the backend supports transactions. Fallback backends persist a
  content-free recovery intent before the transcript row, so retries resume
  the original message id and routing metadata; that metadata is replayed
  unchanged and never rendered as transcript content.
- Raw attachment references remain durable, while extracted document text and
  audio transcripts are secret-redacted when projected into model context.
- Context-window limits count the effective model-visible transcript, not
  hidden durable rows. Truncated windows report the exact last omitted
  sequence and kind so loop policy can react without guessing.
- Backend-neutral by construction: persistence is `ScopedFilesystem` only;
  backend choice happens in composition (`.claude/rules/database.md`).

## Tests

```bash
cargo test -p ironclaw_threads
# focused contract suites:
cargo test -p ironclaw_threads --test session_thread_contract
cargo test -p ironclaw_threads --test filesystem_session_thread_contract
cargo test -p ironclaw_threads --test filesystem_message_range_contract
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: `docs/internal/reborn/target-architecture/families/domains.md` and
  PROPOSAL §6.4.1.
