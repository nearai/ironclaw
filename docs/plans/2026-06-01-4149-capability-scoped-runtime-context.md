# Issue #4149 Plan: Capability-Scoped Runtime Context in Prompt Bundles

**Date:** 2026-06-01
**Status:** implementation-ready plan outline
**Issue:** <https://github.com/nearai/ironclaw/issues/4149>
**Target branch:** `reborn-integration`
**Primary owner boundary:** Reborn loop prompt/userland over host-mediated capability surface

## Goal

Give Reborn model calls accurate runtime context about the environment they can
actually use, without leaking host internals or widening authority.

The model-visible runtime context must describe IronClaw as a
capability-scoped virtual OS, not just a process cwd plus git metadata. It must
be derived from the final model-visible capability surface for the current model
call, especially `LoopModelCapabilityView`, the filtered
`VisibleCapabilitySurface`, and the capability IDs that remain after loop
strategy and host policy have both narrowed the surface.

The final prompt bundle should include a distinct runtime-context section after
identity and before skills, safety, visible-surface summaries, and thread
context. It must preserve the static `default-system.md` identity source and
must not inject dynamic runtime facts into identity.

## Current Ground Truth

### Code paths

- `crates/ironclaw_agent_loop/src/executor/prompt.rs`
  - `PromptPlanningPipeline::run` asks the capability strategy for a filter.
  - It then calls `visible_surface`, which asks the host for visible
    capabilities and applies the strategy filter.
  - It builds `LoopModelCapabilityView` from the final filtered
    `surface.descriptors`.
  - It passes `surface.version` and `LoopModelCapabilityView` into prompt bundle
    construction before any model call.
- `crates/ironclaw_turns/src/run_profile/host.rs`
  - `LoopRunContext` already carries scope, turn/run IDs, resolved run profile,
    and optional resolved model route.
  - `LoopModelCapabilityView` currently contains final visible capability IDs.
  - `LoopContextBundle` has identity messages, transcript messages,
    instruction snippets, and memory snippets. It has no distinct runtime
    context field.
- `crates/ironclaw_turns/src/run_profile/prompt.rs`
  - `HostManagedLoopPromptPort` owns host-built prompt bundle materialization.
  - It already accepts/currently validates capability surface version and can
    carry the current `VisibleCapabilitySurface`.
  - This is the right place to derive the host-owned runtime posture record.
- `crates/ironclaw_turns/src/run_profile/instruction_bundle.rs`
  - `InstructionBundleRequest` currently contains `context_bundle`,
    `visible_surface`, `safety_context`, and `inline_messages`.
  - `InstructionBundleBuilder` is deterministic and fingerprints each prompt
    section.
  - The visible surface currently materializes as a system message that includes
    capability IDs, safe names, and safe descriptions.
- `crates/ironclaw_loop_support/src/capability_surface_filter.rs`
  - `CapabilitySurfaceVisibleFilter` enforces model-visible capability IDs at
    invocation time, including lazy `capability_info` targets.
- `crates/ironclaw_loop_support/src/subagent_prompt_port.rs`
  - Subagent prompt composition must receive a narrowed runtime context matching
    the child flavor surface, not the parent/full host surface.
- `crates/ironclaw_reborn/src/loop_driver_host.rs`
  - Reborn host composition should wire runtime-context posture derivation into the
    prompt port for real loop execution.
- `crates/ironclaw_reborn/src/model_gateway.rs`
  - Model gateway tests should prove the final model request sees the runtime
    stage through the caller path.

### Contract docs

- `docs/reborn/contracts/filesystem.md`
  - Runtime code sees `ScopedPath`.
  - Host policy reasons over `MountView`.
  - Backend code alone touches `HostPath`.
  - Host paths must not appear in user-visible errors.
- `docs/reborn/contracts/capabilities.md`
  - `CapabilityHost` is the caller-facing capability service.
  - Obligations and dispatch fail closed before side effects.
  - Errors must not expose secrets, raw output, provider errors, or raw host
    paths.
- `docs/reborn/contracts/runtime-profiles.md`
  - Runtime profiles select backend permissiveness inside deployment-mode
    ceilings.
  - Profiles do not bypass `CapabilityHost`.
  - Local host workspace/shell, hosted sandbox, network, secrets, and approvals
    are profile outcomes that can be disclosed safely only in sanitized form.
- `docs/reborn/2026-05-28-loop-profiles-planner-executor.md`
  - Prompt preparation already happens after final capability narrowing.
  - Subagent flavors have explicit capability allowlists:
    - `general`: read, list, search
    - `researcher`: read, list, search, HTTP
    - `explorer`: read, list, search, glob
    - `coder`: read, write, apply_patch, shell, list, search, glob

### Locked findings to preserve

- IronClaw environment is a capability-scoped virtual OS, not just cwd/git
  metadata.
- Runtime context must be derived from the final model-visible capability
  surface and `LoopModelCapabilityView`, not host registry alone.
- `HostManagedLoopPromptPort` should run host-owned runtime posture derivation
  with:
- `LoopRunContext`
- prompt mode
- `LoopModelCapabilityView`
- filtered `VisibleCapabilitySurface` or equivalent filtered capability IDs
- explicit run-kind metadata for child/subagent prompts, including flavor and
  parent-boundary/nesting posture
- safe runtime disclosures
- Prefer a distinct runtime stage/field in `InstructionBundleRequest` and
  `InstructionBundleBuilder` rather than reusing
  `LoopContextBundle.instruction_snippets`.
- Preserve static `default-system.md` and identity source. Dynamic runtime facts
  do not belong in identity.
- Runtime context describes usable affordances:
  - visible aliases
  - permissions
  - process placement
  - workspace/memory/network posture
  - subagent flavor narrowing
- Runtime context must avoid, by default:
  - raw host paths
  - environment variables
  - secrets
  - canonical tenant targets
  - hidden capabilities
  - full tool schemas

## Capability and OS Availability Cross-Check

This section is the pre-review evidence checkpoint for filesystem/OS claims. It
is intentionally phrased as "what the model can actually do" after final
capability filtering, not what IronClaw internals can do.

The key boundary is:

```text
Capability surface = what the model can call.
Runtime context    = how to interpret the environment those calls run in.
```

Runtime context must not repeat the full visible capability list. It should name
only compact operating assumptions that prevent wrong behavior: scoped path
semantics, process placement, network posture when it changes interpretation,
subagent role/boundary, date/timezone, and a reminder that callable actions come
from the capability surface.

### Evidence summary

