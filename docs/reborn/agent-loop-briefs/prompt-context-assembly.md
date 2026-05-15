# WS-15 — Prompt Context Assembly: Identity-File Surface

**Workstream:** WS-15 (follow-up; not in the skeleton WS-0..WS-8 set)
**Crates touched:** `ironclaw_turns` + `ironclaw_loop_support` +
`ironclaw_reborn` + `src/workspace`
**Depends on:** WS-8 (skeleton landed and green)
**Parallel with:** WS-9..WS-13. WS-14's live-default cutover is
gated on this workstream because the default Reborn loop must retain
the existing workspace identity prompt behavior.
**Master doc:** [`../agent-loop-skeleton.md`](../agent-loop-skeleton.md) §11–§12

---

## 1. Scope

`LoopContextBundle.identity_messages: Vec<LoopContextMessage>`
([`crates/ironclaw_turns/src/run_profile/host.rs`](../../../crates/ironclaw_turns/src/run_profile/host.rs)
near line 583) is the slot for identity-style content — `AGENTS.md`,
`SOUL.md`, `IDENTITY.md`, `HEARTBEAT.md`, `TOOLS.md`, and
`BOOTSTRAP.md`. Personal/profile-derived files such as `USER.md` and
`context/assistant-directives.md` stay out of this WS-15 surface until
explicit run-context privacy policy exists. Today the slot is populated
with `Vec::new()` unconditionally by
`ThreadBackedLoopContextPort::load_loop_context()` in
[`crates/ironclaw_loop_support/src/lib.rs`](../../../crates/ironclaw_loop_support/src/lib.rs).

This brief adds a `HostIdentityContextSource` trait — analogous to the
existing `HostSkillContextSource` in
[`crates/ironclaw_loop_support/src/skill_context.rs`](../../../crates/ironclaw_loop_support/src/skill_context.rs) —
and wires it through the context port so identity files actually reach
the prompt. It also ships the concrete workspace-backed source that
reads the current identity files through `Workspace::read_primary()`.

This brief includes two additive contract changes in
`ironclaw_turns`: `LoopContextMessage.message_ref` becomes optional
for summary-only identity entries, and `LoopContextRequest` carries
the requested `PromptMode`. The assembly order remains
`identity_messages` → `instruction_snippets` → `messages`, but
`HostManagedLoopPromptPort` must learn how to materialize
`message_ref: None` entries instead of blindly copying
`message.message_ref`.

## 2. Files

### NEW
- `crates/ironclaw_loop_support/src/identity_context.rs` —
  `HostIdentityContextSource` trait, `HostIdentityContextCandidate`
  value type, `HostIdentityContextBuildError`, and
  `build_identity_messages(...)` helper. Mirrors `skill_context.rs`
  shape.
- `src/workspace/reborn_identity_context.rs` (or equivalent
  workspace-owned module) — concrete `WorkspaceIdentityContextSource`
  backed by the existing workspace read APIs and
  `ironclaw_memory::safety::DEFAULT_PROMPT_PROTECTED_PATHS`.

### MODIFIED
- `crates/ironclaw_turns/src/run_profile/host.rs` —
  `LoopContextMessage.message_ref` becomes
  `Option<LoopMessageRef>` (additive contract change in turns).
  `None` means "this entry is summary-only — the prompt port MUST NOT
  attempt to resolve content; use `safe_summary` verbatim instead."
  Mirrors the skill pattern: `SkillTrustLevel::Installed` carries
  `prompt_content: None`. Call-site updates: the existing nine
  consumers wrap their writes in `Some(...)` and pattern-match on
  reads. See §3.2's `IdentityTrustLevel` invariant for the
  Installed-trust attenuation enforced via this field. Counts as
  an additive `ironclaw_turns` contract extension per master doc
  §12 crate-ownership rule.
- `crates/ironclaw_loop_support/src/lib.rs` —
  - Add `pub mod identity_context;` and re-exports.
  - `ThreadBackedLoopContextPort` gains an
    `Option<Arc<dyn HostIdentityContextSource>>` field on construction.
  - `load_loop_context()` calls
    `build_identity_messages(source, run_context, request.mode,
    identity_budget)` when set; otherwise `Vec::new()` (today's
    behavior).
