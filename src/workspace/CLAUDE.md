# src/workspace

Owns persistent workspace memory and file-like project/user context.

## Agent-loop boundary

- `reborn_identity_context.rs` is the workspace-owned reader for stable identity
  files exposed to Reborn prompt context.
- Workspace may read and summarize identity files through narrow source traits.
- Prompt bundle assembly, strategy decisions, driver execution, and model calls
  stay outside this directory.

## Boundaries

- Preserve file-like semantics, chunking/search behavior, scope isolation, and
  protected-path safety.
- Do not persist agent-loop execution state here; loop checkpoints and turn
  state belong to turn/run storage.
- Do not add prompt-ordering, model-provider, capability, or product-workflow
  policy here.

## Adding code

- Add a new file when a workspace-backed source has its own trust, scope, or
  summarization rules.
- Reuse existing workspace read/search APIs before adding a side path.
- Keep identity-context code narrow: stable identity file discovery, read, safe
  summary, and trust classification.

## Common mistakes

- Do not let workspace readers decide which loop family or prompt mode runs.
- Do not expose raw protected file contents to untrusted prompt surfaces.
- Do not make workspace a generic state store for runtime progress.