| Claim | Evidence | Model-visible conclusion |
| --- | --- | --- |
| Directory listing is a Reborn first-party capability, not an `ls` tool. | `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs` declares `builtin.list_dir`; `crates/ironclaw_loop_support/src/capability_port/surface_snapshot.rs` derives provider tool names such as `builtin__list_dir`. | The capability surface names the list tool. Runtime context may add only the cross-capability trap: do not infer shell/`ls` from filesystem access. |
| `ls` can work only through the shell surface. | `crates/ironclaw_host_runtime/src/first_party_tools/shell.rs` dispatches `builtin.shell`; local and sandbox process ports run commands through `sh -c`. | Runtime context should mention process placement only when shell/process is visible. It should not enumerate shell commands. |
| File read/write/patch/search are scoped capability calls. | `crates/ironclaw_first_party_extensions/src/coding/paths.rs` resolves user paths through `ScopedPath` plus `MountView` into `VirtualPath`; `read`, `write`, `list`, and `execute` permissions are separate. | Runtime context says paths are scoped aliases, not raw host paths. The capability surface says which file operations are callable. |
| `/workspace` is an alias, not a global host root. | `docs/reborn/contracts/filesystem.md` defines scoped aliases; `crates/ironclaw_reborn_composition/src/local_dev_mounts.rs` currently grants `/workspace -> /projects/workspace`. | Runtime context may name the primary scoped alias when granted. Do not expose the backend target or physical path. |
| `/project` is contract language, not proven local-dev default. | `docs/reborn/contracts/filesystem.md` describes `/project`; the local-dev mount implementation found in this pass grants `/workspace` but not `/project`. | Do not mention `/project` unless it is actually granted and useful as an operating assumption. |
| `/memory` is virtual file-like storage, not the OS filesystem. | `docs/reborn/contracts/filesystem.md` defines `/memory` as a scoped virtual root; `crates/ironclaw_host_runtime/src/memory_context.rs` handles memory prompt snippets separately. | Mention memory only when its path semantics affect how the model should reason. Do not list memory tools here. |
| Shell/process is explicit and policy-routed. | `builtin.shell` has `PermissionMode::Ask`, bounded timeout/output, process/code/fs/network effects, and dispatches through a resolved `RuntimeProcessPort`. Runtime policy routes local dev to local host and hosted multi-tenant to tenant sandbox. | If shell/process is visible, runtime context may include placement such as `local_host` or `tenant_sandbox` plus approval/policy posture. Avoid host binary paths and raw cwd. |
| The shell schema does not expose model-controlled env. | `crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs` exposes `command`, optional `workdir`, and optional `timeout`. Local process env is scrubbed by default; sandbox env is internally constructed. | Do not tell the model it can set arbitrary environment variables through `builtin.shell`. Do not emit env vars in runtime context. |
| Git clone/worktree is not a first-class Reborn context affordance. | No dedicated git capability or `LoopRunContext` git fields were found. Shell validation does not specially block git commands, but execution depends on shell visibility, approval/hooks, writable mounts, network, binary availability, and repo metadata. | Do not mention clone/worktree by default. Mention git posture only if a future git context source proves it is directly relevant. |
| HTTP is not shell network. | `builtin.http` / `builtin.http.save` use runtime HTTP egress; network policy is scoped and fail-closed. | Runtime context may mention brokered/allowlisted network posture only when it changes interpretation. The HTTP tool inventory stays in capability surfacing. |
| Date/timezone is not evidence of filesystem/OS authority. | `builtin.time` can return UTC and optional timezone-local values; no Reborn prompt block currently injects complete date/cwd/model runtime context. | Date/timezone can be rendered from a safe source, but it should be categorized separately from filesystem/process affordances. |
| Model/backend route is already in run context. | `LoopRunContext.resolved_model_route` is persisted before driver side effects; routed gateway fails without the expected snapshot. | Render model/backend identity only through policy. It is useful context, but not an authority source. |

### Subagent runtime needs

Built-in subagent flavor allowlists are static in
`crates/ironclaw_reborn/src/subagent/flavors.rs` and are enforced through
`SubagentPromptComposer`, `CapabilitySurfaceProfileFilter`, model tool
definition filtering, and invocation-time denial.

Runtime context for subagents should not restate each flavor's full allowlist.
It should state the child role and parent boundary, then rely on the visible
capability surface for exact callable tools.

| Flavor | Runtime context should mention | Keep in capability surface |
| --- | --- | --- |
| `general` | Child run is a general helper narrowed from the parent. | Exact read/list/search tools. |
| `researcher` | Child run is for research; network posture only if brokered/allowlisted semantics matter. | Exact read/list/search/HTTP tools. |
| `explorer` | Child run is for codebase exploration; path model if file capabilities are visible. | Exact read/list/search/glob tools. |
| `coder` | Child run is for scoped code changes; path model and process placement if shell is visible. | Exact read/write/patch/shell/list/search/glob tools. |

No built-in subagent flavor includes `builtin.spawn_subagent`; all built-in
flavors disallow nesting. Do not mention nested spawn in normal runtime context;
test that it cannot leak when `allow_nesting=false`.

### Prompt boundary summary

```text
Host/runtime facts
  -> policy and mounts decide what is safe
  -> final visible capability surface narrows what the model can call
  -> runtime context explains operating assumptions only
  -> capability surface lists callable actions
  -> CapabilityHost / filters still enforce every invocation
```

The design should therefore avoid ambient wording such as "you are in a repo
and can run `ls`" or "you can create worktrees". It should also avoid large
unavailable inventories. The accurate phrasing is: "paths use scoped aliases";
"do not infer shell from filesystem tools"; and "use the visible capability
surface as the source of truth for callable actions."

## Options and Tradeoffs

### Option A: Append runtime facts to identity

Rejected.

This would make a per-turn dynamic section look like static identity. It also
risks rewriting or shadowing `default-system.md`, which should remain an
editable identity source and not a runtime telemetry carrier.

### Option B: Reuse `LoopContextBundle.instruction_snippets`

Rejected for this phase.

Pros:

- Smaller implementation.
- Reuses existing snippet materialization.

Cons:

- Runtime context becomes indistinguishable from skills, project instructions,
  and ordinary instruction snippets.
- Ordering and fingerprinting are less explicit.
- Security review has to infer that `instruction_snippets` sometimes contains
  host runtime disclosures.

If the typed runtime-context stage is blocked by API churn, stop and replan
rather than silently falling back to snippets. Any alternative must still
preserve a distinct runtime stage, deterministic ordering after identity,
section fingerprinting, sanitizer/redaction tests, and caller-level evidence
that the final model request receives the stage.

### Option C: Add a distinct runtime stage to prompt bundle construction

Recommended.

Pros:

- Preserves identity as static identity.
- Makes runtime context a first-class host-built system section.
- Allows deterministic ordering after identity and before skills/safety/surface.
- Supports explicit redaction, section fingerprinting, tests, and future
  runtime-context policy changes.
- Keeps model-visible disclosures tied to the exact filtered surface used for
  that model call.

Cons:

- Requires a small public API update in `ironclaw_turns`.
- Requires updating host-managed prompt port tests and Reborn host wiring.

