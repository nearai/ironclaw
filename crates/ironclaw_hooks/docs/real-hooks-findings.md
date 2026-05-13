# Real-hooks ergonomics findings

> Companion to [`crates/ironclaw_hooks/tests/real_hooks.rs`](../tests/real_hooks.rs).
>
> Purpose: build three representative hooks against the *public* API
> from outside the crate, mimicking what an extension or system author
> would actually write. Each piece of friction is recorded so it can
> be triaged — either we accept it (and document the workaround) or
> we fix it. **Friction = design smell** is the working hypothesis.

The three hooks:

1. **`polymarket-daily-cap`** — Installed-tier predicate hook, rate-cap
   on `polymarket.place_order` at 10 calls / 24h, deny on excess.
2. **`large-stake-approval-gate`** — Installed-tier predicate hook,
   NumericSum over `amount_usd` field, PauseApproval at $1000/24h.
3. **`pii-redaction-warning`** — Trusted-tier Rust hook implementing
   `PrivilegedBeforePromptHook`, injects a trusted instruction snippet.

These span the design space: declarative predicate + invocation
counter (Hook 1), declarative predicate + numeric resolver (Hook 2),
programmatic Rust hook at a different attach point with a different
trust tier (Hook 3).

## Findings, ranked by friction severity

### F1 — `SanitizedArguments::unresolved()` was `pub(crate)` (FIXED)

**What happened:** External tests cannot construct any
`BeforeCapabilityHookContext` with a known `provider` because the
`unresolved()` constructor was sealed to the crate. The only public
path was `new_unresolved(...)`, which sets `provider: None` — and
with `provider = None`, the FU1 scope filter (`OwnCapabilities`)
correctly drops the hook before predicate evaluation, making Hook 1
look like a non-firing hook for the wrong reason.

**Why this is friction:** A hook author writing predicate tests for
their own hook cannot reproduce the production dispatch shape (named
provider + unresolved args) without going through Reborn's middleware.
TDD of a predicate's match logic becomes impossible at the crate
boundary.

**Fix:** Made `SanitizedArguments::unresolved` `pub`. The
`from_json` (sanitizing) constructor stays sealed — that's the trust
boundary. `unresolved` is the safe default and exposing it cannot
weaken any trust property: any predicate that needs args fails closed
against it.

**Recommendation:** None remaining; done in this PR.

---

### F2 — Manifest deny `reason` text is replaced by a closed-vocabulary label

**What happened:** Hook 1's manifest sets
`OnExceededAction::Deny { reason: "daily place_order cap exceeded" }`.
At dispatch time, the decision-visible reason is the static label
`"hook_predicate_denied"`, not the manifest text. Same for
`PauseApproval` (`"hook_predicate_pause_requested"`). The manifest
text is preserved in audit milestones but never reaches the model.

**Why this exists (per the existing code comment in
`installed_hook.rs::evaluate`):** Sinks take `&'static str` reasons
"to keep adversarial format!-built strings out of the seam." A
malicious Installed extension that controlled the deny reason could
inject prompt-text or pretend to be a system message. So the dispatcher
*intentionally* collapses dynamic reasons into a closed vocabulary
for the model-visible path.

**Why this is still friction:**

- It surprises hook authors who reasonably expect their manifest
  reason to surface to the agent. The first test iteration of Hook 1
  asserted on the manifest text and failed. A hook author writing
  end-to-end tests will hit the same wall.
- The closed vocabulary is currently undocumented at the public API
  level. It exists only in a comment on a private method.
- Audit gets the rich text via `HookDecisionEmitted` / observer facts,
  but a hook author has no obvious way to *see* that during dev
  without standing up the milestone-sink wiring.

**Recommendations:**

1. Document the closed-vocabulary deny / pause-approval reasons in
   the rustdoc of `OnExceededAction` and on `GateDecisionView`. A
   hook author should learn this from `cargo doc`, not from a failing
   test.
2. Consider adding `OnExceededAction::Deny { code: DenyReasonCode }`
   alongside the freeform `reason` so authors can pick from a curated
   vocabulary of model-visible codes. This would let predicate hooks
   surface *useful* model-visible context without opening a freeform
   string channel.