- `crates/ironclaw_turns/src/run_profile/prompt.rs` —
  `HostManagedLoopPromptPort` handles identity messages whose
  `message_ref` is `None` by creating stable summary-only model refs
  from `safe_summary`, analogous to skill snippet refs. It must not
  drop those entries and must not try to resolve a missing
  `LoopMessageRef`.
- `crates/ironclaw_reborn/src/text_loop_driver.rs` —
  - `TextOnlyModelReplyDriverConfig` gains
    `pub identity_source: Option<Arc<dyn HostIdentityContextSource>>`,
    default `None`.
  - Pass-through to `ThreadBackedLoopContextPort` constructor inside
    the driver's host composition.
- `crates/ironclaw_reborn/src/planned_driver.rs` (this lands with WS-7
  / WS-15 together) — same pass-through. Identity source is hosted at
  driver-composition level, not inside the framework.
- `src/workspace/mod.rs` — re-export or compose
  `WorkspaceIdentityContextSource` from the workspace module so the
  app composition root can pass it into the Reborn driver configs.

### NOT TOUCHED
- `crates/ironclaw_turns/src/run_profile/host.rs` —
  `LoopContextBundle.identity_messages: Vec<LoopContextMessage>` slot
  is unchanged in shape (the inner `LoopContextMessage` gains
  the `Option<...>` field above, but the bundle itself is unaffected).
- Memory vector-search / `memory_snippets` population. WS-15 is only
  the identity-file prompt surface.

## 3. Specification

### 3.1 `HostIdentityContextSource` trait

```rust
//! crates/ironclaw_loop_support/src/identity_context.rs

use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, LoopContextMessage, LoopRunContext, PromptMode,
};
use thiserror::Error;

/// Host-owned source for identity-style context that the model receives
/// as system messages before the conversation transcript.
///
/// Identity files canonically include: `AGENTS.md`, `SOUL.md`,
/// `IDENTITY.md`, `HEARTBEAT.md`, `TOOLS.md`, and `BOOTSTRAP.md`.
/// The broader prompt-protected filename list is owned by
/// `ironclaw_memory::safety::DEFAULT_PROMPT_PROTECTED_PATHS`; WS-15
/// intentionally excludes personal/profile-derived files such as
/// `USER.md` and `context/assistant-directives.md` until run-context
/// privacy policy exists.
///
/// Implementations own storage lookups, trust resolution, and content
/// safety filtering. This trait returns host-approved candidates — raw
/// file content for trusted candidates, refs+summary only for installed
/// (read-only) candidates — so `ironclaw_loop_support` stays a thin
/// adapter that doesn't open files itself.
#[async_trait]
pub trait HostIdentityContextSource: Send + Sync {
    async fn load_identity_candidates(
        &self,
        run_context: &LoopRunContext,
        mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError>;
}
```

### 3.2 `HostIdentityContextCandidate`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIdentityContextCandidate {
    /// Canonical filename — e.g. "AGENTS.md", "SOUL.md".
    /// Validated against `DEFAULT_PROMPT_PROTECTED_PATHS` so unknown
    /// files cannot inject content under the identity banner.
    pub name: IdentityFileName,

    /// Stable host ref the model port resolves to the file's content
    /// at stream-build time. Identity content is NEVER passed as a
    /// raw string through `LoopContextMessage.safe_summary` — the
    /// ironclaw_turns crate's "no raw prompt content in contracts"
    /// rule prohibits it.
    ///
    /// **Trust-attenuation invariant** (mirrors the skill pattern in
    /// `SkillContextService` — `Installed` skills carry
    /// `prompt_content: None`):
    /// - `Trusted` trust level: `message_ref` MUST be `Some(...)` —
    ///   content resolves to verbatim file bytes
    /// - `Installed` trust level: `message_ref` MUST be `None` —
    ///   only the `safe_summary` reaches the prompt; the candidate
    ///   structurally cannot leak content
    ///
    /// The invariant is enforced at candidate construction (see
    /// `HostIdentityContextCandidate::new_trusted` /
    /// `new_installed_summary_only` constructors below) so a
    /// downstream bug in `build_identity_messages` cannot resolve
    /// an Installed candidate's ref — the ref doesn't exist.
    pub message_ref: Option<LoopMessageRef>,

    /// Short host-redacted summary. For Trusted candidates this is
    /// prompt-milestone telemetry only; prompt content flows through
    /// `message_ref`. For Installed candidates this is the attenuated
    /// model-visible text because `message_ref` is intentionally
    /// absent. Must not contain raw file content.
    pub safe_summary: String,

    /// Trust level — drives the same Trusted-vs-Installed attenuation
    /// rules that govern SKILL.md content (see
    /// `.claude/rules/skills.md`). Installed-trust identity content
    /// is summary-only; Trusted content carries through verbatim.
    pub trust_level: IdentityTrustLevel,

    /// Mode gate — `Always` or `OnCodeAct`. A
    /// `OnCodeAct` candidate is filtered out before assembly when the
    /// request mode is `TextOnly`. The two variants cover the currently
    /// supported identity-file routing rules; add another variant only
    /// when a concrete source emits it.
    pub applies_when: IdentityApplicability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityTrustLevel { Installed, Trusted }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityApplicability {
    Always,
    OnCodeAct,
}