## Recommended Architecture

### Initial slice defaults

Lock these defaults for the first implementation slice so issue #4149 cannot be
implemented as a narrower prompt-cleanup refactor that omits the ambient context
that prompted the issue:

- Runtime context is an operating-assumptions primer, not a capability
  inventory. It should usually be 3-6 bullets.
- Render scoped path semantics and the primary granted workspace alias when file
  or shell capabilities make path interpretation relevant. Prefer `/workspace`
  when granted; do not advertise `/project` as a default local-dev alias until
  the runtime evidence shows it is mounted.
- Render current date and timezone when a safe source exists. Do not render the
  current clock time by default.
- Render safe model/backend identity from `LoopRunContext.resolved_model_route`
  only through a deployment/runtime policy allowlist of canonical route labels.
  Raw route strings, endpoints, deployment IDs, provider account IDs, and
  free-form model labels must never be rendered; policy-hidden or unknown
  values become `policy_hidden`.
- Render platform/shell/process posture only when process or shell-like
  capabilities are visible. Phrase this as placement and policy
  (`local_host`, `tenant_sandbox`, `unavailable`) rather than host internals.
- Render network posture only when brokered/allowlisted/direct semantics change
  how the model should reason. Do not repeat HTTP tool availability here.
- Keep git branch/status, clone, and worktree out of the first default section
  unless an explicit capability-backed or policy-gated `Git Context` source is
  added later.
- Always end with a short source-of-truth reminder: callable actions come from
  the visible capability/tool surface.
- Use one concise runtime system message unless provider ordering constraints
  force splitting it later.

### New concepts

Add a first-class runtime-context prompt stage with the smallest useful typed
surface:

- `LoopRuntimeContext`
- a host-owned posture derivation function or helper, kept module-local for the
  first slice

Use one runtime-context value object, not a stack of separate request, bundle,
disclosure, and source payload types. Model absence, redaction, and empty
surface as one explicit typed state, not as both `Option<LoopRuntimeContext>`
and a separate default object. A request/input struct is fine if Rust ergonomics
require it, but it should remain private or module-local. Do not add an
injectable public runtime-context source trait in the first slice; defer that
until there is a second concrete runtime-context producer.

```text
PromptStage
  -> final filtered VisibleCapabilitySurface
  -> LoopModelCapabilityView
  -> HostManagedLoopPromptPort
  -> module-local posture derivation helper
  -> compact LoopRuntimeContext posture record
  -> InstructionBundleRequest.runtime_context: LoopRuntimeContext
  -> InstructionBundleBuilder renders and fingerprints runtime section
  -> LoopModelRequest messages
```

### Extensibility pattern

Use one host-owned posture derivation step plus one builder renderer for runtime
visibility:

```text
derive_loop_runtime_context(input)
  1. validate final visible surface and run scope
  2. derive a compact typed LoopRuntimeContext posture record
  3. sanitize all labels while deriving the record

InstructionBundleBuilder
  4. materialize and fingerprint one deterministic Runtime Context section
```

`LoopRuntimeContext` should be a small typed record, not a generic extension
framework. It can contain optional posture fields such as:

- path model and primary scoped alias
- process placement
- network posture
- subagent role and parent-boundary posture
- date/timezone
- policy-approved model-route label

The derivation step may use ordinary private helper functions internally, but
they are not a framework, trait family, registration surface, or independently
rendered pipeline. They should simply keep the derivation code readable while
authority checks and sanitization remain in the host-owned prompt-port path, and
final rendering/fingerprinting remain solely in `InstructionBundleBuilder`.

If deriving a non-authority posture field fails, omit that field and record a
sanitized diagnostic for host logs/metrics if available. If final visible
surface validation or run-scope validation fails, runtime posture derivation
fails closed before prompt construction.

Because the first slice uses a fixed typed record, duplicate/conflicting dynamic
facts should not be representable across arbitrary emitters. If two source
fields would conflict while deriving the record, choose the most restrictive
safe posture or omit the field. The rendered section must be deterministic for
the same input.

This pattern keeps future visibility flexible without adding a second capability
surface or a recursive extension hierarchy. Adding a new runtime fact in the
first slice should usually mean adding one typed optional field and one posture
test, not creating a new abstraction. Revisit extension hooks only after a
second real runtime-context consumer proves the typed record is too small.

Avoid these shapes for the first slice:

- No per-loop-type template hierarchy. Loop/run type should affect fact
  predicates, not create separate prompt templates.
- No chain-of-responsibility or multi-emitter pipeline for the first slice.
- No plugin-style public API for runtime-fact providers in the first slice.

### Runtime context source inputs

The source should receive:

- `LoopRunContext`
- `PromptMode`
- `LoopModelCapabilityView`
- validated filtered `VisibleCapabilitySurface`
- filtered visible capability IDs
- `LoopRuntimeRunKind` or equivalent explicit metadata for root versus
  subagent prompts, including child flavor, parent-boundary label, and
  `allow_nesting` posture
- safe host/runtime disclosures already computed by the Reborn host

It must not derive authority from the host registry alone. Host registry data may
help explain descriptors, but only after intersecting with the final
model-visible view.

For every normal model-call prompt mode, a current validated
`VisibleCapabilitySurface` is required. If the host cannot supply or validate
the current surface, prompt construction must fail closed rather than generating
runtime context from host metadata alone. Legacy or unsupported prompt modes may
omit runtime context only through an explicitly tested branch.

Runtime posture derivation must also treat every runtime label as untrusted
unless it is an enum or constant generated by IronClaw. Mount aliases, repo
names, branch names, provider labels, descriptor text, and product/channel
labels must be normalized before they enter `LoopRuntimeContext`, then rendered
only by `InstructionBundleBuilder`.

### Runtime context output

Posture derivation should return a single `LoopRuntimeContext` value that
`InstructionBundleBuilder` can deterministically materialize and fingerprint.
Its state should be explicit, for example:

- `Present`: one or more safe runtime facts are rendered.
- `Redacted`: the visible surface is non-empty, but no additional runtime facts
  are disclosed by policy.
- `EmptySurface`: no runtime affordances are visible because the final
  capability/tool surface is empty.
- `Absent`: a tested unsupported/legacy prompt mode cannot legally carry the
  runtime section.

For normal model-call prompt modes, the runtime section should always be present.
If the final surface is empty, derive an explicit empty posture that does not
invent workspace, process, memory, or network affordances. If the surface is
non-empty but all runtime disclosures are hidden or posture derivation has no
positive facts, derive a neutral redaction posture such as "No additional runtime
facts are disclosed beyond the visible capability/tool surface." Do not claim
that no affordances exist unless the final visible surface is actually empty.
Omit the runtime section only for `LoopRuntimeContext::Absent` or the equivalent
typed state, and test that exception directly.

