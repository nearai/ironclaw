# Hooks operator runbook

> Audience: operators and SREs running an IronClaw deployment with
> third-party Installed extensions registering hooks. This runbook
> covers the cases where hook behavior at runtime needs human
> intervention to recover.
>
> Companion to [`threat-model.md`](./threat-model.md) and
> [`prior-art.md`](./prior-art.md). Where this doc says "see
> threat-model finding X", look up the finding for the design
> rationale.

## 1. A hook poisoned (sticky failure)

### What you'll see

- `RuntimeEvent::HookFailed { hook_id, category, .. }` events for a
  specific `hook_id` followed by silence: the hook isn't invoked
  again, but capability invocations still flow.
- The durable runtime event stream shows `HookDispatched` for the hook stops appearing after
  the first failure event, even though the hook is still registered.

### Why this happens (by design)

The dispatcher implements **sticky poison**: when a hook panics or
trips its failure-policy category (timeout, malformed decision,
attenuation violation), the hook's registry slot is marked poisoned
and the dispatcher *skips* it for the remainder of the process's
lifetime. This is a deliberate divergence from K8s admission webhooks
(which retry per-request). The reasoning is in `prior-art.md` (Axis
5): in an agent loop, a repeatedly-panicking hook is more likely
buggy than transiently faulty, and retrying it makes the loop
unobservable.

### Recovery options, ranked

1. **Per-build dispatcher (preferred — no operator action).** The
   factory pattern (`with_hook_dispatcher_factory`) constructs a
   fresh dispatcher per host build. A new run picks up a fresh
   dispatcher with no poison carried over. If your deployment already
   uses `with_hook_dispatcher_factory`, the next run recovers
   automatically.
2. **Process restart.** If the deployment uses the legacy
   `with_hook_dispatcher` (single shared `Arc`), the poison persists
   across runs. Restart the process to clear it. This is acceptable
   for deployments where runs are short and restart is cheap.
3. **Reinstall the offending extension.** If the hook is misbehaving
   for a structural reason (manifest schema drift, predicate
   regression, manifest-window parse failure), update the extension
   to a fixed version and reinstall. Reinstallation re-derives a
   different `HookId` (extension version is hashed in), so the new
   binding is fresh and the old poisoned slot becomes irrelevant.
4. **Disable the extension.** Last resort — drop the extension from
   the manifest list. The lost functionality is the cost of the
   buggy hook.

### What you should NOT do

- **Do not manually clear the poison.** There is no API for it, by
  design. A poisoned hook indicates a real failure; clearing without
  fixing the cause re-introduces the failure.
- **Do not retry the same `HookId`.** Re-installing the same extension
  version produces the same content-addressed `HookId` and the
  registry still has the poison.

### Detecting the situation

- Alert on `HookFailed` events with `category` other than
  `AttenuationViolation` (which is usually a code-level bug rather
  than runtime).
- Alert on a sustained gap between `HookDispatched` event rate and
  registered-hook count — poisoned hooks register but never dispatch.

---

## 2. Predicate evaluator approaches its state ceiling (D5 pressure)

### What you'll see

- `PredicateEvaluator::evictions_observed()` counter advances.
- Hooks at high-cardinality attach points (many tenants, many
  capability names, many distinct hook ids) start producing
  intermittent fail-closed decisions where they used to allow.
- No `HookFailed` events — the eviction is silent at the hook level.

### Why this happens (by design)

The predicate evaluator caps both history maps at
`MAX_HISTORY_KEYS = 8192` (per map). When a new `(tenant ×
capability × hook × field)` key arrives at the cap, the LRU entry
(the key whose oldest retained timestamp is earliest) is evicted.
The cap defends against unbounded growth across permutations
(threat-model D5).

### What an eviction *means*

The evicted key's rolling window is lost. The next invocation
matching that key starts fresh at count=0 or sum=0. For
`InvocationCount` predicates this is a *partial bypass of the rate
limit* — the malicious case is an attacker who can produce many
distinct keys (by varying tenant id, for instance) to force eviction
of a key they want to flood.

For `NumericSum` predicates the same applies: rolling sums are
forgotten when their key is evicted.

### Recovery options