3. Add a one-line example to the predicate-hook docs showing how to
   inspect the rich audit reason in a dev loop.

---

### F3 — Hook 2 (NumericSum) cannot be exercised end-to-end from outside Reborn

**What happened:** Hook 2 uses
`ValueOrRateBound::NumericSum { field: "amount_usd", ... }`. The
predicate needs *resolved* `SanitizedArguments` to evaluate the sum.
Resolved arguments require sanitization (`SanitizedArguments::from_json`,
sealed) plus a `CapabilityInputResolver` wired through middleware.
Both seams live in `ironclaw_reborn`. From a standalone
`ironclaw_hooks` test, the best a hook author can do is:

- Validate the manifest (`entry.validate()`).
- Install through the registrar.
- Confirm that dispatch with unresolved args **fails closed**.

The actual "the cap trips at $1000" behavior can only be asserted in
an `ironclaw_reborn` integration test
(`hooks_integration::numeric_sum_predicate_caps_total_value_against_real_inputs`,
which already exists).

**Why this is friction:** A third-party extension author writing a
NumericSum hook has no way to TDD the *fire* condition without
either (a) depending on `ironclaw_reborn` as a dev-dep (heavy + the
extension probably shouldn't reach into Reborn at all), or (b)
faking the resolver in their crate, which requires the
`SanitizedArguments::from_json` constructor to be reachable.

**Recommendations (pick one):**

1. Expose a `SanitizedArguments::for_tests(serde_json::Value)`
   constructor behind a `#[cfg(feature = "test-support")]` feature
   flag. Extension authors opt in via dev-dep with the feature;
   production code can't touch it. Preserves the sanitizer-only-on-
   trusted-path property while removing the test-only blocker.
2. Ship a `MockCapabilityInputResolver` as part of `ironclaw_hooks`'s
   public surface that takes a `serde_json::Value` and an opaque
   transform fn, and runs the resolver+sanitize path under test
   harness control. More machinery but doesn't require feature-gating.
3. Accept the friction and document that NumericSum hooks must be
   integration-tested via `ironclaw_reborn`. Cheapest, but it's a
   real barrier to adoption — declarative predicate hooks promised
   "no need to depend on the runtime crate to author one."

**Tentative pick:** (1). Lowest blast radius, clearest semantics.

---

### F4 — Trusted Rust hooks at `before_prompt` work cleanly (no friction)

Hook 3 was the easiest of the three to write. The trait surface is
small (`PrivilegedBeforePromptHook::evaluate` takes a context + a
sink), the sink is well-documented, and the type-level
trust-class enforcement was clear from compiler errors — when I
accidentally tried `RestrictedMutatorSink::add_trusted_snippet` (a
nonexistent method on the restricted sink), the compiler error was
informative.

**The good:**

- `PatchOrdinalHint::{Last, NearTop}` is a clear, small enum.
- `HookPatchView` projection is exactly what a test wants.
- Budget-aware bow-out (returning early when
  `remaining_snippet_byte_budget` is too small) is idiomatic.
- `HookDispatcherBuilder::install_trusted_before_prompt(...)` chain
  reads naturally.

**No recommendations.** This is what the rest of the API should
feel like.

---

### F5 — `HookId::derive` requires the crate's `identity::ExtensionId`, not `ironclaw_host_api::ExtensionId`

**What happened:** A hook author already holds an
`ironclaw_host_api::ExtensionId` (the authoritative identifier). To
mint a `HookId` via `HookId::derive`, they need
`ironclaw_hooks::identity::ExtensionId` — a different newtype.
Conversion is a one-liner (`identity::ExtensionId(host_ext.as_str().to_string())`)
but the duplication surprises readers.

**Why this exists:** The hash derivation type is a transparent string
newtype; the host-api type is validated and comparable.
`HookRegistrar` already does the mirroring internally.

**Why this is friction:** Hook authors who build hook IDs by hand
(as Hook 3 does, because it's a Trusted in-process hook installed
directly without going through the registrar) have to know about both
types. Discoverability is poor — `cargo doc` shows two
`ExtensionId` types and the relationship isn't obvious.

**Recommendations:**

1. Add a `From<&ironclaw_host_api::ExtensionId>` impl for
   `ironclaw_hooks::identity::ExtensionId`. One-liner, makes the
   conversion ergonomic.
2. Document the relationship between the two types in the crate-level
   rustdoc.

---

### F6 — `HookManifestEntry` constructor is bare-struct-literal-only

**What happened:** Hook 1 and Hook 2 each construct a
`HookManifestEntry` via a 7-field struct literal. There's no builder
and no `Default` impl. Adding a new optional field to the struct
would silently change behavior at every call site (because
`#[serde(default)]` makes the field optional at deserialization but
not at construction).

**Why this is friction:** Friction-by-future-tense. The framework will
grow optional manifest fields (versioning, attribution, additional
scopes). Today's hook-author code becomes broken-by-omission tomorrow.

**Recommendations:**

1. `#[derive(Default)]` is not viable directly because of the inner
   enums, but a `HookManifestEntry::new(id, kind, body)` constructor
   with `..Default::default()`-style field overrides via a small
   builder would isolate the surface.
2. Alternatively, mark `HookManifestEntry` `#[non_exhaustive]` and
   provide a builder. `#[non_exhaustive]` forces external constructors
   to go through a builder, which buys forward compatibility.

---

### F7 — `HookPriority::DEFAULT` is the only easily-discoverable priority

**What happened:** Building any of the three hooks, the natural
question is "what priority do I want?" The docs don't surface
guidance (e.g., "use `DEFAULT` unless you have a concrete reason; if
you do, here's the convention"). `HookPriority` exposes `DEFAULT` and
arithmetic; no named variants like `HIGH`, `LOW`, `LATE`.

**Why this is friction:** A hook author guesses or copies. Two hooks
at the same priority order by hook-id (stable but author-opaque).
Result: subtle behavioral coupling.

**Recommendation:** Add a short rustdoc example to `HookPriority`
explaining the priority space, the stable tiebreaker, and when to
deviate from `DEFAULT`. Optionally add named constants (`EARLY`,
`LATE`).

---

## Summary

| ID | Severity | Status |
|---|---|---|
| F1 — Sealed `unresolved()` blocks external dispatch tests | High | **Fixed** — `pub fn unresolved()` |
| F2 — Closed-vocabulary deny reason is undocumented | Med | **Fixed** — rustdoc on `OnExceededAction` and `GateDecisionView`. The `DenyReasonCode` enum is deferred (still worth doing but not blocking) |
| F3 — NumericSum can't be TDD'd outside Reborn | Med | **Fixed** — `SanitizedArguments::for_tests(value)` under `test-support` feature flag |
| F4 — Trusted Rust before_prompt hooks (no friction) | — | — |
| F5 — Two `ExtensionId` types are confusing | Low | **Fixed** — `From<&ironclaw_host_api::ExtensionId>` impl + cross-link rustdoc |
| F6 — `HookManifestEntry` struct literal is fragile | Low | **Fixed** — `#[non_exhaustive]` + `new(id, kind, body)` + `with_*` builder methods |
| F7 — Priority guidance is missing | Low | **Fixed** — rustdoc on `HookPriority` with explicit guidance and named constants |

**Big-picture observation:** Hook 3 (Trusted Rust hook) was easier to
write than Hook 1 (declarative predicate). That's surprising — the
predicate language was supposed to be the *easier* path. Three of the
seven findings (F1, F2, F3) target predicate-authoring ergonomics.
The declarative path needs the most polish before third-party
extension authors will trust it for non-trivial policy.

## What this exercise did NOT cover

- Predicate authors who'd use TOML rather than Rust struct literals.
  The serialization round-trip is already tested in `manifest.rs`,
  but typo / schema-mismatch ergonomics are a separate exercise.
- Hooks at `after_*` observer points (the dispatcher composes them
  similarly to before_prompt; not exercised here).
- Hooks under load (timeouts, panics, contention). The
  failure-policy matrix has unit tests; ergonomics under those
  conditions is a separate exercise.
- WASM-body hooks (stubbed in v1).