Recommended model-visible shape:

```text
## Runtime Context

You are operating inside IronClaw's capability-scoped runtime environment.

- Paths use scoped aliases, not raw host paths. <primary_workspace_alias> is the
  primary workspace alias for this run.
- Process placement: <process_placement>; shell commands, when visible, run
  there under approval and policy limits.
- Subagent mode: <subagent_flavor>; this child run is narrowed from the parent
  run.
- Date/timezone: <current_date>, <timezone>.

Use the visible capability/tool surface as the source of truth for callable
actions.
```

The renderer uses one fact-driven template and omits bullets whose posture facts
are absent. Do not list unavailable tools or repeat all visible tool names.
Mention an unavailable posture only for the empty-surface case or when it
prevents a common unsafe inference.

### Runtime text sanitization

Before materializing the runtime context as a system message:

- Render enum-like postures from fixed strings only, for example
  `unavailable`, `local_host`, `tenant_sandbox`, `brokered`, and `allowlisted`.
- Render capability IDs only after intersecting with the final visible surface
  and checking them against the canonical affordance table.
- Render aliases only after validating that they are scoped absolute aliases
  from the current `MountView`; reject or replace control characters,
  backticks, markdown fences, newlines, shell metacharacter-heavy strings, URLs,
  Windows paths, and `..` segments.
- Render provider/model labels through a bounded identifier formatter and a
  small policy allowlist of canonical route labels. Unknown, deployment-specific,
  endpoint-like, account-like, or free-form labels must become a neutral
  placeholder such as `policy_hidden`, not raw text.
- Normalize labels before validation, for example NFC normalization for Unicode
  text.
- Strip or escape non-printing characters, bidi controls, zero-width
  characters, ANSI escape sequences, and other invisible prompt-control text.
- Enforce hard per-field and total runtime-section length caps before rendering
  labels into the system message.
- Never copy capability descriptions, product labels, repo names, branch names,
  mount labels, provider errors, or user-controlled metadata directly into the
  runtime system message.

Do not expose:

- raw host filesystem paths
- physical cwd unless it is intentionally exposed as a scoped alias or local
  single-user policy explicitly allows it
- environment variables
- secret names or values
- bearer tokens or API-key source
- canonical tenant/project/user storage paths
- hidden or denied capabilities
- complete parameter schemas
- full git status by default
- unescaped or free-form runtime labels

## Visualized Prompt Shape

### Plain-English intro

The new runtime context is a short orientation note. It tells the model how to
interpret the current IronClaw environment, while the visible capability surface
continues to tell the model exactly which tools it can call.

The design intentionally avoids long "unavailable" inventories. If something is
not visible, the runtime context usually stays silent. The only exceptions are a
compact empty-surface posture and short warnings for common traps, such as
"filesystem access does not imply shell access."

### Previous vs new behavior

Before: prompt construction could provide identity, skills, memory, safety, the
visible capability surface, and transcript context, but it had no dedicated
place for safe runtime assumptions such as scoped path aliases, process
placement, subagent role, or date/timezone.

After: prompt construction includes one concise `Runtime Context` system section
after identity and before skills/safety/surface. It explains operating
assumptions and points the model back to the capability surface for callable
actions.

### Diagram

```text
  BEFORE:                         AFTER:

  Identity                        Identity
     |                               |
     v                               v
  Skills / instructions           Runtime Context
     |                               |
     v                               v
  Safety                          Skills / instructions
     |                               |
     v                               v
  Capability Surface              Safety
     |                               |
     v                               v
  Transcript                      Capability Surface
                                     |
                                     v
                                  Transcript
```

The ownership flow is:

```text
  Validated filtered VisibleCapabilitySurface
    + LoopModelCapabilityView.visible_capability_ids
    + LoopRunContext
    + PromptMode
    + explicit root/subagent run-kind metadata
    + safe host/runtime disclosures
               |
               v
       module-local runtime posture derivation
               |
               v
        LoopRuntimeContext
               |
               v
  InstructionBundleBuilder runtime section
               |
               v
       final model messages
```

### Runtime fact selection guide

This table is a fact-selection guide for the single renderer, not a separate
template matrix. The implementation should emit optional posture facts when the
final surface and run-kind metadata warrant them.

| Situation | Runtime facts that may be selected | Keep out of runtime context |
| --- | --- | --- |
| Root/root-coder loop | Path model, primary workspace alias, process placement if shell/process is visible, date/timezone, capability-surface reminder. | Full tool list, unavailable tools, full git status. |
| Tool-less or minimal loop | Date/timezone if useful, plus one sentence that no additional runtime facts are disclosed beyond the visible surface. | Long unavailable inventory. |
| File-only loop | Scoped path model and primary alias. Optional warning that directory inspection uses the visible file-listing capability, not shell. | Shell placement, `ls`, process details. |
| Shell-capable loop | Scoped path model, default workdir alias if known, process placement, approval/sandbox posture. | Exact shell schema, command examples, arbitrary env claims. |
| HTTP/research loop | Date/timezone, network posture only if brokered/allowlisted/direct semantics matter. | Shell network implications, clone/worktree. |
| Subagent loop | Subagent role/flavor, parent-boundary, path model if file capabilities are visible, process placement if shell is visible. | Full child allowlist, parent capabilities, nested spawn inventory. |
| Empty final surface | One short statement that no runtime affordances are visible in the current capability/tool surface. | Enumerating every absent capability. |
| Non-empty surface with fully redacted runtime facts | One short statement that no additional runtime facts are disclosed beyond the visible capability/tool surface. | Claiming that no affordances exist. |

### Example runtime sections

The examples below use placeholders such as `<primary_workspace_alias>` and
`<process_placement>` intentionally. Concrete values are rendered from
`LoopRuntimeContext` at runtime; do not copy these snippets as static prompt
text.

These examples demonstrate outputs of the same fact-driven renderer with
different selected facts. They are not separate templates.

The rendered runtime section must append one generic source-of-truth reminder:
"Use the visible capability/tool surface as the source of truth for callable
actions." The examples below elide that required closing line unless they are
specifically demonstrating the empty-surface branch; they keep to environment
facts only so they do not become a second capability surface.

Root coder with scoped workspace and shell:

```text
## Runtime Context

You are operating inside IronClaw's capability-scoped runtime.

- Paths use scoped aliases, not raw host paths. <primary_workspace_alias> is the
  primary workspace alias for this run.
- Process placement: <process_placement>; shell commands, when visible, run
  there under approval and policy limits.
- Date/timezone: <current_date>, <timezone>.
```

File-only explorer:

```text
## Runtime Context

You are operating inside IronClaw's capability-scoped runtime.

- Paths use scoped aliases, not raw host paths. <primary_workspace_alias> is the
  primary workspace alias for this run.
- Filesystem access does not imply shell access; use the visible capability
  surface for directory inspection and other callable actions.
- Date/timezone: <current_date>, <timezone>.
```