impl HostIdentityContextCandidate {
    /// Constructs a Trusted candidate — content is resolvable via the
    /// supplied `message_ref`. Verbatim file bytes reach the model at
    /// stream-build time.
    pub fn new_trusted(
        name: IdentityFileName,
        message_ref: LoopMessageRef,
        safe_summary: String,
        applies_when: IdentityApplicability,
    ) -> Self {
        Self {
            name,
            message_ref: Some(message_ref),
            safe_summary,
            trust_level: IdentityTrustLevel::Trusted,
            applies_when,
        }
    }

    /// Constructs an Installed candidate — summary-only, no content
    /// ref. The structural absence of `message_ref` makes it impossible
    /// for `build_identity_messages` to resolve verbatim content for
    /// this candidate. Mirrors `SkillTrustLevel::Installed` carrying
    /// `prompt_content: None` in `SkillContextService`.
    pub fn new_installed_summary_only(
        name: IdentityFileName,
        safe_summary: String,
        applies_when: IdentityApplicability,
    ) -> Self {
        Self {
            name,
            message_ref: None,
            safe_summary,
            trust_level: IdentityTrustLevel::Installed,
            applies_when,
        }
    }
}
```

Loop-family-scoped applicability (e.g. an `OnFamilyId(LoopFamilyId)` variant) is
deliberately omitted from this brief: the skeleton ships only
`families::default()` (WS-3.5), so the variant would have nothing to scope
against. `LoopFamilyId` IS a defined type as of WS-3.5; once a second family
ships and per-family identity scoping becomes useful, this enum can grow the
variant in a strictly additive change.

`IdentityFileName` is a newtype per `.claude/rules/types.md` —
construction validates against the canonical filename list, so the
trait surface cannot smuggle arbitrary filenames in.

### 3.3 `build_identity_messages` helper

```rust
pub async fn build_identity_messages(
    source: &(dyn HostIdentityContextSource + Send + Sync),
    run_context: &LoopRunContext,
    mode: PromptMode,
    budget: IdentityBudget,
) -> Result<Vec<LoopContextMessage>, AgentLoopHostError> {
    let candidates = source
        .load_identity_candidates(run_context, mode)
        .await
        .map_err(HostIdentityContextBuildError::into_host_error)?;

    let mut out = Vec::with_capacity(candidates.len());
    let mut used = 0u32;
    for c in candidates {
        if !applies(&c.applies_when, mode, run_context) { continue; }
        let cost = estimate_cost(&c);
        if used.saturating_add(cost) > budget.token_ceiling {
            // Soft drop on budget — telemetry-only, not an error. The
            // host has decided ordering; later entries are lower
            // priority. Mirrors skill-budget behavior.
            break;
        }
        used += cost;
        // Trust attenuation by structural absence: Installed candidates
        // carry `message_ref: None`, so `LoopContextMessage.message_ref`
        // is also None for them. The prompt port converts those entries
        // into stable summary-only refs backed by `safe_summary`.
        // Trusted candidates carry `Some(ref)` which resolves to
        // verbatim file bytes.
        out.push(LoopContextMessage {
            message_ref: c.message_ref,
            role: "system".to_string(),
            safe_summary: c.safe_summary,
        });
    }
    Ok(out)
}
```

`IdentityBudget` is local to `ironclaw_loop_support`; default 8K
tokens, configurable via the driver config. Identity messages share
the prompt's overall token budget with `instruction_snippets`
(SKILL.md) but are accounted separately for telemetry.

#### Deterministic ordering contract

Anthropic prompt caching only hits when `identity_messages` bytes are
identical across iterations. Iteration order from the source is the
load-bearing contributor:

- `HostIdentityContextSource::load_identity_candidates` MUST return
  candidates in a canonical deterministic order. Recommended: a fixed
  precedence list keyed off the `DEFAULT_PROMPT_PROTECTED_PATHS`
  ordering already defined in
  [`crates/ironclaw_memory/src/safety.rs`](../../../crates/ironclaw_memory/src/safety.rs)
  near line 153 (`SOUL.md, AGENTS.md, USER.md, IDENTITY.md, SYSTEM.md,
  MEMORY.md, TOOLS.md, HEARTBEAT.md, BOOTSTRAP.md,
  context/assistant-directives.md`), filtered to the WS-15 stable
  identity set. Alphabetical-by-`IdentityFileName.as_str()` is an
  acceptable alternative as long as the impl picks one and is consistent
  across calls.
- `build_identity_messages` MUST preserve the source-provided order
  and MUST NOT re-sort by name inside the helper — re-sorting would
  mask source-side ordering bugs and decouple the helper from the
  canonical filename ordering owned by `ironclaw_memory`.

### 3.3.5 Cache stability: stable vs. volatile identity content

`HostManagedLoopPromptPort` assembles the prompt in the order
`identity_messages → instruction_snippets → messages` (see §3.4 and
[`crates/ironclaw_turns/src/run_profile/prompt.rs`](../../../crates/ironclaw_turns/src/run_profile/prompt.rs)).
Anthropic prompt caching requires the prefix — `identity_messages` —
to be byte-stable across iterations of a single run for cache hits.

The canonical identity files
([`crates/ironclaw_memory/src/safety.rs`](../../../crates/ironclaw_memory/src/safety.rs)
near line 153) split into two buckets:

- **Stable** (byte-stable across the run): `AGENTS.md`, `SOUL.md`,
  `IDENTITY.md`, `TOOLS.md`, `BOOTSTRAP.md`. These land in
  `identity_messages`.
- **Personal/profile-derived**: `USER.md` and
  `context/assistant-directives.md`. WS-15 excludes these from
  `identity_messages` until `LoopRunContext` carries explicit privacy
  policy for shared/group runs.
- **Volatile** (may change mid-run): `HEARTBEAT.md`. This holds
  timestamped, frequently-changing proactive findings and must NOT
  land in `identity_messages` — every turn's prefix would otherwise
  differ and the prompt cache would miss every turn. WS-15 excludes it
  from the stable identity bundle; a later heartbeat-specific context
  path can route it after the cache-sealed identity prefix once
  `LoopRunContext` carries an explicit heartbeat/run-kind signal.

### 3.3.6 Prompt assembly for summary-only identity entries

`LoopContextMessage.message_ref: None` is valid only for
Installed-trust identity entries. Prompt assembly must preserve those
entries as leading system messages:

- If `message_ref == Some(ref)`, `HostManagedLoopPromptPort` builds
  `LoopModelMessage { role, content_ref: ref }` exactly as today.
- If `message_ref == None`, `HostManagedLoopPromptPort` creates a
  stable synthetic `LoopMessageRef` from
  `(safe_summary, ordinal)` under an identity-summary prefix and emits
  `LoopModelMessage { role: "system", content_ref }`.
- The model-message resolver that already handles skill snippet refs
  gains the matching identity-summary lookup and resolves the
  synthetic ref to `safe_summary`. It must reject role mismatches and
  fail closed if the source no longer produces the same summary for
  that ordinal.

This is a required code change in
[`crates/ironclaw_turns/src/run_profile/prompt.rs`](../../../crates/ironclaw_turns/src/run_profile/prompt.rs)
and the corresponding host-managed model resolver. A prompt port that
blindly maps every context message with `content_ref:
message.message_ref` is incomplete after this brief.

### 3.4 Adapter wiring

```rust
//! crates/ironclaw_loop_support/src/lib.rs (delta)

