---
name: reborn-refactor-simplifier
description: Use when refactoring IronClaw Reborn to remove complexity, shrink composition/product/capability layers, review whether a refactor really simplified the code, or mine recent PR feedback for deletion-oriented follow-up work.
---

# Reborn Refactor Simplifier

Use this skill to turn a "cleanup/refactor" request into actual simplification:
fewer concepts, fewer wrappers, fewer facades, fewer branches, fewer test doubles,
and less code in `ironclaw_reborn_composition`.

The core failure mode to prevent: **a refactor that preserves behavior by adding
new names and adapters while the old path stays alive.** That is migration work,
not simplification. Treat it honestly, then delete the old path.

## Default Loop

1. **State the simplification axis in one sentence.**
   - Good: "Product writes should go through one ProductOperation registry, not
     direct facade methods plus API-only capabilities plus commands."
   - Bad: "Clean up ProductSurface."

2. **Mine the last 5 days of maintainer PRs and feedback.**

   ```bash
   gh pr list --state all --author ilblackdragon --limit 100 \
     --json number,title,state,createdAt,updatedAt,mergedAt,url \
     --jq '.[] | select(.updatedAt >= "YYYY-MM-DDT00:00:00Z") | "#\(.number)\t\(.state)\t\(.title)\t\(.url)"'
   gh pr view <PR> --comments --json number,title,state,body,comments,reviews,url
   ```

   Read open PRs too. They contain the most useful real-time feedback.

3. **Measure the current branch before changing code.**

   ```bash
   git status -sb
   git diff --stat origin/main...HEAD
   git diff --name-only origin/main...HEAD
   rg -n "trait .*:|Arc<dyn|api_only|execute_command|match profile|RuntimeProfile::|ProductSurface|RebornServicesApi|CapabilityDispatchRequest|RuntimeLaneRequest|HostRuntime|CapabilityDispatcher" crates
   ```

4. **Classify the refactor.**

   | Class | Meaning | Bar |
   | --- | --- | --- |
   | Rename | New name, old concept still alive | Not simplification |
   | Migration | New path exists, old path still feeds callers | Useful, but not done |
   | Load-bearing | Replacement is consumed on the production path | Real progress |
   | Deletion | Old path/types/branches/tests are gone | Simplification |

5. **Pick the deletion move, then implement.**
   Prefer one deletion-oriented slice over a broad additive migration. If a
   temporary bridge is necessary, name the old call sites it will delete and add
   or update a ratchet that fails if new old-shape call sites appear.

6. **Validate through the caller.**
   Run the narrow tests that cover the production path, plus architecture tests
   when dependencies, facade shape, or composition ownership changed.

## Slop Detectors

Treat these as presumptive blockers in Reborn refactor PRs:

- **Rename without deletion.** A trait or facade gets a better name, but the
  giant method set and forwarding impl remain.
- **Second capability system.** "Capability descriptors" execute through a
  string ladder or direct product command path instead of the normal manifest,
  policy, idempotency, evidence, and runtime registration path.
- **Three dispatch styles.** Example smell: `invoke`, `query`, and
  `execute_command` all dispatch product operations.
- **Composition owns behavior.** Anything in `ironclaw_reborn_composition` that
  parses product semantics, enforces product policy, stages product results, or
  owns concrete channel/extension behavior should be moved behind an owning
  crate's factory or registry.
- **Thin pass-through trait.** One production impl, many test fakes, no forbidden
  dependency edge. Delete or make concrete.
- **Test doubles hiding behavior.** Prefer real stores over fake whole-trait
  implementations. Use fault-injecting decorators when testing failures.
- **Policy split across layers.** Authorization, approvals, resource gates,
  credential visibility, runtime policy, and evidence must have one canonical
  owner. Do not "pre-check" in host_runtime and re-check in capabilities.
- **Ratchet green, behavior not changed.** A ratchet proves names or inventory,
  not that old production wiring disappeared.
