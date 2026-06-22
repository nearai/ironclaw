---
paths:
  - "crates/ironclaw_reborn_composition/**/*.rs"
  - "crates/ironclaw_loop_support/**/*.rs"
  - "crates/ironclaw_agent_loop/**/*.rs"
  - "crates/ironclaw_reborn/**/*.rs"
---
# Agent-Loop Capability Handlers — Don't Kill the Whole Run

This rule exists because the same terminal-failure shipped **three times
in one sitting** while building local-dev synthetic capabilities
(`skill_activate`, `project_create`). Each looked different at the
source but produced the identical run-ending log signature:

```
WARN ironclaw_agent_loop::executor::mapping: capability host error mapped to HostUnavailable kind="…" safe_summary="…"
WARN ironclaw_reborn::planned_driver: planned driver executor returned sanitized error error=HostUnavailable { stage: Capability }
WARN ironclaw_reborn::turn_runner: driver invocation failed, recording terminal failure … error=driver error: agent loop driver is unavailable: Capability: unavailable
```

**If you see `Capability: unavailable` killing a turn, a capability
handler returned an `Err(AgentLoopHostError)` it should not have, or
emitted an unsafe safe-summary.** Start here.

## The two failure paths are not interchangeable

A `LoopCapabilityPort` / synthetic-capability handler's `invoke` returns
`Result<CapabilityOutcome, AgentLoopHostError>`. Those are **two
different audiences**:

| Return | What happens | Use for |
|---|---|---|
| `Ok(CapabilityOutcome::Failed(..))` / `Ok(CapabilityOutcome::Denied(..))` | Model-visible tool error; **run continues**, model can retry/adjust (`handle_capability_error`, `crates/ironclaw_agent_loop/src/executor/capabilities.rs`) | Anything the model or user can fix |
| `Err(AgentLoopHostError)` | `capability_host_error` (`crates/ironclaw_agent_loop/src/executor/mapping.rs`) maps **every** non-`Cancelled` kind to a terminal `HostUnavailable { stage: Capability }` — the whole turn run dies | Genuine host/infra faults only |

### Invariant 1 — `Err` is terminal; reserve it for host/infra faults

Map your backend/selection/validation errors deliberately. A failure
the model can recover from by changing its request is **`Failed`**, not
`Err`:

- budget/quota exceeded, "too many/too-large" → `CapabilityFailure { error_kind: InvalidInput | Resource, .. }`
- invalid/ambiguous arguments the model chose → `InvalidInput`
- not-permitted → `CapabilityOutcome::Denied` (or `PolicyDenied`)
- transient backend unavailability → `Failed { Unavailable }` (surface it, let the model tell the user) — only escalate to `Err` if the run genuinely cannot proceed
- **only** a true internal bug (poisoned state, unreachable invariant) → `Err(AgentLoopHostError::new(Internal, ..))`

Pattern: don't `?` / `map_err(into_host_error)` a whole error enum
straight into `Err`. Match it and route the recoverable arms to
`Ok(CapabilityOutcome::Failed(..))`. See
`skill_activation_selection_outcome` and `project_service_outcome` in
`crates/ironclaw_reborn_composition/src/runtime/local_dev/` for the
shape.

**Review flag:** a handler doing `.map_err(<to AgentLoopHostError>)?`
or `return Err(..)` on a backend/selector/validator result without a
comment justifying why that specific failure is *unrecoverable* (and
therefore worth ending the run).

### Invariant 2 — safe summaries are host-authored text, never interpolated untrusted data

`CapabilityResultMessage.safe_summary` (and any host safe-summary) is
validated before the result ref is written —
`append_capability_result_ref` (`crates/ironclaw_loop_support/src/lib.rs`)
→ `validate_loop_safe_summary`
(`crates/ironclaw_turns/src/run_profile/host.rs`) + `ToolResultSafeSummary`
(`crates/ironclaw_threads/src/tool_result_reference.rs`). Validation
**rejects** the delimiters ``{ } [ ] ` < > / \`` (see
`RAW_PAYLOAD_OR_PATH_DELIMITERS`), control chars, and secret markers
(`password`, `api key`, `bearer `, …). A rejection there is itself an
`InvalidInvocation` `Err` → terminal `HostUnavailable` (Invariant 1).

So **never interpolate model/user-controlled text** (names, paths, tool
args, file contents, provider strings) into a safe summary — any of
those can legally contain a delimiter or a marker word and will kill the
run. The actual data belongs in the result `output` (the model sees it
there); the summary stays a fixed, host-authored string.

```rust
// BAD — a project named "a/b <c>" or "passwords" ends the turn:
safe_summary: format!("created project \"{}\"", project.name),
// GOOD — fixed text; name/id travel in `output`:
safe_summary: "created project".to_string(),
```

A bounded *count* or a charset-guaranteed-safe id (e.g. a ULID) is fine;
free-form text is not.

**Review flag:** `safe_summary: format!(..)` (or any builder) whose
arguments include a value sourced from tool input, a DB record's
user-facing field, a path, or a provider response.

## Testing these (ties into testing.md "test through the caller")

A unit test that only checks `parse_*` or a small mapper does **not**
cover either invariant — both fire at the executor/transcript boundary,
past the helper:

- For Invariant 1: assert the recoverable arms return
  `CapabilityOutcome::Failed`/`Denied`, and that only the internal arm
  returns `Err`.
- For Invariant 2: a port-level test that invokes the capability gets
  `Completed(message)` **without** running the summary validator (it
  fires later, in `append_capability_result_ref`). Either drive the full
  executor, or re-run the real validator on the returned summary —
  `LoopSafeSummary::new(message.safe_summary).expect(..)` — using an
  input (e.g. a name with `/ < >`) that would have tripped the old bug.

## References

- Incidents (this codebase): `skill_activate` budget overflow →
  terminal; `project_create` safe-summary interpolating the raw project
  name → terminal.
- Terminal mapper: `crates/ironclaw_agent_loop/src/executor/mapping.rs`
  (`capability_host_error`).
- Recoverable handler: `crates/ironclaw_agent_loop/src/executor/capabilities.rs`
  (`handle_capability_error`).
- Safe-summary validation: `crates/ironclaw_loop_support/src/lib.rs`
  (`append_capability_result_ref`),
  `crates/ironclaw_threads/src/tool_result_reference.rs`.
- Exemplar handlers: `crates/ironclaw_reborn_composition/src/runtime/local_dev/{skill_activation,project_create,outbound_delivery}.rs`.
