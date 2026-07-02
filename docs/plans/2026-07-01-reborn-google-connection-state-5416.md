# Reborn Google connection-state contradiction (#5416)

**Status:** plan (pre-implementation)
**Issue:** nearai/ironclaw#5416 — `[QA] Incorrect Google connection state causes contradictory authentication flow` (label `bug_bash_P2`)
**Scope:** Reborn stack only. Engine v1/v2 out of scope.

## 1. Symptom

Fresh user, no Google connected. User: "connect to Gmail". Agent replies "Gmail is
already connected". User questions it; agent flips to "only installed, not
activated" and then activates it. Two contradictory answers, eroded trust.

## 2. Root cause

A Google extension has **two independent axes**:

- **lifecycle** — is the extension installed / enabled (`LifecyclePhase`)
- **credential** — does a valid Google OAuth token exist for this caller

The model-facing **extension search** path asserts "connected / ready" from the
lifecycle axis alone. An extension can be `Active` (enabled) while the OAuth
token is missing or expired — activation gates on credentials, but the lifecycle
record is durable and outlives the token (expiry/revocation). That is exactly
the "fresh session, no Google connected" state in the report.

Concrete defects (all in `ironclaw_reborn_composition/src/extension_lifecycle.rs`
unless noted):

- **A1 — phase overload / missing cred check on `Active`.**
  `search_installation_phase()` (~:1193) checks credentials **only** on the
  `Installed` branch (mapping `Installed → Configured` when creds present). An
  already-`Active` extension returns `Active` early with **no** credential check.
- **A2 — destructive blanket suppression.**
  `suppress_search_credential_onboarding()` (~:1226) unconditionally clears
  `credential_requirements` and `onboarding` on every search summary, erasing the
  information the "ready?" predicate then inspects.
- **A3 — predicate degenerates to phase-only.**
  `extension_search_has_ready_result()` (~:1231) checks `phase ∈ {Configured,
  Active}` AND requirements empty AND onboarding none — but A2 always makes the
  latter two true, so it collapses to phase-only. It gates the model-visible
  message: *"Search found installed extension results that are already configured
  or active. Treat those results as ready for this connection request; do not ask
  the user for credentials unless a later tool call reports auth_required."*
  (~:207). That string is the literal driver of "already connected".
- **B — wire `authenticated` bit fails open.**
  `reborn_services/extensions.rs::extension_info()` (:254) derives `authenticated`
  from `ExtensionCredentialReadiness`; the `Unknown` arm (:268) falls back to
  `lifecycle_authenticated` (phase), so the settings-UI list claims "connected"
  whenever the credential service is unwired/unavailable.

## 3. Why this keeps happening (the reason a minimal patch is wrong)

This code is on its **third** iteration of the same oscillating bug:

- **#4996** `[codex] Fix stale extension search onboarding` — added
  `search_setup_is_complete(phase)`.
- **#5037** `suppress stale extension search credential prompts` — replaced it with
  the blanket `suppress_search_credential_onboarding()`.

#4996/#5037 fixed the **opposite** symptom — the agent *nagging* for credentials
when already set up. The blanket-suppress overshot and now the agent *claims
connected* when it isn't. The flow oscillates between "nag-when-ready" and
"claim-ready-when-not" because **no single value answers "are the credentials
actually present?"** — the search path infers it from `LifecyclePhase` plus a
destructive mutation. A minimal "check creds on the `Active` branch too" patch is
patch #4 and sets up patch #5.

### Structural defect behind all three: credential presence is answered twice

"Is the credential present?" is resolved by **two** trait fronts over the **same**
backend (`RebornProductAuthServices`), with **divergent** result shapes and
fail-modes:

| Surface | Port | Result | Fail-mode |
|---|---|---|---|
| search / activate | `RuntimeExtensionActivationCredentialGate` → `RuntimeCredentialAccountSelectionService` | `missing_requirements: Vec` | gate `None`/Backend err → whole search errors or `false` |
| extension list | `ExtensionCredentialSetupService` → `ProductAuthExtensionCredentialSetup` | `Option<CredentialAccountProjection>` → `ExtensionCredentialReadiness` | `Unknown` → fail **open** |