- **File-growth refactor.** If the refactor grows a busy file, especially WebUI
  handlers, `reborn_services.rs`, or composition runtime/factory, stop and
  extract an operation registry or owning module.
- **Non-hermetic env/test behavior.** Mask ambient `NEARAI_*`/provider env in
  tests unless the test explicitly exercises env resolution.

## Ownership Routing

Use this placement rule before writing code:

| Concern | Owner |
| --- | --- |
| Composition/build graph/readiness/workers | `ironclaw_reborn_composition` |
| Product views, commands, facade descriptors | `ironclaw_product_workflow` |
| WebUI routes/handlers/descriptors/auth middleware | `ironclaw_webui` |
| Product adapter protocol parse/render | adapter crate (`ironclaw_slack_extension`, etc.) |
| Generic extension host lifecycle/ingress/delivery assembly | `ironclaw_extension_host` |
| Concrete first-party tool behavior and GSuite visibility policy | `ironclaw_first_party_extensions` |
| Declarative extension metadata/manifests | `ironclaw_extensions` |
| Capability authorize/resume/spawn workflow | `ironclaw_capabilities` |
| Already-authorized lane routing | `ironclaw_dispatcher` / `ironclaw_host_runtime` |
| Credential records/flows/generic account ownership | `ironclaw_auth` |
| Product credential selection policy | product/extension owner injected at host boundary |
| Resource reservation and budget gates | `ironclaw_resources` + loop/runtime caller seam |
| Runtime profile resolution | `ironclaw_runtime_policy` |

Composition may call owner factories. It should not absorb their behavior.

## Review Lessons To Apply

These are dated examples from July 2026 PRs. Re-verify with `gh pr view` and live
code before citing them in a PR body.

