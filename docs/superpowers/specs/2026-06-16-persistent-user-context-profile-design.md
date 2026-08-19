# Persistent User Context (Agent-Context Profile) — Design Spec

_Date: 2026-06-16. Status: design, pending implementation plan._
_Companion: `docs/reborn/2026-06-16-memory-how-it-works.md` (current-state explainer + industry survey)._

## 1. Goal & Scope

Give IronClaw Reborn a per-user, **always-injected** context layer so it behaves as a competent general assistant — knowing the user's timezone, locale, and soft preferences without being told each turn.

This is **part 1 of a two-part memory split**:
1. **(this spec)** Persistent context injected into the prompt every turn, user-scoped.
2. *(later, separate spec)* Searchable / indexed memory.

**In scope:** the LLM-visible `agent_context` only — a structured profile (timezone, locale, location) plus the existing freeform prose file.

**Explicitly out of scope** (named, not silently dropped):
- Searchable/indexed memory (part 2).
- All **system config** — LLM provider/model selection, approval policy (`always-approve`), sandbox defaults. These keep their existing owners and are **never** absorbed into this profile (see §9).
- geo→timezone *derivation*.
- Project-scope override.
- Migrating legacy engine `ValidTimezone` / thread-metadata timezone plumbing.

## 2. Background — what already exists

From the current-state explainer, two always-injected layers already exist in Reborn:

1. **Identity files** (`USER.md`, `AGENTS.md`, `SOUL.md`, `MEMORY.md`, … and a structured `context/profile.json`) — loaded each turn into `LoopContextBundle.identity_messages` via `build_identity_messages` (`crates/ironclaw_loop_support/src/identity_context.rs`). Allow-list in `crates/ironclaw_memory/src/safety.rs:153`.
2. **Runtime context** — `LoopRuntimeContext` (`crates/ironclaw_turns/src/run_profile/runtime_context.rs:13`), re-rendered per turn by `render_model_content` (`:74`). It already carries a typed `user_timezone: Option<Tz>` and renders correct DST-aware local time via `chrono-tz` **when the timezone is known** — but the field has **no production producer**: the sole construction site, `crates/ironclaw_reborn/src/loop_driver_host.rs:1563`, passes `user_timezone: None`. So today the model always sees UTC + "timezone unknown."

Also relevant:
- **`builtin.time`** (`crates/ironclaw_host_runtime/src/first_party_tools/time.rs`) is a real, authoritative capability: `chrono` + `chrono_tz` (real IANA tzdata → DST-correct), ops `now/parse/convert/format/diff`. Used for arbitrary date math on demand. It even errors on ambiguous/nonexistent local times rather than guessing.
- **`memory_write`** (`crates/ironclaw_host_runtime/src/first_party_tools/memory.rs`) already writes scoped, versioned, schema-validatable memory documents. `DocumentMetadata.schema` supports JSON-schema validation at write time; `.config` ancestor metadata is inherited.

This design **fills the missing producer** and **adds a typed write path** rather than inventing new storage.

## 3. Architecture — two containers, by data shape

The core principle: **a value that must come out typed (a `Tz`) cannot live in a freeform `.md`** — a Markdown file has no enforced schema, and a model editing it anytime can corrupt any convention. So structured and prose data get different containers, different write verbs, and different render paths. Zero overlap.

```
STRUCTURED  → context/profile.json (schema-validated memory doc)
   fields: timezone, locale, location
   written by: builtin.profile_set   (typed, closed enum)
   read by:    producer at loop start → LoopRuntimeContext
   rendered:   render_model_content() — deterministic, every turn

PROSE       → USER.md (freeform)
   content: role, communication style, soft preferences
   written by: memory_write          (existing, free text)
   rendered:   build_identity_messages() → identity_messages (existing path)

ON-DEMAND   → builtin.time (unchanged)
   arbitrary date math / conversions the ambient line can't cover
```