1. **Confirm the eviction pressure is benign.** A counter that ticks
   slowly under legitimate growth (new tenants, new capabilities)
   doesn't indicate compromise. A counter that spikes during a
   specific workload window is the signal to investigate.
2. **Audit incoming traffic for tenant-key cardinality.** If a
   single source is producing thousands of distinct tenant ids, that
   itself is a security event regardless of the eviction.
3. **Reduce hook density per attach point.** Per-extension caps
   (`MAX_HOOKS_PER_EXTENSION_PER_KIND = 8`) bound new installs but
   don't shrink an existing install. Audit installed extensions for
   redundant hooks.
4. **Increase `MAX_HISTORY_KEYS` if legitimate workload growth has
   genuinely outgrown the default.** This is a code change, not a
   runtime knob — the cap is a const. Bumping it 2–4× is reasonable;
   any larger jump should come with an analysis of why the
   per-extension cap (D3/D4) isn't already constraining growth.

### What you should NOT do

- **Do not flush the evaluator manually.** Throwing away the entire
  history map is much worse than letting LRU run: it resets *every*
  active counter, including legitimate workload's.

### Detecting the situation

- Alert when `evictions_observed()` advances by more than 0 in a
  rolling 1-hour window. The baseline should be exactly zero in
  steady state.
- Dashboard the counter alongside per-extension hook counts and
  per-tenant traffic.

---

## 3. Hook registration is rejected at install time

### What you'll see

- Extension install fails with `HookError::RegistryConstruction`
  citing one of:
  - "per-extension cap" (threat-model D3)
  - "per-kind cap" (threat-model D4)
  - manifest validation error (typically scope/grant mismatch or
    bad window)

### Recovery

These are by-design rejections. The extension author has either:

1. Declared too many hooks (over `MAX_HOOKS_PER_EXTENSION = 32`) —
   refactor to fewer hooks (most extensions need 1–5; reaching the
   cap indicates a design issue).
2. Stacked too many hooks at one attach point (over
   `MAX_HOOKS_PER_EXTENSION_PER_KIND = 8`) — same advice.
3. Declared `SameTenant` scope without the required grant — surface
   the grant in the install UX.
4. Used an unparseable window string — see the error message for
   the bad value.

### What you should NOT do

- **Do not raise the caps to ship a specific extension.** The caps
  exist to bound blast radius. An extension that hits them is
  evidence of either a design problem in the extension or an attack;
  in either case, raising the cap is the wrong answer. File an
  issue against the extension.

---

## 4. Approval gate-ref consumed but the user didn't act

### What you'll see

- A `PauseApproval` decision was emitted (audit shows
  `HookDecisionEmitted` with summary `PauseApproval`).
- The corresponding gate-ref (`gate:hook-approval-<uuid>`) appears
  in your approval gateway's outstanding-ref list, but no resolution
  event has arrived.

### Recovery

This isn't a hooks-framework concern — gate-refs are minted by the
factory but consumed and timed-out by the approval gateway (see
threat-model finding S1 on the factory/gateway split). The hook
framework's responsibility ends at minting an unguessable gate-ref.

Follow the approval gateway's own runbook for stale gate-ref
resolution.

---

## 5. General debugging

### "I want to see what a hook decided"

The model-visible `GateDecisionView` carries only the closed-vocabulary
label (`hook_predicate_denied` etc.). The rich manifest-supplied
reason is projected into the durable runtime event stream (the hook
milestone projection — distinct from formal `AuditEnvelope`
control-plane records, which a separate `Audit*` path would emit):

```
RuntimeEvent::HookDecisionEmitted { hook_id, summary, .. }
```

Query the event store by `hook_id` (the 64-char blake3 hex) to see
the per-dispatch trace.

### "I want to know which extension owns a hook"

`HookBinding.owning_extension` is set from the manifest at install
time. The registrar logs the mapping. For a content-addressed
`HookId`, the extension can be recovered by replaying
`HookId::derive` candidates against installed extensions — or just
look at the install logs.

### "I want to disable hooks temporarily"

Construct the host with the legacy `with_hook_dispatcher` taking an
empty `HookDispatcherBuilder::new(HookRegistry::new()).build_arc()`.
No hooks are registered; no dispatch happens. This is a config
change, not a runtime toggle.