Two answers to one question, guaranteed to drift — and they have (one fails
closed, one fails open).

## 4. Design (behavior-preserving reframe, not patch #4)

Make credential readiness an **explicit axis** the search path computes once, and
delete the phase-overload + blanket suppression so the "ready" decision reads
truth. Keep the two ports for now (converging them is a flagged follow-up), but
give the search path the **same 3-state semantics** the list path already uses.

### 4.1 One model-facing state — collapse the two axes into `availability`

The model does not need the lifecycle phase or the raw credential-readiness enum.
Its only questions for "connect X" are **"is it installed?"** and **"is the
credential present?"**. Those two internal facts project to exactly one
model-legible field. Exposing `Installed`/`Configured`/`Active` phase jargon *and*
a separate readiness axis is both confusing to the model and the very
two-signals-for-one-question shape §3 blames.

**New wire enum** `ExtensionAvailability` (in `lifecycle.rs`, next to
`LifecyclePhase`; `#[serde(rename_all = "snake_case")]`, wire-stable per `types.md`
— helper methods for rendering, no `format!("{:?}")`):

| Model value | Meaning (what the model should do) | Projected from |
|---|---|---|
| `available` | usable — connect without asking the user for anything | installed **and** credentials satisfied (or none required) |
| `needs_auth` | installed, a required credential is missing → run sign-in | installed **and** `MissingRequired` |
| `not_installed` | discover / install it first | no installation record |
| `unknown` | could not determine (transient backend) → don't claim either way | credential backend blip |

Crucially, **`Active` vs `Installed` vs `Disabled` does not change the model
value** — the projection is `installed? × credential-readiness`, nothing else. The
lifecycle phase stays entirely internal (it drives the install→activate *tool
flow*, not the model's connection judgment; `activate` is idempotent and
re-enables a `Disabled` extension). This is the real collapse the reported bug was
missing.

Internally the credential fact still comes from the existing search gate
(`RuntimeExtensionActivationCredentialGate.missing_requirements`); the projection:

| Installation | Gate result | `availability` |
|---|---|---|
| none | — | `not_installed` |
| present | no required credentials | `available` |
| present | `Ok(missing.is_empty())` | `available` |
| present | `Ok(non-empty)` | `needs_auth` |
| present | `Err(Backend)` | `unknown` |

`missing_requirements()` only ever returns `Ok(vec)` or `Err(Backend)`:
`CredentialStageError::AuthRequired` is folded to `Ok(false)` upstream
(`product_auth_runtime_credentials.rs:164-174`), so there is no `AuthRequired` arm
to build (the existing `map_search_credential_stage_error` `AuthRequired` arm is
already dead code).