Research subagent:

```text
## Runtime Context

You are operating inside IronClaw's capability-scoped runtime.

- Subagent mode: <subagent_flavor>; this child run is narrowed from the parent
  run.
- Network posture: <network_posture>, when network semantics affect this run.
- Date/timezone: <current_date>, <timezone>.
```

Non-empty surface with fully redacted runtime facts:

```text
## Runtime Context

You are operating inside IronClaw's capability-scoped runtime.

No additional runtime facts are disclosed beyond the visible capability/tool
surface for this model call.

Use the visible capability/tool surface as the source of truth for callable
actions.
```

### Review notes

- Runtime context should be boring and short. If it starts looking like a tool
  list, the content belongs in capability surfacing instead.
- Runtime context may mention an unavailable posture only when silence would
  cause a common wrong inference, or when the entire visible surface is empty.
- Dynamic labels must pass through the sanitizer before entering the system
  message because this section has high prompt trust.

### Ordering

`InstructionBundleBuilder` should materialize sections in this order:

1. inline messages, if the current builder semantics require them first
2. identity messages from `default-system.md` and other trusted identity sources
3. runtime context
4. non-runtime instruction snippets, including skills
5. memory snippets
6. safety context
7. visible capability surface summary
8. thread/transcript messages

If the existing builder has a different established position for inline or
thread messages, preserve the existing semantics except for the new invariant:
runtime context appears after identity and before skills, safety,
visible-surface summaries, and thread context.

### Surface derivation

Runtime posture derivation must use the same final surface as the prompt/model
call:

- `PromptPlanningPipeline::visible_surface` returns the host surface after
  strategy filter narrowing.
- `LoopModelCapabilityView.visible_capability_ids` is derived from that final
  surface.
- Runtime context generation must consume those IDs or descriptors directly.
- `CapabilitySurfaceVisibleFilter` remains the invocation-time enforcement
  guard; runtime context is explanatory only and must not become an authority
  grant.

### Filesystem and OS wording rules

- Say scoped alias and path model rather than raw cwd when filesystem tools are
  visible but shell is not.
- Mention `ls` only to prevent a bad inference: filesystem access does not imply
  shell access. Do not list `ls` as a general runtime affordance.
- Say process placement only when shell/process is visible.
- Say network posture only when the runtime's broker/allowlist/direct placement
  matters. Do not repeat HTTP tool availability.
- Keep git clone/worktree out of runtime context unless a later git context
  source proves it is directly relevant.
- Say memory is virtual/file-like when path semantics matter; do not describe it
  as an OS directory unless the filesystem capability surface exposes the alias.

### Subagent narrowing

Subagent runtime context must reflect the child flavor:

- `general`: disclose only child role/boundary plus path model when relevant.
- `researcher`: disclose child role/boundary and network posture only when
  brokered/allowlisted semantics matter.
- `explorer`: disclose child role/boundary plus path model when relevant.
- `coder`: disclose child role/boundary plus path model and process placement if
  shell/process is visible.

Parent/full host capabilities must not bleed into child runtime context. Nested
subagent availability remains undisclosed when the flavor disallows nesting.

## PR Sequence

Keep each PR small enough for focused review and easy rollback.

### PR size gate

Each implementation PR in this sequence must stay under 1000 lines of
non-generated code changes, counting additions plus deletions across production
and test code. Documentation updates can accompany the PR that changes behavior,
but they do not justify a larger code diff.

If a slice is likely to exceed the cap, split before implementation rather than
opening a large PR. Prefer these split points:

- type/API shape before runtime posture derivation wiring
- source wiring before Reborn disclosure implementation
- typed runtime posture record before optional/future fields
- root-loop behavior before subagent behavior
- implementation before snapshot/docs-only evidence

Suggested pre-PR check:

```bash
git diff --numstat origin/reborn-integration...HEAD
```

The reviewer should be able to confirm that non-generated code additions plus
deletions are below 1000 lines for each phase PR.

### PR 1: Prompt contract and builder stage

**Goal:** Add a typed runtime-context stage to prompt bundle construction.

**Files:**

- `crates/ironclaw_turns/src/run_profile/host.rs`
- `crates/ironclaw_turns/src/run_profile/instruction_bundle.rs`
- `crates/ironclaw_turns/src/run_profile/prompt.rs`

**Tasks:**

- Add the single public `LoopRuntimeContext` value type in `host.rs` or the
  nearest run-profile module.
- Add `runtime_context: LoopRuntimeContext` to `InstructionBundleRequest`. Keep
  the request field singular; runtime context is one prompt section with an
  explicit state such as `Present`, `Redacted`, `EmptySurface`, or `Absent`.
- Add deterministic runtime-section materialization in
  `InstructionBundleBuilder`.
- Add explicit runtime-section fingerprint inputs.
- Order runtime after identity and before skills/safety/surface/thread context.
- Preserve `LoopContextBundle.instruction_snippets` behavior for ordinary
  instructions and skills.
- Builder-level omission is allowed only when `runtime_context` is explicitly in
  the typed `Absent` state. Host-managed model-call construction normally
  supplies `Present`, `Redacted`, or `EmptySurface`.
- If the typed API and builder changes approach the 1000-line code cap, split
  API/type additions from builder materialization tests.

**Tests:**

- Builder unit test proves section order: identity, runtime, skill/instruction,
  memory, safety, surface, transcript.
- Update existing order-sensitive tests, including
  `instruction_bundle_builder_orders_sections_and_rebuilds_deterministically`,
  so the expected order is identity, runtime, skill/instruction, memory,
  safety, surface, transcript.
- Builder unit test proves runtime section changes alter fingerprint
  deterministically.
- Builder unit test proves a supplied `EmptySurface` `LoopRuntimeContext`
  materializes a deterministic runtime section without inventing workspace,
  process, memory, or network affordances.
- Builder unit test proves the builder omits the runtime section only for the
  explicit `Absent` state, preserving unsupported/legacy prompt paths without a
  parallel nullable field.

**Verification command:**

```bash
cargo test -p ironclaw_turns instruction_bundle
```

### PR 2: Host-managed runtime posture derivation

**Goal:** Let `HostManagedLoopPromptPort` derive a host-owned runtime posture
record using the final model-visible capability view.

**Files:**

- `crates/ironclaw_turns/src/run_profile/prompt.rs`
- `crates/ironclaw_turns/src/run_profile/host.rs`
- `crates/ironclaw_loop_support/src/lib.rs`
- `crates/ironclaw_loop_support/src/capability_port.rs`
- `crates/ironclaw_loop_support/src/capability_surface_filter.rs`

**Tasks:**

- Add module-local runtime posture derivation owned by the host-managed prompt
  path. Do not add an injectable public source trait in this slice.