- **ProductSurface (#6441/#6480/#6536/#6538):** naming the boundary is not enough.
  If `ProductSurface` remains a large `RebornServicesApi`-shaped facade, it is a
  transition label. Real simplification means product operations live in typed
  views/capabilities/commands with one dispatch model, or focused domain ports.
- **API-only capabilities (#6480):** descriptors that execute through a direct
  string ladder create a second capability system. Make them first-party
  manifests/handlers or call them product commands and keep them out of the
  capability descriptor path.
- **Outbound extraction (#6529):** moving files out of composition is only half
  the job. Runtime synthetic handlers and first-party registration stay stuck
  until a product-neutral registration interface exists.
- **Runtime unification (#6442):** deleting a local builder first exposes hidden
  services in the production-shaped path. Follow-up work must move those
  explicit services behind owning factories, or the big builder just gets bigger.
- **Sealed dispatch (#6438/#6450):** sealed authority must remain the sole
  witness to lane handoff. Validate reservations adjacent to handoff, preserve
  resource errors, and assert full reservation scope/estimate, not only IDs.
- **DTO retirement (#6447):** banning retired names is good, but live wrappers
  like `CapabilityDispatchRequest` or private lane request structs still need
  their own deletion plan.
- **Store cleanup (#6400/#6403/#6430):** real-store tests and fault decorators
  reveal behavior. Mock stores lie politely and then send you the bill.
- **Docs honesty (#6444/#6399):** do not write present-tense architecture claims
  unless code and tests enforce them. "Target" and "current" are different words
  because they do different jobs.

## ProductSurface / Composition Deletion Recipe

When asked to simplify channels/product/composition:

1. Count current surface methods and direct handler calls.

   ```bash
   rg -n "pub trait RebornServicesApi|pub trait ProductSurface|async fn .*\\(" crates/ironclaw_product_workflow/src/reborn_services.rs
   rg -n "state\\.services\\(\\)\\.[a-zA-Z_]+|execute_command|invoke\\(|query\\(" crates/ironclaw_webui/src/webui_v2 crates/ironclaw_product_workflow/src
   rg -n "ProductCapabilityDescriptor::api_only|match capability|as_str\\(\\)" crates/ironclaw_product_workflow/src crates/ironclaw_reborn_composition/src
   ```

2. Group operations by natural owner: lifecycle, run control, views, product
   commands, real model/tool capabilities.

3. Collapse to **one operation registry** where possible:

   ```text
   ProductOperation {
     id,
     input decoder,
     authorization requirement,
     executor,
     response/read-back mapper,
   }
   ```

4. Delete direct facade methods only after converted callers use the registry.
   Update `reborn_facade_method_freeze_ratchet.rs` in the same slice.

5. If a capability is model-visible and side-effecting, require evidence and
   read-back. If read-back is impossible, the outcome must be explicitly
   unverified.

## First-Party Extension Simplification Recipe

When asked to simplify `ironclaw_first_party_extensions`, remember its charter:
concrete userland behavior only. Host runtime, composition, authorization,
approval, resource accounting, lifecycle, and raw secret authority stay out.

Run:

```bash
wc -l crates/ironclaw_first_party_extensions/src/**/*.rs crates/ironclaw_first_party_extensions/src/*.rs | sort -nr | head -30
rg -n "pub trait|Arc<dyn|Box<dyn|DispatchRequest|DispatchResult|match .*operation|match .*capability|as_str\\(\\)|RuntimeHttpEgress|CredentialStager" crates/ironclaw_first_party_extensions/src
rg -n "arch-exempt: large_file|plan #[0-9]+" crates/ironclaw_first_party_extensions/src
rg -n "include_bytes!|include_str!|manifest.toml|schemas/|prompts/" crates/ironclaw_first_party_extensions/src/packages
```

Classify each finding:

- **Executor orchestration** is allowed here, but should be small: parse input,
  call scoped handles, map output, return evidence/usage.
- **Host mediation traits** are allowed only when they keep raw host authority
  out of this crate. Example: a credential stager trait can be justified if the
  concrete implementation lives in composition/host runtime and stages a
  one-shot secret without exposing bytes.
- **Provider operation switches** are debt when one file owns the whole family.
  Prefer one module per provider operation group when request construction,
  response decoding, retry/auth-expiry behavior, and tests have grown together.
- **Asset manifests** should be generated or table-driven when package modules
  manually repeat every schema/prompt path. Do not hand-roll a second inventory
  that can drift from manifests.
- **Large-file exemptions** are not approval. They are a debt marker. Re-check
  whether the stated plan is still valid and whether tests can move to a
  dedicated contract file.

Typical deletion slices:

1. Split a giant executor into `dispatch.rs`, `request_builders/`,
   `response_decode.rs`, and operation modules without changing public types.
2. Move tests out of the production file first if the file size hides the real
   production complexity.
3. Replace a hand-written package asset list with a manifest-derived or
   operation-table-driven list, then delete duplicated asset path literals.
4. If a request/result wrapper only mirrors host-runtime dispatch, keep it only
   when it is the narrow first-party boundary. Otherwise collapse it into the
   owner call.

## Composition Final-Shape Check

Before ending a composition refactor, run this mental diff:

```text
Did this PR make composition more like:
  build config -> assemble bundles -> start workers -> expose readiness

or more like:
  parse product intent -> enforce domain policy -> dispatch product commands
```

Only the first is acceptable long-term.

Useful searches:

```bash
find crates/ironclaw_reborn_composition/src -maxdepth 2 -type f | sort
rg -n "slack|telegram|gsuite|operator_config|llm_admin|product_auth|outbound_preferences|api_only|execute_command" crates/ironclaw_reborn_composition/src
rg -n "pub use" crates/ironclaw_reborn_composition/src/lib.rs
```

For each remaining product word in composition, write one of:

- `assembly only: calls owner factory/register function`
- `temporary bridge: deletes after <PR/ratchet>`
- `misplaced behavior: move it`

## Handoff Format

End with:

- **Deleted:** concrete old files/types/methods/branches removed.
- **Made load-bearing:** new path and production caller that consumes it.
- **Still transition:** old wrappers/branches/facade methods left alive.
- **Next deletion:** one follow-up slice that should remove more than it adds.
- **Validation:** exact tests/checks run.

If "Deleted" is empty, say plainly: this was migration, not simplification.