pub struct ThreadBackedLoopContextPort {
    // ... existing fields ...
    identity_source: Option<Arc<dyn HostIdentityContextSource>>,
    identity_budget: IdentityBudget,
}

#[async_trait]
impl LoopContextPort for ThreadBackedLoopContextPort {
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        let messages = self.thread_service.load_context_window(...).await?;
        let instruction_snippets = match self.skill_source.as_ref() {
            Some(src) => build_skill_instruction_snippets(src.as_ref(), &self.run_ctx).await?,
            None => Vec::new(),
        };
        let identity_messages = match self.identity_source.as_ref() {
            Some(src) => build_identity_messages(
                src.as_ref(),
                &self.run_ctx,
                request.mode,                 // added to LoopContextRequest
                self.identity_budget,
            ).await?,
            None => Vec::new(),
        };
        Ok(LoopContextBundle {
            identity_messages,
            messages,
            instruction_snippets,
            memory_snippets: Vec::new(),    // out of scope here
        })
    }
}
```

### 3.4.5 Per-run caching

Without explicit caching, `ThreadBackedLoopContextPort::load_loop_context()`
re-reads identity files on every iteration. Anthropic prompt caching
still hits on byte-stable content, but the per-tick disk read is
wasteful and opens a race window where a file changes mid-run and the
prefix flips. Required behavior:

- For **stable** files (per §3.3.5), `ThreadBackedLoopContextPort`
  MUST pin the raw `Vec<HostIdentityContextCandidate>` for the run on
  first `load_loop_context()` call and reuse that pinned snapshot on
  subsequent calls, including resume after process restart. The pin is
  durable: store either the snapshot bytes or a `StableIdentitySnapshotRef`
  plus digest in run/checkpoint metadata before any blocking checkpoint
  can resume through this driver. A process-local
  `Arc<OnceLock<Vec<HostIdentityContextCandidate>>>` is only a
  read-through cache for that durable snapshot, not the source of truth.
  **Per-call filtering MUST happen after snapshot retrieval** —
  `build_identity_messages` re-applies `applies_when` against the
  request's `PromptMode` on every call.
  *Why not cache the filtered `Vec<LoopContextMessage>`:* `PromptMode`
  can change between iterations (a `PlannedDriver`'s `ContextStrategy`
  may switch from `TextOnly` to `CodeAct` or vice versa per turn).
  Caching the filtered result locks in the first iteration's mode and
  silently produces wrong identity content for subsequent iterations
  with a different mode. Caching candidates (which are mode-independent)
  preserves the cache's purpose (avoid disk re-reads) without freezing
  the mode-dependent filter.
- For **volatile** files (HEARTBEAT.md), the source MUST be
  re-invoked each call; the volatile bucket is appended to
  `instruction_snippets` per §3.3.5 rather than cached as part of the
  stable identity prefix.
- Implementations MUST NOT install file watchers in v1 — file
  changes mid-run are explicitly out of scope under the master doc §5
  layer-1 immutability rule.
- On resume, the host reloads the pinned snapshot by ref/digest. If the
  snapshot is missing or its digest does not match the checkpoint/run
  metadata, resume fails closed with `LoopExit::Failed {
  reason_kind: CheckpointUnavailable }` rather than rebuilding identity
  from current workspace files. If a future migration wants to accept
  identity drift, it needs an explicit migration policy and audit event;
  silent drift is forbidden.

### 3.5 Mode plumbing note

`LoopContextPort::load_loop_context` does not currently see the
`PromptMode`. `HostManagedLoopPromptPort::build_prompt_bundle` calls
the context port without forwarding `request.mode`. Three options,
recommended in order:

1. **(Recommended)** Add a `LoopContextRequest.mode: PromptMode` field
   in a one-line `ironclaw_turns` follow-up. Strictly additive; no
   downstream breakage. Brief author should land this micro-PR before
   WS-15's main code, or fold it into WS-15 as a single-line touch in
   the contracts crate.
2. **Driver-time hint** — `ThreadBackedLoopContextPort` is built per
   driver; pass mode at construction. Works for `TextOnlyModelReplyDriver`
   (single mode) but breaks for `PlannedDriver` where
   `ContextStrategy` can return different modes per iteration.
3. **Filter at the prompt port** — `HostManagedLoopPromptPort` does
   the `applies_when` filtering after receiving the bundle. Wastes
   work on candidates that won't apply; correctness identical.

WS-15 ships option 1.

### 3.6 Driver-level plumbing

```rust
//! crates/ironclaw_reborn/src/text_loop_driver.rs (delta)