Disambiguation between the two write verbs is structural, not hoped-for: `profile_set` has a **closed field enum** (can't hold prose), and the profile **self-advertises its slots** in the injected context each turn (the model sees `location=(unset)` and a tool whose description names exactly that slot). Mis-routing is low-harm and self-correcting (an unset field just falls back to `builtin.time` / "ask the user").

## 4. Components

### 4a. Structured profile store
- A schema-validated memory document at relative path **`context/profile.json`**, user-scoped (`project_id = None`).
- JSON schema enforced **server-side** via `.config` metadata inheritance — the model writes through `profile_set` (§4b), never authors the schema itself; an invalid value is rejected at write time.
- Reuses the memory backend (CAS writes, version history, PostgreSQL/libSQL parity). **No new table.**
- `context/profile.json` is already in the protected-path allow-list (`safety.rs:164`). Because the structured fields are now rendered via runtime context (§4c), the **raw JSON is not also injected as prose** — avoid double-injection/noise. (Implementation detail: ensure the structured profile doc is consumed by the producer and not additionally dumped into `identity_messages`. Confirm during implementation whether to drop it from the prose allow-list or leave it injected; default is **not** to inject the raw JSON.)

### 4b. `builtin.profile_set` capability (new)
- **Closed typed param set** — one or more of `timezone | locale | location` per call. The enum deliberately **cannot name** any system-config field (§9).
- Handler pipeline:
  1. Validate each supplied field (§5).
  2. **Field-level merge** into the existing profile (set `timezone` without clobbering `locale`).
  3. Write `context/profile.json` via the memory backend (validated against the server-owned schema).
  4. Invalidate the producer cache (§4c) for this scope.
- `PermissionMode::Allow`. Routed through `ToolDispatcher::dispatch` — inherits audit (`ActionRecord`), param validation, redaction, output sanitization. Everything-goes-through-tools compliant.
- Lives under `crates/ironclaw_host_runtime/src/first_party_tools/` as its own file (per crate guidance: one first-party tool per capability). Manifest + handler registered in `first_party_tools/mod.rs`; input schema in `schemas.rs`.

### 4c. Producer → runtime wiring
- Introduce a typed `UserProfileContext { locale: Option<Locale>, location: Option<String> }` carried on `LoopRuntimeContext` **alongside** the existing `user_timezone: Option<Tz>`.
- At loop start — **`crates/ironclaw_reborn/src/loop_driver_host.rs:1563`** — read `context/profile.json` for the run's scope, parse, and populate:
  - `user_timezone` ← validated `Tz` (replacing the hardcoded `None`).
  - `UserProfileContext` ← the remaining fields.
- Extend `render_model_content` (`runtime_context.rs:74`) to append one profile line after the existing time line, e.g.:
  `User profile: locale=ja-JP, location=Tokyo, Japan.`
  The timezone continues to flow through the existing time-rendering branch (correct DST local time).
- **Never guess** (honors the existing `user_timezone` invariant — `None` means unknown, never a host default): any field that is missing or fails to parse renders as unset.
- **Performance:** the profile read joins the existing concurrent loop-start fetch budget (mirroring `CommunicationContextProvider`'s already-running fetch with a bounded timeout), and/or is cached per scope with invalidation on `profile_set` write. It must not block loop start on the critical path. Producer wiring follows the module-owned-initialization rule (factory in the owning module, orchestrated from composition).

### 4d. Prose container
- `USER.md` remains the freeform identity file injected via the existing `build_identity_messages` path. No mechanism change — this spec only **documents it as the home** for soft prefs and ensures the guidance (§6) points the model there for non-structured facts.

## 5. Field set (v1)

| Field | Type / validation | Feeds | Notes |
|---|---|---|---|
| `timezone` | IANA string, validated by `parse::<Tz>()` (chrono-tz) | `user_timezone` → DST-correct local-time render | The high-value field. Must be an explicit IANA name. |
| `locale` | BCP-47 tag (e.g. `en-US`), light syntactic validation | profile line | Language / date / number hints |
| `location` | Free-text label (e.g. `"Tokyo, Japan"`), bounded length | profile line | **Model context only — NOT a timezone source in v1** |

- Use strong types (per `.claude/rules/types.md`): a `ValidTimezone`-style newtype around `Tz`, a `Locale` newtype. No stringly-typed internals.
- **Deferred:** geo→timezone derivation. v1 requires an explicit IANA `timezone`; `location` is a context label only. This preserves the never-guess invariant and needs no geo database.

## 6. Behavioral requirement — proactive elicitation

The model should help the user populate the profile, since an unset profile is invisible to the user.

- **Primary (reliable):** when a profile field is **unset** and the current request **needs it**, the model should ask the user for it and **offer to save it** via `profile_set`. This is driven by the always-injected render lines, generalizing the existing tz-unknown hint:
  - timezone unset + local time matters → "The user's timezone is unknown — ask, then offer to save it with `profile_set` so future answers are correct." (extends the current `render_model_content` `None` branch to mention `profile_set`).
  - other fields → a compact "unset profile fields: location — ask and offer to save if relevant" hint when any are unset.
- **Secondary (best-effort, explicitly soft):** if the user *alludes* to a field ("it's getting late here") without stating it, the model *may* offer to save it. This is prompt-only guidance with **no guarantee** — flagged as such; not a code path, not a success criterion.
- Elicitation guidance is **prompt/render text**, not new control flow. It must not nag: guidance is conditioned on the field being unset *and* relevant.

## 7. Data flow

**Write.** User: "I'm in Tokyo, use 24-hour time." → model calls `profile_set(timezone="Asia/Tokyo", location="Tokyo, Japan")` → handler validates (`parse::<Tz>()` ok), merges, writes `context/profile.json`, invalidates cache.

**Read / inject (every turn).** Loop start (`loop_driver_host.rs:1563`) → producer reads profile → fills `user_timezone = Asia/Tokyo` + `UserProfileContext{ locale, location }` → `render_model_content` emits:
`Current date/time at loop start: 2026-06-16T00:14Z (09:14 Tue, Asia/Tokyo). ... User profile: locale=ja-JP, location=Tokyo, Japan.`

## 8. Error handling
- **Write path:** per-field validation at the `profile_set` boundary; an invalid value returns a clear, field-level error the model can correct and retry. Merge is per-field — no partial corruption of other fields.
- **Read path:** a parse failure on any field → that field renders unset, with a `debug!` log (never `info!`/`warn!` — REPL/TUI corruption rule). The run continues. Fail-soft, never fabricate a value.
- Follow `.claude/rules/error-handling.md`: no `unwrap_or_default()` masking a store error on the profile read; a genuine store failure is surfaced/logged, not silently turned into an empty profile without a `// silent-ok:` justification.

## 9. Security boundary (non-negotiable)

System config is **never** part of this profile:
- `profile_set`'s closed enum **cannot name** provider/model, approval policy, sandbox defaults, or any authorization-affecting field. The producer reads **only** `agent_context`.
- Rationale: `always-approve` is an authorization-bypass switch — if the model could see it that is prompt-injection recon; if it could write it that is privilege escalation. Provider/model selection must not be model-rerouteable.
- These configs already have owners and must stay there (no parallel authorization/settings path — per `.claude/rules/architecture.md` and Reborn boundary rules):
  - Provider/model/db/security → `src/settings.rs` `Settings` (instance-level; `owner_id` is the instance owner scope, plus a per-user DB settings table for some fields).
  - Approval policy → **computed, not stored** — resolved from a `RuntimeProfile` by `RuntimeProfileApprovalGatePolicy` (`crates/ironclaw_reborn_composition/src/runtime_profile_approval_policy.rs`), per run/scope.

## 10. Scope model & configuration scope map

### 10.1 This profile's scope
- **v1: user-scoped only** — the profile describes the human user, independent of agent or project, so it is keyed at `(tenant_id, user_id)` with **`agent_id = None` and `project_id = None`**. One profile per user, applies everywhere.
- **Reserved, not built:** project-level override. The memory document's 4-tuple scope already supports it; a later slice would merge project-over-user in the producer.

### 10.2 Configuration scope map

Where every per-X configuration/context item lives, to make this profile's boundary explicit. Scope axis: **User** (the human) · **Agent** (the assistant persona) · **Project** (a workspace) · **Thread** (one conversation/run) · **Instance** (the whole IronClaw deployment — shown because provider/approval surfaced in discussion, though outside the four core scopes). "Sees" = injected into / visible to the LLM. "Writes" = the LLM can mutate it via a capability.

| Item | Scope | Sees | Writes | Store / owner | This spec |
|---|---|---|---|---|---|
| timezone | User | ✅ | ✅ `profile_set` | `context/profile.json` | **builds** |
| locale | User | ✅ | ✅ `profile_set` | `context/profile.json` | **builds** |
| location | User | ✅ | ✅ `profile_set` | `context/profile.json` | **builds** |
| communication style / soft prefs | User | ✅ | ✅ `memory_write` | `USER.md` (prose) | documents |
| agent persona / identity | Agent | ✅ | ✅ `memory_write` | `AGENTS.md`, `SOUL.md` (agent-scoped) | existing |
| agent tool/capability set | Agent | ✅ (as tools) | ❌ | extension registry / runtime policy | existing |
| project instructions / context | Project | ✅ | ✅ `memory_write` | project-scoped memory docs | existing |
| project sandbox / container policy | Project | ❌ | ❌ | sandbox config (system) | existing |
| project-scoped memory/notes | Project | on search | ✅ `memory_write` | memory docs (`project_id` set) | part 2 |
| current date/time at loop start | Thread | ✅ | ❌ (system) | `LoopRuntimeContext` (per-run) | touched (tz render) |
| run origin / surface / adapter | Thread | ✅ | ❌ | `LoopRuntimeContext.product_context` | existing |
| reply delivery target | Thread | ✅ (advisory) | ❌ | communication context (owner-keyed) | existing |
| connected channels | User/Thread | ✅ (advisory) | ❌ | communication context | existing |
| conversation history / compaction | Thread | ✅ | n/a | thread store | existing |
| active-run lock | Thread | ❌ | ❌ | `ironclaw_turns` (scoped thread key) | existing |
| LLM provider / model | Instance | ❌ | ❌ | `src/settings.rs` `Settings` | **excluded (§9)** |
| db backend / secrets key | Instance | ❌ | ❌ | `src/settings.rs` `Settings` | **excluded (§9)** |
| approval policy (`always-approve`) | Run/scope | ❌ | ❌ | **computed** from `RuntimeProfile` (`runtime_profile_approval_policy.rs`) — not stored | **excluded (§9)** |

Reading the table: everything this spec **builds** is User-scoped and both LLM-visible and LLM-writable. Everything Instance-scoped is neither visible nor writable (the §9 safety boundary). Thread-scoped runtime context is visible but system-owned (the LLM never writes it) — this spec only *touches* it by giving the existing `user_timezone` slot a producer. Approval policy is special: it is **computed per run**, not stored anywhere as a toggle, which is the deeper reason it cannot be consolidated into a profile record.

## 11. Testing
- Unit: field validators (`timezone` IANA parse incl. a rejecting case; `locale` syntax; `location` length bound).
- **Caller-level (per `.claude/rules/testing.md` — "Test Through the Caller"):**
  - Drive the `profile_set` **handler**: assert `context/profile.json` field-merge (set one field, others preserved) and schema rejection of an invalid value.
  - Drive the **producer** at the `loop_driver_host` construction site: assert `user_timezone` is populated from the stored profile and that `render_model_content` output contains the correct local-time line and profile line.
- Run at the integration tier across **both** DB backends (PostgreSQL + libSQL) for memory-doc parity.
- `cargo test -p ironclaw_architecture` after any public-API / boundary / dependency change (new capability + new `LoopRuntimeContext` field).
- Render snapshot/replay coverage where the runtime-context fingerprint is asserted (model-visible rendering only).

## 12. Open implementation details (decide during planning, not blockers)
- Whether to drop `context/profile.json` from the prose injection allow-list or keep it injected (default: do not inject raw JSON; render via runtime context only).
- Exact cache vs. concurrent-read strategy for the loop-start profile read.
- Newtype names / module placement for `ValidTimezone`, `Locale`, `UserProfileContext` (reuse legacy `ValidTimezone` shape if cleanly extractable, else define fresh in the runtime-context crate).
- JSON schema document content + where the `.config` carrying it is provisioned.

## 13. Out of scope (restated)
Searchable memory (part 2); geo→timezone derivation; project-scope override; system-config consolidation/migration; legacy `ValidTimezone`/thread-metadata timezone migration.