**`ExtensionCredentialReadiness` (the list path's 4-state) is untouched** — it stays
internal to `reborn_services` for the settings DTO's `authenticated`/`needs_setup`
bits (Defect B, §4.5). No cross-crate move; the model surface uses the collapsed
`ExtensionAvailability`, the settings surface keeps its richer internal axis. The
one thing both share is the credential *source* (`RebornProductAuthServices`);
convergence of the two ports remains the deferred follow-up (§7).

### 4.2 Pure lifecycle phase, internal only

`search_installation_phase()` returns `phase_for_activation_state()` **only** — drop
the `Installed → Configured` credential smuggling. That synthetic `Configured`
existed solely to carry "creds present" into the single field the predicate read;
with `availability` the phase no longer needs to lie, and it is no longer on the
model surface at all (§4.3).

### 4.3 Replace `installation_phase` on the model surface with `availability`

`LifecycleSearchExtensionSummary` (`lifecycle.rs:358`) today carries
`installation_phase: Option<LifecyclePhase>`. Verified consumers: only the
same-file `search()` predicate, `extension_lifecycle_command.rs`, the registry
producer in `reborn_services/extensions.rs`, and this file's tests — **no
webui_v2 / gateway / frontend reader**. Replace it:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub availability: Option<ExtensionAvailability>,
```

The model reads one first-class token (`availability: "needs_auth"`) instead of a
lifecycle phase it must interpret. Both producers of the summary
(`extension_lifecycle.rs::search_summary` and the registry path in
`reborn_services/extensions.rs`) set it.

Replace `suppress_search_credential_onboarding` (delete it) with
availability-driven handling in `search_summary`:

- `available` → clear `credential_requirements` + `onboarding` (satisfied / not
  needed; the honest version of #5037's intent).
- `needs_auth` / `unknown` → **keep** `credential_requirements` + `onboarding` so
  the model has the actionable how-to-connect detail (provider, OAuth scopes,
  setup URL).
- `not_installed` → unchanged (no installation, discovery result).

### 4.4 Message off `availability`, not payload re-inspection

`search()` computes `availability` per extension and decides the message directly:

```
ready = any result has availability == Available
```

`extension_search_has_ready_result` becomes a pure function of the collected
`availability` values (delete the payload-field re-inspection). The "treat as ready
/ do not ask for credentials" message fires only when at least one result is
`available` — which, by construction, cannot happen with a missing required token.

**Concurrency (round-2 finding B).** Today the sequential per-extension credential
check in `search()` (`extension_lifecycle.rs:194` — `for extension { push(search_
summary(..).await) }`) only fires for `Installed`-phase extensions (A1 skips
`Active`). Projecting `availability` for **every** result widens a live
O(extensions × requirements) sequential await chain (each `missing_requirements`
hits account selection per requirement). The list path already solved this one file
away: `lifecycle_extension_infos` (`reborn_services/extensions.rs:192-213`) fans out
with `stream::iter(..).buffered(EXTENSION_READINESS_CONCURRENCY)` (const = 8,
ceiling test at `:515`; `futures = "0.3"` is already a composition dep,
`Cargo.toml:111`). Adopt the same pattern for the search loop. Lives in the
extracted module (§4.6).

**Backend degrade.** A `Backend` blip currently **fails the entire search**
(`map_search_credential_stage_error` → `Transient`). New: it becomes `unknown`
(search still returns; that extension shown not-`available`). Fail-closed for the
"ready" claim, search stays up.

**Behavior-flip to lock with a test (round-1 finding #7):** the old
`search_credentials_configured` short-circuited `requirements.is_empty() → false`
*before* consulting the gate, so a **credential-less** extension never reached
"ready" via search. Under the projection it becomes `available` for an installed
credential-less extension. That is the intended, more-correct

**Concurrency (round-2 finding B).** Today the sequential per-extension credential
check in `search()` (`extension_lifecycle.rs:194` — `for extension { push(search_
summary(..).await) }`) only fires for `Installed`-phase extensions (A1 skips
`Active`). Closing A1 means computing readiness for **every** result, widening a
live O(extensions × requirements) sequential await chain (each
`missing_requirements` hits account selection per requirement). The list path
already solved this one file away: `lifecycle_extension_infos`
(`reborn_services/extensions.rs:192-213`) fans out with
`stream::iter(..).buffered(EXTENSION_READINESS_CONCURRENCY)` (const = 8, with a
ceiling test at `:515`). Adopt the **same** canonical pattern for the search loop —
don't leave a widened sequential fan-out. This lives naturally in the extracted
module (§4.6).

**Behavior-flip to lock with a test (round-1 finding #7):** the old
`search_credentials_configured` short-circuited `requirements.is_empty() → false`
*before* consulting the gate, so a **credential-less** extension never reached
"ready" via search. Under the projection it becomes `available` for an installed
credential-less extension. That is the intended, more-correct behavior (an
extension needing no credentials is legitimately "don't ask for credentials"), but
it is a silent change — pin it explicitly in §6.

### 4.5 Defect B — fail closed, without creating a new dead-end state

`reborn_services/extensions.rs::extension_info()` (:254). Flipping only the
`authenticated` `Unknown` arm to `false` is **incomplete** and relocates the
contradiction (round-1 finding #3): `needs_setup` (:282) is
`readiness == MissingRequired || phase ∈ {Installed, Configured, Failed}`. For
`phase ∈ {Active, Activating}` with `readiness == Unknown`, you'd get
`authenticated: false, needs_setup: false`, and `for_installed_with_credential_status`
(`extension_onboarding.rs:33-54`) has no `Unknown` branch so it falls through to
generic onboarding — the UI/model would see "not connected" with **no** way to fix
it. Same class of contradiction as #5416, just on the settings surface.

Fix all three coherently for `Unknown`:

- `authenticated` → `false` (fail closed).
- `needs_setup` → also `true` when `readiness == Unknown` (so the state is
  actionable), or introduce a distinct "needs reverify" signal — prefer reusing
  `needs_setup` unless a separate signal earns its keep.
- `extension_onboarding::for_installed_with_credential_status` → add an `Unknown`
  branch that yields a reconnect/reverify affordance rather than generic install
  onboarding.

Self-corrects on the next successful poll (the retryable path already exists).

### 4.6 Decomposition (file health)

`extension_lifecycle.rs` is 4,326 lines total, but `#[cfg(test)] mod tests` starts
at ~:1348 — **production code is ~1,300 lines**, already under the 1,500 "aim
smaller" bar (round-1 finding #6). So the file-size framing is weaker than it
looks; the extraction is still worth doing on its own merits (a cohesive
~100–150-line concern), just not justified by "4× the smell line".

Extract the **search-readiness mapping + `ready` predicate** into a focused module
(neutral name, e.g. `extension_credential_readiness.rs`, since it's the shared
credential axis, not search-only). **Do not move `phase_for_activation_state`** —
it's shared by `list_installed` (:241) and `installed_summaries` (:312); moving it
into a search-named module makes the list path import from "search". Keep it in the
parent file.

The real long-term decomposition is that ~15 operations
(install/activate/remove/search/list/restore/rollback/…) share one impl block. The
tracking issue should scope **"split by operation"**, not a vague "further
decomposition".

## 5. Net effect

- **Deletes:** `suppress_search_credential_onboarding` (destructive mutation),
  the `Installed → Configured` phase overload, `search_credentials_configured`'s
  bool-collapse, the payload re-inspection in the predicate, and the
  search-fails-on-Backend-error path.
- **Adds:** one collapsed model-facing `ExtensionAvailability` wire enum
  (`available`/`needs_auth`/`not_installed`/`unknown`), replacing `installation_phase`
  on the search DTO; a focused module for the projection + predicate.
- **Leaves internal:** `ExtensionCredentialReadiness` (list path only) and
  `LifecyclePhase` (drives the tool flow, not the model surface) — no cross-crate
  move, no phase jargon exposed to the model.
- Fewer moving parts than the "A+B minimal" patch; the model sees **one explicit
  state** instead of two raw axes; and the oscillation is structurally impossible
  (the projection reads credential truth).

## 6. Tests (test-first, through the caller)

Per `.claude/rules/testing.md` "test through the caller":

1. **Search — Active + no token (the bug).** Drive the `builtin.extension_search`
   handler (`ExtensionLifecycleToolHandler::dispatch`) with gmail `Enabled` and no
   OAuth account. Assert: response `message` does **not** contain the "treat as
   ready / do not ask" string; the gmail summary retains `credential_requirements`
   + `onboarding`; `availability == NeedsAuth`.
2. **Search — Active + token present (no regression).** Same handler, credential
   account configured. Assert: message present; requirements + onboarding cleared;
   `availability == Available`.
3. **Search — Installed (inactive) + token present.** Assert: `available`, message
   present, requirements cleared. (Confirms lifecycle Active-vs-inactive does not
   change the model state.)
4. **Search — backend blip → Unknown.** Credential service returns a retryable
   error. Assert: search **succeeds** (no error), `availability == Unknown`,
   requirements retained, no "ready" message.
5. **Search — credential-less extension (behavior-flip lock, finding #7).** A
   bundled extension with **no** credential requirements, `Installed`. Assert
   `availability == Available` and message present — pins the intended change from
   the old `requirements.is_empty() → false`.
6. **Defect B — list fails closed AND stays actionable.** Drive `extension_info`
   via the facade (`RebornServices`) with readiness `Unknown` on an `Active`
   extension; assert `authenticated == false`, `needs_setup == true`, and
   onboarding yields a reconnect/reverify affordance (not generic install
   onboarding).

Extend existing search tests (`extension_lifecycle.rs` tests ~:1938) rather than
standing up parallel suites; add the new module's unit tests for the projection
function (each `(installation, gate result)` → `availability` row).

**Must-change existing assertion (round-2 finding).** `extension_lifecycle.rs:2359`
currently asserts `github.installation_phase == Some(Configured)` for an
Installed+credentialed extension found via search — the exact `Installed→Configured`
smuggling §4.2 deletes, and `installation_phase` itself is being removed from the
model DTO. Under the new design that becomes `availability == Some(Available)`
(requirements still empty, onboarding still none). Update in place.

## 7. Non-goals / follow-ups

- **Defect C — model capability surface `NeedsAuth`.** `VisibleCapabilityAccess`
  (`host_runtime/surface.rs`) has only `Available`/`RequiresApproval`; the catalog
  has no credential-presence handle. Adding a credential-aware access state is a
  cross-crate architectural change (the engine-v2 analogue is even marked
  "intentional"). Deferred — only reachable once gmail is activated, and this plan
  fixes the search/list messaging that drives the reported contradiction. Track
  separately.
- **Converge the two credential ports.** `RuntimeCredentialAccountSelectionService`
  + `ExtensionCredentialSetupService` should become one credential-readiness port
  so there is exactly one answer to "is the credential present?". Strategic; file a
  tracking issue. This plan aligns their *semantics* (3-state, fail-closed) without
  merging the traits.

## 8. Touch points

- `crates/ironclaw_product_workflow/src/lifecycle.rs` — new `ExtensionAvailability`
  wire enum next to `LifecyclePhase` (+ serde/snake_case); replace
  `installation_phase` with `availability: Option<ExtensionAvailability>` on
  `LifecycleSearchExtensionSummary`.
- `crates/ironclaw_product_workflow/src/lib.rs` — add `ExtensionAvailability` to the
  `pub use lifecycle::{...}` block (:111).
- `crates/ironclaw_reborn_composition/src/extension_lifecycle.rs` — search path
  (delete overload/suppression; project `availability`; move projection+predicate
  cluster out; keep `phase_for_activation_state` here).
- `crates/ironclaw_reborn_composition/src/extension_availability.rs` — **new**
  (`(installation, gate result) → ExtensionAvailability` projection + `ready`
  predicate + `buffered(8)` fan-out).
- `crates/ironclaw_reborn_composition/src/extension_lifecycle_command.rs` — registry
  summary producer sets `availability` (`not_installed` for registry-only rows).
- `crates/ironclaw_reborn_composition/src/extension_lifecycle_capabilities.rs` —
  caller-level test entry (`ExtensionLifecycleToolHandler`).
- `crates/ironclaw_product_workflow/src/reborn_services/extensions.rs` — Defect B
  (`Unknown` → `authenticated=false` + `needs_setup=true`); registry search producer
  sets `availability`.
- `crates/ironclaw_product_workflow/src/reborn_services/extension_onboarding.rs` —
  `Unknown` reconnect/reverify branch in `for_installed_with_credential_status`.

> **Note:** this §8 is the *interim* fix. The user's north-star directive (one
> unified tool-lifecycle pipeline) supersedes the layering here — see the north-star
> plan (separate doc) and §7. The interim fix is designed to be a strict subset of
> the north star (the `ExtensionAvailability` projection becomes the pipeline's
> single connection-state output), not throwaway.
```
