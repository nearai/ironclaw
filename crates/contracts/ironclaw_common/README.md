# ironclaw_common

Domain-free cross-cutting primitives that carry long-lived, persisted
wire-compatibility guarantees: validated identity newtypes, attachment format
vocabulary, base-dir/path resolution, PKCE and hashing helpers, timezone
validation, preview truncation. It is a separate crate because these primitives
have consumers across every layer, and the one documented persisted-compat
exception (`#[serde(transparent)]` + `from_trusted` on the identity newtypes)
needs exactly one home so it cannot silently reappear elsewhere. This crate is
data, not behavior — it defines almost no traits.

- **Family / layer:** `crates/contracts/` / `contracts` · **Package:** `ironclaw_common` · **Manifest:** `crates/contracts/ironclaw_common/Cargo.toml`
- **Use this when:** you need a genuinely domain-free primitive (a credential
  or extension name, an attachment format, a validated timezone) that already
  lives here.
- **Don't use this when:** the type belongs to a subsystem — product wire DTOs
  → `ironclaw_product_contracts`; authority/turn vocabulary →
  `ironclaw_host_api`; automation names → `ironclaw_triggers`. If a shared
  type serves one subsystem, keep it in that subsystem until a second real
  caller exists.

## Public surface

- `identity` (via crate-root re-exports): `CredentialName`, `ExtensionName`,
  `McpServerName`, `ExternalThreadId` — the newtype-discipline anchor
  (`.claude/rules/types.md`), home of the persisted-compat exception.
- `attachment` (re-exported): `AttachmentKind`, `AttachmentRef`,
  `IncomingAttachment`, `normalize_mime_type`; plus `attachment_format`.
  Distinct from the channel-facing
  `ironclaw_extension_contracts::channel_adapter::ChannelAttachmentRef`.
- `paths` (`ironclaw_base_dir`), `pkce`, `hashing`, `env_helpers`,
  `timezone` (`ValidTimezone`), `util` (`truncate_for_preview`).
- **Three residents that are documented exceptions, not precedent:**
  `llm_costs`, `model_selection`, `provider_transcript` — LLM domain data
  whose evictions are blocked by pinned boundary rules. `AGENTS.md` carries
  the per-module evidence; do not re-litigate it, and do not add a fourth.
  (`llm_costs` additionally sits inside the family vendor census as a frozen,
  shrink-only residue — `reborn_contracts_vendor_census.rs`.)

## Depends on / consumed by

- **Internal deps: none** — a leaf by contract. An upward edge fails the layer
  matrix; a new sideways contracts edge fails
  `reborn_same_layer_edge_inventory.rs`.
- **Consumed by** 20 workspace manifests (reproduce:
  `grep -rl '^ironclaw_common = ' --include=Cargo.toml crates tests Cargo.toml | wc -l`).

## Invariants

- **Leaf status** and **no frameworks/drivers**
  (`reborn_contracts_crates_hold_no_framework_dependencies`); **size ceiling**
  (`reborn_contracts_crates_carry_a_checked_size_ceiling`).
- **Wire compatibility:** serialized/persisted types here keep stable names
  and validation behavior; changes need compatibility tests.
- **Vendor census:** `llm_costs`' vendor-name occurrences are frozen
  shrink-only (`reborn_contracts_family_names_llm_vendors_only_in_censused_scopes`).

## Tests

```bash
cargo test -p ironclaw_common
cargo test -p ironclaw_architecture_tests
```

## See also

- Working rules (canonical): [`AGENTS.md`](./AGENTS.md).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.1.5.