- Extend `HostManagedLoopPromptPort::build_prompt_bundle` flow so it passes
  `LoopRunContext`, prompt mode, `LoopModelCapabilityView`, and filtered
  `VisibleCapabilitySurface` to posture derivation.
- Ensure derivation receives the actual current `PromptMode`, not a default or
  reconstructed value.
- Keep derivation best-effort only for non-authority metadata. If derivation
  cannot validate against the current surface version, fail closed with
  `StaleSurface` or `InvalidInvocation` rather than generating context from a
  stale/broader surface.
- Ensure derivation only includes posture data for capabilities in
  `LoopModelCapabilityView.visible_capability_ids`.
- If the final visible surface is empty, generate an explicit `EmptySurface`
  `LoopRuntimeContext` rather than inventing workspace, process, memory, or
  network affordances. If runtime facts are policy-redacted while the surface is
  non-empty, generate `Redacted`.
- Keep `CapabilitySurfaceVisibleFilter` as enforcement. Do not make runtime
  context a second permission system.
- If prompt-port wiring plus loop-support filter updates approach the 1000-line
  code cap, land the host-managed prompt-port posture derivation before any
  Reborn disclosure implementation.

**Tests:**

- Prompt port test proves posture derivation receives the final filtered capability
  IDs, not the unfiltered host registry.
- Prompt port test proves posture derivation receives the current `PromptMode` and
  the filtered `VisibleCapabilitySurface`, not default mode or unfiltered host
  registry state.
- Prompt port test proves normal model-call runtime posture derivation fails
  closed when a current validated `VisibleCapabilitySurface` is missing or does
  not match the requested surface version.
- Prompt port test proves stale surface version blocks runtime context and
  prompt construction consistently with existing surface validation.
- Prompt port test proves runtime context appears in the materialized model
  messages before skill snippets.
- Update existing prompt-port ordering tests, including
  `loop_prompt_port_keeps_identity_before_skill_snippets_and_records_skill_metadata`,
  so they prove the new invariant: identity before runtime before skill
  snippets, with identity still recorded independently from runtime context.
- `tests::run_profile::prompt::runtime_context_emits_empty_posture_when_no_facts_are_visible`
  proves empty visible surface emits an explicit `EmptySurface` runtime section
  with no invented affordances and that unsupported prompt modes omit the
  section only through the explicit `Absent` branch.

**Verification command:**

```bash
cargo test -p ironclaw_turns host_managed_prompt_port
cargo test -p ironclaw_loop_support capability_surface_filter
```

### PR 3: Reborn runtime disclosures

**Goal:** Implement safe, capability-scoped runtime disclosures for real Reborn
loop execution.

**Files:**

- `crates/ironclaw_reborn/src/loop_driver_host.rs`
- `crates/ironclaw_reborn/src/model_gateway.rs`
- `crates/ironclaw_loop_support/src/lib.rs`
- `docs/reborn/contracts/filesystem.md`
- `docs/reborn/contracts/runtime-profiles.md`
- `docs/reborn/contracts/capabilities.md`

**Tasks:**

- Add Reborn-owned runtime posture derivation.
- Derive a compact typed `LoopRuntimeContext` posture record from the final
  filtered surface and safe host disclosures. `InstructionBundleBuilder`
  remains the only code that materializes and fingerprints the runtime section.
- Render scoped path semantics and primary aliases from safe host disclosures
  after intersecting with the final visible surface.
- Add one canonical capability-to-affordance descriptor table/helper in the
  capability surface layer. Exact capability IDs, tool names, schemas, and
  callable affordances stay there. Runtime context may consume only coarse
  posture buckets derived from that table, such as `has_file_paths`,
  `has_process`, `has_brokered_network`, or `is_child_run`.
- Route all rendered runtime labels through the sanitization boundary. Unknown
  or user-controlled descriptor text must be dropped, normalized, or replaced
  with a fixed placeholder before it can enter the runtime system message.
- Render the file-only trap as posture-level text only when relevant:
  filesystem path access does not imply shell access.
- Render process placement as a safe posture such as local host, sandboxed,
  tenant-scoped sandbox, or dedicated runner only when process/shell semantics
  are visible. Do not render raw binary paths or host cwd.
- Render network posture only when brokered, allowlisted, or direct semantics
  change how the model should reason. Do not repeat HTTP tool availability.
- Render workspace/memory posture through actually granted scoped aliases only
  when path semantics matter, never backend `HostPath` or canonical tenant
  target paths by default.
- Keep git/worktree posture out of the runtime context by default. A later
  `Git Context` feature can add it only when it has git binary availability,
  repo/worktree metadata, writable workspace, network posture, and policy
  evidence.
- Render safe model/backend identity from `LoopRunContext.resolved_model_route`
  only as a policy-approved canonical label. Raw route strings, endpoints,
  deployment IDs, provider account IDs, and free-form provider/model labels must
  render as `policy_hidden` or be omitted.
- Render date/timezone only if the safe source is available. Do not render
  current clock time by default.
- Update contract docs to define runtime context as model-visible explanation of
  usable affordances, not an authority source.
- Keep product adapters out of runtime-context posture derivation. Runtime
  disclosures must come from `LoopRunContext`, the final parent/child
  capability surface, and host-owned disclosure sources only. Product adapters
  consume the host-built prompt/model path and must not inspect raw OS/runtime
  state or bypass `CapabilityHost` / `HostRuntime`.

**Tests:**

- Reborn host/model-gateway caller-level test proves the final model request
  includes runtime context.
- Redaction test proves raw host paths, env vars, secrets, and hidden capability
  IDs are absent.
- `tests::run_profile::prompt::runtime_context_redacts_untrusted_aliases_and_provider_labels`
  proves aliases, descriptor text, repo names, branch names, raw host paths, and
  provider/model labels containing prompt-control text, markdown fences,
  newlines, URLs, `..`, shell-like metacharacters, bidi/zero-width controls,
  ANSI escapes, and over-length values are dropped, escaped, capped, or replaced
  before message materialization.
- `tests::run_profile::prompt::runtime_context_renders_policy_permitted_model_route`
  proves policy-approved route labels render only through canonical allowlisted
  values, while raw route strings, endpoints, deployment IDs, provider account
  IDs, and free-form model labels render as `policy_hidden` or are omitted.
- Renderer tests prove all rendered facts pass through the shared sanitizer and
  cannot bypass final surface filtering.
- Renderer tests prove posture-field derivation failures are sanitized and
  isolated, conflicting source fields choose the most restrictive safe posture
  or omit the field, and final output stays deterministic.
- Surface test proves a denied capability in the host registry is not disclosed.
- Surface test proves hidden or denied capability classes do not affect coarse
  posture buckets.
- Empty-surface test proves no workspace/process/memory/network affordance is
  invented when zero capability IDs survive filtering.