pub struct TextOnlyModelReplyDriverConfig {
    // ... existing fields ...
    pub identity_source: Option<Arc<dyn HostIdentityContextSource>>,
    pub identity_budget: IdentityBudget,
}
```

`PlannedDriver` (WS-7) takes the same config shape. The identity source
is composed in at driver-build time and held by the host facade — it
is not visible to strategies or the framework crate, consistent with
the §9 "message projection stays host-side" rule from the master doc.

## 4. Concrete `WorkspaceIdentityContextSource`

The concrete implementation is in scope for WS-15 and lives in
`src/workspace/`, where the existing workspace identity rules already
belong:

- Reads through `Workspace::read_primary()` so identity files never
  fall back to secondary read scopes. This preserves the existing
  isolation rule documented in
  [`src/workspace/README.md`](../../../src/workspace/README.md):
  shared memory may span scopes, but `AGENTS.md`, `SOUL.md`,
  `USER.md`, `IDENTITY.md`, `TOOLS.md`, and `BOOTSTRAP.md` are
  primary-scope only.
- Filenames pulled from
  `ironclaw_memory::safety::DEFAULT_PROMPT_PROTECTED_PATHS`.
- Trust: host-owned stable identity files → `Trusted`; personal/profile-derived
  files such as `USER.md` and `context/assistant-directives.md` are excluded
  until an explicit run-context privacy policy can authorize them.
- `HEARTBEAT.md` is a **volatile** identity file (see §3.3.5) and
  MUST NOT be returned in the stable identity bundle. WS-15 does not
  add the separate volatile instruction source because the current
  `LoopRunContext` contract has no heartbeat/run-kind signal to gate
  that content safely.
- `applies_when`: most files use `Always`; `TOOLS.md` should use
  `OnCodeAct` (it's irrelevant in `TextOnly` mode where no tools are
  visible).

The app composition root wires this source into both
`TextOnlyModelReplyDriverConfig` and `PlannedDriverConfig`. WS-14's
implicit-default cutover must use this concrete source, not a mock and
not `None`.

TOOLS.md is intended for *narrative tool guidance* (e.g. "prefer
`shell` over `file_read` when…"), not for re-declaring the concrete
tool surface — that surface flows through `LoopCapabilityPort::visible_capabilities()`
in WS-9. Workspace authors should not duplicate tool-surface metadata
(names, parameters, JSON schemas) in TOOLS.md prose; if a future
deduplication pass is needed, it lives in the concrete
`WorkspaceIdentityContextSource` impl, not in `build_identity_messages`
(which is content-agnostic).

## 5. Verification

Unit tests (in `crates/ironclaw_loop_support`):
- `identity_context::tests::filters_by_applies_when` — mock source
  returns `[Always, OnCodeAct]`; `TextOnly` request returns one
  message.
- `identity_context::tests::respects_budget` — three large candidates;
  ceiling 1000 tokens; only first two land.
- `identity_context::tests::installed_trust_summary_only` — content
  for `Installed`-trust candidate not exposed; `safe_summary` only.
- `identity_context::tests::ordering_is_deterministic` — invoking the
  source twice with the same `LoopRunContext` produces identical
  `Vec<LoopContextMessage>` (compare byte-equal serialization).
  Guards the §3.3 ordering contract.
- `lib::tests::context_port_populates_identity_when_source_set` —
  `ThreadBackedLoopContextPort` with a mock identity source returns
  non-empty `identity_messages`.
- `lib::tests::context_port_empty_identity_when_source_unset` —
  baseline; identity_messages stays `Vec::new()`.
- `lib::tests::context_port_caches_stable_identity_within_run` —
  invoke `load_loop_context()` twice on the same port; the mock
  identity source is called exactly once and both returned bundles
  are byte-equal. Guards the §3.4.5 per-run caching contract.
- `lib::tests::context_port_resume_uses_pinned_identity_snapshot` —
  seed a durable stable-identity snapshot, restart the port with a
  source that now returns different file bytes, and assert the resumed
  prompt uses the pinned snapshot.
- `lib::tests::context_port_resume_rejects_identity_digest_drift` —
  checkpoint metadata points at a snapshot whose digest no longer
  matches; assert resume maps to `CheckpointUnavailable` instead of
  rebuilding from current workspace files.

Unit tests (in `crates/ironclaw_turns`):
- `prompt::tests::identity_message_with_ref_maps_to_content_ref` —
  existing behavior for Trusted entries stays intact.
- `prompt::tests::summary_only_identity_message_maps_to_synthetic_ref`
  — `message_ref: None` produces a leading system model message whose
  synthetic ref resolves to `safe_summary`.

Workspace tests (in `src/workspace`):
- `workspace_identity_context_reads_primary_scope_only` — multi-scope
  workspace with different `AGENTS.md` content in primary and
  secondary scopes returns only the primary content.
- `workspace_identity_context_uses_protected_path_canon` — source
  iterates the same canonical protected-path list as
  `ironclaw_memory::safety::DEFAULT_PROMPT_PROTECTED_PATHS`.
- `workspace_identity_context_routes_heartbeat_as_volatile` —
  `HEARTBEAT.md` is excluded from stable `identity_messages` and
  appears only in the volatile instruction path for heartbeat runs.

Integration test (in `crates/ironclaw_reborn`):
- `text_loop_driver_with_identity` — drives a `TextOnlyModelReplyDriver`
  with the concrete workspace identity source seeded with two identity
  files. Asserts:
  - `prompt_bundle_built` milestone payload includes the identity
    filenames in its metadata block.
  - The model port receives a message list whose first two entries
    have role `"system"` and resolve to the identity refs.
  - With the source unset, the same test produces a bundle with zero
    leading identity messages (regression guard for the "stays empty"
    baseline).

## 6. Compatibility & rollout

- No `ironclaw_turns` contract change beyond the
  `LoopContextRequest.mode` field in §3.5 and optional
  `LoopContextMessage.message_ref` in §3.2. Additive, no migration,
  but every prompt/model resolver that reads `LoopContextMessage` must
  handle `None` before WS-15 can compile.
- `Option<Arc<dyn HostIdentityContextSource>> = None` is the new
  default for both drivers — existing tests, smoke runs, and the
  skeleton WS-8 integration suite all continue to pass with empty
  `identity_messages`.
- The concrete `WorkspaceIdentityContextSource` ships in this
  workstream. `identity_source = None` is acceptable for legacy tests
  and explicit non-default profiles, but not for WS-14's live default
  cutover.
- Stable identity snapshots are part of resume compatibility. Operators
  may edit `AGENTS.md`/`TOOLS.md` while a run is blocked, but the resumed
  attempt keeps the checkpoint-pinned snapshot or fails closed if that
  snapshot cannot be loaded.

## 7. Out of scope (for this brief)

- Wiring memory_snippets (the fourth `LoopContextBundle` slot). Same
  shape, different source (vector search results), tracked separately.
- Token-counting infrastructure changes — uses the host's existing
  estimator.
- Identity-file authoring tools, CLI commands to seed `SOUL.md`, etc.
- Per-thread / per-run identity overrides — the first cut treats
  identity as workspace-scoped, not run-scoped.

## 8. Deferred

**Finding #2 (serrrfirat): Group-run policy gating for personal identity files.**

`WorkspaceIdentityContextSource::load_identity_candidates` currently ignores
`run_context` entirely. To stay fail-closed without broadening WS-15's
contract, the concrete workspace source excludes `USER.md` and
`context/assistant-directives.md` from identity candidates entirely.

Deferred to WS17 when `LoopRunContext` gains an explicit
group-chat/shared-thread policy bit (`is_shared_context()`). At that point,
`load_identity_candidates` can add policy-authorized personal/profile-derived
candidates with caller-level regression coverage for both allowed and denied
group/shared contexts.

**Heartbeat context injection.**

`HEARTBEAT.md` is intentionally excluded from WS-15 stable
`identity_messages` because it is volatile and would break prompt-cache
stability. Restoring heartbeat-specific prompt context for Reborn should be a
separate follow-up that introduces an explicit run-kind/heartbeat signal on
`LoopRunContext` plus a volatile instruction source evaluated per context load.
