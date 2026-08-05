# Agent Map — ironclaw_common

## Start Here

- No crate-local `CLAUDE.md` exists yet; use this map plus the repo rules below.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these sources of truth before changing shared types:
- `.claude/rules/types.md`
- `CLAUDE.md`

## What This Crate Owns

The target-architecture contract is PROPOSAL §6.1.5: *domain-free cross-cutting
primitives with persisted-compatibility guarantees*, and **no internal
dependency**. The owned set is closed:

- Validated identity newtypes (`CredentialName`, `ExtensionName`, `McpServerName`, `ExternalThreadId`) with their length constants and validation errors — `identity.rs`. This is the home of the documented `#[serde(transparent)]` + `from_trusted` persisted-compatibility exception.
- Attachment helpers (`AttachmentKind`, `AttachmentRef`, `IncomingAttachment`, `normalize_mime_type`) — `attachment.rs` — and the format registry — `attachment_format.rs`.
- Base-dir/path resolution (`ironclaw_base_dir`, `compute_ironclaw_base_dir`) — `paths.rs`.
- PKCE (`pkce.rs`) and hashing (`hashing.rs`) primitives.
- Environment override helpers (`env_or_override`, `set_runtime_env`, `register_secondary_fallback`, `lock_env`) — `env_helpers.rs`.
- Timezone validation (`ValidTimezone`, `deserialize_option_lenient`) — `timezone.rs`.
- Preview truncation (`truncate_for_preview`, `truncate_preview`) — `util.rs`.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

### The three exceptions, and why they are still here

`llm_costs`, `model_selection`, and `provider_transcript` are LLM *domain* data
that §6.1.5 assigns to `ironclaw_llm`. WS1.6 measured all three against the live
tree and each move is blocked by a **pinned rule**, not by taste:

- `provider_transcript` — `ironclaw_agent_loop` calls
  `is_only_provider_transcript_artifact_lines`, and its boundary rule allows
  *contracts-layer normal dependencies only* (`reborn_dependency_boundaries.rs`).
  `ironclaw_llm` is `layer = "substrates"`, so the move either breaks the
  zero-exception contracts-only property WS1.2 just achieved or needs a new
  `LAYER_MATRIX_EXCEPTION` — and that ratchet is shrink-only.
- `model_selection` — `ironclaw_openai_compat`'s boundary rule lists
  `ironclaw_llm` as **forbidden** outright. `ironclaw_llm` also has zero uses of
  this module, so the move would place a symbol in a crate that does not use it
  in order to serve two crates that may not reach it.
- `llm_costs` — `ironclaw_llm` uses exactly two of its seven public items
  (`model_cost`, `default_cost`); the dominant consumers are `ironclaw_turn_runner`,
  `ironclaw_composition`, and `ironclaw_assistant` (`RunCost`, a product
  wire DTO). Moving it would hand `ironclaw_assistant` — the crate §6.9.1 exists
  to narrow — a `reqwest`/`rig-core`/Bedrock provider dependency for a pricing
  table. The real seam already exists as `ModelCostTable`; routing the static
  table behind that port is a design change, not a narrowing.

Do not treat these as precedent, and do not add a fourth.

## Do Not Move In Here

- Wire protocols. `AppEvent` and its DTO family lived here until WS1.6 deleted them (zero consumers workspace-wide); product wire DTOs belong in `ironclaw_product_contracts`.
- LLM domain data, budget-policy constants, prompt-construction data, or automation vocabulary — see §6.1.5's eviction list. `AutomationName` now lives in `ironclaw_triggers`, which owns automations.
- Runtime orchestration, persistence, network clients, web/TUI behavior, policy engines, or domain logic owned by more specific Reborn crates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.
- **Any internal (`ironclaw_*`) dependency.** This crate is a leaf by contract.

## Validation

- Fast local check: `cargo test -p ironclaw_common`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If a type is serialized over API or persisted data, add compatibility tests for stable names and validation behavior.

## Agent Notes

- Keep this crate minimal; new dependencies here affect much of the workspace.
- Prefer validated newtypes and wire-stable enums over raw strings.
- If a shared type only serves one subsystem, keep it in that subsystem crate until a second real caller exists.