- Runtime wording tests prove posture-level behavior: scoped alias text appears
  when file path semantics are relevant, shell is not inferred from file access,
  process placement appears only when process/shell semantics are visible, and
  network posture appears only when broker/allowlist/direct semantics matter.
- Git/worktree tests prove runtime context stays silent about clone/worktree by
  default. Future git-context tests must prove all git prerequisites before
  adding git posture.
- Descriptor-table tests prove exact capability IDs and callable affordances
  remain in the capability surface layer; runtime context consumes only coarse
  posture buckets derived from that table.
- Product-adapter boundary test or architecture guardrail proves adapters do not
  construct or mutate runtime context directly.
- Local/sandbox posture tests cover at least one local profile and one sandboxed
  profile if current fixtures can construct both.
- If the renderer, sanitizer, and first posture fields approach the 1000-line
  code cap, split sanitizer/renderer infrastructure from the first Reborn
  posture facts.

**Verification command:**

```bash
cargo test -p ironclaw_reborn runtime_context
cargo test -p ironclaw_reborn model_gateway
```

### PR 4: Subagent flavor-specific narrowing

**Goal:** Make subagent runtime context match the child flavor surface.

**Files:**

- `crates/ironclaw_loop_support/src/subagent_prompt_port.rs`
- `crates/ironclaw_loop_support/src/lib.rs`
- `crates/ironclaw_reborn/src/loop_driver_host.rs`
- `docs/reborn/2026-05-28-loop-profiles-planner-executor.md`

**Tasks:**

- Pass explicit child flavor, parent-boundary, and `allow_nesting` posture into
  runtime-context generation through the child prompt path, alongside the child
  run's final filtered surface.
- Do not infer child flavor or nesting posture from the filtered surface alone;
  runtime posture derivation must receive a typed root/subagent run-kind input.
- Ensure runtime context for each built-in flavor states only child role,
  parent-boundary, and posture hints such as path model or process placement
  when relevant.
- Prevent parent capabilities from leaking into the child runtime section.
- Keep exact child tool names, capability IDs, allowlists, and nested-spawn
  eligibility in the capability surface/subagent capability tests, not the
  runtime primer.
- Keep subagent result delivery and child-run mechanics unchanged.

**Tests:**

- `general` subagent context states child role/boundary without listing the
  flavor allowlist.
- `researcher` subagent context states child role/boundary and network posture
  only when brokered/allowlisted semantics matter; it does not list HTTP tools.
- `explorer` subagent context states child role/boundary and path model when
  file path semantics are relevant; it does not list glob/search tools.
- `coder` subagent context states child role/boundary plus path model and
  process placement when shell/process semantics are visible; it does not list
  write/apply_patch/shell as an allowlist.
- Caller-level subagent prompt test proves narrowing through the actual prompt
  port path, not a helper-only unit test.
- Caller-level subagent prompt test proves child flavor and parent-boundary
  metadata are passed explicitly into runtime posture derivation.
- Subagent capability-surface tests, not runtime-context tests, prove
  `builtin.spawn_subagent` and equivalent nested-run tools are absent when
  `allow_nesting=false`.
- If subagent prompt-port changes and Reborn host wiring approach the 1000-line
  code cap, land child-surface/runtime-context narrowing before adding
  snapshot/doc evidence.

**Verification command:**

```bash
cargo test -p ironclaw_loop_support subagent
cargo test -p ironclaw_reborn subagent
```

### PR 5: Integration evidence and compatibility docs

**Goal:** Prove the runtime stage is stable in real prompt bundles and documented
as part of Reborn prompt behavior.

**Files:**

- `docs/reborn/2026-05-28-loop-profiles-planner-executor.md`
- `docs/reborn/contracts/runtime-profiles.md`
- `docs/reborn/contracts/filesystem.md`
- `docs/reborn/contracts/capabilities.md`
- `FEATURE_PARITY.md` if prompt/runtime-context status is tracked there
- relevant replay or fixture files if the model request snapshot harness exists

**Tasks:**

- Update Reborn loop docs to show the runtime-context stage in prompt
  preparation.
- Add a bounded prompt snapshot or model-request fixture that verifies section
  order and redaction behavior.
- Check `FEATURE_PARITY.md` and update only if this work changes a tracked
  Reborn prompt/profile capability status.
- Run architecture boundary checks to ensure no prompt/userland code bypasses
  capability host/runtime ownership.
- Add or update a boundary check documenting that product adapters are not a
  runtime-context derivation. Adapter metadata can appear only as bounded
  product/channel metadata already carried into the host-owned loop context.

**Tests:**

- Snapshot or fixture test proves identity remains static and runtime context is
  separate.
- Architecture boundary test passes unchanged or is intentionally updated only
  for new allowed dependencies.
- Replay/model gateway evidence confirms runtime context reaches the final model
  request.

**Verification command:**

```bash
cargo test -p ironclaw_architecture reborn_dependency_boundaries
cargo test -p ironclaw_reborn runtime_context_model_request_snapshot
cargo test -p ironclaw_reborn model_gateway_runtime_context
scripts/check-boundaries.sh
```

## Test Plan

### Unit tests

- `InstructionBundleBuilder` section ordering.
- Runtime-section fingerprint determinism.
- Runtime-section redaction helper rejects or strips raw host paths, env-var
  shaped data, secrets, and hidden capability IDs.
- Runtime-label sanitizer covers malicious/free-form aliases, descriptor text,
  repo names, branch names, provider/model labels, markdown fences, newlines,
  URLs, Windows paths, `..`, and shell-like metacharacters.
- Model-route sanitizer covers both the positive policy case and negative raw
  route case: allowed canonical labels may render, while endpoints, deployment
  IDs, provider account IDs, and free-form route strings become
  `policy_hidden` or are omitted.
- Runtime renderer tests cover posture derivation, final-surface filtering,
  sanitization, deterministic rendering, and the rule that no raw posture source
  can emit directly into prompt markdown.
- Centralized capability-surface mapping covers exact capability IDs, callable
  affordances, and coarse posture buckets. Runtime context tests consume only
  the coarse buckets.
- Runtime wording mapping proves scoped path semantics, no shell inference from
  filesystem access, process placement when process/shell semantics are visible,
  and network posture when broker/allowlist/direct semantics matter.
- Git/worktree mapping proves no clone/worktree posture is emitted by the
  runtime context by default; future git-context tests must prove all git
  prerequisites before adding posture text.
- Empty visible surface mapping emits an explicit `EmptySurface` runtime posture
  and no invented affordances.
- Safe date/timezone rendering covers available-source and unavailable-source
  cases without injecting current clock time.

### Caller-level integration tests

- `HostManagedLoopPromptPort` builds a prompt with runtime context using the
  final filtered surface.
