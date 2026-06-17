# ProductContextFactory — generic inbound/outbound turn-context resolution

Date: 2026-06-13
Status: Approved (design)
Branch: `context-slice-4828` (PR #4836)
Relates: #4828 (runtime-context slice), supersedes the scattered `run_origin` plumbing introduced on this branch.

## Problem

The runtime-context slice work (PR #4836) introduced a thin `run_origin: Option<TurnRunOrigin>`
field threaded through the turn pipeline. Code review surfaced that the logic around it is a
quick-fix patch with duplicated logic and blurred ownership:

1. **Origin classification has no owner.** `WebUiChat` is minted in two files
   (`reborn_services.rs`, `runtime.rs` local-dev); `ProductInbound` in two
   (`inbound_turn.rs`, `conversations/inbound.rs`); `ScheduledTrigger` decided inline in
   `conversations/inbound.rs`. Each submit site independently decides origin. The
   trusted-vs-untrusted trigger bug (an untrusted inbound with `adapter_kind == "trigger"`
   being mislabeled `ScheduledTrigger`) is a direct symptom — the rule lives inline at one
   site and nothing stops another site getting it wrong.
2. **Channel projection duplicated in the wrong crate.** The canonical lifecycle→extension
   projection lives in `reborn_services::extensions::list_extensions`. The communication
   provider bypasses it, re-calls `LifecycleProductFacade::execute(ExtensionList)`, and
   re-maps with a stubbed `extension_is_channel_surface` (hardcoded `false`).
3. **Adapter identity downgraded to `String`** crossing into `TurnRunOrigin`, losing its type.

## Decisions (locked during brainstorming)

- **A — Resolve at ingress, persist on the turn.** The context is resolved once at each submit
  site and persisted as a typed value on the turn, replacing the thin `run_origin` field. The
  communication slice renders what is stored; no re-resolution at loop start for the per-turn facts.
- **A — Factory is a pure ingress resolver, no I/O, no facades.** Live account state (connected
  channels, current delivery target) is a property of the user's account *now*, not of the turn,
  and stays in the composition-layer `CommunicationContextProvider`.
- **1 — New `ironclaw_product_context` crate** owns the resolver. The graph forces this: the
  resolver must be callable from `ironclaw_conversations` (which does not depend on
  `ironclaw_product_adapters`) and from `ironclaw_product_workflow`, so it must be a low crate
  both can depend on. The persisted type is forced into `ironclaw_turns` (the bottom crate that
  defines `SubmitTurnRequest`), so it must be fully generic.
- **(a) — Slice-only owner now.** The factory derives a generic `TurnOwner`. Unifying the
  outbound delivery engine's personal-vs-shared key derivation onto this owner is a **follow-up (b)**.

## Architecture

### Crate & type layout

```
ironclaw_turns (bottom; persisted, generic data only)
  enum TurnOriginKind   { WebUi, Inbound, ScheduledTrigger }
  enum TurnSurfaceType  { Direct, Channel }            // dm vs channel; product-agnostic
  newtype RunOriginAdapter (bounded_string_id!)        // generic; replaces raw String
  enum TurnOwner        { Personal { user: UserId },
                          SharedAgent { agent: AgentId, project: Option<ProjectId> } }
  struct ProductTurnContext {
      origin: TurnOriginKind,
      surface_type: Option<TurnSurfaceType>,
      adapter: Option<RunOriginAdapter>,
      owner: TurnOwner,
  }
  // Serialize/Deserialize; carried via #[serde(default)] optional field on the structs below.

ironclaw_product_context (NEW; resolver, no I/O)
  enum InboundClassification { TrustedTrigger, TrustedOther, Untrusted }  // input-only, not persisted
  fn resolve_inbound(
      classification: InboundClassification,           // call site collapses trust + trigger-adapter into one value
      adapter: RunOriginAdapter,
      surface_type: Option<TurnSurfaceType>,
      owner: TurnOwner,
  ) -> ProductTurnContext
  fn resolve_web_ui(owner: TurnOwner) -> ProductTurnContext
  // The ONE rule: origin == ScheduledTrigger iff classification == TrustedTrigger;
  // otherwise Inbound. resolve_web_ui always yields WebUi.
  // deps: ironclaw_turns, ironclaw_host_api
```

`ironclaw_conversations` and `ironclaw_product_workflow` add a dependency on
`ironclaw_product_context`. No cycle: the new crate depends only on `turns` + `host_api`, both of
which those crates already depend on.

The "trusted-trigger adapter" identity constant (`TRIGGER_TRUSTED_ADAPTER_KIND`, owned by
`ironclaw_triggers`) is compared at the `conversations` call site (which already depends on
`ironclaw_triggers`) to decide whether the inbound is `InboundClassification::TrustedTrigger`, so the
resolver itself depends on neither `ironclaw_triggers` nor any rich adapter type.

### Data flow

```
ingress (4 sites)               resolver (1 owner)        loop start           render
webui submit ───────┐
product inbound ────┤─ map rich→generic ─► resolve_*() ─► ProductTurnContext ─► persisted on
conversation inbound┤  (trivial enum maps)                (typed, immutable)     SubmitTurnRequest
trigger (trusted) ──┘                                                            → TurnRunState
                                                                                 → TurnRunRecord
                                                                                 → LoopRunContext
composition CommunicationContextProvider:  renders persisted ProductTurnContext ◄┘
  + fetches LIVE account state (connected channels, delivery target) via facades
  → one combined runtime-context slice
```

### Call-site changes

Each of the four submit sites stops hand-building origin variants and instead maps its locals to
the generic inputs and calls the resolver:

- `reborn_services.rs` (webui) and `runtime.rs` (local-dev webui): `resolve_web_ui(owner)`.
- `inbound_turn.rs` (product inbound): map `ProductAdapterId → RunOriginAdapter`, route kind →
  `TurnSurfaceType`; `resolve_inbound(InboundClassification::Untrusted, …)` (product inbound is
  untrusted ingress).
- `conversations/inbound.rs`: collapse `BindingResolutionPolicy` into `InboundClassification`,
  map `ConversationRouteKind → TurnSurfaceType` and `AdapterKind → RunOriginAdapter`;
  `resolve_inbound(classification, …)`. `TrustedTrigger` is reached only when the trusted-trigger
  submit seam carries `TrustedInboundKind::Trigger` on the request — trigger-ness is read from
  that typed field, not re-derived from the adapter-kind string (Option 7, folded from #4851).

The contradictory-pair bug becomes structurally impossible: callers collapse trust + trigger
signals into one `InboundClassification` before the resolver, so a mismatched (trust, trigger)
pair is unrepresentable, and `ScheduledTrigger` requires `InboundClassification::TrustedTrigger`.
`resolve_inbound` is the single *intended* mint point, but `ProductTurnContext::new` is a public
low-level constructor in `ironclaw_turns` and is not a hard cross-crate seal (Rust has no
friend-crate visibility). The enforced trust boundary is upstream: `TrustedTrigger` is produced
only by the trusted-trigger submit seam carrying a typed `TrustedInboundKind::Trigger`, never by
re-deriving trigger-ness from the adapter-kind string.

### Render

`render_model_content` reads `ProductTurnContext.origin` + `.surface_type` + `.owner` for the
origin/surface line (e.g. "Run origin: scheduled trigger fire", "replies post to this channel"
vs "this DM"). The live composition provider supplies the connected-channels and
delivery-target lines as today. All external strings (adapter, channel name, delivery display)
are sanitized at render time.

## Migration

- Delete the branch-only `run_origin: Option<TurnRunOrigin>` and the `TurnRunOrigin` enum.
- Add `product_context: Option<ProductTurnContext>` to `SubmitTurnRequest`, `TurnRunState`,
  `TurnRunRecord`, and `LoopRunContext`, all `#[serde(default)]`. No DB migration (JSON snapshot).
- Child/subagent runs inherit the parent's `product_context`.
- `create_host` carries `claimed.state.product_context` into `LoopRunContext`.

## Testing

- Resolver unit tests in `ironclaw_product_context`: the `InboundClassification` × surface matrix
  (`TrustedTrigger` ⇒ `ScheduledTrigger`; `TrustedOther`/`Untrusted` ⇒ `Inbound`), including the
  security case (`Untrusted` with a trigger adapter name ⇒ `Inbound`, never `ScheduledTrigger`).
- Call-site/contract tests updated to assert the persisted `ProductTurnContext` (webui ⇒ WebUi;
  product inbound ⇒ Inbound with adapter + surface; trusted trigger ⇒ ScheduledTrigger;
  untrusted "trigger" ⇒ Inbound).
- Render tests assert origin/surface/owner lines and string sanitization.
- Snapshot round-trip test: `product_context` survives persistence (replaces the `run_origin`
  round-trip test).

## Out of scope (tracked follow-ups)

- **(b) Owner unification**: have `OutboundResolutionEngine` consume the persisted `TurnOwner`
  instead of re-deriving the personal-vs-shared `CommunicationPreferenceKey` from `TurnScope` at
  delivery time. Touches `ironclaw_outbound` and the triggered-delivery path; needs its own
  delivery tests.
- **Channel-projection ownership**: replace the `extension_is_channel_surface` stub with a real
  channel-surface query owned by the extensions/lifecycle layer, populated by #4778's
  `ProductAdapter` surface-kind projection. Until then the live channels render `unknown`.
- **`delivery_tools_visible` at the prompt boundary**: compute tool visibility from the same
  filtered `visible_surface` the prompt uses, rather than the raw host surface (two-truths fix).
- **Untrusted-label prompt hardening**: render external labels as bounded JSON-escaped data or
  opaque ids rather than char-sanitized display strings.

## Reconciliation with in-flight review fixes

The background fixer's origin-classification items (trusted-policy gating, trigger-literal dedup)
are absorbed by `resolve_inbound` and become throwaway. Its orthogonal items survive and should
land first: concurrent facade fetches, `// silent-ok:` on the surface read, mock-captures-all-args,
channel-name sanitization, trait doc comment, channel→`Unknown` interim render, and the
contract/render tests.