- `HostManagedLoopPromptPort` passes the current `PromptMode` and the filtered
  `VisibleCapabilitySurface` into runtime posture derivation.
- Existing ordering tests are updated rather than bypassed:
  `instruction_bundle_builder_orders_sections_and_rebuilds_deterministically`
  and
  `loop_prompt_port_keeps_identity_before_skill_snippets_and_records_skill_metadata`
  both cover the new identity -> runtime -> skills order.
- Reborn model gateway receives the runtime section in the actual model request.
- `tests::reborn::model_gateway::model_request_includes_runtime_context_via_host_prompt_bundle`
  exercises the full
  `ThreadBackedLoopModelGateway -> HostManagedLoopPromptPort -> InstructionBundleBuilder`
  path and proves runtime-section ordering after identity.
- Subagent prompt path receives the flavor-narrowed runtime section.
- `tests::subagent_prompt_port::subagent_runtime_context_is_narrowed_to_child_surface`
  proves parent-surface posture facts do not leak into child runtime context and
  nested-spawn hints are absent when `allow_nesting=false`.
- Subagent prompt tests prove child flavor, parent-boundary, and nesting posture
  are explicit runtime-source inputs, not derived from tool allowlist shape.
- Stale surface or mismatched run scope fails before model request construction.
- Non-shell subagent prompts never include shell, `ls`, git clone, or git
  worktree guidance.
- A `researcher` runtime section can mention brokered/allowlisted network
  posture when relevant, while exact HTTP tool availability remains in the
  capability surface.
- A `coder` runtime section can mention process placement when process/shell
  semantics are visible, while exact shell/write/patch tool availability remains
  in the capability surface.
- Built-in subagent runtime sections mention child role/boundary only; exact
  nested-spawn availability is asserted by capability-surface tests.

### Security/regression tests

- Hidden/denied host-registry capability is not named.
- Full tool schema is not emitted.
- Raw host path is not emitted in hosted/sandboxed contexts.
- Physical cwd is not emitted by default when scoped aliases are available.
- Env vars and secrets are not emitted.
- Raw model route strings, provider endpoints, deployment IDs, provider account
  IDs, and free-form provider/model labels are not emitted; only policy-approved
  canonical labels may appear.
- Canonical tenant/user/project storage paths are not emitted by default.
- Runtime context does not grant invocation authority; denied capability calls
  still fail through `CapabilitySurfaceVisibleFilter`.
- Runtime context never describes HTTP as equivalent to shell network access.
- Runtime context never describes `/project` as default unless a current
  runtime disclosure/mount proves it exists.
- Runtime context never includes unescaped user-controlled labels or descriptor
  text in the system message.

### Suggested commands

```bash
cargo test -p ironclaw_turns instruction_bundle
cargo test -p ironclaw_turns host_managed_prompt_port
cargo test -p ironclaw_loop_support capability_surface_filter
cargo test -p ironclaw_loop_support subagent
cargo test -p ironclaw_reborn runtime_context
cargo test -p ironclaw_reborn model_gateway
cargo test -p ironclaw_architecture reborn_dependency_boundaries
scripts/check-boundaries.sh
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Risks

- **Authority confusion:** The model may treat runtime context as permission.
  Mitigation: phrase the section as affordance explanation and keep
  `CapabilitySurfaceVisibleFilter` as enforcement.
- **Surface drift:** Runtime context generated from a stale or broader surface
  could disclose hidden capabilities. Mitigation: bind generation to current
  surface version and `LoopModelCapabilityView`.
- **Path leakage:** Local development wants useful cwd/workspace information,
  while hosted contexts must not reveal host paths. Mitigation: render scoped
  aliases by default; only disclose raw local paths behind explicit
  local-single-user policy.
- **Prompt injection through runtime labels:** Aliases, descriptor text, repo
  names, branch names, product/channel labels, and provider/model labels could
  contain prompt-control text if copied directly into a system message.
  Mitigation: render only fixed enum strings or sanitized bounded identifiers,
  and replace unknown free-form labels with neutral placeholders.
- **Identity contamination:** Runtime facts could be added to `SYSTEM.md`.
  Mitigation: add a separate runtime stage and tests that identity content stays
  static.
- **Subagent over-disclosure:** Child prompt could inherit parent affordances.
  Mitigation: derive from the child run's final visible surface and add
  flavor-specific caller tests.
- **Prompt bloat:** Repeating surface and runtime explanations can add tokens.
  Mitigation: runtime context summarizes categories and aliases, while visible
  surface remains the place for safe capability names/descriptions.

## Open Decisions

- Whether a later local-single-user profile should disclose physical cwd as an
  explicitly trusted raw alias. First slice prefers scoped aliases.
- Whether policy should always disclose model/backend identity locally or allow
  every deployment mode to hide it. First slice makes this policy controlled.
- Whether git branch/status should become a separate opt-in `Git Context`
  source. First slice keeps git out unless represented as a scoped,
  capability-backed affordance.
- Whether later provider-specific behavior should split runtime context into
  multiple typed messages. First slice uses one concise runtime system message.

## Success Criteria

- Final Reborn model requests include a distinct runtime-context section.
- The runtime section is derived from `LoopModelCapabilityView` and the final
  filtered `VisibleCapabilitySurface`, not from the unfiltered host registry.
- Normal model-call prompt modes always receive a runtime section, even when the
  final visible surface is empty; `EmptySurface` posture must not invent
  affordances.
- The initial runtime section includes scoped workspace aliases actually granted
  when path semantics are relevant, safe date/timezone when available, and
  policy-gated model/backend identity.
- Runtime context materializes after identity and before skills/safety/visible
  surface/thread context.
- Static `default-system.md` identity remains unchanged and dynamic runtime
  facts are not injected into identity.
- Runtime context names usable affordances and postures, not raw implementation
  internals.
- Hidden capabilities, raw host paths, env vars, secrets, canonical tenant
  targets, and full schemas are absent by default.
- Runtime labels and descriptors are sanitized or replaced before entering the
  system message; user-controlled free-form text is never copied verbatim.
- Current clock time and git status are absent by default unless explicitly
  enabled by a later policy/capability-backed context source.
- `ls`, git clone, and git worktree are not advertised as default affordances;
  they appear only under their capability and policy prerequisites.
- Subagent runtime context is narrowed for `general`, `researcher`, `explorer`,
  and `coder` flavors.
- Capability-to-affordance wording is derived from one canonical descriptor
  table/helper, not repeated ad hoc across prompt text, docs, and tests.
- Caller-level tests prove the final model request receives the section through
  the real prompt port/model gateway path.
- Product adapters do not construct or mutate runtime context and cannot bypass
  the host-owned prompt/runtime context path.
- Architecture boundary checks still pass.
- Each implementation PR in the sequence stays under 1000 non-generated code
  lines changed, or is split before review.

## PLANNING COMPLETE
